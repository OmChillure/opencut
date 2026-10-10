//! Captions on the cut, and a cover range that can hide a jump.

use oc_time::Time;
use oc_timeline::{
    CaptionCue, CaptionEffect, CaptionFont, CaptionMood, CaptionPlace, CaptionRecipe, Clip,
    ClipKind, EditPlan, MediaId, Timeline, TrackKind,
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

/// Give every caption the video's one theme.
/// A clip that already shares that font, effect, and a place from the theme stays.
/// Lines that were styled one by one, and the old default bottom bar, are dressed again.
#[must_use]
pub fn redress_unset_captions(timeline: &mut Timeline) -> bool {
    let recipe = recipe_for_timeline(timeline);
    let open = oc_timeline::look_for("", 0, &recipe, false);
    let seated = oc_timeline::look_for("", 0, &recipe, true);
    let mut changed = false;
    for track in &mut timeline.tracks {
        if track.kind != TrackKind::Caption || track.muted || track.hidden {
            continue;
        }
        for clip in &mut track.clips {
            if clip.disabled {
                continue;
            }
            let ClipKind::Caption { cues, .. } = &mut clip.kind else {
                continue;
            };
            if cues.is_empty() || cues_share_theme(cues, open, seated) {
                continue;
            }
            let faces = vec![true; cues.len()];
            oc_timeline::dress_cues(cues, &recipe, &faces);
            changed = true;
        }
    }
    changed
}

fn cues_share_theme(
    cues: &[CaptionCue],
    open: (CaptionPlace, CaptionFont, CaptionEffect),
    seated: (CaptionPlace, CaptionFont, CaptionEffect),
) -> bool {
    cues.iter().all(|cue| {
        cue.font == open.1 && cue.effect == open.2 && (cue.place == open.0 || cue.place == seated.0)
    })
}

/// The theme saved on this timeline. No named mood stays clean, including a vertical frame.
#[must_use]
pub fn caption_recipe_for(timeline: &Timeline) -> CaptionRecipe {
    recipe_for_timeline(timeline)
}

fn recipe_for_timeline(timeline: &Timeline) -> CaptionRecipe {
    if let Some(plan) = &timeline.edit_plan {
        if let Some(look) = &plan.caption_look {
            let mut recipe = look.clone();
            if recipe.base.is_none() {
                recipe.base = Some(mood_for_plan(plan));
            }
            return recipe;
        }
        if let Some(mood) = plan.caption_mood {
            return mood.recipe();
        }
        return mood_for_plan(plan).recipe();
    }
    CaptionMood::Clean.recipe()
}

fn mood_for_plan(plan: &EditPlan) -> CaptionMood {
    CaptionMood::resolve(plan.caption_mood, &plan.style)
}

/// True when a caption track already has words to burn.
#[must_use]
pub fn has_burnable_captions(timeline: &Timeline) -> bool {
    timeline.tracks.iter().any(|track| {
        track.kind == TrackKind::Caption
            && !track.muted
            && !track.hidden
            && track.clips.iter().any(|clip| {
                !clip.disabled
                    && matches!(
                        &clip.kind,
                        ClipKind::Caption { cues, .. } if !cues.is_empty()
                    )
            })
    })
}

pub fn program_clips(timeline: &Timeline) -> Vec<&Clip> {
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

pub(crate) fn pick_cover<'a>(covers: &'a [CoverShot], a: &Clip, b: &Clip) -> Option<&'a CoverShot> {
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
    used.iter()
        .any(|(a, b)| start < *b - 0.05 && end > *a + 0.05)
}

pub fn mapped_cues(clips: &[&Clip], lines: &[SpokenLine]) -> Vec<CaptionCue> {
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
            if is_filler(&line.text) {
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
                place: Default::default(),
                font: Default::default(),
                effect: Default::default(),
            });
        }
    }
    cues.sort_by(|a, b| a.start.cmp(&b.start));
    cues
}

pub(crate) fn is_filler(text: &str) -> bool {
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
    use crate::ops::Op;
    use oc_time::Duration;
    use oc_timeline::{CaptionStyle, ClipId, ClipLook, Timeline, Track, Transform};

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
    fn mapped_cues_land_on_the_cut_not_the_source() {
        let media = MediaId::new();
        let clip = video(media, 0.0, 6.0, 40.0);
        let lines = vec![SpokenLine {
            media,
            start: 41.0,
            end: 43.0,
            text: "on the cut".into(),
        }];
        let cues = mapped_cues(&[&clip], &lines);
        assert_eq!(cues.len(), 1);
        assert!((cues[0].start.as_seconds() - 1.0).abs() < 1e-6);
        assert!((cues[0].end.as_seconds() - 3.0).abs() < 1e-6);
    }

    #[test]
    fn an_empty_caption_track_is_not_burnable() {
        let mut tl = tl(vec![video(MediaId::new(), 0.0, 4.0, 0.0)]);
        assert!(!has_burnable_captions(&tl));
        let mut undo = oc_timeline::UndoStack::new();
        crate::apply(
            &mut tl,
            &mut undo,
            Op::AddCaptions {
                style: CaptionStyle::Stacked,
                cues: vec![CaptionCue {
                    start: Time::from_seconds(0.2),
                    end: Time::from_seconds(2.0),
                    text: "on screen".into(),
                    speaker: None,
                    place: Default::default(),
                    font: Default::default(),
                    effect: Default::default(),
                }],
            },
        )
        .unwrap();
        assert!(has_burnable_captions(&tl));
    }

    #[test]
    fn unset_vertical_captions_share_one_theme() {
        let mut timeline = Timeline::new(oc_timeline::FrameRate::FPS_30, 1080, 1920);
        let track = timeline
            .first_track(TrackKind::Caption)
            .expect("captions")
            .id;
        timeline
            .add_clip(
                track,
                Clip {
                    id: ClipId::new(),
                    media_id: None,
                    kind: ClipKind::Caption {
                        style: CaptionStyle::Stacked,
                        cues: vec![
                            plain_cue(0.0, "the city opens up from here"),
                            plain_cue(4.0, "food and refreshments are on the house"),
                        ],
                    },
                    start: Time::ZERO,
                    duration: Duration::from_seconds(8.0),
                    source_in: Time::ZERO,
                    speed: 1.0,
                    group_id: None,
                    link_id: None,
                    disabled: false,
                    look: ClipLook::default(),
                },
            )
            .unwrap();
        assert!(redress_unset_captions(&mut timeline));
        let cues = caption_cues(&timeline);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].place, CaptionPlace::Bottom);
        assert_eq!(cues[0].font, CaptionFont::Sans);
        assert_eq!(cues[0].effect, CaptionEffect::Fade);
        assert_eq!(cues[1].place, cues[0].place);
        assert_eq!(cues[1].font, cues[0].font);
        assert_eq!(cues[1].effect, cues[0].effect);
        assert!(!redress_unset_captions(&mut timeline));
    }

    #[test]
    fn a_style_word_leaves_the_caption_clean() {
        let mut timeline = Timeline::new(oc_timeline::FrameRate::FPS_30, 1080, 1920);
        timeline.edit_plan = Some(plan_with_style("hype"));
        assert_eq!(caption_recipe_for(&timeline).base, Some(CaptionMood::Clean));
        timeline.edit_plan = Some(plan_with_style("documentary"));
        assert_eq!(caption_recipe_for(&timeline).base, Some(CaptionMood::Clean));
        let mut chosen = plan_with_style("hype");
        chosen.caption_mood = Some(CaptionMood::Kinetic);
        timeline.edit_plan = Some(chosen);
        assert_eq!(
            caption_recipe_for(&timeline).base,
            Some(CaptionMood::Kinetic)
        );
        timeline.edit_plan = None;
        assert_eq!(caption_recipe_for(&timeline).base, Some(CaptionMood::Clean));
    }

    fn plan_with_style(style: &str) -> EditPlan {
        EditPlan {
            style: style.into(),
            aspect: "vertical".into(),
            letterbox: false,
            music_id: None,
            music_volume: None,
            captions: true,
            caption_mood: None,
            caption_look: None,
            grade: oc_timeline::Grade::default(),
            slots: Vec::new(),
        }
    }

    fn plain_cue(start: f64, text: &str) -> CaptionCue {
        CaptionCue {
            start: Time::from_seconds(start),
            end: Time::from_seconds(start + 3.0),
            text: text.into(),
            speaker: None,
            place: CaptionPlace::Bottom,
            font: CaptionFont::Sans,
            effect: CaptionEffect::None,
        }
    }

    fn caption_cues(timeline: &Timeline) -> Vec<CaptionCue> {
        timeline
            .tracks
            .iter()
            .flat_map(|track| track.clips.iter())
            .find_map(|clip| match &clip.kind {
                ClipKind::Caption { cues, .. } => Some(cues.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }

    #[test]
    fn cover_uses_a_range_outside_the_takes() {
        let media = MediaId::new();
        let other = MediaId::new();
        let left = video(media, 0.0, 4.0, 10.0);
        let right = video(media, 4.0, 4.0, 30.0);
        let outside = [CoverShot {
            media: other,
            start: 1.0,
            end: 3.0,
        }];
        let picked = pick_cover(&outside, &left, &right).unwrap();
        assert_eq!(picked.media, other);
        let inside = [CoverShot {
            media: other,
            start: 10.0,
            end: 12.0,
        }];
        assert!(pick_cover(&inside, &left, &right).is_none());
        let stranger = video(MediaId::new(), 4.0, 4.0, 30.0);
        assert!(pick_cover(&outside, &left, &stranger).is_none());
    }
}
