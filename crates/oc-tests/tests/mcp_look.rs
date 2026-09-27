//! MCP schemas and TOOL-line parsing for the new look / mix / graphic ops.

use oc_core::{
    GraphicKind, McpCall, Op, TransitionKind, inspect_from_mcp, mcp_tools, op_from_mcp,
};
use serde_json::json;

fn call(name: &str, arguments: serde_json::Value) -> McpCall {
    McpCall {
        name: name.into(),
        arguments,
    }
}

fn clip() -> &'static str {
    "11111111-1111-1111-1111-111111111111"
}

#[test]
fn catalog_lists_look_tools() {
    let names: Vec<_> = mcp_tools().into_iter().map(|t| t.name).collect();
    for need in [
        "set_transition",
        "set_grade",
        "set_fx",
        "set_fade",
        "set_volume",
        "add_title",
        "list_cues",
        "place_clip",
        "clear_timeline",
        "set_mix",
        "set_curves",
        "set_mask",
        "set_speed_keys",
        "add_generator",
    ] {
        assert!(names.contains(&need.to_string()), "missing {need} in {names:?}");
    }
}

#[test]
fn parse_transition_kinds() {
    for (raw, want) in [
        ("dissolve", TransitionKind::Dissolve),
        ("slide", TransitionKind::SlideRight),
        ("wipe", TransitionKind::WipeLeft),
        ("cut", TransitionKind::Cut),
        ("fade_black", TransitionKind::FadeBlack),
        ("circleopen", TransitionKind::CircleOpen),
        ("wipe_right", TransitionKind::WipeRight),
    ] {
        let op = op_from_mcp(&call(
            "set_transition",
            json!({ "clip_id": clip(), "kind": raw }),
        ))
        .unwrap();
        match op {
            Op::SetTransition { kind, .. } => assert_eq!(kind, want),
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn parse_add_title_kinds() {
    let op = op_from_mcp(&call(
        "add_title",
        json!({ "kind": "lower_third", "text": "Ada", "start": 1.5, "duration": 2.0 }),
    ))
    .unwrap();
    match op {
        Op::AddGraphic { graphic, start, duration, .. } => {
            assert_eq!(graphic.kind, GraphicKind::LowerThird);
            assert_eq!(graphic.text, "Ada");
            assert!((start.as_seconds() - 1.5).abs() < 1e-6);
            assert!((duration.as_seconds() - 2.0).abs() < 1e-6);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn parse_place_clip_excerpt() {
    let op = op_from_mcp(&call(
        "place_clip",
        json!({
            "media_id": clip(),
            "start": 0,
            "source_in": 12.4,
            "duration": 3.8
        }),
    ))
    .unwrap();
    match op {
        Op::PlaceMedia {
            source_in,
            duration,
            ..
        } => {
            assert!((source_in.as_seconds() - 12.4).abs() < 1e-6);
            assert!((duration.as_seconds() - 3.8).abs() < 1e-6);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn parse_mixer_curves_mask_remap_and_generator() {
    let mix = op_from_mcp(&call(
        "set_mix",
        json!({ "gain_db": -3.0, "pan": 0.5 }),
    ))
    .unwrap();
    match mix {
        Op::SetMix { track_id: None, mix } => {
            assert!((mix.gain_db + 3.0).abs() < 1e-3);
            assert!((mix.pan - 0.5).abs() < 1e-3);
        }
        other => panic!("{other:?}"),
    }

    let curves = op_from_mcp(&call(
        "set_curves",
        json!({ "clip_id": clip(), "channel": "red", "mid": 0.7 }),
    ))
    .unwrap();
    match curves {
        Op::SetCurves { curves, .. } => {
            assert!(curves.all.is_empty());
            assert_eq!(curves.red.len(), 3);
            assert!((curves.red[1].y - 0.7).abs() < 1e-3);
        }
        other => panic!("{other:?}"),
    }

    let mask = op_from_mcp(&call(
        "set_mask",
        json!({ "clip_id": clip(), "shape": "ellipse", "invert": true }),
    ))
    .unwrap();
    match mask {
        Op::SetMask { mask: Some(shape), .. } => {
            assert_eq!(shape.shape, oc_core::MaskShape::Ellipse);
            assert!(shape.invert);
        }
        other => panic!("{other:?}"),
    }

    let cleared = op_from_mcp(&call(
        "set_mask",
        json!({ "clip_id": clip(), "clear": true }),
    ))
    .unwrap();
    assert!(matches!(cleared, Op::SetMask { mask: None, .. }));

    let keys = op_from_mcp(&call(
        "set_speed_keys",
        json!({ "clip_id": clip(), "keys": [{"at": 0, "speed": 1}, {"at": 0.4, "speed": 2.5}, {"at": 1, "speed": 1}] }),
    ))
    .unwrap();
    match keys {
        Op::SetSpeedKeys { keys, .. } => assert_eq!(keys.len(), 3),
        other => panic!("{other:?}"),
    }

    let bars = op_from_mcp(&call(
        "add_generator",
        json!({ "kind": "color_bars", "at": 2.0, "duration": 4.0 }),
    ))
    .unwrap();
    match bars {
        Op::AddGenerator {
            generator: oc_core::Generator::ColorBars,
            at,
            duration,
        } => {
            assert!((at.as_seconds() - 2.0).abs() < 1e-6);
            assert!((duration.as_seconds() - 4.0).abs() < 1e-6);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
#[test]
fn grade_defaults_are_zero_and_transition_keeps_duration() {
    let grade = op_from_mcp(&call("set_grade", json!({ "clip_id": clip(), "lut": "film" }))).unwrap();
    match grade {
        Op::SetGrade { grade, .. } => {
            assert_eq!(grade.exposure, 0.0);
            assert_eq!(grade.contrast, 0.0);
            assert_eq!(grade.saturation, 0.0);
            assert_eq!(grade.temperature, 0.0);
            assert_eq!(grade.lut, oc_core::Lut::Film);
        }
        other => panic!("{other:?}"),
    }
    let fx = op_from_mcp(&call("set_fx", json!({ "clip_id": clip() }))).unwrap();
    match fx {
        Op::SetFx { fx, .. } => {
            assert_eq!(fx.blur, 0.0);
            assert_eq!(fx.grain, 0.0);
            assert_eq!(fx.vignette, 0.0);
        }
        other => panic!("{other:?}"),
    }
    let mix = op_from_mcp(&call(
        "set_transition",
        json!({ "clip_id": clip(), "kind": "dissolve", "duration": 0.4 }),
    ))
    .unwrap();
    match mix {
        Op::SetTransition { duration, .. } => assert_eq!(duration, Some(0.4)),
        other => panic!("{other:?}"),
    }
    let batched = op_from_mcp(&call(
        "set_grade",
        json!({ "all": true, "exposure": 0.1 }),
    ))
    .unwrap();
    assert!(matches!(batched, Op::StyleClips { all: true, .. }));
}

#[test]
fn parse_fade_and_volume() {
    let fade = op_from_mcp(&call(
        "set_fade",
        json!({ "clip_id": clip(), "fade_in": 0.5, "fade_out": 1.0 }),
    ))
    .unwrap();
    match fade {
        Op::SetFade {
            fade_in, fade_out, ..
        } => {
            assert!((fade_in.as_seconds() - 0.5).abs() < 1e-6);
            assert!((fade_out.as_seconds() - 1.0).abs() < 1e-6);
        }
        other => panic!("{other:?}"),
    }
    let vol = op_from_mcp(&call(
        "set_volume",
        json!({ "clip_id": clip(), "volume": 0.35 }),
    ))
    .unwrap();
    match vol {
        Op::SetVolume { volume, .. } => assert!((volume - 0.35).abs() < 1e-4),
        other => panic!("{other:?}"),
    }
}

#[test]
fn list_cues_is_inspect_not_an_op() {
    let c = call("list_cues", json!({ "media_id": clip() }));
    assert!(inspect_from_mcp(&c).is_some());
    assert!(op_from_mcp(&c).is_err());
}

#[test]
fn unknown_tool_errors() {
    assert!(op_from_mcp(&call("explode", json!({}))).is_err());
}

#[test]
fn pick_reel_from_cues_skips_filler() {
    use oc_core::{Time, pick_reel_excerpts};
    let cues = [
        (Time::from_seconds(0.0), Time::from_seconds(0.4), "um"),
        (
            Time::from_seconds(2.0),
            Time::from_seconds(6.0),
            "what if we left tonight?",
        ),
        (
            Time::from_seconds(20.0),
            Time::from_seconds(24.0),
            "the road was empty",
        ),
        (
            Time::from_seconds(40.0),
            Time::from_seconds(45.0),
            "we made it by dawn",
        ),
    ];
    let takes = pick_reel_excerpts(&cues, 30.0);
    assert!(takes.len() >= 2, "{takes:?}");
    assert!(takes[0].source_in.as_seconds() >= 1.5);
}
