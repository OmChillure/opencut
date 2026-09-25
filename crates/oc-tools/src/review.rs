//! Read a cut the way a picture editor would, before accepting it.

use oc_timeline::{Clip, ClipKind, MediaId, Timeline, TrackKind};
#[cfg(test)]
use oc_time::{Duration, Time};

#[derive(Clone, Debug)]
pub struct Spoken {
    pub media: MediaId,
    pub start: f64,
    pub end: f64,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CutReview {
    pub text: String,
    pub issues: bool,
}

/// Facts about the timeline plus problems the director should fix.
/// `request` is the user's ask, used only to read a target length.
pub fn review_cut(timeline: &Timeline, speech: &[Spoken], request: &str) -> CutReview {
    let mut lines = Vec::new();
    let mut issues = Vec::new();
    let videos = video_clips(timeline);
    let duration = timeline.duration().as_seconds();
    lines.push(format!("duration {duration:.1}s"));
    if videos.is_empty() {
        issues.push("timeline has no picture".into());
    }
    if let Some((lo, hi)) = target_range(request) {
        if duration + 0.4 < lo || duration > hi + 0.8 {
            issues.push(format!(
                "length {duration:.1}s is outside the asked {lo:.0}–{hi:.0}s"
            ));
        }
    }
    if let Some(first) = videos.iter().map(|c| c.start.as_seconds()).min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)) {
        if first > 0.35 {
            issues.push(format!("picture starts at {first:.1}s, not at 0"));
        }
    }
    if let Some(words_at) = first_speech_time(&videos, speech) {
        lines.push(format!("first words at {words_at:.1}s"));
        if wants_hook(request) && !wants_slow_open(request) && words_at > 3.5 {
            issues.push(format!("hook is late: first words at {words_at:.1}s"));
        }
    }
    for note in leftover_speech(&videos, speech, request) {
        issues.push(note);
    }
    for hole in holes(&videos) {
        issues.push(hole);
    }
    for jump in jump_cuts(&videos) {
        issues.push(jump);
    }
    for stack in stacked_talk(&videos, speech) {
        issues.push(stack);
    }
    let mut text = String::from("Cut review:\n");
    for line in &lines {
        text.push_str(line);
        text.push('\n');
    }
    if issues.is_empty() {
        text.push_str("no structural problems\n");
    } else {
        for issue in &issues {
            text.push_str("fix: ");
            text.push_str(issue);
            text.push('\n');
        }
    }
    CutReview {
        text,
        issues: !issues.is_empty(),
    }
}

fn video_clips(timeline: &Timeline) -> Vec<&Clip> {
    let mut clips = Vec::new();
    for track in &timeline.tracks {
        if track.kind != TrackKind::Video || track.hidden {
            continue;
        }
        for clip in &track.clips {
            if clip.disabled {
                continue;
            }
            if matches!(clip.kind, ClipKind::Video { .. }) {
                clips.push(clip);
            }
        }
    }
    clips.sort_by(|a, b| a.start.cmp(&b.start));
    clips
}

fn first_speech_time(clips: &[&Clip], speech: &[Spoken]) -> Option<f64> {
    let mut best: Option<f64> = None;
    for clip in clips {
        let Some(media) = clip.media_id else { continue };
        let speed = if clip.speed.is_finite() && clip.speed > 0.0 { clip.speed } else { 1.0 };
        for cue in speech.iter().filter(|s| s.media == media) {
            let src_in = clip.source_in.as_seconds();
            let src_out = src_in + clip.duration.as_seconds() * f64::from(speed);
            let overlap_start = cue.start.max(src_in);
            let overlap_end = cue.end.min(src_out);
            if overlap_end - overlap_start < 0.15 {
                continue;
            }
            let timeline_at = clip.start.as_seconds() + (overlap_start - src_in) / f64::from(speed);
            best = Some(best.map_or(timeline_at, |b| b.min(timeline_at)));
        }
    }
    best
}

fn holes(clips: &[&Clip]) -> Vec<String> {
    let mut notes = Vec::new();
    if clips.is_empty() {
        return notes;
    }
    let mut covered: Vec<(f64, f64)> = clips
        .iter()
        .map(|c| (c.start.as_seconds(), c.end().as_seconds()))
        .collect();
    covered.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut cursor = 0.0;
    for (start, end) in covered {
        if start - cursor > 0.4 {
            notes.push(format!("gap {cursor:.1}–{start:.1}s with no picture"));
        }
        cursor = cursor.max(end);
    }
    notes
}

fn jump_cuts(clips: &[&Clip]) -> Vec<String> {
    let mut by_track: Vec<&Clip> = clips.to_vec();
    by_track.sort_by(|a, b| a.start.cmp(&b.start));
    let mut notes = Vec::new();
    for pair in by_track.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let Some(ma) = a.media_id else { continue };
        let Some(mb) = b.media_id else { continue };
        if ma != mb {
            continue;
        }
        let join = (a.end().as_seconds() - b.start.as_seconds()).abs();
        if join > 0.35 {
            continue;
        }
        let gap = b.source_in.as_seconds() - a.source_out().as_seconds();
        if !(0.15..2.5).contains(&gap) {
            continue;
        }
        let at = b.start.as_seconds();
        let covered = clips.iter().any(|c| {
            c.id != a.id
                && c.id != b.id
                && c.start.as_seconds() <= at
                && c.end().as_seconds() >= at + 0.1
        });
        if !covered {
            notes.push(format!(
                "jump cut at {at:.1}s, {gap:.1}s removed from the same shot with nothing covering it"
            ));
        }
    }
    notes
}

fn stacked_talk(clips: &[&Clip], speech: &[Spoken]) -> Vec<String> {
    let mut notes = Vec::new();
    for i in 0..clips.len() {
        for j in (i + 1)..clips.len() {
            let a = clips[i];
            let b = clips[j];
            let start = a.start.as_seconds().max(b.start.as_seconds());
            let end = a.end().as_seconds().min(b.end().as_seconds());
            if end - start < 0.4 {
                continue;
            }
            if clip_has_speech(a, speech, start, end) && clip_has_speech(b, speech, start, end) {
                notes.push(format!(
                    "two talking shots stacked {start:.1}–{end:.1}s"
                ));
            }
        }
    }
    notes
}

fn clip_has_speech(clip: &Clip, speech: &[Spoken], tl0: f64, tl1: f64) -> bool {
    let Some(media) = clip.media_id else { return false };
    let speed = if clip.speed.is_finite() && clip.speed > 0.0 { clip.speed } else { 1.0 };
    let local0 = clip.source_in.as_seconds() + (tl0 - clip.start.as_seconds()).max(0.0) * f64::from(speed);
    let local1 = clip.source_in.as_seconds() + (tl1 - clip.start.as_seconds()).max(0.0) * f64::from(speed);
    speech.iter().any(|s| {
        s.media == media && s.end > local0 + 0.05 && s.start < local1 - 0.05
    })
}

fn wants_hook(request: &str) -> bool {
    let t = request.to_ascii_lowercase();
    t.contains("hook") || t.contains("reel") || t.contains("short") || t.contains("tiktok")
}

fn wants_slow_open(request: &str) -> bool {
    let t = request.to_ascii_lowercase();
    t.contains("slow")
        || t.contains("silent")
        || t.contains("no hook")
        || t.contains("music open")
        || t.contains("product")
}

fn leftover_speech(clips: &[&Clip], speech: &[Spoken], request: &str) -> Vec<String> {
    let mut notes = Vec::new();
    let keep_silence = {
        let t = request.to_ascii_lowercase();
        t.contains("silence") || t.contains("b-roll") || t.contains("broll") || t.contains("music")
    };
    for clip in clips {
        let Some(media) = clip.media_id else { continue };
        let src_in = clip.source_in.as_seconds();
        let src_out = clip.source_out().as_seconds();
        let inside: Vec<&Spoken> = speech
            .iter()
            .filter(|s| s.media == media && s.end > src_in + 0.05 && s.start < src_out - 0.05)
            .collect();
        if inside.is_empty() {
            if !keep_silence && clip.duration.as_seconds() > 1.5 {
                notes.push(format!(
                    "silence kept {:.1}–{:.1}s",
                    clip.start.as_seconds(),
                    clip.end().as_seconds()
                ));
            }
            continue;
        }
        let filler_time: f64 = inside
            .iter()
            .filter(|s| line_is_filler(&s.text))
            .map(|s| {
                let a = s.start.max(src_in);
                let b = s.end.min(src_out);
                (b - a).max(0.0)
            })
            .sum();
        if filler_time > 0.45 {
            notes.push(format!(
                "filler kept {:.1}–{:.1}s",
                clip.start.as_seconds(),
                clip.end().as_seconds()
            ));
        }
    }
    notes
}

fn line_is_filler(text: &str) -> bool {
    let t = text.trim().to_ascii_lowercase();
    if t.is_empty() {
        return false;
    }
    t.split_whitespace().all(|w| {
        matches!(
            w.trim_matches(|c: char| !c.is_alphanumeric()),
            "um" | "uh" | "uhm" | "hmm" | "mm" | "yeah" | "yep" | "ok" | "okay" | "so" | "like"
        )
    })
}

fn target_range(request: &str) -> Option<(f64, f64)> {
    let lower = request.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        let n: f64 = lower[start..i].parse().ok()?;
        let rest = lower[i..].trim_start();
        if let Some(after) = rest.strip_prefix('-').or_else(|| rest.strip_prefix('–')).or_else(|| rest.strip_prefix("to ")) {
            let after = after.trim_start();
            let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(hi) = digits.parse::<f64>() {
                let (a, b) = scale_pair(&lower[i + 1..], n, hi);
                return Some((a.min(b), a.max(b)));
            }
        }
        if rest.starts_with("min") {
            return Some((n * 60.0 - 5.0, n * 60.0 + 8.0));
        }
        if rest.starts_with('s') || rest.starts_with("sec") {
            return Some((n - 4.0, n + 6.0));
        }
    }
    None
}

fn scale_pair(after_sep: &str, a: f64, b: f64) -> (f64, f64) {
    let trimmed = after_sep.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c.is_whitespace());
    if trimmed.starts_with("min") {
        (a * 60.0, b * 60.0)
    } else {
        (a, b)
    }
}

#[cfg(test)]
fn video_clip(media: MediaId, start: f64, dur: f64, source_in: f64) -> Clip {
    use oc_timeline::{ClipId, ClipLook, Transform};
    Clip {
        id: ClipId::new(),
        media_id: Some(media),
        kind: ClipKind::Video {
            transform: Transform::default(),
        },
        start: Time::from_seconds(start),
        duration: Duration::from_seconds(dur),
        source_in: Time::from_seconds(source_in),
        speed: 1.0,
        group_id: None,
        link_id: None,
        disabled: false,
        look: ClipLook::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oc_timeline::{Timeline, Track};

    fn tl_with(clips: Vec<Clip>) -> Timeline {
        let mut track = Track::new(TrackKind::Video, "V1");
        track.clips = clips;
        let mut tl = Timeline::default();
        tl.tracks.push(track);
        tl
    }

    #[test]
    fn flags_late_hook_long_cut_and_jump() {
        let media = MediaId::new();
        let tl = tl_with(vec![
            video_clip(media, 0.0, 4.0, 10.0),
            video_clip(media, 4.0, 4.0, 14.4),
        ]);
        let speech = vec![Spoken {
            media,
            start: 20.0,
            end: 28.0,
            text: "late line".into(),
        }];
        let review = review_cut(&tl, &speech, "make a 30s reel");
        assert!(review.issues, "{}", review.text);
        assert!(review.text.contains("length"), "{}", review.text);
        assert!(review.text.contains("jump cut"), "{}", review.text);
    }

    #[test]
    fn flags_stacked_talk() {
        let a = MediaId::new();
        let b = MediaId::new();
        let mut tl = Timeline::default();
        let mut v1 = Track::new(TrackKind::Video, "V1");
        let mut v2 = Track::new(TrackKind::Video, "V2");
        v1.clips.push(video_clip(a, 0.0, 6.0, 0.0));
        v2.clips.push(video_clip(b, 1.0, 4.0, 0.0));
        tl.tracks.push(v1);
        tl.tracks.push(v2);
        let speech = vec![
            Spoken { media: a, start: 0.0, end: 6.0, text: "hello there friend".into() },
            Spoken { media: b, start: 0.0, end: 4.0, text: "other person talks".into() },
        ];
        let review = review_cut(&tl, &speech, "cut this");
        assert!(review.text.contains("stacked"), "{}", review.text);
    }

    #[test]
    fn filler_and_silence_are_sent_back() {
        let media = MediaId::new();
        let tl = tl_with(vec![
            video_clip(media, 0.0, 3.0, 0.0),
            video_clip(media, 3.0, 3.0, 40.0),
        ]);
        let speech = vec![Spoken {
            media,
            start: 0.0,
            end: 3.0,
            text: "um uh like".into(),
        }];
        let review = review_cut(&tl, &speech, "cut this");
        assert!(review.text.contains("filler kept"), "{}", review.text);
        assert!(review.text.contains("silence kept"), "{}", review.text);
    }

    #[test]
    fn slow_open_is_allowed() {
        let media = MediaId::new();
        let tl = tl_with(vec![video_clip(media, 0.0, 8.0, 0.0)]);
        let speech = vec![Spoken {
            media,
            start: 6.0,
            end: 8.0,
            text: "the line starts late".into(),
        }];
        let review = review_cut(&tl, &speech, "slow open, then the line");
        assert!(!review.text.contains("hook is late"), "{}", review.text);
    }

    #[test]
    fn clean_short_cut_has_no_fix_lines() {
        let media = MediaId::new();
        let tl = tl_with(vec![video_clip(media, 0.0, 8.0, 2.0)]);
        let speech = vec![Spoken {
            media,
            start: 2.2,
            end: 6.0,
            text: "the hook line is here".into(),
        }];
        let review = review_cut(&tl, &speech, "trim the open");
        assert!(!review.issues, "{}", review.text);
    }
}
