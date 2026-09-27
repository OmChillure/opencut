//! Picture finish for a reel: grade, vignette, punch-in, fades, captions, cover.
//!
//! Kdenlive puts Transform, Volume, and a mix on every clip. Shotcut's size/position
//! filter is the same zoom. This plans those ops once the cut is the right length.

use crate::ops::Op;
use oc_time::{Duration, Time};
use oc_timeline::{
    AspectRatio, CaptionCue, CaptionStyle, Clip, ClipKind, Grade, MediaId, Timeline, TrackKind,
    TransitionKind,
};

/// One spoken line in source time, used to lay captions on the cut.
#[derive(Clone, Debug)]
pub struct SpokenLine {
    pub media: MediaId,
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// A quiet or other-camera range that can cover a jump. Source times.
#[derive(Clone, Debug)]
pub struct CoverShot {
    pub media: MediaId,
    pub start: f64,
    pub end: f64,
}

/// True when the ask is a finished piece, not a single razor edit.
#[must_use]
pub fn wants_picture_finish(request: &str) -> bool {
    let t = request.to_ascii_lowercase();
    if t.contains("reel")
        || t.contains("short")
        || t.contains("tiktok")
        || t.contains("cinematic")
        || t.contains("vlog")
        || t.contains("interview")
        || t.contains("podcast")
        || t.contains("documentary")
        || t.contains("commercial")
        || t.contains("product")
    {
        return true;
    }
    let shaping = t.contains("make") || t.contains("cut") || t.contains("edit") || t.contains("turn");
    shaping && (t.contains("min") || t.contains(" sec") || t.contains("second"))
}

/// Grade already applied means the finish pass ran.
#[must_use]
pub fn already_finished(timeline: &Timeline) -> bool {
    program_clips(timeline).iter().any(|c| !c.look.grade.is_identity())
}

/// Ops that make a structural cut look finished. Empty when there is no picture.
#[must_use]
pub fn finish_reel(
    timeline: &Timeline,
    lines: &[SpokenLine],
    covers: &[CoverShot],
    request: &str,
) -> Vec<Op> {
    let clips = program_clips(timeline);
    if clips.is_empty() {
        return Vec::new();
    }
    let mut ops = Vec::new();
    let look = finish_look(request);
    if look.vertical && timeline.width >= timeline.height {
        ops.push(Op::Reframe {
            aspect: AspectRatio::Vertical,
        });
    }
    let last = clips.len() - 1;
    for (i, clip) in clips.iter().enumerate() {
        ops.push(Op::SetGrade {
            clip_id: clip.id,
            grade: look.grade,
        });
        ops.push(Op::SetFx {
            clip_id: clip.id,
            fx: look.fx,
        });
        let punch = look.punch && i > 0 && same_media(clips[i - 1], clip);
        let scale = if punch { look.punch_scale } else { 1.0 };
        ops.push(Op::SetTransform {
            clip_id: clip.id,
            x: 0.0,
            y: 0.0,
            scale,
            rotation: 0.0,
        });
        if look.moves && punch {
            ops.push(Op::SetMove {
                clip_id: clip.id,
                end_x: 0.0,
                end_y: 0.0,
                end_scale: look.punch_scale + 0.08,
                ease: oc_timeline::Ease::InOut,
            });
        }
        let long_enough = clip.duration.as_seconds() >= 1.2;
        let fade_in = if i == 0 && long_enough { 0.4 } else { 0.0 };
        let fade_out = if i == last && long_enough { 0.5 } else { 0.0 };
        if fade_in > 0.0 || fade_out > 0.0 {
            ops.push(Op::SetFade {
                clip_id: clip.id,
                fade_in: Duration::from_seconds(fade_in),
                fade_out: Duration::from_seconds(fade_out),
            });
        }
        if punch && clip.duration.as_seconds() >= 2.2 && clips[i - 1].duration.as_seconds() >= 2.2 {
            let join = (clips[i - 1].end().as_seconds() - clip.start.as_seconds()).abs();
            if join < 0.35 {
                ops.push(Op::SetTransition {
                    clip_id: clips[i - 1].id,
                    kind: TransitionKind::Dissolve,
                    duration: Some(0.4),
                });
            }
        }
    }
    for (i, clip) in clips.iter().enumerate().skip(1) {
        let prev = clips[i - 1];
        if !same_media(prev, clip) {
            continue;
        }
        let join = (prev.end().as_seconds() - clip.start.as_seconds()).abs();
        if join > 0.35 {
            continue;
        }
        let Some(cover) = pick_cover(covers, prev, clip) else {
            continue;
        };
        let dur = 0.9_f64.min(cover.end - cover.start);
        let at = (clip.start.as_seconds() - dur * 0.5).max(prev.start.as_seconds());
        ops.push(Op::Cover {
            media_id: cover.media,
            at: Time::from_seconds(at),
            source_in: Time::from_seconds(cover.start),
            duration: Duration::from_seconds(dur),
        });
    }
    let cues = mapped_cues(&clips, lines);
    if !cues.is_empty() {
        ops.push(Op::AddCaptions {
            style: CaptionStyle::Stacked,
            cues,
        });
    }
    ops
}

struct Look {
    grade: Grade,
    fx: oc_timeline::Fx,
    punch: bool,
    punch_scale: f32,
    moves: bool,
    vertical: bool,
}

fn finish_look(request: &str) -> Look {
    let t = request.to_ascii_lowercase();
    let vertical = wants_vertical(request);
    if t.contains("interview") || t.contains("podcast") || t.contains("talking") {
        return Look {
            grade: Grade::interview(),
            fx: oc_timeline::Fx::default(),
            punch: false,
            punch_scale: 1.0,
            moves: false,
            vertical,
        };
    }
    if t.contains("ad") || t.contains("product") || t.contains("commercial") {
        return Look {
            grade: Grade::ad(),
            fx: oc_timeline::Fx { vignette: 0.15, ..oc_timeline::Fx::default() },
            punch: true,
            punch_scale: 1.08,
            moves: true,
            vertical,
        };
    }
    if t.contains("doc") || t.contains("documentary") || t.contains("film") {
        return Look {
            grade: Grade::documentary(),
            fx: oc_timeline::Fx::default(),
            punch: false,
            punch_scale: 1.0,
            moves: false,
            vertical: false,
        };
    }
    if t.contains("vlog") {
        return Look {
            grade: Grade::vlog(),
            fx: oc_timeline::Fx { grain: 0.08, vignette: 0.2, blur: 0.0 },
            punch: true,
            punch_scale: 1.1,
            moves: true,
            vertical,
        };
    }
    Look {
        grade: Grade::punchy(),
        fx: oc_timeline::Fx::film(),
        punch: true,
        punch_scale: 1.18,
        moves: true,
        vertical: true || vertical,
    }
}

fn wants_vertical(request: &str) -> bool {
    let t = request.to_ascii_lowercase();
    t.contains("reel")
        || t.contains("short")
        || t.contains("tiktok")
        || t.contains("vertical")
        || t.contains("9:16")
}

fn program_clips(timeline: &Timeline) -> Vec<&Clip> {
    let mut clips = Vec::new();
    for track in &timeline.tracks {
        if track.kind != TrackKind::Video || track.hidden {
            continue;
        }
        if track.name == "GFX" || track.name == "B-roll" {
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

fn same_media(a: &Clip, b: &Clip) -> bool {
    matches!((a.media_id, b.media_id), (Some(x), Some(y)) if x == y)
}

fn pick_cover<'a>(covers: &'a [CoverShot], a: &Clip, b: &Clip) -> Option<&'a CoverShot> {
    let media = a.media_id?;
    if b.media_id != Some(media) {
        return None;
    }
    let used = [
        (a.source_in.as_seconds(), a.source_out().as_seconds()),
        (b.source_in.as_seconds(), b.source_out().as_seconds()),
    ];
    covers.iter().find(|c| {
        let long_enough = c.end - c.start >= if c.media == media { 1.0 } else { 0.9 };
        long_enough && !overlaps_any(c.start, c.end, &used)
    })
}

fn overlaps_any(start: f64, end: f64, used: &[(f64, f64)]) -> bool {
    used.iter().any(|(a, b)| start < *b - 0.05 && end > *a + 0.05)
}

fn mapped_cues(clips: &[&Clip], lines: &[SpokenLine]) -> Vec<CaptionCue> {
    let mut cues = Vec::new();
    for clip in clips {
        let Some(media) = clip.media_id else { continue };
        let speed = if clip.speed.is_finite() && clip.speed > 0.0 {
            f64::from(clip.speed)
        } else {
            1.0
        };
        let src_in = clip.source_in.as_seconds();
        let src_out = src_in + clip.duration.as_seconds() * speed;
        for line in lines.iter().filter(|l| l.media == media) {
            if filler(&line.text) {
                continue;
            }
            let overlap_start = line.start.max(src_in);
            let overlap_end = line.end.min(src_out);
            if overlap_end - overlap_start < 0.25 {
                continue;
            }
            let tl0 = clip.start.as_seconds() + (overlap_start - src_in) / speed;
            let tl1 = clip.start.as_seconds() + (overlap_end - src_in) / speed;
            cues.push(CaptionCue {
                start: Time::from_seconds(tl0),
                end: Time::from_seconds(tl1.max(tl0 + 0.3)),
                text: line.text.clone(),
                speaker: None,
            });
        }
    }
    cues.sort_by(|a, b| a.start.cmp(&b.start));
    cues
}

fn filler(text: &str) -> bool {
    let t = text.trim().to_ascii_lowercase();
    if t.is_empty() {
        return true;
    }
    t.split_whitespace().all(|w| {
        matches!(
            w.trim_matches(|c: char| !c.is_alphanumeric()),
            "um" | "uh" | "uhm" | "hmm" | "mm" | "yeah" | "yep" | "ok" | "okay" | "so" | "like"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use oc_timeline::{ClipId, ClipLook, Timeline, Track, Transform};

    fn video(media: MediaId, start: f64, dur: f64, source_in: f64) -> Clip {
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

    fn tl(clips: Vec<Clip>) -> Timeline {
        let mut track = Track::new(TrackKind::Video, "V1");
        track.clips = clips;
        let mut timeline = Timeline::default();
        timeline.tracks.push(track);
        timeline.width = 1920;
        timeline.height = 1080;
        timeline
    }

    #[test]
    fn reel_request_gets_grade_punch_captions_and_vertical() {
        let media = MediaId::new();
        let timeline = tl(vec![
            video(media, 0.0, 8.0, 2.0),
            video(media, 8.0, 8.0, 40.0),
        ]);
        let lines = vec![SpokenLine {
            media,
            start: 2.2,
            end: 6.0,
            text: "Here is the hook".into(),
        }];
        let ops = finish_reel(&timeline, &lines, &[], "Make a 1 min reel from this");
        assert!(ops.iter().any(|op| matches!(op, Op::Reframe { aspect: AspectRatio::Vertical })));
        assert!(ops.iter().any(|op| matches!(op, Op::SetGrade { .. })));
        assert!(ops.iter().any(|op| matches!(
            op,
            Op::SetTransform { scale, .. } if (*scale - 1.18).abs() < 0.01
        )));
        assert!(ops.iter().any(|op| matches!(op, Op::AddCaptions { .. })));
        assert!(ops.iter().any(|op| matches!(op, Op::SetTransition { kind: TransitionKind::Dissolve, .. })));
    }

    #[test]
    fn interview_stays_flat_and_landscape() {
        let media = MediaId::new();
        let timeline = tl(vec![video(media, 0.0, 6.0, 1.0), video(media, 6.0, 6.0, 20.0)]);
        let ops = finish_reel(&timeline, &[], &[], "cut an interview from this");
        assert!(ops.iter().all(|op| !matches!(op, Op::Reframe { .. })));
        assert!(ops.iter().all(|op| !matches!(op, Op::SetMove { .. })));
        assert!(ops.iter().any(|op| matches!(op, Op::SetGrade { grade, .. } if grade.lut == oc_timeline::Lut::None)));
    }

    #[test]
    fn trim_request_is_not_a_finish() {
        assert!(!wants_picture_finish("trim the start"));
        assert!(wants_picture_finish("Make a 1 min reel from this"));
    }

    #[test]
    fn cover_uses_a_range_outside_the_takes() {
        let media = MediaId::new();
        let other = MediaId::new();
        let timeline = tl(vec![
            video(media, 0.0, 4.0, 10.0),
            video(media, 4.0, 4.0, 30.0),
        ]);
        let covers = vec![CoverShot {
            media: other,
            start: 1.0,
            end: 3.0,
        }];
        let ops = finish_reel(&timeline, &[], &covers, "cut a short");
        assert!(ops.iter().any(|op| matches!(op, Op::Cover { media_id, .. } if *media_id == other)));
    }
}
