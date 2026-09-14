use oc_time::Time;
use oc_timeline::{CaptionStyle, ClipKind, MediaId, Timeline, Transform};
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
    },
    Caption {
        text: String,
        style: CaptionStyle,
        speaker: Option<String>,
    },
}

/// What should be drawn at `time`. No GPU yet — wgpu binds to this later.
#[must_use]
pub fn plan_frame(timeline: &Timeline, time: Time) -> FramePlan {
    let mut layers = Vec::new();
    for track in &timeline.tracks {
        if track.muted {
            continue;
        }
        let Some(clip) = track.clip_at(time) else {
            continue;
        };
        if clip.disabled {
            continue;
        };
        match &clip.kind {
            ClipKind::Video { transform } => {
                if let Some(media_id) = clip.media_id
                    && let Some(source_time) = clip.source_time_at(time)
                {
                    layers.push(Layer::Video {
                        media_id,
                        source_time,
                        transform: *transform,
                    });
                }
            }
            ClipKind::Caption { style, cues } => {
                let local = Time::from_ticks((time - clip.start).as_ticks());
                if let Some(cue) = cues.iter().find(|c| local >= c.start && local < c.end)
                {
                    layers.push(Layer::Caption {
                        text: cue.text.clone(),
                        style: *style,
                        speaker: cue.speaker.clone(),
                    });
                }
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

#[cfg(test)]
mod tests {
    use super::*;
    use oc_time::Duration;
    use oc_timeline::{Clip, ClipId, TrackKind};

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
