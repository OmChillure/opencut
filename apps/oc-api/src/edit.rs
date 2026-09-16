//! Shared inspect + apply so chat and the MCP child hit the same tools.

use oc_core::{
    inspect_from_mcp, apply, op_from_mcp, AssembleItem, Inspect, MediaId, McpCall, Op, Time,
    Timeline, TrackKind, UndoStack,
};
use oc_core::time::TICKS_PER_SECOND;
use oc_db::Db;
use serde_json::Value;
use std::collections::HashMap;
use uuid::Uuid;

pub(crate) struct Speech {
    pub words: u32,
    pub speech_seconds: f64,
    pub hook_in: Time,
    pub text: String,
}

pub(crate) async fn call_tool(
    db: &Db,
    project_id: Uuid,
    name: &str,
    arguments: Value,
) -> Result<String, String> {
    let mut project = oc_db::get_project(db, project_id)
        .await
        .map_err(|e| e.to_string())?;
    let media = oc_db::list_media(db, project_id)
        .await
        .map_err(|e| e.to_string())?;
    let transcripts = oc_db::list_transcripts_for_project(db, project_id)
        .await
        .map_err(|e| e.to_string())?;
    let speech = speech_by_media(&transcripts);
    let looks = look_by_media(
        &oc_db::list_analysis_for_project(db, project_id)
            .await
            .unwrap_or_default(),
    );
    let call = McpCall {
        name: name.to_string(),
        arguments,
    };
    if let Some(inspect) = inspect_from_mcp(&call) {
        let out = run_inspect(inspect, &project.timeline, &media, &speech, &looks);
        tracing::info!(project = %project_id, tool = name, chars = out.len(), "inspect");
        return Ok(out);
    }
    let op = op_from_mcp(&call).map_err(|e| {
        tracing::error!(project = %project_id, tool = name, "bad tool: {e}");
        e
    })?;
    let op = hydrate_op(op, &media, &speech, &looks);
    let mut undo = UndoStack::new();
    let applied = apply(&mut project.timeline, &mut undo, op).map_err(|e| {
        tracing::error!(project = %project_id, tool = name, "apply failed: {e}");
        e.to_string()
    })?;
    oc_db::save_timeline(db, project_id, &project.timeline)
        .await
        .map_err(|e| e.to_string())?;
    tracing::info!(project = %project_id, tool = name, note = %applied.note, "applied");
    Ok(applied.note)
}

pub(crate) fn speech_by_media(rows: &[oc_db::TranscriptCueRow]) -> HashMap<Uuid, Speech> {
    let mut map: HashMap<Uuid, Speech> = HashMap::new();
    for row in rows {
        let entry = map.entry(row.media_id).or_insert_with(|| Speech {
            words: 0,
            speech_seconds: 0.0,
            hook_in: Time::ZERO,
            text: row.full_text.clone(),
        });
        let words = row
            .text
            .split_whitespace()
            .filter(|w| w.chars().any(|c| c.is_alphanumeric()))
            .count() as u32;
        let secs = (row.end_ticks - row.start_ticks).max(0) as f64 / TICKS_PER_SECOND as f64;
        if entry.hook_in.as_ticks() == 0 && words >= 4 {
            entry.hook_in = Time::from_ticks(row.start_ticks);
        }
        entry.words += words;
        entry.speech_seconds += secs;
    }
    map
}

pub(crate) fn look_by_media(rows: &[oc_db::AnalysisRow]) -> HashMap<Uuid, oc_db::AnalysisRow> {
    rows.iter().cloned().map(|r| (r.media_id, r)).collect()
}

pub(crate) fn run_inspect(
    inspect: Inspect,
    timeline: &Timeline,
    media: &[oc_db::MediaRow],
    speech: &HashMap<Uuid, Speech>,
    looks: &HashMap<Uuid, oc_db::AnalysisRow>,
) -> String {
    match inspect {
        Inspect::ListBin => {
            if media.is_empty() {
                return "bin: empty (nothing registered for this project yet)".into();
            }
            let mut out = format!("bin: {} items\n", media.len());
            for row in media {
                let (kind, dur) = spec_from_row(row);
                let words = speech.get(&row.id).map(|s| s.words).unwrap_or(0);
                let look = looks.get(&row.id).map(|l| l.look.as_str()).unwrap_or("-");
                out.push_str(&format!(
                    "{id}  {kind:?}  {dur:.1}s  words={words}  look={look}  {name}\n",
                    id = row.id,
                    dur = dur.as_seconds(),
                    name = row.filename
                ));
            }
            out
        }
        Inspect::ListTimeline => timeline_brief(timeline),
        Inspect::GetMedia { media_id } => {
            let id = media_id.as_uuid();
            let Some(row) = media.iter().find(|m| m.id == id) else {
                return format!("media {id} not in bin");
            };
            let (kind, dur) = spec_from_row(row);
            let mut out = format!(
                "{id}  {kind:?}  {dur:.1}s  {}\n",
                row.filename,
                dur = dur.as_seconds()
            );
            if let Some(s) = speech.get(&id) {
                let excerpt: String = s.text.chars().take(240).collect();
                out.push_str(&format!(
                    "speech words={} hook@{:.1}s \"{}\"\n",
                    s.words,
                    s.hook_in.as_seconds(),
                    excerpt.replace('\n', " ")
                ));
            } else {
                out.push_str("speech: none yet\n");
            }
            if let Some(l) = looks.get(&id) {
                out.push_str(&format!(
                    "look {} motion={:.2} scenes={}\n",
                    l.look, l.motion, l.scenes
                ));
            }
            out
        }
    }
}

pub(crate) fn hydrate_op(
    op: Op,
    media: &[oc_db::MediaRow],
    speech: &HashMap<Uuid, Speech>,
    looks: &HashMap<Uuid, oc_db::AnalysisRow>,
) -> Op {
    match op {
        Op::PlaceMedia {
            media_id,
            track_id,
            start,
            duration,
            kind,
            mode,
        } => {
            if let Some(row) = media.iter().find(|r| r.id == media_id.as_uuid()) {
                let (row_kind, row_dur) = spec_from_row(row);
                Op::PlaceMedia {
                    media_id,
                    track_id,
                    start,
                    duration: if duration.as_ticks() > 0 {
                        duration
                    } else {
                        row_dur
                    },
                    kind: if kind == TrackKind::Caption {
                        kind
                    } else {
                        row_kind
                    },
                    mode,
                }
            } else {
                Op::PlaceMedia {
                    media_id,
                    track_id,
                    start,
                    duration,
                    kind,
                    mode,
                }
            }
        }
        Op::Assemble { items, style } => {
            let items = if items.is_empty() {
                media
                    .iter()
                    .rev()
                    .map(|row| item_from_row(row, speech.get(&row.id), looks.get(&row.id)))
                    .collect()
            } else {
                items
                    .into_iter()
                    .map(|item| {
                        if let Some(row) = media.iter().find(|r| r.id == item.media_id.as_uuid()) {
                            item_from_row(row, speech.get(&row.id), looks.get(&row.id))
                        } else {
                            item
                        }
                    })
                    .collect()
            };
            Op::Assemble { items, style }
        }
        other => other,
    }
}

fn spec_from_row(row: &oc_db::MediaRow) -> (TrackKind, oc_core::Duration) {
    let kind = kind_from_media(&row.content_type, &row.filename);
    let seconds = row
        .duration_ticks
        .filter(|t| *t > 0)
        .map(|t| t as f64 / TICKS_PER_SECOND as f64)
        .unwrap_or(match kind {
            TrackKind::Audio => 8.0,
            TrackKind::Caption => 3.0,
            TrackKind::Video => {
                if row.content_type.starts_with("image/") {
                    3.0
                } else {
                    5.0
                }
            }
        });
    (kind, oc_core::Duration::from_seconds(seconds))
}

fn kind_from_media(content_type: &str, filename: &str) -> TrackKind {
    if content_type.starts_with("audio/") {
        return TrackKind::Audio;
    }
    let ext = filename.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "mp3" | "wav" | "aac" | "m4a" | "ogg" | "flac" => TrackKind::Audio,
        _ => TrackKind::Video,
    }
}

fn item_from_row(
    row: &oc_db::MediaRow,
    speech: Option<&Speech>,
    look: Option<&oc_db::AnalysisRow>,
) -> AssembleItem {
    let (kind, duration) = spec_from_row(row);
    let still = row.content_type.starts_with("image/");
    let mut item = AssembleItem {
        media_id: MediaId::from_uuid(row.id),
        duration,
        kind,
        still,
        ..AssembleItem::default()
    };
    if let Some(s) = speech {
        item.words = s.words;
        item.speech_seconds = s.speech_seconds;
        item.hook_in = s.hook_in;
        item.text = s.text.clone();
    }
    if let Some(l) = look {
        item.look = l.look.clone();
        item.motion = l.motion as f32;
        item.scenes = l.scenes.max(0) as u32;
    }
    item
}

fn timeline_brief(tl: &Timeline) -> String {
    let mut out = String::from("Current timeline:\n");
    for track in &tl.tracks {
        out.push_str(&format!(
            "- track {} ({:?}) {}\n",
            track.id, track.kind, track.name
        ));
        for clip in &track.clips {
            let media = clip
                .media_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "-".into());
            out.push_str(&format!(
                "    clip {} media={} start={:.2}s dur={:.2}s\n",
                clip.id,
                media,
                clip.start.as_seconds(),
                clip.duration.as_seconds()
            ));
        }
    }
    if tl.tracks.iter().all(|t| t.clips.is_empty()) {
        out.push_str("(no clips yet)\n");
    }
    out
}
