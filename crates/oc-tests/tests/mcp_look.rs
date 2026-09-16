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
