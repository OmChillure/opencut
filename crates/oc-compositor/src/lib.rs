mod draw;
#[cfg(feature = "gpu")]
mod gpu;

pub use draw::{FrameSource, Surface, composite};
#[cfg(feature = "gpu")]
pub use gpu::{MonitorPath, monitor_path, present_monitor};

use oc_time::Time;
use oc_timeline::{
    AlphaShape, CaptionEffect, CaptionFont, CaptionPlace, CaptionStyle, Clip, ClipKind, Crop,
    Curves, FrameCard, Fx, Generator, Grade, Graphic, MediaId, Timeline, TrackKind, Transform,
    TransitionKind,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FramePlan {
    pub time: Time,
    pub width: u32,
    pub height: u32,
    pub layers: Vec<Layer>,
    /// The video element cannot show this frame. The monitor paints `composite` instead.
    #[serde(default)]
    pub needs_paint: bool,
    #[serde(default = "black")]
    pub background: String,
    #[serde(default)]
    pub letterbox: bool,
}

fn black() -> String {
    "#000000".into()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Layer {
    Video {
        media_id: MediaId,
        source_time: Time,
        transform: Transform,
        grade: Grade,
        fx: Fx,
        opacity: f32,
        transition: TransitionKind,
        /// 0 at the start of a mix, 1 at the cut. Incoming plates use 1.
        mix: f32,
        #[serde(default)]
        curves: Curves,
        #[serde(default)]
        mask: Option<AlphaShape>,
        #[serde(default)]
        crop: Option<Crop>,
        #[serde(default)]
        card: Option<FrameCard>,
        #[serde(default)]
        generator: Option<Generator>,
        /// Draw over the picture under it (design, b-roll, the corner window).
        #[serde(default)]
        overlay: bool,
        /// The shot arriving through `transition`.
        #[serde(default)]
        incoming: bool,
        /// Preview stands in for ffmpeg `deshake`. Export still uses the filter.
        #[serde(default)]
        stabilize: bool,
    },
    Caption {
        text: String,
        style: CaptionStyle,
        speaker: Option<String>,
        #[serde(default)]
        place: CaptionPlace,
        #[serde(default)]
        font: CaptionFont,
        #[serde(default)]
        effect: CaptionEffect,
    },
    Graphic {
        graphic: Graphic,
        opacity: f32,
    },
}

/// What should be drawn at `time`, back to front: program, design, titles, corner window, captions.
#[must_use]
pub fn plan_frame(timeline: &Timeline, time: Time) -> FramePlan {
    let mut layers = Vec::new();
    let mut needs_paint = false;
    let mut base_id = None;

    if let Some(clip) = base_clip(timeline, time) {
        base_id = Some(clip.id);
        let name = track_name_of(timeline, clip.id).unwrap_or("V1");
        if plate_needs_paint(clip, name) {
            needs_paint = true;
        }
        let mix = mix_at(timeline, clip, time);
        // A join is the compositor's mix, including the pixelize mosaic.
        // The video element only crossfades; it cannot draw that mosaic.
        if mix.progress > 0.0 && clip.look.transition != TransitionKind::Cut {
            needs_paint = true;
        }
        push_plate(&mut layers, clip, name, time, mix.progress, false, 0.0);
        if mix.progress > 0.0
            && let Some(incoming) = mix.incoming
        {
            push_plate(&mut layers, incoming, name, time, 1.0, true, mix.window);
        }
    }

    for track in &timeline.tracks {
        if !is_picture_overlay(&track.name)
            || track.kind != TrackKind::Video
            || track.muted
            || track.hidden
            || track.name == "Front"
        {
            continue;
        }
        let Some(clip) = track.clip_at(time) else {
            continue;
        };
        if clip.disabled || base_id == Some(clip.id) {
            continue;
        }
        if !matches!(clip.kind, ClipKind::Video { .. }) {
            continue;
        }
        if clip.media_id.is_none() && clip.look.generator.is_none() {
            continue;
        }
        needs_paint = true;
        push_plate(&mut layers, clip, &track.name, time, 0.0, false, 0.0);
    }

    for track in &timeline.tracks {
        if track.muted || track.hidden {
            continue;
        }
        for clip in &track.clips {
            if clip.disabled || !clip.contains(time) {
                continue;
            }
            if let ClipKind::Graphic { graphic } = &clip.kind {
                let local = (time - clip.start).as_seconds();
                layers.push(Layer::Graphic {
                    graphic: graphic.clone(),
                    opacity: clip.look.fade_gain(local, clip.duration.as_seconds()) as f32,
                });
            }
        }
    }

    for track in &timeline.tracks {
        if track.name != "Front" || track.muted || track.hidden {
            continue;
        }
        let Some(clip) = track.clip_at(time) else {
            continue;
        };
        if clip.disabled || !matches!(clip.kind, ClipKind::Video { .. }) {
            continue;
        }
        if clip.media_id.is_none() && clip.look.generator.is_none() {
            continue;
        }
        needs_paint = true;
        push_plate(&mut layers, clip, "Front", time, 0.0, false, 0.0);
    }

    for track in &timeline.tracks {
        if track.kind != TrackKind::Caption || track.muted || track.hidden {
            continue;
        }
        let Some(clip) = track.clip_at(time) else {
            continue;
        };
        if clip.disabled {
            continue;
        }
        if let ClipKind::Caption { style, cues } = &clip.kind {
            let local_t = Time::from_ticks((time - clip.start).as_ticks());
            if let Some(cue) = cues.iter().find(|c| local_t >= c.start && local_t < c.end) {
                layers.push(Layer::Caption {
                    text: cue.text.clone(),
                    style: *style,
                    speaker: cue.speaker.clone(),
                    place: cue.place,
                    font: cue.font,
                    effect: cue.effect,
                });
            }
        }
    }

    FramePlan {
        time,
        width: timeline.width,
        height: timeline.height,
        layers,
        needs_paint,
        background: timeline.background.clone(),
        letterbox: timeline.letterbox,
    }
}

fn is_picture_overlay(name: &str) -> bool {
    matches!(name, "Design" | "Front" | "GFX")
}

fn plate_needs_paint(clip: &Clip, track_name: &str) -> bool {
    let rotation = match &clip.kind {
        ClipKind::Video { transform } => transform.rotation.abs() > 0.4,
        _ => false,
    };
    is_picture_overlay(track_name)
        || rotation
        || clip.look.move_to.is_some()
        || clip.look.crop.is_some()
        || clip.look.mask.is_some()
        || clip.look.generator.is_some()
        || clip.look.card.is_some()
        || clip.look.overlay
        || !clip.look.curves.is_identity()
        || clip.look.fx.blur > 0.02
        || clip.look.fx.vignette > 0.02
        || !clip.look.grade.is_identity()
        || clip.look.stabilize
}

fn track_name_of(timeline: &Timeline, id: oc_timeline::ClipId) -> Option<&str> {
    timeline
        .tracks
        .iter()
        .find(|track| track.clips.iter().any(|clip| clip.id == id))
        .map(|track| track.name.as_str())
}

/// The program shot under the playhead. A clip still dissolving out wins over the shot it joins.
fn base_clip(timeline: &Timeline, time: Time) -> Option<&Clip> {
    let mut best: Option<&Clip> = None;
    let mut outgoing: Option<&Clip> = None;
    for track in &timeline.tracks {
        if track.kind != TrackKind::Video
            || track.muted
            || track.hidden
            || is_picture_overlay(&track.name)
        {
            continue;
        }
        for clip in &track.clips {
            if clip.disabled || !clip.contains(time) || !is_picture(clip) {
                continue;
            }
            if best.as_ref().is_none_or(|kept| clip.start >= kept.start) {
                best = Some(clip);
            }
            if mix_at(timeline, clip, time).progress > 0.0 {
                outgoing = Some(clip);
            }
        }
    }
    outgoing.or(best)
}

fn is_picture(clip: &Clip) -> bool {
    matches!(clip.kind, ClipKind::Video { .. })
        && (clip.media_id.is_some() || clip.look.generator.is_some())
}

fn push_plate(
    layers: &mut Vec<Layer>,
    clip: &Clip,
    track_name: &str,
    time: Time,
    mix: f32,
    incoming: bool,
    preroll: f64,
) {
    let ClipKind::Video { .. } = &clip.kind else {
        return;
    };
    let media_id = clip
        .media_id
        .unwrap_or_else(|| MediaId::from_uuid(uuid::Uuid::nil()));
    let early = incoming && !clip.contains(time);
    let source_time = if early {
        let elapsed = (time - (clip.start - oc_time::Duration::from_seconds(preroll)))
            .as_seconds()
            .max(0.0);
        clip.source_in
            + oc_time::Duration::from_seconds(elapsed * f64::from(clip.speed_at(clip.start)))
    } else {
        clip.source_time_at(time).unwrap_or(clip.source_in)
    };
    let local = if early {
        (time - (clip.start - oc_time::Duration::from_seconds(preroll)))
            .as_seconds()
            .max(0.0)
    } else {
        (time - clip.start).as_seconds().max(0.0)
    };
    let opacity = clip.look.fade_gain(local, clip.duration.as_seconds()) as f32;
    layers.push(Layer::Video {
        media_id,
        source_time,
        transform: pose_at(clip, time),
        grade: clip.look.grade,
        fx: clip.look.fx,
        opacity,
        transition: clip.look.transition,
        mix,
        curves: clip.look.curves.clone(),
        mask: clip.look.mask,
        crop: clip.look.crop,
        card: clip.look.card,
        generator: clip.look.generator.clone(),
        overlay: is_picture_overlay(track_name),
        incoming,
        stabilize: clip.look.stabilize,
    });
}

fn pose_at(clip: &Clip, time: Time) -> Transform {
    let ClipKind::Video { transform } = &clip.kind else {
        return Transform::default();
    };
    let Some(end) = clip.look.move_to else {
        return *transform;
    };
    let dur = clip.duration.as_seconds().max(1e-4);
    let along = if clip.contains(time) {
        ((time - clip.start).as_seconds() / dur).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let p = ease(clip.look.move_ease.unwrap_or_default(), along) as f32;
    Transform {
        x: transform.x + (end.x - transform.x) * p,
        y: transform.y + (end.y - transform.y) * p,
        scale: transform.scale + (end.scale - transform.scale) * p,
        rotation: transform.rotation + (end.rotation - transform.rotation) * p,
    }
}

fn ease(ease: oc_timeline::Ease, along: f64) -> f64 {
    let u = along.clamp(0.0, 1.0);
    match ease {
        oc_timeline::Ease::Linear => u,
        oc_timeline::Ease::In => u * u,
        oc_timeline::Ease::Out => 1.0 - (1.0 - u) * (1.0 - u),
        oc_timeline::Ease::InOut => {
            if u < 0.5 {
                2.0 * u * u
            } else {
                1.0 - 2.0 * (1.0 - u) * (1.0 - u)
            }
        }
    }
}

struct MixAt<'a> {
    progress: f32,
    window: f64,
    incoming: Option<&'a oc_timeline::Clip>,
}

fn is_join(a: &oc_timeline::Clip, b: &oc_timeline::Clip) -> bool {
    let a0 = a.start.as_seconds();
    let a1 = a.end().as_seconds();
    let b0 = b.start.as_seconds();
    b0 > a0 + 0.05 && b0 < a1 + 0.2 && b0 > a1 - 1.2
}

/// Progress 0..1 through an outgoing mix (same-track or V1→V2 join).
fn mix_at<'a>(timeline: &'a Timeline, clip: &oc_timeline::Clip, time: Time) -> MixAt<'a> {
    let none = MixAt {
        progress: 0.0,
        window: 0.0,
        incoming: None,
    };
    if clip.look.transition == TransitionKind::Cut {
        return none;
    }
    let next = timeline
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter())
        .find(|other| {
            other.id != clip.id
                && !other.disabled
                && matches!(other.kind, ClipKind::Video { .. })
                && is_join(clip, other)
        });
    let Some(next) = next else {
        return none;
    };
    let dur = clip
        .look
        .mix_window(clip.duration.as_seconds(), next.duration.as_seconds());
    if dur <= 1e-4 {
        return none;
    }
    let start = clip.end() - oc_time::Duration::from_seconds(dur);
    if time < start || time >= clip.end() {
        return none;
    }
    let p = ((time - start).as_seconds() / dur).clamp(0.0, 1.0);
    MixAt {
        progress: p as f32,
        window: dur,
        incoming: Some(next),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oc_time::Duration;
    use oc_timeline::{Clip, ClipId, ClipLook, TrackKind};

    #[test]
    fn plans_video_layer() {
        let mut tl = Timeline::default();
        let track = tl.first_track(TrackKind::Video).unwrap().id;
        let media = MediaId::new();
        tl.add_clip(
            track,
            Clip {
                id: ClipId::new(),
                media_id: Some(media),
                kind: ClipKind::Video {
                    transform: Transform::default(),
                },
                start: Time::ZERO,
                duration: Duration::from_seconds(2.0),
                source_in: Time::from_seconds(5.0),
                speed: 1.0,
                group_id: None,
                link_id: None,
                disabled: false,
                look: ClipLook::default(),
            },
        )
        .unwrap();
        let plan = plan_frame(&tl, Time::from_seconds(0.5));
        match &plan.layers[0] {
            Layer::Video {
                media_id,
                source_time,
                ..
            } => {
                assert_eq!(*media_id, media);
                assert!((source_time.as_seconds() - 5.5).abs() < 1e-6);
            }
            _ => panic!("expected video layer"),
        }
        assert!(!plan.needs_paint);
    }

    #[test]
    fn a_grade_asks_for_a_painted_frame() {
        let mut tl = Timeline::default();
        let track = tl.first_track(TrackKind::Video).unwrap().id;
        let mut look = ClipLook::default();
        look.grade.contrast = 0.2;
        look.grade.lut = oc_timeline::Lut::Warm;
        tl.add_clip(
            track,
            Clip {
                id: ClipId::new(),
                media_id: Some(MediaId::new()),
                kind: ClipKind::Video {
                    transform: Transform::default(),
                },
                start: Time::ZERO,
                duration: Duration::from_seconds(2.0),
                source_in: Time::ZERO,
                speed: 1.0,
                group_id: None,
                link_id: None,
                disabled: false,
                look,
            },
        )
        .unwrap();
        let plan = plan_frame(&tl, Time::from_seconds(0.4));
        assert!(plan.needs_paint);
    }

    #[test]
    fn pixelize_paints_only_during_the_join() {
        let mut tl = Timeline::default();
        let track = tl.first_track(TrackKind::Video).unwrap().id;
        let mut outgoing = ClipLook::default();
        outgoing.transition = TransitionKind::Pixelize;
        tl.add_clip(
            track,
            Clip {
                id: ClipId::new(),
                media_id: Some(MediaId::new()),
                kind: ClipKind::Video {
                    transform: Transform::default(),
                },
                start: Time::ZERO,
                duration: Duration::from_seconds(2.0),
                source_in: Time::ZERO,
                speed: 1.0,
                group_id: None,
                link_id: None,
                disabled: false,
                look: outgoing,
            },
        )
        .unwrap();
        tl.add_clip(
            track,
            Clip {
                id: ClipId::new(),
                media_id: Some(MediaId::new()),
                kind: ClipKind::Video {
                    transform: Transform::default(),
                },
                start: Time::from_seconds(2.0),
                duration: Duration::from_seconds(2.0),
                source_in: Time::ZERO,
                speed: 1.0,
                group_id: None,
                link_id: None,
                disabled: false,
                look: ClipLook::default(),
            },
        )
        .unwrap();
        assert!(!plan_frame(&tl, Time::from_seconds(0.4)).needs_paint);
        assert!(plan_frame(&tl, Time::from_seconds(1.5)).needs_paint);
    }

    #[test]
    fn stabilize_asks_for_a_painted_frame() {
        let mut tl = Timeline::default();
        let track = tl.first_track(TrackKind::Video).unwrap().id;
        let mut look = ClipLook::default();
        look.stabilize = true;
        tl.add_clip(
            track,
            Clip {
                id: ClipId::new(),
                media_id: Some(MediaId::new()),
                kind: ClipKind::Video {
                    transform: Transform::default(),
                },
                start: Time::ZERO,
                duration: Duration::from_seconds(2.0),
                source_in: Time::ZERO,
                speed: 1.0,
                group_id: None,
                link_id: None,
                disabled: false,
                look,
            },
        )
        .unwrap();
        let plan = plan_frame(&tl, Time::from_seconds(0.4));
        assert!(plan.needs_paint);
        assert!(matches!(
            plan.layers[0],
            Layer::Video {
                stabilize: true,
                ..
            }
        ));
    }

    #[test]
    fn doubled_speed_reads_twice_the_source() {
        let mut tl = Timeline::default();
        let track = tl.first_track(TrackKind::Video).unwrap().id;
        let clip = Clip {
            id: ClipId::new(),
            media_id: Some(MediaId::new()),
            kind: ClipKind::Video {
                transform: Transform::default(),
            },
            start: Time::ZERO,
            duration: Duration::from_seconds(2.0),
            source_in: Time::from_seconds(5.0),
            speed: 2.0,
            group_id: None,
            link_id: None,
            disabled: false,
            look: ClipLook::default(),
        };
        tl.add_clip(track, clip).unwrap();
        let plan = plan_frame(&tl, Time::from_seconds(0.5));
        match &plan.layers[0] {
            Layer::Video { source_time, .. } => {
                assert!((source_time.as_seconds() - 6.0).abs() < 1e-2);
            }
            _ => panic!("expected video"),
        }
    }

    #[test]
    fn a_design_sits_over_the_speaker_and_asks_to_be_painted() {
        let mut tl = Timeline::default();
        let v1 = tl.first_track(TrackKind::Video).unwrap().id;
        let speaker = MediaId::new();
        let design = MediaId::new();
        tl.add_clip(
            v1,
            Clip {
                id: ClipId::new(),
                media_id: Some(speaker),
                kind: ClipKind::Video {
                    transform: Transform::default(),
                },
                start: Time::ZERO,
                duration: Duration::from_seconds(4.0),
                source_in: Time::ZERO,
                speed: 1.0,
                group_id: None,
                link_id: None,
                disabled: false,
                look: ClipLook::default(),
            },
        )
        .unwrap();
        let design_track = tl.add_track(TrackKind::Video, "Design");
        tl.add_clip(
            design_track,
            Clip {
                id: ClipId::new(),
                media_id: Some(design),
                kind: ClipKind::Video {
                    transform: Transform::default(),
                },
                start: Time::ZERO,
                duration: Duration::from_seconds(2.0),
                source_in: Time::ZERO,
                speed: 1.0,
                group_id: None,
                link_id: None,
                disabled: false,
                look: ClipLook::default(),
            },
        )
        .unwrap();
        let plan = plan_frame(&tl, Time::from_seconds(0.5));
        assert!(plan.needs_paint);
        let ids: Vec<_> = plan
            .layers
            .iter()
            .filter_map(|layer| match layer {
                Layer::Video {
                    media_id, overlay, ..
                } => Some((*media_id, *overlay)),
                _ => None,
            })
            .collect();
        assert_eq!(ids, vec![(speaker, false), (design, true)]);
    }
}
