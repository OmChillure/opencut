//! MCP tool list so any LLM provider can call the same edits as the UI.

use crate::ops::{AssembleItem, AssembleStyle, ExportPreset, Op, TimeRange, TimelineEditMode};
use oc_timeline::{Fx, Grade, Graphic, GraphicKind, TransitionKind};
use crate::registry::tools;
use crate::ToolGroup;
use oc_time::{Duration, Time};
use oc_timeline::{ClipId, MediaId, TrackId, TrackKind};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpCall {
    pub name: String,
    pub arguments: Value,
}

/// Read-only workspace queries. The host answers these; they are not timeline ops.
#[derive(Clone, Debug)]
pub enum Inspect {
    ListBin,
    ListTimeline,
    GetMedia { media_id: MediaId },
    ListCues { media_id: MediaId },
}

#[must_use]
pub fn inspect_from_mcp(call: &McpCall) -> Option<Inspect> {
    match call.name.as_str() {
        "list_bin" => Some(Inspect::ListBin),
        "list_timeline" => Some(Inspect::ListTimeline),
        "get_media" => media_id(&call.arguments, "media_id")
            .ok()
            .map(|media_id| Inspect::GetMedia { media_id }),
        "list_cues" => media_id(&call.arguments, "media_id")
            .ok()
            .map(|media_id| Inspect::ListCues { media_id }),
        _ => None,
    }
}

/// Tools an AI provider may call. Modes (select/razor/…) stay UI-only.
#[must_use]
pub fn mcp_tools() -> Vec<McpTool> {
    let mut out = Vec::new();
    for spec in tools() {
        if spec.group == ToolGroup::Modes {
            continue;
        }
        if let Some(tool) = mcp_for_id(spec.id, spec.label, spec.tip) {
            out.push(tool);
        }
    }
    out.extend([
        McpTool {
            name: "list_bin".into(),
            description:
                "List imported media in this project (id, kind, duration, speech/look summary). \
                 Call this when you need the bin. Do not assume it is empty."
                    .into(),
            input_schema: object(&[]),
        },
        McpTool {
            name: "list_timeline".into(),
            description: "List tracks and clips currently on the timeline. Call when you need the cut.".into(),
            input_schema: object(&[]),
        },
        McpTool {
            name: "get_media".into(),
            description:
                "Details for one media id: duration, timestamped speech cues, look. \
                 Call this (or list_cues) before cutting a long file."
                    .into(),
            input_schema: object(&[("media_id", str_prop("Media id from list_bin"), true)]),
        },
        McpTool {
            name: "list_cues".into(),
            description:
                "Timestamped speech cues for one media id (source seconds + text). \
                 Use this to pick excerpts from a long take."
                    .into(),
            input_schema: object(&[("media_id", str_prop("Media id from list_bin"), true)]),
        },
        McpTool {
            name: "clear_timeline".into(),
            description: "Remove every clip from the timeline. Use before a new cut.".into(),
            input_schema: object(&[]),
        },
        McpTool {
            name: "set_transition".into(),
            description: "Same-track mix at this clip's outgoing cut. \
                 kind: cut, dissolve, fade_black, fade_white, slide_left/right/up/down, \
                 wipe_left/right/up/down, circle_open, radial, pixelize, …"
                .into(),
            input_schema: object(&[
                ("clip_id", str_prop("Outgoing clip id"), true),
                ("kind", str_prop("xfade / editor name, e.g. dissolve, wipe_left"), true),
            ]),
        },
        McpTool {
            name: "set_grade".into(),
            description: "Color grade on a clip (exposure, contrast, saturation, temperature)."
                .into(),
            input_schema: object(&[("clip_id", str_prop("Clip id"), true)]),
        },
        McpTool {
            name: "set_fx".into(),
            description: "Blur, grain, vignette on a clip.".into(),
            input_schema: object(&[("clip_id", str_prop("Clip id"), true)]),
        },
        McpTool {
            name: "set_fade".into(),
            description: "Fade in/out on a clip (seconds).".into(),
            input_schema: object(&[
                ("clip_id", str_prop("Clip id"), true),
                ("fade_in", num_prop("Fade in seconds"), false),
                ("fade_out", num_prop("Fade out seconds"), false),
            ]),
        },
        McpTool {
            name: "set_volume".into(),
            description: "Set audio clip volume (1.0 = unity).".into(),
            input_schema: object(&[
                ("clip_id", str_prop("Clip id"), true),
                ("volume", num_prop("Volume 0-2"), true),
            ]),
        },
        McpTool {
            name: "add_title".into(),
            description: "Put a title, lower third, card, shape, or sticker on the picture."
                .into(),
            input_schema: object(&[
                ("kind", str_prop("title, lower_third, card, shape, sticker"), false),
                ("text", str_prop("On-screen text"), false),
                ("start", num_prop("Timeline start in seconds"), false),
                ("duration", num_prop("Duration in seconds"), false),
            ]),
        },
        McpTool {
            name: "move".into(),
            description: "Move a clip to a track and start time.".into(),
            input_schema: object(&[
                ("clip_id", str_prop("Clip id"), true),
                ("track_id", str_prop("Destination track id"), true),
                ("start", num_prop("Start time in seconds"), true),
            ]),
        },
        McpTool {
            name: "add_captions".into(),
            description: "Replace caption cues on the timeline.".into(),
            input_schema: object(&[(
                "cues",
                (
                    json!({
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "start": { "type": "number" },
                                "end": { "type": "number" },
                                "text": { "type": "string" }
                            },
                            "required": ["start", "end", "text"]
                        }
                    }),
                    "Caption cues",
                ),
                true,
            )]),
        },
        McpTool {
            name: "slip".into(),
            description: "Keep clip position and duration; change which source frames play.".into(),
            input_schema: object(&[
                ("clip_id", str_prop("Clip id"), true),
                ("source_in", num_prop("New source in-point in seconds"), true),
            ]),
        },
        McpTool {
            name: "roll".into(),
            description: "Move the cut between two touching clips. Sequence length stays the same.".into(),
            input_schema: object(&[
                ("clip_id", str_prop("Left clip at the join"), true),
                ("at", num_prop("New join time in seconds"), true),
            ]),
        },
        McpTool {
            name: "slide".into(),
            description: "Move a clip; neighbors absorb the time. Sequence length stays the same.".into(),
            input_schema: object(&[
                ("clip_id", str_prop("Middle clip"), true),
                ("start", num_prop("New start in seconds"), true),
            ]),
        },
        McpTool {
            name: "ripple_trim".into(),
            description: "Trim a clip and shift later clips on the same track.".into(),
            input_schema: object(&[
                ("clip_id", str_prop("Clip id"), true),
                ("start", num_prop("New start in seconds"), true),
                ("duration", num_prop("New duration in seconds"), true),
            ]),
        },
        McpTool {
            name: "rate_stretch".into(),
            description: "Change speed by stretching the clip duration.".into(),
            input_schema: object(&[
                ("clip_id", str_prop("Clip id"), true),
                ("duration", num_prop("New duration in seconds"), true),
            ]),
        },
        McpTool {
            name: "set_speed".into(),
            description: "Set playback speed. Duration changes so source length stays.".into(),
            input_schema: object(&[
                ("clip_id", str_prop("Clip id"), true),
                ("speed", num_prop("Speed, 1.0 is normal"), true),
            ]),
        },
        McpTool {
            name: "split_all".into(),
            description: "Cut every clip under this time.".into(),
            input_schema: object(&[("at", num_prop("Timeline time in seconds"), true)]),
        },
        McpTool {
            name: "multicam_cut".into(),
            description: "Cut all video tracks at this time and enable only the chosen camera.".into(),
            input_schema: object(&[
                ("track_id", str_prop("Camera track to cut to"), true),
                ("at", num_prop("Timeline time in seconds"), true),
            ]),
        },
        McpTool {
            name: "insert_space".into(),
            description: "Open a gap at this time.".into(),
            input_schema: object(&[
                ("at", num_prop("Timeline time in seconds"), true),
                ("amount", num_prop("Gap in seconds"), true),
                ("track_id", str_prop("Track, or omit for all tracks"), false),
            ]),
        },
        McpTool {
            name: "delete_space".into(),
            description: "Close the next gap at this time.".into(),
            input_schema: object(&[
                ("at", num_prop("Timeline time in seconds"), true),
                ("track_id", str_prop("Track, or omit for all tracks"), false),
            ]),
        },
        McpTool {
            name: "group".into(),
            description: "Group clips so they move together.".into(),
            input_schema: object(&[(
                "clip_ids",
                (
                    json!({ "type": "array", "items": { "type": "string" } }),
                    "Clip ids",
                ),
                true,
            )]),
        },
        McpTool {
            name: "ungroup".into(),
            description: "Ungroup the group that contains this clip.".into(),
            input_schema: object(&[("clip_id", str_prop("Any clip in the group"), true)]),
        },
        McpTool {
            name: "link".into(),
            description: "Link video and audio clips (J/L cuts stay together).".into(),
            input_schema: object(&[(
                "clip_ids",
                (
                    json!({ "type": "array", "items": { "type": "string" } }),
                    "Clip ids",
                ),
                true,
            )]),
        },
        McpTool {
            name: "unlink".into(),
            description: "Unlink this clip from its pair.".into(),
            input_schema: object(&[("clip_id", str_prop("Clip id"), true)]),
        },
        McpTool {
            name: "detach_audio".into(),
            description: "Copy this video clip's audio onto an audio track.".into(),
            input_schema: object(&[("clip_id", str_prop("Video clip id"), true)]),
        },
        McpTool {
            name: "add_marker".into(),
            description: "Add a named marker on the timeline.".into(),
            input_schema: object(&[
                ("time", num_prop("Time in seconds"), true),
                ("name", str_prop("Label"), false),
            ]),
        },
        McpTool {
            name: "set_mark_in".into(),
            description: "Set the In point for 3-point editing.".into(),
            input_schema: object(&[("time", num_prop("Time in seconds"), true)]),
        },
        McpTool {
            name: "set_mark_out".into(),
            description: "Set the Out point for 3-point editing.".into(),
            input_schema: object(&[("time", num_prop("Time in seconds"), true)]),
        },
        McpTool {
            name: "reframe".into(),
            description: "Change the project aspect ratio.".into(),
            input_schema: object(&[("aspect", str_prop("landscape, vertical, square, or tall"), true)]),
        },
        McpTool {
            name: "duck".into(),
            description: "Lower music under speech. amount is 0-1.".into(),
            input_schema: object(&[("amount", num_prop("How much to duck, 0-1"), true)]),
        },
        McpTool {
            name: "place_clip".into(),
            description:
                "Put a take on the timeline. media_id from the bin. \
                 source_in + duration select a slice of that file (required for a long source). \
                 start is where it lands on the timeline."
                    .into(),
            input_schema: object(&[
                ("media_id", str_prop("Media id from the bin"), true),
                ("start", num_prop("Timeline start in seconds (default 0)"), false),
                (
                    "source_in",
                    num_prop("In-point in the source file, seconds (default 0)"),
                    false,
                ),
                (
                    "duration",
                    num_prop("Take length in seconds (default: rest of the file)"),
                    false,
                ),
                ("track_id", str_prop("Track id, or omit for first matching track"), false),
                (
                    "mode",
                    str_prop("normal, insert, or overwrite (default insert)"),
                    false,
                ),
            ]),
        },
        McpTool {
            name: "assemble".into(),
            description: "Shortcut: lay several *bin items* into one short. \
                 Not for picking highlights inside one long file — use place_clip excerpts. \
                 Omit media_ids to use the whole bin. target_seconds defaults to ~45."
                .into(),
            input_schema: object(&[
                (
                    "media_ids",
                    (
                        json!({ "type": "array", "items": { "type": "string" } }),
                        "Media ids from the bin, in order",
                    ),
                    false,
                ),
                (
                    "style",
                    str_prop("vlog (default) or sequential"),
                    false,
                ),
                (
                    "target_seconds",
                    num_prop("Aim length in seconds (default 45)"),
                    false,
                ),
            ]),
        },
    ]);
    out
}

pub fn op_from_mcp(call: &McpCall) -> Result<Op, String> {
    match call.name.as_str() {
        "split" => Ok(Op::Split {
            clip_id: clip_id(&call.arguments, "clip_id")?,
            at: seconds(&call.arguments, "at")?,
        }),
        "merge" => Ok(Op::Merge {
            clip_id: clip_id(&call.arguments, "clip_id")?,
        }),
        "cut" | "extract" => Ok(Op::RippleDelete {
            clip_id: clip_id(&call.arguments, "clip_id")?,
        }),
        "delete" | "lift" => Ok(Op::RemoveClip {
            clip_id: clip_id(&call.arguments, "clip_id")?,
        }),
        "trim" | "trim_end" => Ok(Op::Trim {
            clip_id: clip_id(&call.arguments, "clip_id")?,
            start: seconds(&call.arguments, "start")?,
            duration: Duration::from_seconds(number(&call.arguments, "duration")?),
        }),
        "move" => Ok(Op::Move {
            clip_id: clip_id(&call.arguments, "clip_id")?,
            track_id: track_id(&call.arguments, "track_id")?,
            start: seconds(&call.arguments, "start")?,
        }),
        "export" => Ok(Op::Export {
            preset: ExportPreset::Youtube1080,
        }),
        "remove_silence" => {
            let ranges = call
                .arguments
                .get("ranges")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let ranges = ranges
                .iter()
                .filter_map(|r| {
                    Some(TimeRange {
                        start: Time::from_seconds(r.get("start")?.as_f64()?),
                        end: Time::from_seconds(r.get("end")?.as_f64()?),
                    })
                })
                .collect();
            Ok(Op::RemoveSilence { ranges })
        }
        "slip" => Ok(Op::Slip {
            clip_id: clip_id(&call.arguments, "clip_id")?,
            source_in: seconds(&call.arguments, "source_in")?,
        }),
        "roll" => Ok(Op::Roll {
            clip_id: clip_id(&call.arguments, "clip_id")?,
            at: seconds(&call.arguments, "at")?,
        }),
        "slide" => Ok(Op::Slide {
            clip_id: clip_id(&call.arguments, "clip_id")?,
            start: seconds(&call.arguments, "start")?,
        }),
        "ripple_trim" => Ok(Op::RippleTrim {
            clip_id: clip_id(&call.arguments, "clip_id")?,
            start: seconds(&call.arguments, "start")?,
            duration: Duration::from_seconds(number(&call.arguments, "duration")?),
        }),
        "rate_stretch" => Ok(Op::RateStretch {
            clip_id: clip_id(&call.arguments, "clip_id")?,
            duration: Duration::from_seconds(number(&call.arguments, "duration")?),
        }),
        "set_speed" => Ok(Op::SetSpeed {
            clip_id: clip_id(&call.arguments, "clip_id")?,
            speed: number(&call.arguments, "speed")? as f32,
        }),
        "split_all" => Ok(Op::SplitAll {
            at: seconds(&call.arguments, "at")?,
        }),
        "multicam_cut" => Ok(Op::MulticamCut {
            track_id: track_id(&call.arguments, "track_id")?,
            at: seconds(&call.arguments, "at")?,
        }),
        "insert_space" => Ok(Op::InsertSpace {
            track_id: optional_track(&call.arguments, "track_id"),
            at: seconds(&call.arguments, "at")?,
            amount: Duration::from_seconds(number(&call.arguments, "amount")?),
        }),
        "delete_space" => Ok(Op::DeleteSpace {
            track_id: optional_track(&call.arguments, "track_id"),
            at: seconds(&call.arguments, "at")?,
        }),
        "group" => Ok(Op::Group {
            clip_ids: clip_ids(&call.arguments, "clip_ids")?,
        }),
        "ungroup" => Ok(Op::Ungroup {
            clip_id: clip_id(&call.arguments, "clip_id")?,
        }),
        "link" => Ok(Op::Link {
            clip_ids: clip_ids(&call.arguments, "clip_ids")?,
        }),
        "unlink" => Ok(Op::Unlink {
            clip_id: clip_id(&call.arguments, "clip_id")?,
        }),
        "detach_audio" => Ok(Op::DetachAudio {
            clip_id: clip_id(&call.arguments, "clip_id")?,
        }),
        "add_marker" => Ok(Op::AddMarker {
            time: seconds(&call.arguments, "time")?,
            name: call
                .arguments
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("Marker")
                .into(),
            color: 0,
        }),
        "set_mark_in" => Ok(Op::SetMarkIn {
            time: Some(seconds(&call.arguments, "time")?),
        }),
        "set_mark_out" => Ok(Op::SetMarkOut {
            time: Some(seconds(&call.arguments, "time")?),
        }),
        "reframe" => {
            let raw = call
                .arguments
                .get("aspect")
                .and_then(Value::as_str)
                .unwrap_or("landscape");
            let aspect = match raw {
                "vertical" => oc_timeline::AspectRatio::Vertical,
                "square" => oc_timeline::AspectRatio::Square,
                "tall" => oc_timeline::AspectRatio::Tall,
                _ => oc_timeline::AspectRatio::Landscape,
            };
            Ok(Op::Reframe { aspect })
        }
        "duck" => Ok(Op::Duck {
            amount: number(&call.arguments, "amount").unwrap_or(0.6) as f32,
        }),
        "add_captions" => {
            let cues = call
                .arguments
                .get("cues")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let cues = cues
                .iter()
                .filter_map(|c| {
                    Some(oc_timeline::CaptionCue {
                        start: Time::from_seconds(c.get("start")?.as_f64()?),
                        end: Time::from_seconds(c.get("end")?.as_f64()?),
                        text: c.get("text")?.as_str()?.to_string(),
                        speaker: c
                            .get("speaker")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    })
                })
                .collect();
            Ok(Op::AddCaptions {
                style: oc_timeline::CaptionStyle::default(),
                cues,
            })
        }
        "list_bin" | "list_timeline" | "get_media" | "list_cues" => {
            Err("inspect tools are handled by the host".into())
        }
        "place" | "place_clip" | "place_excerpt" => Ok(Op::PlaceMedia {
            media_id: media_id(&call.arguments, "media_id")?,
            track_id: optional_track(&call.arguments, "track_id"),
            start: seconds(&call.arguments, "start").unwrap_or(Time::ZERO),
            duration: call
                .arguments
                .get("duration")
                .and_then(Value::as_f64)
                .map(Duration::from_seconds)
                .unwrap_or(Duration::ZERO),
            source_in: seconds(&call.arguments, "source_in").unwrap_or(Time::ZERO),
            kind: match call
                .arguments
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("video")
            {
                "audio" => TrackKind::Audio,
                "caption" => TrackKind::Caption,
                _ => TrackKind::Video,
            },
            mode: match call
                .arguments
                .get("mode")
                .and_then(Value::as_str)
                .unwrap_or("insert")
            {
                "overwrite" => TimelineEditMode::Overwrite,
                "normal" => TimelineEditMode::Normal,
                _ => TimelineEditMode::Insert,
            },
        }),
        "clear_timeline" | "clear" => Ok(Op::ClearTimeline),
        "set_transition" => {
            let raw = call
                .arguments
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("dissolve");
            Ok(Op::SetTransition {
                clip_id: clip_id(&call.arguments, "clip_id")?,
                kind: TransitionKind::from_key(raw),
            })
        }
        "set_grade" => Ok(Op::SetGrade {
            clip_id: clip_id(&call.arguments, "clip_id")?,
            grade: Grade {
                exposure: number(&call.arguments, "exposure").unwrap_or(0.08) as f32,
                contrast: number(&call.arguments, "contrast").unwrap_or(0.14) as f32,
                saturation: number(&call.arguments, "saturation").unwrap_or(0.12) as f32,
                temperature: number(&call.arguments, "temperature").unwrap_or(0.06) as f32,
            },
        }),
        "set_fx" => Ok(Op::SetFx {
            clip_id: clip_id(&call.arguments, "clip_id")?,
            fx: Fx {
                blur: number(&call.arguments, "blur").unwrap_or(0.0) as f32,
                grain: number(&call.arguments, "grain").unwrap_or(0.18) as f32,
                vignette: number(&call.arguments, "vignette").unwrap_or(0.35) as f32,
            },
        }),
        "set_fade" => Ok(Op::SetFade {
            clip_id: clip_id(&call.arguments, "clip_id")?,
            fade_in: Duration::from_seconds(number(&call.arguments, "fade_in").unwrap_or(0.8)),
            fade_out: Duration::from_seconds(number(&call.arguments, "fade_out").unwrap_or(0.8)),
        }),
        "set_volume" => Ok(Op::SetVolume {
            clip_id: clip_id(&call.arguments, "clip_id")?,
            volume: number(&call.arguments, "volume").unwrap_or(1.0) as f32,
        }),
        "add_title" | "add_graphic" => {
            let kind = match call
                .arguments
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("title")
            {
                "lower_third" | "lower" => GraphicKind::LowerThird,
                "card" => GraphicKind::Card,
                "shape" => GraphicKind::Shape,
                "sticker" => GraphicKind::Sticker,
                _ => GraphicKind::Title,
            };
            let text = call
                .arguments
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or(match kind {
                    GraphicKind::Title => "Title",
                    GraphicKind::LowerThird => "Name",
                    GraphicKind::Card => "Card",
                    GraphicKind::Sticker => "★",
                    GraphicKind::Shape => "",
                })
                .to_string();
            Ok(Op::AddGraphic {
                graphic: Graphic { kind, text },
                start: seconds(&call.arguments, "start").unwrap_or(Time::ZERO),
                duration: call
                    .arguments
                    .get("duration")
                    .and_then(Value::as_f64)
                    .map(Duration::from_seconds)
                    .unwrap_or(Duration::from_seconds(4.0)),
                track_id: optional_track(&call.arguments, "track_id"),
            })
        }
        "assemble" => {
            let ids = call
                .arguments
                .get("media_ids")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let items = ids
                .iter()
                .filter_map(Value::as_str)
                .filter_map(|raw| Uuid::parse_str(raw).ok())
                .map(|id| AssembleItem {
                    media_id: MediaId::from_uuid(id),
                    ..AssembleItem::default()
                })
                .collect();
            let style = match call
                .arguments
                .get("style")
                .and_then(Value::as_str)
                .unwrap_or("vlog")
            {
                "sequential" => AssembleStyle::Sequential,
                _ => AssembleStyle::Vlog,
            };
            let target_seconds = call.arguments.get("target_seconds").and_then(Value::as_f64);
            Ok(Op::Assemble {
                items,
                style,
                target_seconds,
            })
        }
        other => Err(format!("unknown tool {other}")),
    }
}

fn mcp_for_id(
    id: crate::ToolId,
    label: &'static str,
    tip: &'static str,
) -> Option<McpTool> {
    use crate::ToolId::*;
    let (name, schema) = match id {
        Split => (
            "split",
            object(&[
                ("clip_id", str_prop("Clip to cut"), true),
                ("at", num_prop("Timeline time in seconds"), true),
            ]),
        ),
        Merge => (
            "merge",
            object(&[("clip_id", str_prop("Left clip to join with the next"), true)]),
        ),
        Extract => (
            "cut",
            object(&[("clip_id", str_prop("Clip to delete, closing the gap"), true)]),
        ),
        Lift => (
            "delete",
            object(&[("clip_id", str_prop("Clip to delete, leaving a hole"), true)]),
        ),
        TrimStart | TrimEnd => (
            if matches!(id, TrimStart) {
                "trim"
            } else {
                "trim_end"
            },
            object(&[
                ("clip_id", str_prop("Clip to trim"), true),
                ("start", num_prop("New start in seconds"), true),
                ("duration", num_prop("New duration in seconds"), true),
            ]),
        ),
        SplitAll => (
            "split_all",
            object(&[("at", num_prop("Timeline time in seconds"), true)]),
        ),
        MarkIn => (
            "set_mark_in",
            object(&[("time", num_prop("Time in seconds"), true)]),
        ),
        MarkOut => (
            "set_mark_out",
            object(&[("time", num_prop("Time in seconds"), true)]),
        ),
        InsertSpace => (
            "insert_space",
            object(&[
                ("at", num_prop("Timeline time in seconds"), true),
                ("amount", num_prop("Gap in seconds"), true),
            ]),
        ),
        DeleteSpace => (
            "delete_space",
            object(&[("at", num_prop("Timeline time in seconds"), true)]),
        ),
        DetachAudio => (
            "detach_audio",
            object(&[("clip_id", str_prop("Video clip id"), true)]),
        ),
        Group => return None,
        Ungroup => (
            "ungroup",
            object(&[("clip_id", str_prop("Any clip in the group"), true)]),
        ),
        Link => return None,
        Unlink => (
            "unlink",
            object(&[("clip_id", str_prop("Clip id"), true)]),
        ),
        AddMarker => (
            "add_marker",
            object(&[
                ("time", num_prop("Time in seconds"), true),
                ("name", str_prop("Label"), false),
            ]),
        ),
        InsertAt | OverwriteAt | AddVideo | AddAudio | AddCaption => return None,
        Select | Razor | Spacer | Slip | Ripple | Roll | Slide | RateStretch | Multicam => {
            return None;
        }
    };
    Some(McpTool {
        name: name.into(),
        description: format!("{label}: {tip}"),
        input_schema: schema,
    })
}

fn object(fields: &[(&str, (Value, &str), bool)]) -> Value {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();
    for (name, (schema, _desc), req) in fields {
        properties.insert((*name).into(), schema.clone());
        if *req {
            required.push(Value::String((*name).into()));
        }
    }
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
    })
}

fn str_prop(description: &str) -> (Value, &str) {
    (json!({ "type": "string", "description": description }), description)
}

fn num_prop(description: &str) -> (Value, &str) {
    (json!({ "type": "number", "description": description }), description)
}

fn clip_id(args: &Value, key: &str) -> Result<ClipId, String> {
    let raw = args
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing {key}"))?;
    Uuid::parse_str(raw)
        .map(ClipId::from_uuid)
        .map_err(|_| format!("bad {key}"))
}

fn track_id(args: &Value, key: &str) -> Result<TrackId, String> {
    let raw = args
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing {key}"))?;
    Uuid::parse_str(raw)
        .map(TrackId::from_uuid)
        .map_err(|_| format!("bad {key}"))
}

fn number(args: &Value, key: &str) -> Result<f64, String> {
    args.get(key)
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("missing {key}"))
}

fn seconds(args: &Value, key: &str) -> Result<Time, String> {
    Ok(Time::from_seconds(number(args, key)?))
}

fn optional_track(args: &Value, key: &str) -> Option<TrackId> {
    track_id(args, key).ok()
}

fn media_id(args: &Value, key: &str) -> Result<MediaId, String> {
    let raw = args
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing {key}"))?;
    Uuid::parse_str(raw)
        .map(MediaId::from_uuid)
        .map_err(|_| format!("bad {key}"))
}

#[cfg(test)]
mod parse_tests {
    use super::*;

    #[test]
    fn place_clip_reads_source_in() {
        let call = McpCall {
            name: "place_clip".into(),
            arguments: json!({
                "media_id": "11111111-1111-1111-1111-111111111111",
                "start": 0,
                "source_in": 81.5,
                "duration": 3.2
            }),
        };
        match op_from_mcp(&call).unwrap() {
            Op::PlaceMedia {
                source_in,
                duration,
                start,
                ..
            } => {
                assert!((source_in.as_seconds() - 81.5).abs() < 1e-6);
                assert!((duration.as_seconds() - 3.2).abs() < 1e-6);
                assert_eq!(start, Time::ZERO);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn list_cues_is_inspect() {
        let call = McpCall {
            name: "list_cues".into(),
            arguments: json!({ "media_id": "11111111-1111-1111-1111-111111111111" }),
        };
        assert!(matches!(inspect_from_mcp(&call), Some(Inspect::ListCues { .. })));
    }
}

fn clip_ids(args: &Value, key: &str) -> Result<Vec<ClipId>, String> {
    let arr = args
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("missing {key}"))?;
    arr.iter()
        .filter_map(Value::as_str)
        .map(|raw| {
            Uuid::parse_str(raw)
                .map(ClipId::from_uuid)
                .map_err(|_| format!("bad {key}"))
        })
        .collect()
}
