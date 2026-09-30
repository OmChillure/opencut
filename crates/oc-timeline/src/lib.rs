mod edit;
mod ids;
mod lut;
mod model;
mod project;
mod undo;

pub use ids::{ClipId, GroupId, LinkId, MarkerId, MediaId, ProjectId, TrackId};
pub use edit::PlaceMode;
pub use lut::{CubeLut, canonical_color, cube_text, ffmpeg_color, parse_cube};
pub use model::{
    AlphaShape, AspectRatio, AudioFx, CaptionCue, CaptionStyle, Clip, ClipKind, ClipLook, Crop,
    CurvePoint, Curves, Ease, EditPlan, EditSlot, Fx, Generator, Grade, Graphic, GraphicKind, Lut,
    Marker, MaskShape, Mix, SpeedKey, Timeline, Track, TrackKind, Transform, TransitionKind,
};
pub use project::Project;
pub use undo::{Edit, UndoEntry, UndoStack};

pub use oc_time::{Duration, FrameRate, Time};

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum TimelineError {
    #[error("clip not found: {0}")]
    ClipNotFound(ClipId),
    #[error("track not found: {0}")]
    TrackNotFound(TrackId),
    #[error("split point is outside the clip")]
    SplitOutOfRange,
    #[error("trim would make the clip empty")]
    EmptyTrim,
    #[error("track kind mismatch")]
    TrackKindMismatch,
    #[error("track is locked")]
    TrackLocked,
    #[error("clips cannot be merged")]
    CannotMerge,
    #[error("clips cannot be rolled")]
    CannotRoll,
    #[error("clips cannot be slid")]
    CannotSlide,
    #[error("speed must be greater than 0")]
    InvalidSpeed,
    #[error("no gap at that time")]
    NoGap,
    #[error("{0}")]
    Message(String),
}

pub type Result<T> = std::result::Result<T, TimelineError>;

#[cfg(test)]
mod tests {
    use super::*;

    fn video_clip(start_s: f64, dur_s: f64) -> Clip {
        Clip {
            id: ClipId::new(),
            media_id: Some(MediaId::new()),
            kind: ClipKind::Video {
                transform: Transform::default(),
            },
            start: Time::from_seconds(start_s),
            duration: Duration::from_seconds(dur_s),
            source_in: Time::ZERO,
            speed: 1.0,
            group_id: None,
            link_id: None,
            disabled: false,
            look: ClipLook::default(),
        }
    }

    #[test]
    fn split_preserves_source() {
        let mut tl = Timeline::default();
        let track = tl.first_track(TrackKind::Video).unwrap().id;
        let id = tl.add_clip(track, video_clip(0.0, 4.0)).unwrap();
        let right = tl.split(id, Time::from_seconds(1.0)).unwrap();
        let left = tl.find_clip(id).unwrap().1;
        assert!((left.duration.as_seconds() - 1.0).abs() < 1e-6);
        let right = tl.find_clip(right).unwrap().1;
        assert!((right.start.as_seconds() - 1.0).abs() < 1e-6);
        assert!((right.source_in.as_seconds() - 1.0).abs() < 1e-6);
        assert!((right.duration.as_seconds() - 3.0).abs() < 1e-6);
        tl.merge_with_next(id).unwrap();
        let joined = tl.find_clip(id).unwrap().1;
        assert!((joined.duration.as_seconds() - 4.0).abs() < 1e-6);
        assert_eq!(tl.first_track(TrackKind::Video).unwrap().clips.len(), 1);
    }

    #[test]
    fn ripple_delete_closes_gap() {
        let mut tl = Timeline::default();
        let track = tl.first_track(TrackKind::Video).unwrap().id;
        let a = tl.add_clip(track, video_clip(0.0, 2.0)).unwrap();
        let b = tl.add_clip(track, video_clip(2.0, 2.0)).unwrap();
        tl.ripple_delete(a).unwrap();
        let b = tl.find_clip(b).unwrap().1;
        assert_eq!(b.start, Time::ZERO);
    }

    #[test]
    fn undo_restores() {
        let mut tl = Timeline::default();
        let mut undo = UndoStack::new();
        let track = tl.first_track(TrackKind::Video).unwrap().id;
        undo.checkpoint(tl.clone());
        tl.add_clip(track, video_clip(0.0, 1.0)).unwrap();
        assert_eq!(tl.first_track(TrackKind::Video).unwrap().clips.len(), 1);
        assert!(undo.undo(&mut tl));
        assert!(tl.first_track(TrackKind::Video).unwrap().clips.is_empty());
        assert!(undo.redo(&mut tl));
        assert_eq!(tl.first_track(TrackKind::Video).unwrap().clips.len(), 1);
    }

    #[test]
    fn undo_history_jumps_by_name() {
        let mut tl = Timeline::default();
        let mut undo = UndoStack::new();
        let track = tl.first_track(TrackKind::Video).unwrap().id;
        undo.checkpoint_named(tl.clone(), "add");
        tl.add_clip(track, video_clip(0.0, 1.0)).unwrap();
        undo.checkpoint_named(tl.clone(), "mute");
        tl.track_mut(track).unwrap().muted = true;
        assert_eq!(undo.labels(), vec!["add".to_string(), "mute".to_string()]);
        assert!(undo.jump(&mut tl, 1));
        assert!(!tl.track(track).unwrap().muted);
        assert_eq!(tl.track(track).unwrap().clips.len(), 1);
        assert!(undo.jump(&mut tl, 0));
        assert!(tl.track(track).unwrap().clips.is_empty());
        assert!(undo.jump(&mut tl, 2));
        assert!(tl.track(track).unwrap().muted);
    }

    #[test]
    fn source_time_follows_the_playhead() {
        let mut clip = video_clip(2.0, 4.0);
        clip.source_in = Time::from_seconds(10.0);
        let at = clip.source_time_at(Time::from_seconds(3.0)).unwrap();
        assert!((at.as_seconds() - 11.0).abs() < 1e-6);
        assert!(clip.source_time_at(Time::from_seconds(0.5)).is_none());
    }

    #[test]
    fn mixer_unity_is_zero_db() {
        let tl = Timeline::default();
        assert!((tl.master.linear() - 1.0).abs() < 1e-4);
        let (l, r) = tl.master.balance();
        assert!((l - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-4);
        assert!((r - l).abs() < 1e-4);
    }
}
