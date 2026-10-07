//! Read a cut the way a picture editor would, before accepting it.

use crate::asks_for_whole_piece;
#[cfg(test)]
use oc_time::{Duration, Time};
use oc_timeline::{Clip, ClipKind, MediaId, Timeline, TrackKind};

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
    /// Taste notes. They block "done" only in director mode, and only for two rounds.
    pub notes: bool,
}

#[derive(Clone, Debug, Default)]
pub struct ReviewFacts {
    pub beats: Vec<f64>,
    pub has_music: bool,
    /// Target average shot length from the style guide, when one is loaded.
    pub target_shot: Option<f64>,
    /// Shot scale in timeline order, when the vision card exists.
    pub scales: Vec<String>,
    pub qualities: Vec<u8>,
    pub motion_dirs: Vec<String>,
    /// Imported file length. Used when the ask is the whole source and there is little speech.
    pub sources: Vec<SourceSpan>,
}

/// One imported file and its length in seconds.
#[derive(Clone, Debug)]
pub struct SourceSpan {
    pub media: MediaId,
    pub duration: f64,
}

/// Picture label for one source range, already matched to a media id.
#[derive(Clone, Debug)]
pub struct ShotNote {
    pub media: MediaId,
    pub start: f64,
    pub end: f64,
    pub scale: String,
    pub quality: u8,
    pub motion_dir: String,
}

impl ReviewFacts {
    /// Scales, quality, and motion in the same order as the picture clips.
    #[must_use]
    pub fn from_timeline(
        timeline: &Timeline,
        shots: &[ShotNote],
        sources: Vec<SourceSpan>,
        beats: Vec<f64>,
        has_music: bool,
    ) -> Self {
        let mut scales = Vec::new();
        let mut qualities = Vec::new();
        let mut motion_dirs = Vec::new();
        for clip in video_clips(timeline) {
            let speed = if clip.speed.is_finite() && clip.speed > 0.0 {
                clip.speed
            } else {
                1.0
            };
            let src_in = clip.source_in.as_seconds();
            let src_out = src_in + clip.duration.as_seconds() * f64::from(speed);
            let mid = (src_in + src_out) * 0.5;
            let note = clip.media_id.and_then(|media| {
                shots
                    .iter()
                    .find(|s| s.media == media && mid >= s.start - 0.05 && mid < s.end + 0.05)
            });
            scales.push(note.map(|s| s.scale.clone()).unwrap_or_default());
            qualities.push(note.map(|s| s.quality).unwrap_or(0));
            motion_dirs.push(note.map(|s| s.motion_dir.clone()).unwrap_or_default());
        }
        Self {
            beats,
            has_music,
            target_shot: None,
            scales,
            qualities,
            motion_dirs,
            sources,
        }
    }
}

/// Facts about the timeline plus problems the director should fix.
/// `request` is the user's ask, used only to read a target length.
pub fn review_cut(timeline: &Timeline, speech: &[Spoken], request: &str) -> CutReview {
    review_with(timeline, speech, request, &ReviewFacts::default())
}

pub fn review_with(
    timeline: &Timeline,
    speech: &[Spoken],
    request: &str,
    facts: &ReviewFacts,
) -> CutReview {
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
    if let Some(first) = videos
        .iter()
        .map(|c| c.start.as_seconds())
        .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
    {
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
    for note in uncovered_source(&videos, speech, request, facts) {
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
    for jump in grade_jumps(&videos) {
        issues.push(jump);
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
    let notes = taste_notes(timeline, &videos, facts);
    for note in &notes {
        text.push_str("note: ");
        text.push_str(note);
        text.push('\n');
    }
    CutReview {
        text,
        issues: !issues.is_empty(),
        notes: !notes.is_empty(),
    }
}

fn taste_notes(timeline: &Timeline, videos: &[&Clip], facts: &ReviewFacts) -> Vec<String> {
    let mut notes = Vec::new();
    if facts.scales.len() >= 3 {
        let mut run = 1;
        for pair in facts.scales.windows(2) {
            if !pair[0].is_empty() && pair[0] == pair[1] {
                run += 1;
                if run >= 3 {
                    notes.push(format!(
                        "same shot size {} times in a row ({})",
                        run, pair[0]
                    ));
                    break;
                }
            } else {
                run = 1;
            }
        }
    }
    let transitions: Vec<_> = videos
        .iter()
        .filter(|c| c.look.transition != oc_timeline::TransitionKind::Cut)
        .map(|c| c.look.transition.label())
        .collect();
    if transitions.len() >= 3 && transitions.windows(2).all(|w| w[0] == w[1]) {
        notes.push(format!(
            "the same transition ({}) is on every join",
            transitions[0]
        ));
    }
    if !facts.beats.is_empty() && videos.len() >= 2 {
        let tol = 1.0 / timeline.frame_rate.as_f64().max(1.0);
        let mut on = 0;
        for clip in videos.iter().skip(1) {
            let at = clip.start.as_seconds();
            if facts.beats.iter().any(|b| (b - at).abs() <= tol * 2.0) {
                on += 1;
            }
        }
        let pct = on as f64 / (videos.len() - 1) as f64;
        if pct < 0.70 {
            notes.push(format!("only {:.0}% of cuts land on a beat", pct * 100.0));
        }
    }
    if let Some(target) = facts.target_shot {
        if !videos.is_empty() {
            let avg =
                videos.iter().map(|c| c.duration.as_seconds()).sum::<f64>() / videos.len() as f64;
            if (avg - target).abs() > target * 0.4 {
                notes.push(format!(
                    "average shot {avg:.1}s vs the style target {target:.1}s"
                ));
            }
        }
    }
    if facts.qualities.iter().any(|q| *q > 0 && *q < 5) {
        notes.push("a shot scored under 5 is in the cut".into());
    }
    let dirs: Vec<_> = facts
        .motion_dirs
        .iter()
        .filter(|d| !d.is_empty() && *d != "none")
        .cloned()
        .collect();
    for pair in dirs.windows(2) {
        if reversed(&pair[0], &pair[1]) {
            notes.push(format!(
                "motion direction reverses across a cut ({} then {})",
                pair[0], pair[1]
            ));
            break;
        }
    }
    if !videos.is_empty() && videos.iter().all(|c| c.look.grade.is_identity()) {
        notes.push("no grade applied".into());
    }
    if facts.has_music {
        let ducked = timeline.tracks.iter().any(|t| {
            t.kind == TrackKind::Audio
                && t.clips.iter().any(|c| match &c.kind {
                    ClipKind::Audio { ducked, .. } => *ducked,
                    _ => false,
                })
        });
        if !ducked {
            notes.push("music is not ducked under speech".into());
        }
    }
    notes
}

fn reversed(a: &str, b: &str) -> bool {
    matches!(
        (a, b),
        ("l2r", "r2l") | ("r2l", "l2r") | ("toward", "away") | ("away", "toward")
    )
}

fn grade_jumps(videos: &[&Clip]) -> Vec<String> {
    let mut ordered: Vec<&Clip> = videos.to_vec();
    ordered.sort_by(|a, b| a.start.cmp(&b.start));
    let mut notes = Vec::new();
    for pair in ordered.windows(2) {
        let gap = pair[1].start.as_seconds() - pair[0].end().as_seconds();
        if gap > 0.25 || gap < -0.35 {
            continue;
        }
        let left = &pair[0].look.grade;
        let right = &pair[1].look.grade;
        let delta = (left.exposure - right.exposure)
            .abs()
            .max((left.temperature - right.temperature).abs())
            .max((left.contrast - right.contrast).abs())
            .max((left.saturation - right.saturation).abs());
        if !delta.is_finite() || delta < 0.22 {
            continue;
        }
        notes.push(format!(
            "grade jumps at {:.1}s: exposure {:.2} then {:.2}, temperature {:.2} then {:.2}",
            pair[1].start.as_seconds(),
            left.exposure,
            right.exposure,
            left.temperature,
            right.temperature
        ));
    }
    notes
}

fn video_clips(timeline: &Timeline) -> Vec<&Clip> {
    let mut clips = Vec::new();
    for track in &timeline.tracks {
        if track.kind != TrackKind::Video
            || track.hidden
            || track.name == "Design"
            || track.name == "Front"
        {
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
        let speed = if clip.speed.is_finite() && clip.speed > 0.0 {
            clip.speed
        } else {
            1.0
        };
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
                notes.push(format!("two talking shots stacked {start:.1}–{end:.1}s"));
            }
        }
    }
    notes
}

fn clip_has_speech(clip: &Clip, speech: &[Spoken], tl0: f64, tl1: f64) -> bool {
    let Some(media) = clip.media_id else {
        return false;
    };
    let speed = if clip.speed.is_finite() && clip.speed > 0.0 {
        clip.speed
    } else {
        1.0
    };
    let local0 =
        clip.source_in.as_seconds() + (tl0 - clip.start.as_seconds()).max(0.0) * f64::from(speed);
    let local1 =
        clip.source_in.as_seconds() + (tl1 - clip.start.as_seconds()).max(0.0) * f64::from(speed);
    speech
        .iter()
        .any(|s| s.media == media && s.end > local0 + 0.05 && s.start < local1 - 0.05)
}

fn wants_full_source(request: &str) -> bool {
    if target_range(request).is_some() {
        return false;
    }
    let t = request.to_ascii_lowercase();
    if t.contains("reel")
        || t.contains("tiktok")
        || t.contains("highlight")
        || t.contains("trailer")
        || t.contains("teaser")
        || (t.contains("short") && !t.contains("shortcut"))
    {
        return false;
    }
    asks_for_whole_piece(&t)
}

/// A whole-source ask that kept less than half the real speech, or half a silent file.
fn uncovered_source(
    clips: &[&Clip],
    speech: &[Spoken],
    request: &str,
    facts: &ReviewFacts,
) -> Vec<String> {
    if !wants_full_source(request) {
        return Vec::new();
    }
    let mut speech_total = 0.0;
    let mut speech_kept = 0.0;
    for line in speech {
        if line_is_filler(&line.text) {
            continue;
        }
        let dur = (line.end - line.start).max(0.0);
        if dur < 0.25 {
            continue;
        }
        speech_total += dur;
        if speech_line_kept(clips, line) {
            speech_kept += dur;
        }
    }
    if speech_total >= 12.0 {
        let pct = speech_kept / speech_total;
        if pct < 0.50 {
            return vec![format!(
                "only {:.0}% of the source speech is in the cut; keep the piece and drop ums, dead air, and retakes",
                pct * 100.0
            )];
        }
        return Vec::new();
    }
    let mut worst: Option<(f64, f64)> = None;
    for src in &facts.sources {
        if src.duration < 20.0 {
            continue;
        }
        let kept = covered_source_seconds(clips, src.media, src.duration);
        let pct = kept / src.duration;
        if worst.is_none_or(|(was, _)| pct < was) {
            worst = Some((pct, src.duration));
        }
    }
    if let Some((pct, duration)) = worst {
        if pct < 0.50 {
            return vec![format!(
                "only {:.0}% of the {duration:.0}s source is in the cut; this ask keeps the piece",
                pct * 100.0
            )];
        }
    }
    Vec::new()
}

fn speech_line_kept(clips: &[&Clip], line: &Spoken) -> bool {
    let need = ((line.end - line.start) * 0.4).clamp(0.2, 1.2);
    clips.iter().any(|clip| {
        let Some(media) = clip.media_id else {
            return false;
        };
        if media != line.media {
            return false;
        }
        let speed = if clip.speed.is_finite() && clip.speed > 0.0 {
            clip.speed
        } else {
            1.0
        };
        let src_in = clip.source_in.as_seconds();
        let src_out = src_in + clip.duration.as_seconds() * f64::from(speed);
        let overlap = line.end.min(src_out) - line.start.max(src_in);
        overlap >= need
    })
}

fn covered_source_seconds(clips: &[&Clip], media: MediaId, duration: f64) -> f64 {
    let mut spans: Vec<(f64, f64)> = clips
        .iter()
        .filter_map(|clip| {
            if clip.media_id != Some(media) {
                return None;
            }
            let speed = if clip.speed.is_finite() && clip.speed > 0.0 {
                clip.speed
            } else {
                1.0
            };
            let a = clip.source_in.as_seconds().clamp(0.0, duration);
            let b = (clip.source_in.as_seconds() + clip.duration.as_seconds() * f64::from(speed))
                .clamp(0.0, duration);
            if b - a < 0.05 { None } else { Some((a, b)) }
        })
        .collect();
    spans.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut total = 0.0;
    let mut cursor = 0.0;
    for (a, b) in spans {
        let start = a.max(cursor);
        if b > start {
            total += b - start;
            cursor = b;
        }
    }
    total
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
        t.contains("silence")
            || t.contains("b-roll")
            || t.contains("broll")
            || t.contains("music")
            || t.contains("cinematic")
            || t.contains("vlog")
            || t.contains("documentary")
            || t.contains("travel")
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
        if let Some(after) = rest
            .strip_prefix('-')
            .or_else(|| rest.strip_prefix('–'))
            .or_else(|| rest.strip_prefix("to "))
        {
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
    let trimmed =
        after_sep.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c.is_whitespace());
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
            Spoken {
                media: a,
                start: 0.0,
                end: 6.0,
                text: "hello there friend".into(),
            },
            Spoken {
                media: b,
                start: 0.0,
                end: 4.0,
                text: "other person talks".into(),
            },
        ];
        let review = review_cut(&tl, &speech, "cut this");
        assert!(review.text.contains("stacked"), "{}", review.text);
    }

    #[test]
    fn a_grade_jump_on_a_join_is_sent_back() {
        let media = MediaId::new();
        let mut joined = video_clip(media, 3.0, 3.0, 3.0);
        joined.look.grade.exposure = 0.4;
        let review = review_cut(
            &tl_with(vec![video_clip(media, 0.0, 3.0, 0.0), joined]),
            &[],
            "cut this",
        );
        assert!(review.text.contains("grade jumps"), "{}", review.text);
        let mut apart = video_clip(media, 8.0, 3.0, 3.0);
        apart.look.grade.exposure = 0.4;
        let separated = review_cut(
            &tl_with(vec![video_clip(media, 0.0, 3.0, 0.0), apart]),
            &[],
            "cut this",
        );
        assert!(
            !separated.text.contains("grade jumps"),
            "{}",
            separated.text
        );
        let mut mono = video_clip(media, 3.0, 3.0, 3.0);
        mono.look.grade.lut = oc_timeline::Lut::Mono;
        let look_change = review_cut(
            &tl_with(vec![video_clip(media, 0.0, 3.0, 0.0), mono]),
            &[],
            "cut this",
        );
        assert!(
            !look_change.text.contains("grade jumps"),
            "{}",
            look_change.text
        );
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

    #[test]
    fn each_taste_note_fires() {
        let media = MediaId::new();
        let mut clips = vec![
            video_clip(media, 0.0, 2.0, 0.0),
            video_clip(media, 2.0, 2.0, 2.0),
            video_clip(media, 4.0, 2.0, 4.0),
            video_clip(media, 6.0, 2.0, 6.0),
        ];
        for clip in &mut clips {
            clip.look.transition = oc_timeline::TransitionKind::Dissolve;
        }
        let tl = tl_with(clips);
        let facts = ReviewFacts {
            beats: vec![0.0, 10.0],
            has_music: true,
            target_shot: Some(8.0),
            scales: vec!["CU".into(), "CU".into(), "CU".into()],
            qualities: vec![3],
            motion_dirs: vec!["l2r".into(), "r2l".into()],
            sources: Vec::new(),
        };
        let review = review_with(&tl, &[], "make a short", &facts);
        for needle in [
            "same shot size",
            "same transition",
            "cuts land on a beat",
            "average shot",
            "scored under 5",
            "motion direction reverses",
            "no grade",
            "not ducked",
        ] {
            assert!(review.notes, "{}", review.text);
            assert!(
                review.text.contains(needle),
                "{needle} missing in {}",
                review.text
            );
        }
        assert!(
            !review.issues || review.text.contains("fix:"),
            "{}",
            review.text
        );
    }

    fn long_speech(media: MediaId) -> Vec<Spoken> {
        (0..8)
            .map(|i| {
                let start = i as f64 * 4.0;
                Spoken {
                    media,
                    start,
                    end: start + 3.0,
                    text: format!("this is line {i} about the day"),
                }
            })
            .collect()
    }

    #[test]
    fn whole_video_rejects_a_highlight_of_the_speech() {
        let media = MediaId::new();
        let tl = tl_with(vec![video_clip(media, 0.0, 3.0, 0.0)]);
        let review = review_cut(&tl, &long_speech(media), "edit this video");
        assert!(review.issues, "{}", review.text);
        assert!(review.text.contains("source speech"), "{}", review.text);
    }

    #[test]
    fn reel_can_leave_source_speech_out() {
        let media = MediaId::new();
        let tl = tl_with(vec![video_clip(media, 0.0, 3.0, 0.0)]);
        let review = review_cut(&tl, &long_speech(media), "make a 30s reel");
        assert!(!review.text.contains("source speech"), "{}", review.text);
    }

    #[test]
    fn whole_video_keeps_a_cut_that_holds_the_lines() {
        let media = MediaId::new();
        let tl = tl_with(vec![video_clip(media, 0.0, 32.0, 0.0)]);
        let review = review_cut(&tl, &long_speech(media), "cut the whole import");
        assert!(!review.text.contains("source speech"), "{}", review.text);
    }

    #[test]
    fn silent_whole_video_must_cover_the_source() {
        let media = MediaId::new();
        let tl = tl_with(vec![video_clip(media, 0.0, 8.0, 0.0)]);
        let facts = ReviewFacts {
            sources: vec![SourceSpan {
                media,
                duration: 100.0,
            }],
            ..ReviewFacts::default()
        };
        let review = review_with(&tl, &[], "edit this footage", &facts);
        assert!(review.issues, "{}", review.text);
        assert!(
            review.text.contains("source is in the cut"),
            "{}",
            review.text
        );
    }

    #[test]
    fn named_length_does_not_demand_the_whole_source() {
        let media = MediaId::new();
        let tl = tl_with(vec![video_clip(media, 0.0, 8.0, 0.0)]);
        let facts = ReviewFacts {
            sources: vec![SourceSpan {
                media,
                duration: 100.0,
            }],
            ..ReviewFacts::default()
        };
        let review = review_with(&tl, &[], "edit this footage in 30s", &facts);
        assert!(
            !review.text.contains("source is in the cut"),
            "{}",
            review.text
        );
    }

    #[test]
    fn shot_notes_land_on_the_clip_they_describe() {
        let media = MediaId::new();
        let tl = tl_with(vec![
            video_clip(media, 0.0, 2.0, 0.0),
            video_clip(media, 2.0, 2.0, 4.0),
            video_clip(media, 4.0, 2.0, 8.0),
        ]);
        let shots: Vec<ShotNote> = [(0.0, 3.0), (3.0, 7.0), (7.0, 12.0)]
            .into_iter()
            .map(|(start, end)| ShotNote {
                media,
                start,
                end,
                scale: "close".into(),
                quality: 8,
                motion_dir: "none".into(),
            })
            .collect();
        let facts = ReviewFacts::from_timeline(&tl, &shots, Vec::new(), Vec::new(), false);
        assert_eq!(facts.scales, ["close", "close", "close"]);
        let review = review_with(&tl, &[], "edit this video", &facts);
        assert!(review.text.contains("same shot size"), "{}", review.text);
        assert!(!review.text.contains("source speech"), "{}", review.text);
    }
}
