//! MCP tool list so any LLM provider can call the same edits as the UI.

use crate::ToolGroup;
use crate::ops::{AssembleItem, AssembleStyle, ExportPreset, Op, TimeRange, TimelineEditMode};
use crate::registry::tools;
use oc_time::{Duration, Time};
use oc_timeline::{
    AlphaShape, AudioFx, CurvePoint, Curves, Fx, Generator, Grade, Graphic, GraphicKind, Lut,
    MaskShape, Mix, SpeedKey, TransitionKind,
};
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
    GetMedia {
        media_id: MediaId,
    },
    ListCues {
        media_id: MediaId,
    },
    GetMusic {
        media_id: MediaId,
    },
    FindShots {
        scale: Option<String>,
        camera: Option<String>,
        motion_dir: Option<String>,
        min_quality: Option<u8>,
        subject: Option<String>,
        limit: usize,
    },
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
        "get_music" => media_id(&call.arguments, "media_id")
            .ok()
            .map(|media_id| Inspect::GetMusic { media_id }),
        "find_shots" => Some(Inspect::FindShots {
            scale: call
                .arguments
                .get("scale")
                .and_then(Value::as_str)
                .map(str::to_string),
            camera: call
                .arguments
                .get("camera")
                .and_then(Value::as_str)
                .map(str::to_string),
            motion_dir: call
                .arguments
                .get("motion_dir")
                .and_then(Value::as_str)
                .map(str::to_string),
            min_quality: number(&call.arguments, "min_quality")
                .ok()
                .map(|n| n.clamp(1.0, 10.0) as u8),
            subject: call
                .arguments
                .get("subject")
                .and_then(Value::as_str)
                .map(str::to_string),
            limit: number(&call.arguments, "limit")
                .ok()
                .map(|n| n as usize)
                .unwrap_or(24),
        }),
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
            name: "see".into(),
            description:
                "Grab one frame of a video at any source time, in seconds. You see the JPEG. \
                 Call this whenever you need to look during the edit."
                    .into(),
            input_schema: object(&[
                ("media_id", str_prop("Media id from list_bin"), true),
                ("at", num_prop("Source time in seconds"), true),
            ]),
        },
        McpTool {
            name: "get_media".into(),
            description:
                "Shot list for one media id. Each range has a scale (wide/medium/close/detail), \
                 a subject (person, product, street, screen, interior, landscape, object), \
                 camera, motion (l2r/r2l/toward/away/none), q (1-10), \
                 a role (speech, silence, filler), and the words in that range. \
                 Shots under q5 are omitted. A row with only wide/close/action has not been \
                 labeled yet. Call this before cutting a long file. Place excerpts on those times."
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
                ("clip_id", str_prop("Outgoing clip id"), false),
                ("clip_ids", str_prop("JSON array of clip ids"), false),
                ("all", str_prop("true sets every video clip"), false),
                ("kind", str_prop("xfade / editor name, e.g. dissolve, wipe_left"), true),
                ("duration", num_prop("Mix length in seconds"), false),
            ]),
        },
        McpTool {
            name: "set_grade".into(),
            description: "Color on a clip. Every number defaults to 0 (no change). \
                 lut: none, film, cool, warm, teal_orange, mono. \
                 Pass clip_ids or all=true to style many clips at once."
                .into(),
            input_schema: object(&[
                ("clip_id", str_prop("Clip id"), false),
                ("clip_ids", str_prop("JSON array of clip ids"), false),
                ("all", str_prop("true styles every video clip"), false),
                ("exposure", num_prop("Exposure, 0 = unchanged"), false),
                ("contrast", num_prop("Contrast, 0 = unchanged"), false),
                ("saturation", num_prop("Saturation, 0 = unchanged"), false),
                ("temperature", num_prop("Temperature, 0 = unchanged"), false),
                ("lift", num_prop("Shadows -1..1"), false),
                ("gamma", num_prop("Mids -1..1"), false),
                ("gain", num_prop("Highlights -1..1"), false),
                ("lut", str_prop("none, film, cool, warm, teal_orange, mono"), false),
            ]),
        },
        McpTool {
            name: "set_fx".into(),
            description: "Blur, grain, and vignette. Each defaults to 0. \
                 Pass clip_ids or all=true to style many clips."
                .into(),
            input_schema: object(&[
                ("clip_id", str_prop("Clip id"), false),
                ("clip_ids", str_prop("JSON array of clip ids"), false),
                ("all", str_prop("true styles every video clip"), false),
                ("blur", num_prop("Blur, 0 = none"), false),
                ("grain", num_prop("Grain, 0 = none"), false),
                ("vignette", num_prop("Vignette, 0 = none"), false),
            ]),
        },
        McpTool {
            name: "set_transform".into(),
            description: "Zoom and pan a video clip. scale 1 is the full frame. \
                 x and y are fractions of the frame, 0 is centered."
                .into(),
            input_schema: object(&[
                ("clip_id", str_prop("Clip id"), true),
                ("scale", num_prop("Zoom, 1 = fit"), false),
                ("x", num_prop("Pan X, fraction of the frame"), false),
                ("y", num_prop("Pan Y, fraction of the frame"), false),
                ("rotation", num_prop("Degrees"), false),
            ]),
        },
        McpTool {
            name: "cover".into(),
            description: "Lay another picture over a timeline time, audio muted. \
                 Use a silent range or a second camera to hide a jump cut."
                .into(),
            input_schema: object(&[
                ("media_id", str_prop("Media id"), true),
                ("at", num_prop("Timeline start seconds"), true),
                ("source_in", num_prop("Source in-point seconds"), true),
                ("duration", num_prop("Seconds on screen"), true),
            ]),
        },
        McpTool {
            name: "set_move".into(),
            description: "Animate zoom and pan across the clip. end_x and end_y are fractions of the frame. \
                 ease is linear, in, out, or in_out."
                .into(),
            input_schema: object(&[
                ("clip_id", str_prop("Clip id"), true),
                ("end_scale", num_prop("Zoom at the tail"), true),
                ("end_x", num_prop("Pan X at the tail, fraction"), false),
                ("end_y", num_prop("Pan Y at the tail, fraction"), false),
                ("ease", str_prop("linear, in, out, in_out"), false),
            ]),
        },
        McpTool {
            name: "set_speed_ramp".into(),
            description: "Speed at the head and at the tail. 1 is normal. Use for a ramp, not a jump cut. \
                 For a change in the middle, use set_speed_keys."
                .into(),
            input_schema: object(&[
                ("clip_id", str_prop("Clip id"), true),
                ("speed", num_prop("Start speed"), true),
                ("end_speed", num_prop("End speed"), true),
            ]),
        },
        McpTool {
            name: "set_speed_keys".into(),
            description: "Time remap. keys are points along the clip: at is 0 at the head and 1 at the tail, \
                 speed is the rate there (1 is normal, 0.25..4). The picture ramps between the keys. \
                 Use this when the speed change is in the middle. set_speed_ramp is enough for head-to-tail."
                .into(),
            input_schema: object(&[
                ("clip_id", str_prop("Clip id"), true),
                (
                    "keys",
                    (
                        json!({
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "at": { "type": "number", "description": "0..1 along the clip" },
                                    "speed": { "type": "number", "description": "Playback rate" }
                                },
                                "required": ["at", "speed"]
                            }
                        }),
                        "Speed keys",
                    ),
                    true,
                ),
            ]),
        },
        McpTool {
            name: "set_mix".into(),
            description: "Audio mixer strip, like Kdenlive. gain_db 0 is unity, pan -1 is left and 1 is right, \
                 solo isolates the track. Pass a track_id from list_timeline. Omit track_id to set the master fader. \
                 Solo does not clear the other strips: set solo false on them yourself for an exclusive solo."
                .into(),
            input_schema: object(&[
                ("track_id", str_prop("Audio track id. Omit for the master."), false),
                ("gain_db", num_prop("Fader dB, -60..12. 0 is unity."), false),
                ("pan", num_prop("Balance -1..1"), false),
                ("solo", str_prop("true or false"), false),
            ]),
        },
        McpTool {
            name: "set_curves".into(),
            description: "Curves (avfilter) on one clip. channel is all, red, green, or blue. \
                 mid is the output of the midpoint: 0.5 is a straight line, below darkens the mids, above lifts them. \
                 Or pass points as {x, y} from 0 to 1. One call replaces the whole curve, so name the channel you mean."
                .into(),
            input_schema: object(&[
                ("clip_id", str_prop("Clip id"), true),
                ("channel", str_prop("all, red, green, or blue"), false),
                ("mid", num_prop("Midpoint output 0..1. 0.5 is unchanged."), false),
            ]),
        },
        McpTool {
            name: "set_mask".into(),
            description: "Alpha shape on a clip so the track below shows outside it. \
                 shape: rectangle, ellipse, triangle, diamond. x and y are the center (0..1), w and h are the size (0..1). \
                 feather is 0..1. invert keeps the outside instead. clear true removes the mask."
                .into(),
            input_schema: object(&[
                ("clip_id", str_prop("Clip id"), true),
                ("shape", str_prop("rectangle, ellipse, triangle, diamond"), false),
                ("x", num_prop("Center X 0..1"), false),
                ("y", num_prop("Center Y 0..1"), false),
                ("w", num_prop("Width 0..1"), false),
                ("h", num_prop("Height 0..1"), false),
                ("feather", num_prop("Feather 0..1"), false),
                ("invert", str_prop("true or false"), false),
                ("clear", str_prop("true removes the mask"), false),
            ]),
        },
        McpTool {
            name: "add_generator".into(),
            description: "Insert a generated clip. These are Kdenlive generators, not footage of the scene: \
                 color (a solid frame; pass color as #rrggbb), color_bars (SMPTE), white_noise (snow plus a noise bed), \
                 counter (a clock plus a 1 kHz tone). Use them for a slate, a hold, bars, a noise bed, or a countdown. \
                 Do not use them as B-roll of the story. Real B-roll is an imported file via place_clip or cover."
                .into(),
            input_schema: object(&[
                ("kind", str_prop("color, color_bars, white_noise, counter"), true),
                ("color", str_prop("#rrggbb, only for kind color"), false),
                ("at", num_prop("Timeline start in seconds"), false),
                ("duration", num_prop("Seconds"), false),
            ]),
        },
        McpTool {
            name: "set_stabilize".into(),
            description: "Stabilize a shaky video clip."
                .into(),
            input_schema: object(&[("clip_id", str_prop("Clip id"), true)]),
        },
        McpTool {
            name: "set_crop".into(),
            description: "Keep a region of the frame. x, y, w, h are fractions from 0 to 1."
                .into(),
            input_schema: object(&[
                ("clip_id", str_prop("Clip id"), true),
                ("x", num_prop("Left 0-1"), true),
                ("y", num_prop("Top 0-1"), true),
                ("w", num_prop("Width 0-1"), true),
                ("h", num_prop("Height 0-1"), true),
            ]),
        },
        McpTool {
            name: "set_audio".into(),
            description: "Normalize loudness, reduce noise, EQ, or compress. Music is only a file the user imported."
                .into(),
            input_schema: object(&[
                ("clip_id", str_prop("Clip id"), true),
                ("normalize", str_prop("true or false"), false),
                ("denoise", str_prop("true or false"), false),
                ("compressor", str_prop("true or false"), false),
                ("low", num_prop("Low shelf dB"), false),
                ("mid", num_prop("Mid dB"), false),
                ("high", num_prop("High shelf dB"), false),
            ]),
        },
        McpTool {
            name: "submit_edit".into(),
            description: "Build the whole cut from a plan. Rust places the slots and snaps to the beat. \
                 It applies grade, fx, fades, cover, music volume, captions, and letterbox only when \
                 you set them. Omit a field to leave that shot alone. Mix the joins: omit transition \
                 for a cut, and set a dissolve, wipe, or slide only on the slot that needs it. Leave \
                 caption_look out; Rust styles each caption line from the words and the shot. Slots \
                 must sit on a real shot or spoken line. A clean review queues the export."
                .into(),
            input_schema: object(&[
                ("style", str_prop("Note for yourself. It does not pick a look."), false),
                ("aspect", str_prop("landscape, vertical, square — omit to keep the project frame"), false),
                ("letterbox", str_prop("true only when the piece should be widescreen"), false),
                ("music_id", str_prop("Imported music file, when the piece needs a bed"), false),
                ("music_volume", str_prop("0–1. Set below 1 to duck the bed under speech"), false),
                ("captions", str_prop("true only when the words should be on screen"), false),
                (
                    "caption_mood",
                    str_prop(
                        "Omit. Shortcut only: clean, kinetic, or bold. Rust already styles each line.",
                    ),
                    false,
                ),
                (
                    "caption_look",
                    str_prop(
                        "Omit this. Set one key only when that kind of line should differ. JSON string. Keys: hook, punch, explain, question, number, on_face. Example: {\"hook\":\"top display typewriter\"}. Any other key is chosen from the words and the shot.",
                    ),
                    false,
                ),
                ("grade", str_prop("Piece grade for slots that omit their own. Omit for no grade."), false),
                (
                    "slots",
                    str_prop(
                        "Array of {media_id, source_in, duration, transition?, transition_duration?, speed?, end_scale?, grade?, fx?, fade_in?, fade_out?, cover?}. Omit transition for a cut. dissolve, fade_black, fade_white, wipe_left, slide_up on the slot that needs that join. end_scale about 1.08 is a push.",
                    ),
                    true,
                ),
            ]),
        },
        McpTool {
            name: "revise_edit".into(),
            description: "Change slots in the saved plan and rebuild. changes: [{slot, ...fields}]. \
                 slot is the 0-based index."
                .into(),
            input_schema: object(&[("changes", str_prop("Array of slot patches"), true)]),
        },
        McpTool {
            name: "get_music".into(),
            description: "BPM, sections, and beats for an audio file. Compact."
                .into(),
            input_schema: object(&[("media_id", str_prop("Music file id"), true)]),
        },
        McpTool {
            name: "find_shots".into(),
            description: "Search every file's shot list. scale, camera, motion_dir, min_quality, subject, limit."
                .into(),
            input_schema: object(&[
                ("scale", str_prop("ECU, CU, MS, WS, EWS"), false),
                ("camera", str_prop("static, pan_l, push_in, …"), false),
                ("motion_dir", str_prop("l2r, r2l, toward, away, none"), false),
                ("min_quality", num_prop("1-10"), false),
                ("subject", str_prop("person, product, street, …"), false),
                ("limit", num_prop("Max rows"), false),
            ]),
        },
        McpTool {
            name: "snap_cuts_to_beats".into(),
            description: "Move each video join to the nearest beat of media_id, within tolerance_frames. Ripples later clips."
                .into(),
            input_schema: object(&[
                ("media_id", str_prop("Music file"), true),
                ("tolerance_frames", num_prop("Frames, default 2"), false),
            ]),
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
            description: "Replace caption cues. Omit caption_look and Rust styles each line from the words and the shot. Set caption_look only to override one kind of line. Set place, font, or effect on a cue only to lock that line.".into(),
            input_schema: object(&[
                (
                    "caption_look",
                    str_prop(
                        "Omit. One key only, same as submit_edit caption_look. Wins over caption_mood.",
                    ),
                    false,
                ),
                (
                    "caption_mood",
                    str_prop("Shortcut when caption_look is omitted: clean, kinetic, or bold. Default clean."),
                    false,
                ),
                (
                    "cues",
                    (
                        json!({
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "start": { "type": "number" },
                                    "end": { "type": "number" },
                                    "text": { "type": "string" },
                                    "place": { "type": "string", "description": "bottom, lower, middle, top" },
                                    "font": { "type": "string", "description": "sans, display, serif, mono" },
                                    "effect": { "type": "string", "description": "pop, typewriter, fade, none" }
                                },
                                "required": ["start", "end", "text"]
                            }
                        }),
                        "Caption cues",
                    ),
                    true,
                ),
            ]),
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
        McpTool {
            name: "group".into(),
            description: "Group two or more clips so they select together.".into(),
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
            name: "link".into(),
            description: "Link clips so a move or split keeps them together. Picture and its sound.".into(),
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
            name: "insert".into(),
            description: "Insert a take and push later clips right. Same fields as place_clip.".into(),
            input_schema: object(&[
                ("media_id", str_prop("Media id from the bin"), true),
                ("start", num_prop("Timeline start in seconds"), true),
                ("source_in", num_prop("In-point in the source file, seconds"), false),
                ("duration", num_prop("Take length in seconds"), false),
                ("track_id", str_prop("Track id, or omit for the first matching track"), false),
            ]),
        },
        McpTool {
            name: "overwrite".into(),
            description: "Overwrite the timeline with a take. Same fields as place_clip.".into(),
            input_schema: object(&[
                ("media_id", str_prop("Media id from the bin"), true),
                ("start", num_prop("Timeline start in seconds"), true),
                ("source_in", num_prop("In-point in the source file, seconds"), false),
                ("duration", num_prop("Take length in seconds"), false),
                ("track_id", str_prop("Track id, or omit for the first matching track"), false),
            ]),
        },
        McpTool {
            name: "jl_cut".into(),
            description: "Split picture and sound at a join. lead is how early the audio starts (J). tail is how long the audio holds after the picture (L), in seconds. The clip must be video; sound moves to the audio track.".into(),
            input_schema: object(&[
                ("clip_id", str_prop("Video clip id"), true),
                ("lead", num_prop("Audio lead in seconds (J-cut). 0 if unused"), false),
                ("tail", num_prop("Audio tail in seconds (L-cut). 0 if unused"), false),
            ]),
        },
        McpTool {
            name: "set_frame_rate".into(),
            description: "Set the project frame rate. Existing clips stay put. fps is 23.976, 24, 25, 29.97, 30, 50, 59.94, or 60.".into(),
            input_schema: object(&[("fps", num_prop("Frames per second"), true)]),
        },
        McpTool {
            name: "set_background".into(),
            description: "Monitor and letterbox color. #rrggbb, or black, white, gray, charcoal.".into(),
            input_schema: object(&[("color", str_prop("#rrggbb or a name"), true)]),
        },
        McpTool {
            name: "import_cube".into(),
            description: "Load a .cube LUT onto one video clip. Pass the file text. Replaces the named LUT preset on that clip.".into(),
            input_schema: object(&[
                ("clip_id", str_prop("Video clip id"), true),
                ("text", str_prop("Full .cube file text"), true),
            ]),
        },
        McpTool {
            name: "add_design".into(),
            description: "Animate what the person is explaining, full frame, for the whole spoken line. \
                 A bullish flag is that chart pattern drawing itself (pole, flag, breakout), not a flag on a pole. \
                 A house rises until it fills the frame. \
                 prompt is the motion across the clip. The subject fills the frame. No letters and no numbers. \
                 text is the short label, the real words, drawn by the editor. Omit text to show the animation alone. \
                 at is the timeline start. duration is the length of that spoken line, up to 15 seconds. \
                 layout: cutaway (animation fills the frame, voice continues), \
                 beside (animation on the empty side, person stays large), \
                 behind (animation and words fill the frame, person stays in a corner in front of the words). \
                 Call see at the start, middle, and end of the line first, then pick the layout from those frames."
                .into(),
            input_schema: object(&[
                ("prompt", str_prop("How the subject builds across the clip until it fills the frame, with no words in the picture"), true),
                ("at", num_prop("Timeline start in seconds"), true),
                ("duration", num_prop("Length of the spoken line in seconds, up to 15"), false),
                ("text", str_prop("Short label, the words they actually said"), false),
                (
                    "layout",
                    str_prop("cutaway, beside, or behind"),
                    false,
                ),
            ]),
        },
        McpTool {
            name: "generate_broll".into(),
            description: "Generate a short silent cutaway with grok-imagine-video, save it in the bin, and cover the speaker with it so the original voice continues. prompt describes the picture. at is the timeline time. duration is seconds (about 1–8). aspect is 16:9, 9:16, 1:1, or 4:3; omit it to follow the timeline.".into(),
            input_schema: object(&[
                ("prompt", str_prop("What the cutaway shows"), true),
                ("at", num_prop("Timeline time in seconds"), true),
                ("duration", num_prop("Seconds, about 1 to 8"), true),
                ("aspect", str_prop("16:9, 9:16, 1:1, or 4:3"), false),
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
            let raw = call
                .arguments
                .get("cues")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let recipe = caption_recipe(&call.arguments);
            let mut cues = Vec::new();
            let mut locked = Vec::new();
            for cue in &raw {
                let Some(start) = cue.get("start").and_then(Value::as_f64) else {
                    continue;
                };
                let Some(end) = cue.get("end").and_then(Value::as_f64) else {
                    continue;
                };
                let Some(text) = cue.get("text").and_then(Value::as_str) else {
                    continue;
                };
                locked.push(
                    cue.get("place").is_some()
                        || cue.get("font").is_some()
                        || cue.get("effect").is_some(),
                );
                cues.push(oc_timeline::CaptionCue {
                    start: Time::from_seconds(start),
                    end: Time::from_seconds(end),
                    text: text.to_string(),
                    speaker: cue
                        .get("speaker")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    place: cue
                        .get("place")
                        .and_then(Value::as_str)
                        .map(oc_timeline::CaptionPlace::parse)
                        .unwrap_or_default(),
                    font: cue
                        .get("font")
                        .and_then(Value::as_str)
                        .map(oc_timeline::CaptionFont::parse)
                        .unwrap_or_default(),
                    effect: cue
                        .get("effect")
                        .and_then(Value::as_str)
                        .map(oc_timeline::CaptionEffect::parse)
                        .unwrap_or_default(),
                });
            }
            let fresh_at: Vec<usize> = locked
                .iter()
                .enumerate()
                .filter(|(_, lock)| !**lock)
                .map(|(index, _)| index)
                .collect();
            if !fresh_at.is_empty() {
                let mut fresh: Vec<_> = fresh_at.iter().map(|index| cues[*index].clone()).collect();
                let faces = vec![true; fresh.len()];
                oc_timeline::dress_cues(&mut fresh, &recipe, &faces);
                for (cue, index) in fresh.into_iter().zip(fresh_at) {
                    cues[index] = cue;
                }
            }
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
            let kind = TransitionKind::from_key(raw);
            let duration = number(&call.arguments, "duration").ok();
            if batch(&call.arguments) {
                Ok(Op::StyleClips {
                    clip_ids: clip_ids_of(&call.arguments),
                    all: flag(&call.arguments, "all"),
                    grade: None,
                    fx: None,
                    transition: Some(kind),
                    transition_seconds: duration,
                })
            } else {
                Ok(Op::SetTransition {
                    clip_id: clip_id(&call.arguments, "clip_id")?,
                    kind,
                    duration,
                })
            }
        }
        "set_grade" => {
            let grade = grade_from(&call.arguments);
            if batch(&call.arguments) {
                Ok(Op::StyleClips {
                    clip_ids: clip_ids_of(&call.arguments),
                    all: flag(&call.arguments, "all"),
                    grade: Some(grade),
                    fx: None,
                    transition: None,
                    transition_seconds: None,
                })
            } else {
                Ok(Op::SetGrade {
                    clip_id: clip_id(&call.arguments, "clip_id")?,
                    grade,
                })
            }
        }
        "set_move" => Ok(Op::SetMove {
            clip_id: clip_id(&call.arguments, "clip_id")?,
            end_x: number(&call.arguments, "end_x").unwrap_or(0.0) as f32,
            end_y: number(&call.arguments, "end_y").unwrap_or(0.0) as f32,
            end_scale: number(&call.arguments, "end_scale").unwrap_or(1.12) as f32,
            ease: oc_timeline::Ease::parse(
                call.arguments
                    .get("ease")
                    .and_then(Value::as_str)
                    .unwrap_or("linear"),
            ),
        }),
        "set_speed_ramp" => Ok(Op::SetSpeedRamp {
            clip_id: clip_id(&call.arguments, "clip_id")?,
            speed: number(&call.arguments, "speed").unwrap_or(1.0) as f32,
            end_speed: number(&call.arguments, "end_speed").unwrap_or(1.0) as f32,
        }),
        "set_speed_keys" => {
            let keys = speed_keys(&call.arguments)?;
            Ok(Op::SetSpeedKeys {
                clip_id: clip_id(&call.arguments, "clip_id")?,
                keys,
            })
        }
        "set_mix" => Ok(Op::SetMix {
            track_id: optional_track(&call.arguments, "track_id"),
            mix: Mix {
                gain_db: number(&call.arguments, "gain_db").unwrap_or(0.0) as f32,
                pan: number(&call.arguments, "pan").unwrap_or(0.0) as f32,
                solo: flag(&call.arguments, "solo"),
            },
        }),
        "set_curves" => Ok(Op::SetCurves {
            clip_id: clip_id(&call.arguments, "clip_id")?,
            curves: curves_from(&call.arguments),
        }),
        "set_mask" => Ok(Op::SetMask {
            clip_id: clip_id(&call.arguments, "clip_id")?,
            mask: if flag(&call.arguments, "clear") {
                None
            } else {
                Some(AlphaShape {
                    shape: mask_shape(
                        call.arguments
                            .get("shape")
                            .and_then(Value::as_str)
                            .unwrap_or("rectangle"),
                    ),
                    x: number(&call.arguments, "x").unwrap_or(0.5) as f32,
                    y: number(&call.arguments, "y").unwrap_or(0.5) as f32,
                    w: number(&call.arguments, "w").unwrap_or(0.5) as f32,
                    h: number(&call.arguments, "h").unwrap_or(0.5) as f32,
                    feather: number(&call.arguments, "feather").unwrap_or(0.0) as f32,
                    invert: flag(&call.arguments, "invert"),
                })
            },
        }),
        "add_generator" => Ok(Op::AddGenerator {
            generator: generator_from(&call.arguments),
            at: seconds(&call.arguments, "at").unwrap_or(Time::ZERO),
            duration: Duration::from_seconds(number(&call.arguments, "duration").unwrap_or(5.0)),
        }),
        "set_stabilize" => Ok(Op::SetStabilize {
            clip_id: clip_id(&call.arguments, "clip_id")?,
            on: call
                .arguments
                .get("on")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        }),
        "set_crop" => Ok(Op::SetCrop {
            clip_id: clip_id(&call.arguments, "clip_id")?,
            x: number(&call.arguments, "x").unwrap_or(0.0) as f32,
            y: number(&call.arguments, "y").unwrap_or(0.0) as f32,
            w: number(&call.arguments, "w").unwrap_or(1.0) as f32,
            h: number(&call.arguments, "h").unwrap_or(1.0) as f32,
        }),
        "set_audio" => Ok(Op::SetAudio {
            clip_id: clip_id(&call.arguments, "clip_id")?,
            audio: AudioFx {
                normalize: flag(&call.arguments, "normalize"),
                denoise: flag(&call.arguments, "denoise"),
                compressor: flag(&call.arguments, "compressor"),
                low: number(&call.arguments, "low").unwrap_or(0.0) as f32,
                mid: number(&call.arguments, "mid").unwrap_or(0.0) as f32,
                high: number(&call.arguments, "high").unwrap_or(0.0) as f32,
            },
        }),
        "set_transform" => Ok(Op::SetTransform {
            clip_id: clip_id(&call.arguments, "clip_id")?,
            x: number(&call.arguments, "x").unwrap_or(0.0) as f32,
            y: number(&call.arguments, "y").unwrap_or(0.0) as f32,
            scale: number(&call.arguments, "scale").unwrap_or(1.0) as f32,
            rotation: number(&call.arguments, "rotation").unwrap_or(0.0) as f32,
        }),
        "cover" => Ok(Op::Cover {
            media_id: media_id(&call.arguments, "media_id")?,
            at: seconds(&call.arguments, "at").unwrap_or(Time::ZERO),
            source_in: seconds(&call.arguments, "source_in").unwrap_or(Time::ZERO),
            duration: Duration::from_seconds(number(&call.arguments, "duration").unwrap_or(0.9)),
        }),
        "set_fx" => {
            let fx = Fx {
                blur: number(&call.arguments, "blur").unwrap_or(0.0) as f32,
                grain: number(&call.arguments, "grain").unwrap_or(0.0) as f32,
                vignette: number(&call.arguments, "vignette").unwrap_or(0.0) as f32,
            };
            if batch(&call.arguments) {
                Ok(Op::StyleClips {
                    clip_ids: clip_ids_of(&call.arguments),
                    all: flag(&call.arguments, "all"),
                    grade: None,
                    fx: Some(fx),
                    transition: None,
                    transition_seconds: None,
                })
            } else {
                Ok(Op::SetFx {
                    clip_id: clip_id(&call.arguments, "clip_id")?,
                    fx,
                })
            }
        }
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
                graphic: Graphic {
                    kind,
                    text,
                    x: None,
                    y: None,
                },
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
        "insert" | "overwrite" => Ok(Op::PlaceMedia {
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
            kind: TrackKind::Video,
            mode: if call.name == "overwrite" {
                TimelineEditMode::Overwrite
            } else {
                TimelineEditMode::Insert
            },
        }),
        "jl_cut" => Ok(Op::JlCut {
            clip_id: clip_id(&call.arguments, "clip_id")?,
            lead: Duration::from_seconds(number(&call.arguments, "lead").unwrap_or(0.0).max(0.0)),
            tail: Duration::from_seconds(number(&call.arguments, "tail").unwrap_or(0.0).max(0.0)),
        }),
        "set_frame_rate" => Ok(Op::SetFrameRate {
            frame_rate: oc_time::FrameRate::nearest(number(&call.arguments, "fps")?),
        }),
        "set_background" => {
            let color = call
                .arguments
                .get("color")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing color".to_string())?;
            Ok(Op::SetBackground {
                color: color.to_string(),
            })
        }
        "import_cube" => {
            let text = call
                .arguments
                .get("text")
                .and_then(Value::as_str)
                .filter(|text| text.contains("LUT_3D_SIZE"))
                .ok_or_else(|| "import_cube needs the .cube text".to_string())?;
            Ok(Op::ImportCube {
                clip_id: clip_id(&call.arguments, "clip_id")?,
                text: text.to_string(),
            })
        }
        "generate_broll" => Err("generate_broll is handled by the host".into()),
        "add_design" => Err("add_design is handled by the host".into()),
        other => Err(format!("unknown tool {other}")),
    }
}

fn mcp_for_id(id: crate::ToolId, label: &'static str, tip: &'static str) -> Option<McpTool> {
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
        Unlink => ("unlink", object(&[("clip_id", str_prop("Clip id"), true)])),
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

fn caption_recipe(arguments: &Value) -> oc_timeline::CaptionRecipe {
    let mood = oc_timeline::CaptionMood::parse(
        arguments
            .get("caption_mood")
            .and_then(Value::as_str)
            .unwrap_or("clean"),
    );
    let parsed = arguments
        .get("caption_look")
        .and_then(|raw| match raw.as_str() {
            Some(text) => oc_timeline::CaptionRecipe::from_loose(text),
            None => serde_json::from_value(raw.clone()).ok(),
        });
    match parsed {
        Some(mut recipe) if !recipe.is_blank() => {
            if recipe.base.is_none() {
                recipe.base = Some(mood);
            }
            recipe
        }
        _ => mood.recipe(),
    }
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
    (
        json!({ "type": "string", "description": description }),
        description,
    )
}

fn num_prop(description: &str) -> (Value, &str) {
    (
        json!({ "type": "number", "description": description }),
        description,
    )
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

fn flag(args: &Value, key: &str) -> bool {
    match args.get(key) {
        Some(Value::Bool(v)) => *v,
        Some(Value::String(s)) => s == "true" || s == "1" || s == "yes",
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0) > 0.0,
        _ => false,
    }
}

fn speed_keys(args: &Value) -> Result<Vec<SpeedKey>, String> {
    let keys: Vec<SpeedKey> = args
        .get("keys")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    let at = row.get("at").and_then(Value::as_f64)? as f32;
                    let speed = row.get("speed").and_then(Value::as_f64)? as f32;
                    Some(SpeedKey { at, speed })
                })
                .collect()
        })
        .unwrap_or_default();
    if keys.len() < 2 {
        return Err("set_speed_keys needs at least two keys, each with at and speed".into());
    }
    Ok(keys)
}

fn curves_from(args: &Value) -> Curves {
    let points = args
        .get("points")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    let x = row.get("x").and_then(Value::as_f64)? as f32;
                    let y = row.get("y").and_then(Value::as_f64)? as f32;
                    Some(CurvePoint { x, y })
                })
                .collect::<Vec<_>>()
        })
        .filter(|points| !points.is_empty())
        .unwrap_or_else(|| {
            let y = number(args, "mid").unwrap_or(0.5) as f32;
            if (y - 0.5).abs() < 0.01 {
                Vec::new()
            } else {
                vec![
                    CurvePoint { x: 0.0, y: 0.0 },
                    CurvePoint { x: 0.5, y },
                    CurvePoint { x: 1.0, y: 1.0 },
                ]
            }
        });
    if points.is_empty() {
        return Curves::default();
    }
    match args.get("channel").and_then(Value::as_str).unwrap_or("all") {
        "red" | "r" => Curves {
            red: points,
            ..Curves::default()
        },
        "green" | "g" => Curves {
            green: points,
            ..Curves::default()
        },
        "blue" | "b" => Curves {
            blue: points,
            ..Curves::default()
        },
        _ => Curves {
            all: points,
            ..Curves::default()
        },
    }
}

fn mask_shape(raw: &str) -> MaskShape {
    match raw.trim().to_ascii_lowercase().as_str() {
        "ellipse" | "circle" => MaskShape::Ellipse,
        "triangle" => MaskShape::Triangle,
        "diamond" => MaskShape::Diamond,
        _ => MaskShape::Rectangle,
    }
}

fn generator_from(args: &Value) -> Generator {
    match args
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("color")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "color_bars" | "bars" | "smpte" => Generator::ColorBars,
        "white_noise" | "noise" => Generator::WhiteNoise,
        "counter" | "countdown" => Generator::Counter,
        _ => Generator::Color {
            color: args
                .get("color")
                .and_then(Value::as_str)
                .unwrap_or("#111111")
                .to_string(),
        },
    }
}

fn grade_from(args: &Value) -> Grade {
    Grade {
        exposure: number(args, "exposure").unwrap_or(0.0) as f32,
        contrast: number(args, "contrast").unwrap_or(0.0) as f32,
        saturation: number(args, "saturation").unwrap_or(0.0) as f32,
        temperature: number(args, "temperature").unwrap_or(0.0) as f32,
        lift: number(args, "lift").unwrap_or(0.0) as f32,
        gamma: number(args, "gamma").unwrap_or(0.0) as f32,
        gain: number(args, "gain").unwrap_or(0.0) as f32,
        lut: lut_from(args.get("lut").and_then(Value::as_str).unwrap_or("none")),
        cube: None,
    }
}

fn batch(args: &Value) -> bool {
    flag(args, "all")
        || args
            .get("clip_ids")
            .and_then(Value::as_array)
            .is_some_and(|rows| !rows.is_empty())
}

fn clip_ids_of(args: &Value) -> Vec<ClipId> {
    args.get("clip_ids").map(parse_id_list).unwrap_or_default()
}

fn lut_from(raw: &str) -> Lut {
    match raw {
        "film" => Lut::Film,
        "cool" => Lut::Cool,
        "warm" => Lut::Warm,
        "teal_orange" | "teal" => Lut::TealOrange,
        "mono" | "bw" | "black_white" => Lut::Mono,
        _ => Lut::None,
    }
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
        assert!(matches!(
            inspect_from_mcp(&call),
            Some(Inspect::ListCues { .. })
        ));
    }

    #[test]
    fn group_link_insert_overwrite_and_jl_parse() {
        let a = "11111111-1111-1111-1111-111111111111";
        let b = "22222222-2222-2222-2222-222222222222";
        let grouped = op_from_mcp(&McpCall {
            name: "group".into(),
            arguments: json!({ "clip_ids": format!("{a},{b}") }),
        })
        .unwrap();
        assert!(matches!(grouped, Op::Group { clip_ids } if clip_ids.len() == 2));
        let linked = op_from_mcp(&McpCall {
            name: "link".into(),
            arguments: json!({ "clip_ids": format!("[\"{a}\",\"{b}\"]") }),
        })
        .unwrap();
        assert!(matches!(linked, Op::Link { .. }));
        let inserted = op_from_mcp(&McpCall {
            name: "insert".into(),
            arguments: json!({ "media_id": a, "start": 1.0, "duration": 2.0 }),
        })
        .unwrap();
        assert!(matches!(
            inserted,
            Op::PlaceMedia {
                mode: TimelineEditMode::Insert,
                ..
            }
        ));
        let overwritten = op_from_mcp(&McpCall {
            name: "overwrite".into(),
            arguments: json!({ "media_id": a, "start": 1.0 }),
        })
        .unwrap();
        assert!(matches!(
            overwritten,
            Op::PlaceMedia {
                mode: TimelineEditMode::Overwrite,
                ..
            }
        ));
        let jl = op_from_mcp(&McpCall {
            name: "jl_cut".into(),
            arguments: json!({ "clip_id": a, "lead": 0.4, "tail": 0.2 }),
        })
        .unwrap();
        assert!(matches!(jl, Op::JlCut { .. }));
        let rate = op_from_mcp(&McpCall {
            name: "set_frame_rate".into(),
            arguments: json!({ "fps": 24 }),
        })
        .unwrap();
        assert!(
            matches!(rate, Op::SetFrameRate { frame_rate } if frame_rate == oc_time::FrameRate::FPS_24)
        );
        assert!(mcp_tools().iter().any(|tool| tool.name == "generate_broll"));
        assert!(mcp_tools().iter().any(|tool| tool.name == "add_design"));
        assert!(
            op_from_mcp(&McpCall {
                name: "add_design".into(),
                arguments: json!({ "prompt": "a house", "at": 1.0 }),
            })
            .is_err()
        );
        assert!(mcp_tools().iter().any(|tool| tool.name == "import_cube"));
    }
}

fn clip_ids(args: &Value, key: &str) -> Result<Vec<ClipId>, String> {
    let Some(value) = args.get(key) else {
        return Err(format!("missing {key}"));
    };
    let ids = parse_id_list(value);
    if ids.is_empty() {
        return Err(format!("missing {key}"));
    }
    Ok(ids)
}

/// A JSON array, a JSON-array string, or comma-separated uuids.
fn parse_id_list(value: &Value) -> Vec<ClipId> {
    match value {
        Value::Array(rows) => rows.iter().flat_map(parse_id_list).collect(),
        Value::String(raw) => {
            let trimmed = raw.trim();
            if trimmed.starts_with('[') {
                if let Ok(parsed) = serde_json::from_str::<Value>(trimmed) {
                    return parse_id_list(&parsed);
                }
            }
            trimmed
                .split(|c: char| c == ',' || c.is_whitespace())
                .filter(|part| !part.is_empty())
                .filter_map(|part| Uuid::parse_str(part.trim()).ok())
                .map(ClipId::from_uuid)
                .collect()
        }
        _ => Vec::new(),
    }
}
