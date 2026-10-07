use crate::media::{EditorTrack, MediaItem, MediaKind, TimelineClip, TrackKindUi};
use oc_core::time::TICKS_PER_SECOND;
use oc_core::{
    Clip, ClipId, Duration, GroupId, LinkId, MediaId, Time, Timeline, Track, TrackId, TrackKind,
    timeline::ClipKind,
};
use uuid::Uuid;

pub fn tracks_from_timeline(timeline: &Timeline) -> Vec<EditorTrack> {
    let mut tracks: Vec<EditorTrack> = timeline
        .tracks
        .iter()
        .map(|track| EditorTrack {
            id: track.id.to_string(),
            name: track.name.clone(),
            kind: kind_ui(track.kind),
            muted: track.muted,
            hidden: track.hidden,
            clips: track
                .clips
                .iter()
                .map(|clip| TimelineClip {
                    id: clip.id.to_string(),
                    media_id: clip.media_id.map(|id| id.to_string()).unwrap_or_default(),
                    start: clip.start.as_seconds(),
                    duration: clip.duration.as_seconds(),
                    source_in: clip.source_in.as_seconds(),
                    speed: f64::from(clip.speed),
                    group_id: clip.group_id.map(|id| id.to_string()).unwrap_or_default(),
                    link_id: clip.link_id.map(|id| id.to_string()).unwrap_or_default(),
                    disabled: clip.disabled,
                    transition: clip.look.transition.label().to_ascii_lowercase(),
                    graphic: match &clip.kind {
                        ClipKind::Graphic { graphic } => {
                            if graphic.text.is_empty() {
                                "shape".into()
                            } else {
                                graphic.text.clone()
                            }
                        }
                        _ => clip
                            .look
                            .graphic
                            .as_ref()
                            .map(|g| g.text.clone())
                            .unwrap_or_default(),
                    },
                })
                .collect(),
        })
        .collect();
    crate::media::close_editor_gaps(&mut tracks);
    tracks
}

pub fn timeline_from_tracks(
    tracks: &[EditorTrack],
    prev: &Timeline,
    width: u32,
    height: u32,
) -> Timeline {
    let mut next = prev.clone();
    next.width = width.max(2);
    next.height = height.max(2);
    next.tracks = tracks
        .iter()
        .map(|track| {
            let id = parse_track_id(&track.id);
            let prev_track = prev.track(id);
            Track {
                id,
                kind: kind_engine(track.kind),
                name: track.name.clone(),
                muted: track.muted,
                hidden: track.hidden,
                locked: prev_track.map(|t| t.locked).unwrap_or(false),
                mix: prev_track.map(|t| t.mix).unwrap_or_default(),
                clips: track
                    .clips
                    .iter()
                    .map(|clip| clip_from_ui(clip, track.kind, prev))
                    .collect(),
            }
        })
        .collect();
    next
}

fn clip_from_ui(clip: &TimelineClip, kind: TrackKindUi, prev: &Timeline) -> Clip {
    let id = parse_clip_id(&clip.id);
    let prev_kind = prev.find_clip(id).map(|(_, c)| c.kind.clone());
    let look = prev
        .find_clip(id)
        .map(|(_, c)| c.look.clone())
        .unwrap_or_default();
    let kind = match kind {
        TrackKindUi::Video => match prev_kind {
            Some(k @ ClipKind::Video { .. }) => k,
            Some(k @ ClipKind::Graphic { .. }) => k,
            _ => ClipKind::Video {
                transform: Default::default(),
            },
        },
        TrackKindUi::Audio => match prev_kind {
            Some(k @ ClipKind::Audio { .. }) => k,
            _ => ClipKind::Audio {
                volume: 1.0,
                ducked: false,
            },
        },
        TrackKindUi::Caption => match prev_kind {
            Some(k @ ClipKind::Caption { .. }) => k,
            _ => ClipKind::Caption {
                style: Default::default(),
                cues: Vec::new(),
            },
        },
    };
    let prev_clip = prev.find_clip(id).map(|(_, c)| c);
    Clip {
        id,
        media_id: parse_media_id(&clip.media_id),
        kind,
        start: Time::from_seconds(clip.start),
        duration: Duration::from_seconds(clip.duration),
        source_in: Time::from_seconds(clip.source_in),
        speed: if clip.speed.is_finite() && clip.speed > 0.0 {
            clip.speed as f32
        } else {
            prev_clip.map(|c| c.speed).unwrap_or(1.0)
        },
        group_id: parse_group(&clip.group_id).or_else(|| prev_clip.and_then(|c| c.group_id)),
        link_id: parse_link(&clip.link_id).or_else(|| prev_clip.and_then(|c| c.link_id)),
        disabled: clip.disabled,
        look,
    }
}

pub fn media_from_api(
    id: &str,
    filename: String,
    content_type: &str,
    duration_ticks: Option<i64>,
    play_url: Option<String>,
) -> MediaItem {
    let kind = if content_type.starts_with("audio/") {
        MediaKind::Audio
    } else if content_type.starts_with("image/") {
        MediaKind::Image
    } else {
        MediaKind::from_name(&filename)
    };
    MediaItem {
        id: id.to_string(),
        name: filename,
        kind,
        url: play_url.unwrap_or_default(),
        content_type: content_type.to_string(),
        duration: duration_ticks
            .map(|ticks| ticks as f64 / TICKS_PER_SECOND as f64)
            .unwrap_or(0.0),
    }
}

fn kind_ui(kind: TrackKind) -> TrackKindUi {
    match kind {
        TrackKind::Video => TrackKindUi::Video,
        TrackKind::Audio => TrackKindUi::Audio,
        TrackKind::Caption => TrackKindUi::Caption,
    }
}

fn kind_engine(kind: TrackKindUi) -> TrackKind {
    match kind {
        TrackKindUi::Video => TrackKind::Video,
        TrackKindUi::Audio => TrackKind::Audio,
        TrackKindUi::Caption => TrackKind::Caption,
    }
}

fn parse_track_id(raw: &str) -> TrackId {
    TrackId::from_uuid(parse_uuid(raw))
}

fn parse_clip_id(raw: &str) -> ClipId {
    ClipId::from_uuid(parse_uuid(raw))
}

fn parse_media_id(raw: &str) -> Option<MediaId> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    Uuid::parse_str(raw).ok().map(MediaId::from_uuid)
}

fn parse_group(raw: &str) -> Option<GroupId> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    Uuid::parse_str(raw).ok().map(GroupId::from_uuid)
}

fn parse_link(raw: &str) -> Option<LinkId> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    Uuid::parse_str(raw).ok().map(LinkId::from_uuid)
}

fn parse_uuid(raw: &str) -> Uuid {
    Uuid::parse_str(raw).unwrap_or_else(|_| Uuid::now_v7())
}

#[cfg(test)]
mod tests {
    use super::*;
    use oc_core::{Clip, ClipKind, ClipLook, TrackKind};

    #[test]
    fn a_clip_keeps_its_place_on_the_way_back() {
        let mut timeline = Timeline::default();
        let track = timeline.first_track(TrackKind::Video).unwrap().id;
        let clip = Clip {
            id: ClipId::new(),
            media_id: Some(MediaId::new()),
            kind: ClipKind::Video {
                transform: Default::default(),
            },
            start: Time::from_seconds(1.0),
            duration: Duration::from_seconds(2.0),
            source_in: Time::from_seconds(3.0),
            speed: 1.5,
            group_id: None,
            link_id: None,
            disabled: false,
            look: ClipLook::default(),
        };
        let id = clip.id;
        timeline.add_clip(track, clip).unwrap();
        let tracks = tracks_from_timeline(&timeline);
        let back = timeline_from_tracks(&tracks, &timeline, 1280, 720);
        let (_, restored) = back.find_clip(id).unwrap();
        assert_eq!(back.width, 1280);
        assert_eq!(back.height, 720);
        assert!((restored.start.as_seconds() - 1.0).abs() < 1e-6);
        assert!((restored.duration.as_seconds() - 2.0).abs() < 1e-6);
        assert!((restored.source_in.as_seconds() - 3.0).abs() < 1e-6);
        assert!((restored.speed - 1.5).abs() < 1e-4);
    }

    #[test]
    fn media_from_api_reads_type_and_ticks() {
        let audio = media_from_api("a", "talk.wav".into(), "audio/wav", Some(120_000), None);
        assert!(matches!(audio.kind, MediaKind::Audio));
        assert!((audio.duration - 1.0).abs() < 1e-9);
        let image = media_from_api(
            "b",
            "still.png".into(),
            "image/png",
            None,
            Some("blob:1".into()),
        );
        assert!(matches!(image.kind, MediaKind::Image));
        assert_eq!(image.url, "blob:1");
        assert_eq!(image.duration, 0.0);
        let video = media_from_api("c", "clip.mp4".into(), "", None, None);
        assert!(matches!(video.kind, MediaKind::Video));
    }
}
