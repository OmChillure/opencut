//! Grade, fx, and fade — Shotcut-style filters on a clip.

use oc_core::{Duration, Fx, Grade, Op, Time, Timeline, TrackKind, UndoStack, apply};
use oc_tests::{audio, video};

#[test]
fn old_timeline_json_gets_identity_look() {
    let json = r#"{
        "frame_rate": { "numerator": 30, "denominator": 1 },
        "width": 1920,
        "height": 1080,
        "tracks": [{
            "id": "11111111-1111-1111-1111-111111111111",
            "kind": "video",
            "name": "V1",
            "clips": [{
                "id": "22222222-2222-2222-2222-222222222222",
                "kind": { "kind": "video", "transform": { "x": 0, "y": 0, "scale": 1, "rotation": 0 } },
                "start": 0,
                "duration": 120000,
                "source_in": 0
            }]
        }]
    }"#;
    let tl: Timeline = serde_json::from_str(json).expect("old project json");
    let clip = &tl.tracks[0].clips[0];
    assert!(clip.look.grade.is_identity());
    assert!(clip.look.fx.is_identity());
    assert_eq!(clip.look.transition, oc_core::TransitionKind::Cut);
}

#[test]
fn punchy_grade_is_not_identity_and_roundtrips() {
    let g = Grade::punchy();
    assert!(!g.is_identity());
    let raw = serde_json::to_string(&g).unwrap();
    let back: Grade = serde_json::from_str(&raw).unwrap();
    assert_eq!(g, back);
}

#[test]
fn set_grade_and_fx_stick_and_undo() {
    let mut tl = Timeline::default();
    let mut undo = UndoStack::new();
    let track = tl.first_track(TrackKind::Video).unwrap().id;
    let id = tl.add_clip(track, video(0.0, 4.0)).unwrap();
    apply(
        &mut tl,
        &mut undo,
        Op::SetGrade {
            clip_id: id,
            grade: Grade::punchy(),
        },
    )
    .unwrap();
    apply(
        &mut tl,
        &mut undo,
        Op::SetFx {
            clip_id: id,
            fx: Fx::film(),
        },
    )
    .unwrap();
    let look = &tl.find_clip(id).unwrap().1.look;
    assert!(!look.grade.is_identity());
    assert!(!look.fx.is_identity());
    assert!(undo.undo(&mut tl));
    assert!(tl.find_clip(id).unwrap().1.look.fx.is_identity());
    assert!(undo.undo(&mut tl));
    assert!(tl.find_clip(id).unwrap().1.look.grade.is_identity());
}

#[test]
fn fade_gain_ramps_in_and_out() {
    let mut look = oc_core::ClipLook::default();
    look.fade_in = Duration::from_seconds(1.0);
    look.fade_out = Duration::from_seconds(1.0);
    assert!((look.fade_gain(0.0, 4.0) - 0.0).abs() < 1e-6);
    assert!((look.fade_gain(0.5, 4.0) - 0.5).abs() < 1e-6);
    assert!((look.fade_gain(2.0, 4.0) - 1.0).abs() < 1e-6);
    assert!((look.fade_gain(3.5, 4.0) - 0.5).abs() < 1e-6);
    assert!((look.fade_gain(4.0, 4.0) - 0.0).abs() < 1e-6);
}

#[test]
fn overlapping_fades_on_short_clip_multiply() {
    let mut look = oc_core::ClipLook::default();
    look.fade_in = Duration::from_seconds(0.8);
    look.fade_out = Duration::from_seconds(0.8);
    let mid = look.fade_gain(0.5, 1.0);
    assert!(mid > 0.0 && mid < 1.0, "{mid}");
}

#[test]
fn set_fade_writes_both_ends() {
    let mut tl = Timeline::default();
    let mut undo = UndoStack::new();
    let track = tl.first_track(TrackKind::Video).unwrap().id;
    let id = tl.add_clip(track, video(0.0, 4.0)).unwrap();
    apply(
        &mut tl,
        &mut undo,
        Op::SetFade {
            clip_id: id,
            fade_in: Duration::from_seconds(0.8),
            fade_out: Duration::from_seconds(0.8),
        },
    )
    .unwrap();
    let look = &tl.find_clip(id).unwrap().1.look;
    assert!((look.fade_in.as_seconds() - 0.8).abs() < 1e-6);
    assert!((look.fade_out.as_seconds() - 0.8).abs() < 1e-6);
}

#[test]
fn volume_rejects_video_accepts_audio() {
    let mut tl = Timeline::default();
    let mut undo = UndoStack::new();
    let v = tl.first_track(TrackKind::Video).unwrap().id;
    let a = tl.first_track(TrackKind::Audio).unwrap().id;
    let vid = tl.add_clip(v, video(0.0, 2.0)).unwrap();
    let aud = tl.add_clip(a, audio(0.0, 2.0)).unwrap();
    let err = apply(
        &mut tl,
        &mut undo,
        Op::SetVolume {
            clip_id: vid,
            volume: 0.5,
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("audio"), "{err}");
    apply(
        &mut tl,
        &mut undo,
        Op::SetVolume {
            clip_id: aud,
            volume: 0.4,
        },
    )
    .unwrap();
    match tl.find_clip(aud).unwrap().1.kind {
        oc_core::ClipKind::Audio { volume, .. } => assert!((volume - 0.4).abs() < 1e-4),
        _ => panic!("expected audio"),
    }
}

#[test]
fn duck_marks_audio_clips() {
    let mut tl = Timeline::default();
    let mut undo = UndoStack::new();
    let a = tl.first_track(TrackKind::Audio).unwrap().id;
    let id = tl.add_clip(a, audio(0.0, 4.0)).unwrap();
    apply(&mut tl, &mut undo, Op::Duck { amount: 0.7 }).unwrap();
    match tl.find_clip(id).unwrap().1.kind {
        oc_core::ClipKind::Audio { ducked, volume } => {
            assert!(ducked);
            assert!(volume < 0.5, "{volume}");
        }
        _ => panic!("expected audio"),
    }
}

#[test]
fn missing_clip_ops_error() {
    let mut tl = Timeline::default();
    let mut undo = UndoStack::new();
    let ghost = oc_core::ClipId::new();
    assert!(
        apply(
            &mut tl,
            &mut undo,
            Op::SetGrade {
                clip_id: ghost,
                grade: Grade::punchy(),
            },
        )
        .is_err()
    );
}

#[test]
fn fade_op_missing_clip() {
    let mut tl = Timeline::default();
    let mut undo = UndoStack::new();
    assert!(
        apply(
            &mut tl,
            &mut undo,
            Op::SetFade {
                clip_id: oc_core::ClipId::new(),
                fade_in: Duration::from_seconds(0.5),
                fade_out: Duration::from_seconds(0.5),
            },
        )
        .is_err()
    );
}

#[test]
fn look_survives_timeline_json() {
    let mut tl = Timeline::default();
    let mut undo = UndoStack::new();
    let track = tl.first_track(TrackKind::Video).unwrap().id;
    let id = tl.add_clip(track, video(0.0, 3.0)).unwrap();
    apply(
        &mut tl,
        &mut undo,
        Op::SetGrade {
            clip_id: id,
            grade: Grade::punchy(),
        },
    )
    .unwrap();
    let raw = serde_json::to_string(&tl).unwrap();
    let back: Timeline = serde_json::from_str(&raw).unwrap();
    assert!(!back.find_clip(id).unwrap().1.look.grade.is_identity());
}

#[test]
fn hidden_and_muted_tracks_are_not_planned() {
    use oc_compositor::plan_frame;
    let mut tl = Timeline::default();
    let track = tl.first_track(TrackKind::Video).unwrap().id;
    let _ = tl.add_clip(track, video(0.0, 2.0)).unwrap();
    tl.track_mut(track).unwrap().hidden = true;
    assert!(plan_frame(&tl, Time::from_seconds(0.5)).layers.is_empty());
    tl.track_mut(track).unwrap().hidden = false;
    tl.track_mut(track).unwrap().muted = true;
    assert!(plan_frame(&tl, Time::from_seconds(0.5)).layers.is_empty());
}

#[test]
fn disabled_clip_is_not_planned() {
    use oc_compositor::plan_frame;
    let mut tl = Timeline::default();
    let track = tl.first_track(TrackKind::Video).unwrap().id;
    let id = tl.add_clip(track, video(0.0, 2.0)).unwrap();
    tl.clip_mut(id).unwrap().disabled = true;
    assert!(plan_frame(&tl, Time::from_seconds(0.5)).layers.is_empty());
}

#[test]
fn time_units_are_ticks_in_json() {
    let t = Time::from_seconds(1.0);
    let n = serde_json::to_value(t).unwrap();
    assert_eq!(n.as_i64(), Some(120_000));
}
