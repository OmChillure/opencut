//! Same-track mix: Kdenlive Mix / Shotcut overlap / ffmpeg xfade.

use oc_compositor::{Layer, plan_frame};
use oc_core::{
    Duration, Op, Time, Timeline, TrackKind, TransitionKind, UndoStack, apply,
};
use oc_tests::{timeline_with_two_shots, video};

#[test]
fn dissolve_sets_outgoing_kind() {
    let (mut tl, a, _b) = timeline_with_two_shots();
    let mut undo = UndoStack::new();
    apply(
        &mut tl,
        &mut undo,
        Op::SetTransition {
            clip_id: a,
            kind: TransitionKind::Dissolve,
            duration: None,
        },
    )
    .unwrap();
    assert_eq!(
        tl.find_clip(a).unwrap().1.look.transition,
        TransitionKind::Dissolve
    );
}

#[test]
fn cut_clears_mix() {
    let (mut tl, a, _) = timeline_with_two_shots();
    let mut undo = UndoStack::new();
    apply(
        &mut tl,
        &mut undo,
        Op::SetTransition {
            clip_id: a,
            kind: TransitionKind::Slide,
            duration: None,
        },
    )
    .unwrap();
    apply(
        &mut tl,
        &mut undo,
        Op::SetTransition {
            clip_id: a,
            kind: TransitionKind::Cut,
            duration: None,
        },
    )
    .unwrap();
    assert_eq!(
        tl.find_clip(a).unwrap().1.look.transition,
        TransitionKind::Cut
    );
}

#[test]
fn mix_window_clamps_to_half_the_shorter_clip() {
    let look = {
        let mut l = oc_core::ClipLook::default();
        l.transition = TransitionKind::Dissolve;
        l
    };
    assert!((look.mix_window(4.0, 4.0) - 0.8).abs() < 1e-6);
    let short = look.mix_window(0.6, 4.0);
    assert!((short - 0.3).abs() < 1e-6, "{short}");
    assert_eq!(look.mix_window(4.0, 0.0), 0.0);
}

#[test]
fn dissolve_without_neighbor_has_zero_mix() {
    let mut tl = Timeline::default();
    let mut undo = UndoStack::new();
    let track = tl.first_track(TrackKind::Video).unwrap().id;
    let id = tl.add_clip(track, video(0.0, 4.0)).unwrap();
    apply(
        &mut tl,
        &mut undo,
        Op::SetTransition {
            clip_id: id,
            kind: TransitionKind::Dissolve,
            duration: None,
        },
    )
    .unwrap();
    let plan = plan_frame(&tl, Time::from_seconds(3.7));
    match &plan.layers[0] {
        Layer::Video { mix, .. } => assert_eq!(*mix, 0.0),
        other => panic!("{other:?}"),
    }
}

#[test]
fn dissolve_emits_incoming_shot_in_the_mix_window() {
    let (mut tl, a, b) = timeline_with_two_shots();
    let mut undo = UndoStack::new();
    apply(
        &mut tl,
        &mut undo,
        Op::SetTransition {
            clip_id: a,
            kind: TransitionKind::Dissolve,
            duration: None,
        },
    )
    .unwrap();
    let a_media = tl.find_clip(a).unwrap().1.media_id;
    let b_media = tl.find_clip(b).unwrap().1.media_id;
    let plan = plan_frame(&tl, Time::from_seconds(3.6));
    let videos: Vec<_> = plan
        .layers
        .iter()
        .filter_map(|l| match l {
            Layer::Video {
                media_id,
                mix,
                source_time,
                ..
            } => Some((*media_id, *mix, source_time.as_seconds())),
            _ => None,
        })
        .collect();
    assert_eq!(videos.len(), 2, "outgoing + incoming: {videos:?}");
    assert_eq!(videos[0].0, a_media.unwrap());
    assert!(videos[0].1 > 0.0 && videos[0].1 < 1.0, "progress {}", videos[0].1);
    assert_eq!(videos[1].0, b_media.unwrap());
}

#[test]
fn mix_progress_is_zero_before_window() {
    let (mut tl, a, _) = timeline_with_two_shots();
    let mut undo = UndoStack::new();
    apply(
        &mut tl,
        &mut undo,
        Op::SetTransition {
            clip_id: a,
            kind: TransitionKind::Wipe,
            duration: None,
        },
    )
    .unwrap();
    let plan = plan_frame(&tl, Time::from_seconds(1.0));
    match &plan.layers[0] {
        Layer::Video { mix, .. } => assert_eq!(*mix, 0.0),
        other => panic!("{other:?}"),
    }
}

#[test]
fn split_does_not_copy_outgoing_mix_onto_the_left() {
    let (mut tl, a, _) = timeline_with_two_shots();
    let mut undo = UndoStack::new();
    apply(
        &mut tl,
        &mut undo,
        Op::SetTransition {
            clip_id: a,
            kind: TransitionKind::Dissolve,
            duration: None,
        },
    )
    .unwrap();
    apply(
        &mut tl,
        &mut undo,
        Op::SetFade {
            clip_id: a,
            fade_in: Duration::from_seconds(0.5),
            fade_out: Duration::from_seconds(0.5),
        },
    )
    .unwrap();
    let right = apply(
        &mut tl,
        &mut undo,
        Op::Split {
            clip_id: a,
            at: Time::from_seconds(2.0),
        },
    )
    .unwrap();
    let _ = right;
    let left = tl.find_clip(a).unwrap().1;
    assert_eq!(left.look.transition, TransitionKind::Cut);
    assert_eq!(left.look.fade_out.as_ticks(), 0);
    assert!(left.look.fade_in.as_seconds() > 0.0);
    let track = tl.first_track(TrackKind::Video).unwrap();
    let right = &track.clips[1];
    assert_eq!(right.look.transition, TransitionKind::Dissolve);
    assert_eq!(right.look.fade_in.as_ticks(), 0);
    assert!(right.look.fade_out.as_seconds() > 0.0);
}

#[test]
fn all_mix_kinds_roundtrip_in_ops_json() {
    for kind in TransitionKind::ALL.iter().copied() {
        let op = Op::SetTransition {
            clip_id: oc_core::ClipId::new(),
            kind,
            duration: None,
        };
        let raw = serde_json::to_string(&op).unwrap();
        let back: Op = serde_json::from_str(&raw).unwrap();
        match back {
            Op::SetTransition { kind: k, .. } => assert_eq!(k, kind),
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn gap_between_clips_is_not_a_mix() {
    let mut tl = Timeline::default();
    let mut undo = UndoStack::new();
    let track = tl.first_track(TrackKind::Video).unwrap().id;
    let a = tl.add_clip(track, video(0.0, 2.0)).unwrap();
    let _b = tl.add_clip(track, video(3.0, 2.0)).unwrap();
    apply(
        &mut tl,
        &mut undo,
        Op::SetTransition {
            clip_id: a,
            kind: TransitionKind::Dissolve,
            duration: None,
        },
    )
    .unwrap();
    let plan = plan_frame(&tl, Time::from_seconds(1.7));
    match &plan.layers[0] {
        Layer::Video { mix, .. } => assert_eq!(*mix, 0.0, "gap should not mix"),
        other => panic!("{other:?}"),
    }
}
