//! Titles / lower thirds / shapes sit on a GFX track over picture.

use oc_compositor::{Layer, plan_frame};
use oc_core::{
    ClipKind, DesignLayout, Duration, Graphic, GraphicKind, MediaId, Op, Time, Timeline, TrackKind,
    UndoStack, apply,
};
use oc_tests::{video, video_on};

#[test]
fn title_lands_on_a_second_video_track() {
    let mut tl = Timeline::default();
    let mut undo = UndoStack::new();
    let v1 = tl.first_track(TrackKind::Video).unwrap().id;
    let _ = tl.add_clip(v1, video(0.0, 6.0)).unwrap();
    apply(
        &mut tl,
        &mut undo,
        Op::AddGraphic {
            graphic: Graphic::title("HELLO"),
            start: Time::from_seconds(1.0),
            duration: Duration::from_seconds(3.0),
            track_id: None,
        },
    )
    .unwrap();
    let video_tracks: Vec<_> = tl
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Video)
        .collect();
    assert!(video_tracks.len() >= 2, "need GFX above V1");
    let gfx = video_tracks.iter().find(|t| t.name == "GFX").unwrap();
    assert_eq!(gfx.clips.len(), 1);
    match &gfx.clips[0].kind {
        ClipKind::Graphic { graphic } => {
            assert_eq!(graphic.kind, GraphicKind::Title);
            assert_eq!(graphic.text, "HELLO");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn compositor_stacks_graphic_over_video() {
    let mut tl = Timeline::default();
    let mut undo = UndoStack::new();
    let v1 = tl.first_track(TrackKind::Video).unwrap().id;
    let _ = tl.add_clip(v1, video(0.0, 6.0)).unwrap();
    apply(
        &mut tl,
        &mut undo,
        Op::AddGraphic {
            graphic: Graphic::lower_third("Ada"),
            start: Time::ZERO,
            duration: Duration::from_seconds(2.0),
            track_id: None,
        },
    )
    .unwrap();
    let plan = plan_frame(&tl, Time::from_seconds(0.5));
    assert!(
        plan.layers
            .iter()
            .any(|l| matches!(l, Layer::Video { .. })),
        "{plan:?}"
    );
    assert!(
        plan.layers.iter().any(|l| matches!(
            l,
            Layer::Graphic {
                graphic,
                ..
            } if graphic.text == "Ada"
        )),
        "{plan:?}"
    );
}

#[test]
fn shape_and_sticker_and_card() {
    let mut tl = Timeline::default();
    let mut undo = UndoStack::new();
    for g in [
        Graphic::shape(),
        Graphic::sticker("★"),
        Graphic::card("CARD"),
    ] {
        apply(
            &mut tl,
            &mut undo,
            Op::AddGraphic {
                graphic: g,
                start: Time::ZERO,
                duration: Duration::from_seconds(1.0),
                track_id: None,
            },
        )
        .unwrap();
    }
    let n = tl
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .filter(|c| matches!(c.kind, ClipKind::Graphic { .. }))
        .count();
    assert_eq!(n, 3);
}

#[test]
fn graphic_has_no_media_id() {
    let mut tl = Timeline::default();
    let mut undo = UndoStack::new();
    apply(
        &mut tl,
        &mut undo,
        Op::AddGraphic {
            graphic: Graphic::title("X"),
            start: Time::ZERO,
            duration: Duration::from_seconds(2.0),
            track_id: None,
        },
    )
    .unwrap();
    let clip = tl
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .find(|c| matches!(c.kind, ClipKind::Graphic { .. }))
        .unwrap();
    assert!(clip.media_id.is_none());
}

#[test]
fn clear_timeline_drops_graphics() {
    let mut tl = Timeline::default();
    let mut undo = UndoStack::new();
    apply(
        &mut tl,
        &mut undo,
        Op::AddGraphic {
            graphic: Graphic::title("X"),
            start: Time::ZERO,
            duration: Duration::from_seconds(2.0),
            track_id: None,
        },
    )
    .unwrap();
    apply(&mut tl, &mut undo, Op::ClearTimeline).unwrap();
    assert!(tl.tracks.iter().all(|t| t.clips.is_empty()));
}

#[test]
fn explanation_drawing_sits_behind_the_speaker() {
    let mut tl = Timeline::default();
    let mut undo = UndoStack::new();
    let v1 = tl.first_track(TrackKind::Video).unwrap().id;
    let speaker = MediaId::new();
    tl.add_clip(v1, video_on(speaker, 0.0, 8.0)).unwrap();
    apply(
        &mut tl,
        &mut undo,
        Op::AddDesign {
            media_id: MediaId::new(),
            at: Time::from_seconds(1.5),
            duration: Duration::from_seconds(3.0),
            layout: DesignLayout::Behind,
            text: "Head and shoulders".into(),
        },
    )
    .unwrap();
    let design = tl.tracks.iter().find(|t| t.name == "Design").unwrap();
    assert!(design.clips[0].look.overlay);
    assert!(design.clips[0].look.move_to.is_some());
    let front = tl.tracks.iter().find(|t| t.name == "Front").unwrap();
    assert_eq!(front.clips[0].media_id, Some(speaker));
    assert!(front.clips[0].look.card.is_some());
    let label = tl
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .find(|c| matches!(c.kind, ClipKind::Graphic { .. }))
        .unwrap();
    match &label.kind {
        ClipKind::Graphic { graphic } => {
            assert_eq!(graphic.text, "Head and shoulders");
            assert_eq!(graphic.kind, GraphicKind::LowerThird);
            assert_eq!(graphic.x, Some(0.28));
            assert_eq!(graphic.y, Some(0.42));
        }
        other => panic!("{other:?}"),
    }
    let review = oc_core::review_cut(
        &tl,
        &[oc_core::Spoken {
            media: speaker,
            start: 0.0,
            end: 8.0,
            text: "this is a head and shoulders pattern".into(),
        }],
        "show the pattern",
    );
    assert!(
        !review.text.contains("stacked"),
        "the corner window is the same person, not a second talk track: {}",
        review.text
    );
}

#[test]
fn house_drawing_sits_beside_the_speaker() {
    let mut tl = Timeline::default();
    let mut undo = UndoStack::new();
    let v1 = tl.first_track(TrackKind::Video).unwrap().id;
    tl.add_clip(v1, video(0.0, 8.0)).unwrap();
    apply(
        &mut tl,
        &mut undo,
        Op::AddDesign {
            media_id: MediaId::new(),
            at: Time::from_seconds(2.0),
            duration: Duration::from_seconds(4.0),
            layout: DesignLayout::Beside,
            text: "Three-bed house".into(),
        },
    )
    .unwrap();
    let design = tl.tracks.iter().find(|t| t.name == "Design").unwrap();
    assert!(design.clips[0].look.card.is_some());
    assert!(tl.tracks.iter().all(|t| t.name != "Front"));
}

#[test]
fn graphic_fade_hits_opacity() {
    let mut tl = Timeline::default();
    let mut undo = UndoStack::new();
    apply(
        &mut tl,
        &mut undo,
        Op::AddGraphic {
            graphic: Graphic::title("X"),
            start: Time::ZERO,
            duration: Duration::from_seconds(2.0),
            track_id: None,
        },
    )
    .unwrap();
    let id = tl
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .find(|c| matches!(c.kind, ClipKind::Graphic { .. }))
        .unwrap()
        .id;
    apply(
        &mut tl,
        &mut undo,
        Op::SetFade {
            clip_id: id,
            fade_in: Duration::from_seconds(1.0),
            fade_out: Duration::ZERO,
        },
    )
    .unwrap();
    let plan = plan_frame(&tl, Time::from_seconds(0.25));
    match plan
        .layers
        .iter()
        .find(|l| matches!(l, Layer::Graphic { .. }))
    {
        Some(Layer::Graphic { opacity, .. }) => {
            assert!((*opacity - 0.25).abs() < 0.02, "{opacity}");
        }
        other => panic!("{other:?}"),
    }
}
