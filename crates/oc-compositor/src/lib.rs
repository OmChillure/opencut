use oc_time::Time;
use oc_timeline::{
    CaptionStyle, ClipKind, Fx, Grade, Graphic, MediaId, Timeline, Transform,
    TransitionKind,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FramePlan {
    pub time: Time,
    pub width: u32,
    pub height: u32,
    pub layers: Vec<Layer>,
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
        mix: f32,
    },
    Caption {
        text: String,
        style: CaptionStyle,
        speaker: Option<String>,
    },
    Graphic {
        graphic: Graphic,
        opacity: f32,
    },
}

/// What should be drawn at `time`. No GPU yet — wgpu binds to this later.
#[must_use]
pub fn plan_frame(timeline: &Timeline, time: Time) -> FramePlan {
    let mut layers = Vec::new();
    for track in &timeline.tracks {
        if track.muted || track.hidden {
            continue;
        }
        let Some(clip) = track.clip_at(time) else {
            continue;
        };
        if clip.disabled {
            continue;
        };
        let local = (time - clip.start).as_seconds();
        let opacity = clip.look.fade_gain(local, clip.duration.as_seconds()) as f32;
        let mix = mix_at(timeline, clip, time);
        match &clip.kind {
            ClipKind::Video { transform } => {
                if let Some(media_id) = clip.media_id
                    && let Some(source_time) = clip.source_time_at(time)
                {
                    layers.push(Layer::Video {
                        media_id,
                        source_time,
                        transform: *transform,
                        grade: clip.look.grade,
                        fx: clip.look.fx,
                        opacity,
                        transition: clip.look.transition,
                        mix: mix.progress,
                    });
                }
                if mix.progress > 0.0 {
                    if let Some(incoming) = mix.incoming {
                        push_incoming(&mut layers, incoming, time, mix.window);
                    }
                }
            }
            ClipKind::Caption { style, cues } => {
                let local_t = Time::from_ticks((time - clip.start).as_ticks());
                if let Some(cue) = cues.iter().find(|c| local_t >= c.start && local_t < c.end)
                {
                    layers.push(Layer::Caption {
                        text: cue.text.clone(),
                        style: *style,
                        speaker: cue.speaker.clone(),
                    });
                }
            }
            ClipKind::Graphic { graphic } => {
                layers.push(Layer::Graphic {
                    graphic: graphic.clone(),
                    opacity,
                });
            }
            ClipKind::Audio { .. } => {}
        }
    }
    FramePlan {
        time,
        width: timeline.width,
        height: timeline.height,
        layers,
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
fn mix_at<'a>(
    timeline: &'a Timeline,
    clip: &oc_timeline::Clip,
    time: Time,
) -> MixAt<'a> {
    let none = MixAt {
        progress: 0.0,
        window: 0.0,
        incoming: None,
    };
    if clip.look.transition == TransitionKind::Cut {
        return none;
    }
    let next = timeline.tracks.iter().flat_map(|t| t.clips.iter()).find(|other| {
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

fn push_incoming(layers: &mut Vec<Layer>, incoming: &oc_timeline::Clip, time: Time, window: f64) {
    let ClipKind::Video { transform } = &incoming.kind else {
        return;
    };
    let Some(media_id) = incoming.media_id else {
        return;
    };
    let elapsed = (time - (incoming.start - oc_time::Duration::from_seconds(window))).as_seconds();
    let source_time = incoming.source_in + oc_time::Duration::from_seconds(elapsed.max(0.0));
    let local = elapsed.max(0.0);
    let opacity = incoming.look.fade_gain(local, incoming.duration.as_seconds()) as f32;
    layers.push(Layer::Video {
        media_id,
        source_time,
        transform: *transform,
        grade: incoming.look.grade,
        fx: incoming.look.fx,
        opacity,
        transition: incoming.look.transition,
        mix: 1.0,
    });
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
    }
}
