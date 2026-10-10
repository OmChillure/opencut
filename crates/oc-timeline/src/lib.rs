mod caption;
mod edit;
mod ids;
mod lut;
mod model;
mod project;
mod undo;

pub use caption::{
    caption_beats, caption_motion, caption_phrase, caption_reveal, dress_cues, look_for,
    shot_is_face,
};
pub use edit::{PlaceMode, marked_place};
pub use ids::{ClipId, GroupId, LinkId, MarkerId, MediaId, ProjectId, TrackId};
pub use lut::{CubeLut, canonical_color, cube_text, ffmpeg_color, parse_cube};
pub use model::{
    AlphaShape, AspectRatio, AudioFx, CaptionCue, CaptionEffect, CaptionFont, CaptionMood,
    CaptionPlace, CaptionRecipe, CaptionStyle, Clip, ClipKind, ClipLook, Crop, CurvePoint, Curves,
    DENOISE_NF_DB, DENOISE_NR_DB, Ease, EditPlan, EditSlot, FrameCard, Fx, Generator, Grade,
    Graphic, GraphicKind, LineLook, Lut, Marker, MaskShape, Mix, SpeedKey, Timeline, Track,
    TrackKind, Transform, TransitionKind, denoise_curve, denoise_sample,
};
pub use project::Project;
pub use undo::{UndoEntry, UndoStack};

/// One timeline clip the render may have to receive from the browser that holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserHold {
    pub id: String,
    pub name: String,
    pub on_server: bool,
    pub in_browser: bool,
}

/// Ids to send for a render. A clip already on the server is skipped.
/// Missing browser clips are an error, and nothing is sent until every one is here.
pub fn browser_spool_ids(clips: &[BrowserHold]) -> std::result::Result<Vec<String>, String> {
    let mut missing = Vec::new();
    let mut send = Vec::new();
    for clip in clips {
        if clip.on_server {
            continue;
        }
        if clip.in_browser {
            if !send.iter().any(|id| id == &clip.id) {
                send.push(clip.id.clone());
            }
            continue;
        }
        if missing.iter().any(|held: &BrowserHold| held.id == clip.id) {
            continue;
        }
        missing.push(clip.clone());
    }
    if missing.is_empty() {
        return Ok(send);
    }
    let names = missing
        .iter()
        .map(|clip| {
            if clip.name.is_empty() {
                clip.id.as_str()
            } else {
                clip.name.as_str()
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    Err(format!("These clips are not in this browser: {names}"))
}

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
    fn film_grain_uses_the_export_noise_amount() {
        let film = Fx::film();
        assert!((film.noise_alls() - 5.04).abs() < 0.01);
        assert!((film.grain_overlay() - 0.126).abs() < 0.001);
        assert_eq!(Fx::default().noise_alls(), 0.0);
        assert_eq!(
            Fx {
                grain: 0.01,
                ..Fx::default()
            }
            .grain_overlay(),
            0.0
        );
        assert!(
            (Fx {
                grain: 2.0,
                ..Fx::default()
            }
            .noise_alls()
                - 40.0)
                .abs()
                < 0.01
        );
    }

    #[test]
    fn denoise_pulls_the_floor_down_by_twelve_db() {
        let floor = 10f32.powf(DENOISE_NF_DB / 20.0);
        let cut = 10f32.powf(-DENOISE_NR_DB / 20.0);
        let quiet = floor * 0.5;
        let gain = cut + (1.0 - cut) * 0.5;
        assert!((denoise_sample(quiet) - quiet * gain).abs() < 1e-5);
        assert!((denoise_sample(0.5) - 0.5).abs() < 1e-5);
        assert!(denoise_sample(quiet).abs() < quiet.abs());
        let curve = denoise_curve(3);
        assert_eq!(curve.len(), 3);
        assert!((curve[0] - denoise_sample(-1.0)).abs() < 1e-5);
        assert!((curve[2] - denoise_sample(1.0)).abs() < 1e-5);
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
    fn source_time_follows_a_constant_rate() {
        let mut clip = video_clip(0.0, 4.0);
        clip.source_in = Time::from_seconds(10.0);
        clip.speed = 2.0;
        let at = clip.source_time_at(Time::from_seconds(1.0)).unwrap();
        assert!((at.as_seconds() - 12.0).abs() < 1e-3);
        assert!((clip.speed_at(Time::from_seconds(1.0)) - 2.0).abs() < 1e-3);
    }

    #[test]
    fn source_time_integrates_a_speed_ramp() {
        let mut clip = video_clip(0.0, 4.0);
        clip.look.speed_to = Some(3.0);
        let at = clip.source_time_at(Time::from_seconds(2.0)).unwrap();
        assert!((at.as_seconds() - 3.0).abs() < 1e-2, "{}", at.as_seconds());
    }

    #[test]
    fn split_a_doubled_clip_uses_source_time() {
        let mut tl = Timeline::default();
        let track = tl.first_track(TrackKind::Video).unwrap().id;
        let mut clip = video_clip(0.0, 4.0);
        clip.speed = 2.0;
        clip.source_in = Time::from_seconds(10.0);
        let id = tl.add_clip(track, clip).unwrap();
        let right = tl.split(id, Time::from_seconds(1.0)).unwrap();
        let right = tl.find_clip(right).unwrap().1;
        assert!(
            (right.source_in.as_seconds() - 12.0).abs() < 1e-2,
            "{}",
            right.source_in.as_seconds()
        );
        tl.merge_with_next(id).unwrap();
        let joined = tl.find_clip(id).unwrap().1;
        assert!((joined.duration.as_seconds() - 4.0).abs() < 1e-2);
    }

    #[test]
    fn split_follows_a_speed_ramp() {
        let mut tl = Timeline::default();
        let track = tl.first_track(TrackKind::Video).unwrap().id;
        let mut clip = video_clip(0.0, 4.0);
        clip.look.speed_to = Some(3.0);
        let id = tl.add_clip(track, clip).unwrap();
        let right_id = tl.split(id, Time::from_seconds(2.0)).unwrap();
        let right = tl.find_clip(right_id).unwrap().1;
        assert!(
            (right.source_in.as_seconds() - 3.0).abs() < 0.05,
            "{}",
            right.source_in.as_seconds()
        );
        assert!(
            (right.speed - 2.0).abs() < 0.05,
            "right starts at the cut speed {}",
            right.speed
        );
        let left = tl.find_clip(id).unwrap().1;
        assert!((left.look.speed_to.unwrap_or(0.0) - 2.0).abs() < 0.05);
        tl.merge_with_next(id).unwrap();
    }

    #[test]
    fn mixer_unity_is_zero_db() {
        let tl = Timeline::default();
        assert!((tl.master.linear() - 1.0).abs() < 1e-4);
        let (l, r) = tl.master.balance();
        assert!((l - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-4);
        assert!((r - l).abs() < 1e-4);
    }

    #[test]
    fn a_render_asks_only_for_picture_and_audio_files() {
        let mut tl = Timeline::default();
        let video = tl.first_track(TrackKind::Video).unwrap().id;
        let audio = tl.first_track(TrackKind::Audio).unwrap().id;
        let captions = tl.first_track(TrackKind::Caption).unwrap().id;
        let picture = video_clip(0.0, 2.0);
        let picture_id = picture.media_id.unwrap();
        tl.add_clip(video, picture.clone()).unwrap();
        tl.add_clip(video, picture).unwrap();
        let mut generated = video_clip(2.0, 1.0);
        generated.look.generator = Some(Generator::ColorBars);
        tl.add_clip(video, generated).unwrap();
        let mut off = video_clip(3.0, 1.0);
        off.disabled = true;
        tl.add_clip(video, off).unwrap();
        let mut sound = video_clip(0.0, 2.0);
        sound.kind = ClipKind::Audio {
            volume: 1.0,
            ducked: false,
        };
        let sound_id = sound.media_id.unwrap();
        tl.add_clip(audio, sound).unwrap();
        let mut caption = video_clip(0.0, 2.0);
        caption.kind = ClipKind::Caption {
            style: CaptionStyle::default(),
            cues: Vec::new(),
        };
        tl.add_clip(captions, caption).unwrap();
        assert_eq!(tl.source_media_ids(), vec![picture_id, sound_id]);
    }

    #[test]
    fn a_browser_spool_skips_server_clips_and_names_what_is_missing() {
        let server = BrowserHold {
            id: "server".into(),
            name: "on-r2.mp4".into(),
            on_server: true,
            in_browser: false,
        };
        let held = BrowserHold {
            id: "held".into(),
            name: "local.mp4".into(),
            on_server: false,
            in_browser: true,
        };
        let gone = BrowserHold {
            id: "gone".into(),
            name: "other-tab.mp4".into(),
            on_server: false,
            in_browser: false,
        };
        assert_eq!(
            browser_spool_ids(&[server.clone(), held.clone()]).unwrap(),
            vec!["held".to_string()]
        );
        let err = browser_spool_ids(&[held, gone, server]).unwrap_err();
        assert_eq!(err, "These clips are not in this browser: other-tab.mp4");
    }
}
