//! Shared fixtures for `oc-tests`.
//!
//! Integration tests live in `tests/` and cover look, mix, graphics,
//! compositor, MCP, and JSON compatibility.

use oc_core::{
    Clip, ClipId, ClipKind, ClipLook, Duration, MediaId, Time, Timeline, TrackKind, Transform,
};

#[must_use]
pub fn video(start: f64, dur: f64) -> Clip {
    video_on(MediaId::new(), start, dur)
}

#[must_use]
pub fn video_on(media: MediaId, start: f64, dur: f64) -> Clip {
    Clip {
        id: ClipId::new(),
        media_id: Some(media),
        kind: ClipKind::Video {
            transform: Transform::default(),
        },
        start: Time::from_seconds(start),
        duration: Duration::from_seconds(dur),
        source_in: Time::ZERO,
        speed: 1.0,
        group_id: None,
        link_id: None,
        disabled: false,
        look: ClipLook::default(),
    }
}

#[must_use]
pub fn audio(start: f64, dur: f64) -> Clip {
    Clip {
        id: ClipId::new(),
        media_id: Some(MediaId::new()),
        kind: ClipKind::Audio {
            volume: 1.0,
            ducked: false,
        },
        start: Time::from_seconds(start),
        duration: Duration::from_seconds(dur),
        source_in: Time::ZERO,
        speed: 1.0,
        group_id: None,
        link_id: None,
        disabled: false,
        look: ClipLook::default(),
    }
}

#[must_use]
pub fn timeline_with_two_shots() -> (Timeline, ClipId, ClipId) {
    let mut tl = Timeline::default();
    let track = tl.first_track(TrackKind::Video).unwrap().id;
    let a = tl.add_clip(track, video(0.0, 4.0)).unwrap();
    let b = tl.add_clip(track, video(4.0, 4.0)).unwrap();
    (tl, a, b)
}
