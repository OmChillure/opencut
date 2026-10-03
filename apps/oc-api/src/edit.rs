//! Shared inspect + apply so chat and the MCP child hit the same tools.

use oc_core::{
    inspect_from_mcp, apply, op_from_mcp, pick_reel_excerpts, AssembleItem, ExportPreset, Inspect,
    MediaId, McpCall, Op, Time, Timeline, TrackKind, UndoStack,
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
    pub cues: Vec<CueBrief>,
}

pub(crate) struct CueBrief {
    pub start: Time,
    pub end: Time,
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
    if name == "submit_edit" || name == "revise_edit" {
        return apply_submitted_plan(db, project_id, name, &call.arguments, &media, &speech, &looks).await;
    }
    if name == "generate_broll" {
        return generate_broll(db, project_id, &mut project.timeline, &call.arguments).await;
    }
    if name == "add_design" {
        return add_design(db, project_id, &mut project.timeline, &call.arguments).await;
    }
    if name == "snap_cuts_to_beats" {
        let raw = call
            .arguments
            .get("media_id")
            .and_then(Value::as_str)
            .ok_or("missing media_id")?;
        let music_id = Uuid::parse_str(raw).map_err(|_| "bad media_id")?;
        let beats = music_beats(looks.get(&music_id)).unwrap_or_default();
        if beats.is_empty() {
            return Err("no beat grid for that file yet".into());
        }
        let tolerance = call
            .arguments
            .get("tolerance_frames")
            .and_then(Value::as_f64)
            .unwrap_or(2.0)
            .clamp(1.0, 48.0) as u32;
        let mut undo = UndoStack::new();
        let applied = apply(
            &mut project.timeline,
            &mut undo,
            Op::SnapCuts {
                tolerance_frames: tolerance,
                beats,
            },
        )
        .map_err(|e| e.to_string())?;
        oc_db::save_timeline(db, project_id, &project.timeline)
            .await
            .map_err(|e| e.to_string())?;
        return Ok(applied.note);
    }
    let op = op_from_mcp(&call).map_err(|e| {
        tracing::error!(project = %project_id, tool = name, "bad tool: {e}");
        e
    })?;
    let op = hydrate_op(op, &media, &speech, &looks);
    let mut undo = UndoStack::new();
    let export = match &op {
        oc_core::Op::Export { preset } => Some(*preset),
        _ => None,
    };
    let applied = apply(&mut project.timeline, &mut undo, op).map_err(|e| {
        tracing::error!(project = %project_id, tool = name, "apply failed: {e}");
        e.to_string()
    })?;
    oc_db::save_timeline(db, project_id, &project.timeline)
        .await
        .map_err(|e| e.to_string())?;
    if let Some(preset) = export {
        queue_export(db, project_id, preset).await?;
    }
    tracing::info!(project = %project_id, tool = name, note = %applied.note, "applied");
    Ok(applied.note)
}

/// Silent cutaway from grok-imagine-video, saved into the bin, then covered over `at`.
async fn generate_broll(
    db: &Db,
    project_id: Uuid,
    timeline: &mut Timeline,
    arguments: &Value,
) -> Result<String, String> {
    let prompt = arguments
        .get("prompt")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| "generate_broll needs a prompt".to_string())?;
    let at = arguments
        .get("at")
        .and_then(Value::as_f64)
        .ok_or_else(|| "generate_broll needs at".to_string())?
        .max(0.0);
    let requested = arguments
        .get("duration")
        .and_then(Value::as_f64)
        .unwrap_or(4.0)
        .clamp(1.0, 8.0)
        .round() as u32;
    let aspect = broll_aspect(
        timeline,
        arguments.get("aspect").and_then(Value::as_str),
    );
    let (bytes, seconds) = oc_providers::imagine_clip(prompt, requested, &aspect).await?;
    let media_id = Uuid::now_v7();
    let filename = format!("broll-{media_id}.mp4");
    let local_key = oc_db::local_media_key(project_id, media_id, &filename);
    let object_key = oc_media::object_key(
        oc_media::ObjectKind::Raw,
        oc_core::ProjectId::from_uuid(project_id),
        MediaId::from_uuid(media_id),
        &filename,
    );
    oc_db::insert_media(db, project_id, media_id, &local_key, &filename, "video/mp4")
        .await
        .map_err(|e| e.to_string())?;
    let stored_key = match oc_db::R2::from_env().await {
        Ok(r2) => match r2
            .put_bytes(&object_key, bytes.to_vec(), "video/mp4")
            .await
        {
            Ok(()) => object_key,
            Err(err) => {
                tracing::warn!("b-roll R2 put failed, keeping a local file: {err}");
                write_local_media(&local_key, &bytes).await?;
                local_key
            }
        },
        Err(err) => {
            tracing::info!("b-roll staying local: {err}");
            write_local_media(&local_key, &bytes).await?;
            local_key
        }
    };
    oc_db::set_media_r2_key(db, media_id, &stored_key)
        .await
        .map_err(|e| e.to_string())?;
    let ticks = (seconds * TICKS_PER_SECOND as f64).round() as i64;
    oc_db::set_media_duration(db, media_id, ticks.max(1))
        .await
        .map_err(|e| e.to_string())?;
    let mut undo = UndoStack::new();
    let applied = apply(
        timeline,
        &mut undo,
        Op::Cover {
            media_id: MediaId::from_uuid(media_id),
            at: Time::from_seconds(at),
            source_in: Time::ZERO,
            duration: oc_core::Duration::from_seconds(seconds),
        },
    )
    .map_err(|e| e.to_string())?;
    oc_db::save_timeline(db, project_id, timeline)
        .await
        .map_err(|e| e.to_string())?;
    Ok(format!(
        "generated b-roll {media_id} ({seconds:.1}s, {aspect}) — {}",
        applied.note
    ))
}

/// Illustration of the thing being explained. A short animation, a label, and a layout.
async fn add_design(
    db: &Db,
    project_id: Uuid,
    timeline: &mut Timeline,
    arguments: &Value,
) -> Result<String, String> {
    let prompt = arguments
        .get("prompt")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| "add_design needs a prompt".to_string())?;
    let at = arguments
        .get("at")
        .and_then(Value::as_f64)
        .ok_or_else(|| "add_design needs at".to_string())?
        .max(0.0);
    let requested = arguments
        .get("duration")
        .and_then(Value::as_f64)
        .unwrap_or(4.0)
        .clamp(2.0, 15.0)
        .round() as u32;
    let text = arguments
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let layout = design_layout(arguments.get("layout").and_then(Value::as_str));
    let aspect = broll_aspect(timeline, arguments.get("aspect").and_then(Value::as_str));
    let (bytes, seconds) =
        oc_providers::imagine_clip(&design_prompt(prompt), requested, &aspect).await?;
    let media_id = Uuid::now_v7();
    let filename = format!("design-{media_id}.mp4");
    let local_key = oc_db::local_media_key(project_id, media_id, &filename);
    let object_key = oc_media::object_key(
        oc_media::ObjectKind::Raw,
        oc_core::ProjectId::from_uuid(project_id),
        MediaId::from_uuid(media_id),
        &filename,
    );
    oc_db::insert_media(db, project_id, media_id, &local_key, &filename, "video/mp4")
        .await
        .map_err(|e| e.to_string())?;
    let stored_key = match oc_db::R2::from_env().await {
        Ok(r2) => match r2
            .put_bytes(&object_key, bytes.to_vec(), "video/mp4")
            .await
        {
            Ok(()) => object_key,
            Err(err) => {
                tracing::warn!("design R2 put failed, keeping a local file: {err}");
                write_local_media(&local_key, &bytes).await?;
                local_key
            }
        },
        Err(err) => {
            tracing::info!("design staying local: {err}");
            write_local_media(&local_key, &bytes).await?;
            local_key
        }
    };
    oc_db::set_media_r2_key(db, media_id, &stored_key)
        .await
        .map_err(|e| e.to_string())?;
    let ticks = (seconds * TICKS_PER_SECOND as f64).round() as i64;
    oc_db::set_media_duration(db, media_id, ticks.max(1))
        .await
        .map_err(|e| e.to_string())?;
    oc_db::set_media_status(db, media_id, "ready")
        .await
        .map_err(|e| e.to_string())?;
    let mut undo = UndoStack::new();
    let applied = apply(
        timeline,
        &mut undo,
        Op::AddDesign {
            media_id: MediaId::from_uuid(media_id),
            at: Time::from_seconds(at),
            duration: oc_core::Duration::from_seconds(seconds),
            layout,
            text,
        },
    )
    .map_err(|e| e.to_string())?;
    oc_db::save_timeline(db, project_id, timeline)
        .await
        .map_err(|e| e.to_string())?;
    Ok(format!(
        "design {media_id} ({layout:?}, {seconds:.1}s, {aspect}) — {}",
        applied.note
    ))
}

fn design_layout(raw: Option<&str>) -> oc_core::DesignLayout {
    match raw.unwrap_or("").trim().to_ascii_lowercase().as_str() {
        "behind" | "back" | "text_behind" | "under" => oc_core::DesignLayout::Behind,
        "beside" | "side" | "split" => oc_core::DesignLayout::Beside,
        _ => oc_core::DesignLayout::Cutaway,
    }
}

fn design_prompt(subject: &str) -> String {
    format!(
        "Full-frame flat motion graphic on one solid background color. \
         The artwork touches the left, right, top, and bottom edges. \
         No empty margin, no border, no watermark, no letters, no numbers, no people. \
         Animate the subject itself happening across the whole clip: \
         the first stroke at the start, the finished form filling the frame at the end. \
         If this is a chart pattern or a diagram, draw that diagram forming, \
         not a real-world object with the same name. {subject}"
    )
}

fn broll_aspect(timeline: &Timeline, requested: Option<&str>) -> String {
    if let Some(raw) = requested.map(str::trim).filter(|text| !text.is_empty()) {
        let key = raw.replace(' ', "");
        if matches!(key.as_str(), "16:9" | "9:16" | "1:1" | "4:3") {
            return key;
        }
    }
    let ratio = timeline.width.max(1) as f64 / timeline.height.max(1) as f64;
    if (ratio - 1.0).abs() < 0.08 {
        "1:1".into()
    } else if ratio > 1.0 && (ratio - 4.0 / 3.0).abs() < (ratio - 16.0 / 9.0).abs() && ratio < 1.5
    {
        "4:3".into()
    } else if ratio < 1.0 {
        "9:16".into()
    } else {
        "16:9".into()
    }
}

async fn write_local_media(key: &str, bytes: &[u8]) -> Result<(), String> {
    let path = oc_db::local_media_path(key).ok_or_else(|| "bad local media key".to_string())?;
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(|e| e.to_string())?;
    }
    tokio::fs::write(path, bytes)
        .await
        .map_err(|e| e.to_string())
}

pub(crate) async fn queue_export(
    db: &Db,
    project_id: Uuid,
    preset: ExportPreset,
) -> Result<(), String> {
    let id = oc_db::enqueue_job(
        db,
        "export",
        serde_json::json!({
            "project_id": project_id,
            "preset": preset,
        }),
    )
    .await
    .map_err(|e| e.to_string())?;
    tracing::info!(project = %project_id, job = %id, ?preset, "export queued");
    Ok(())
}

pub(crate) fn speech_by_media(rows: &[oc_db::TranscriptCueRow]) -> HashMap<Uuid, Speech> {
    let mut map: HashMap<Uuid, Speech> = HashMap::new();
    for row in rows {
        let entry = map.entry(row.media_id).or_insert_with(|| Speech {
            words: 0,
            speech_seconds: 0.0,
            hook_in: Time::ZERO,
            text: row.full_text.clone(),
            cues: Vec::new(),
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
        entry.cues.push(CueBrief {
            start: Time::from_ticks(row.start_ticks),
            end: Time::from_ticks(row.end_ticks),
            text: row.text.clone(),
        });
    }
    map
}

pub(crate) fn look_by_media(rows: &[oc_db::AnalysisRow]) -> HashMap<Uuid, oc_db::AnalysisRow> {
    rows.iter().cloned().map(|r| (r.media_id, r)).collect()
}

pub(crate) async fn place_captions(
    db: &Db,
    project_id: Uuid,
) -> Result<(Timeline, String), String> {
    let mut project = oc_db::get_project(db, project_id)
        .await
        .map_err(|e| e.to_string())?;
    let transcripts = oc_db::list_transcripts_for_project(db, project_id)
        .await
        .map_err(|e| e.to_string())?;
    let speech = speech_by_media(&transcripts);
    let lines: Vec<oc_core::SpokenLine> = speech
        .iter()
        .flat_map(|(id, s)| {
            let media = MediaId::from_uuid(*id);
            s.cues.iter().map(move |c| oc_core::SpokenLine {
                media,
                start: c.start.as_seconds(),
                end: c.end.as_seconds(),
                text: c.text.clone(),
            })
        })
        .collect();
    if lines.is_empty() {
        let media = oc_db::list_media(db, project_id)
            .await
            .map_err(|e| e.to_string())?;
        let mut queued = 0;
        for row in media {
            if row.content_type.starts_with("image/") {
                continue;
            }
            oc_db::enqueue_job(
                db,
                "transcribe",
                serde_json::json!({
                    "project_id": project_id,
                    "media_id": row.id,
                    "r2_key": row.r2_key,
                }),
            )
            .await
            .map_err(|e| e.to_string())?;
            queued += 1;
        }
        return Ok((
            project.timeline,
            format!("no words yet — queued {queued} transcript(s). Try again when they finish."),
        ));
    }
    let cues = oc_core::mapped_cues(&oc_core::program_clips(&project.timeline), &lines);
    if cues.is_empty() {
        return Ok((
            project.timeline,
            "speech does not overlap the picture on the timeline".into(),
        ));
    }
    let mut undo = UndoStack::new();
    let applied = apply(
        &mut project.timeline,
        &mut undo,
        Op::AddCaptions {
            style: oc_core::CaptionStyle::Stacked,
            cues,
        },
    )
    .map_err(|e| e.to_string())?;
    oc_db::save_timeline(db, project_id, &project.timeline)
        .await
        .map_err(|e| e.to_string())?;
    Ok((project.timeline, applied.note))
}

pub(crate) fn spoken(speech: &HashMap<Uuid, Speech>) -> Vec<oc_core::Spoken> {
    speech
        .iter()
        .flat_map(|(id, s)| {
            let media = MediaId::from_uuid(*id);
            s.cues.iter().map(move |c| oc_core::Spoken {
                media,
                start: c.start.as_seconds(),
                end: c.end.as_seconds(),
                text: c.text.clone(),
            })
        })
        .collect()
}

pub(crate) fn shot_looks(row: &oc_db::AnalysisRow) -> Vec<oc_media::ShotLook> {
    row.raw
        .as_ref()
        .and_then(|v| serde_json::from_value::<oc_media::VisualDigest>(v.clone()).ok())
        .map(|d| d.shots)
        .unwrap_or_default()
}

pub(crate) fn look_needs_vision(row: &oc_db::AnalysisRow) -> bool {
    row.raw
        .as_ref()
        .and_then(|v| serde_json::from_value::<oc_media::VisualDigest>(v.clone()).ok())
        .is_some_and(|digest| digest.needs_cards() && oc_providers::subscription_ready())
}

/// One JPEG at a source time. The model calls `see` when it wants to look.
pub(crate) async fn see_frame(
    db: &Db,
    r2: Option<&oc_db::R2>,
    project_id: Uuid,
    media_id: Uuid,
    at: f64,
) -> Result<oc_providers::PromptImage, String> {
    let media = oc_db::list_media(db, project_id)
        .await
        .map_err(|e| e.to_string())?;
    let row = media
        .iter()
        .find(|m| m.id == media_id)
        .ok_or_else(|| format!("media {media_id} is not in this project"))?;
    if row.content_type.starts_with("audio/") {
        return Err("see needs a picture, not an audio file".into());
    }
    let mut temps = Vec::new();
    let path = open_for_frames(row, r2, &mut temps)
        .await
        .ok_or_else(|| format!("media {media_id} is not on disk"))?;
    let dest = std::env::temp_dir().join(format!(
        "oc-see-{}-{}.jpg",
        std::process::id(),
        at.to_bits()
    ));
    let grabbed = oc_media::grab_jpeg(&path, at.max(0.0), &dest).await;
    let jpeg = if grabbed.is_ok() {
        tokio::fs::read(&dest).await.ok()
    } else {
        None
    };
    let _ = tokio::fs::remove_file(&dest).await;
    for dir in temps {
        let _ = tokio::fs::remove_dir_all(dir).await;
    }
    let jpeg = jpeg.filter(|b| b.len() >= 32).ok_or_else(|| {
        format!("no frame at {at:.1}s in {media_id}")
    })?;
    Ok(oc_providers::PromptImage {
        caption: format!("media {media_id} @ {at:.1}s"),
        jpeg,
    })
}

async fn open_for_frames(
    row: &oc_db::MediaRow,
    r2: Option<&oc_db::R2>,
    temps: &mut Vec<std::path::PathBuf>,
) -> Option<std::path::PathBuf> {
    if let Some(path) = media_file(row) {
        return Some(path);
    }
    if !oc_db::is_r2_object_key(&row.r2_key) {
        return None;
    }
    let r2 = r2?;
    let bytes = r2.get_bytes(&row.r2_key).await.ok()?;
    let dir = std::env::temp_dir().join(format!("oc-see-src-{}-{}", std::process::id(), row.id));
    tokio::fs::create_dir_all(&dir).await.ok()?;
    let name = row.r2_key.rsplit('/').next().unwrap_or("media.bin");
    let path = dir.join(name);
    tokio::fs::write(&path, bytes).await.ok()?;
    temps.push(dir);
    Some(path)
}

fn media_file(row: &oc_db::MediaRow) -> Option<std::path::PathBuf> {
    if let Some(path) = oc_db::local_media_path(&row.r2_key) {
        if path.is_file() {
            return Some(path);
        }
    }
    let path = std::path::PathBuf::from(&row.r2_key);
    if path.is_file() { Some(path) } else { None }
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
                let cues = speech.get(&row.id).map(|s| s.cues.len()).unwrap_or(0);
                let look = looks.get(&row.id).map(|l| l.look.as_str()).unwrap_or("-");
                out.push_str(&format!(
                    "{id}  {kind:?}  {dur:.1}s  words={words}  cues={cues}  look={look}  {name}\n",
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
                out.push_str(&format!(
                    "speech words={} hook@{:.1}s cues={}\n",
                    s.words,
                    s.hook_in.as_seconds(),
                    s.cues.len()
                ));
            } else {
                out.push_str("speech: none yet\n");
            }
            let picture = looks.get(&id).map(shot_looks).unwrap_or_default();
            if let Some(l) = looks.get(&id) {
                out.push_str(&format!(
                    "look {} motion={:.2} scenes={} shots={}\n",
                    l.look,
                    l.motion,
                    l.scenes,
                    picture.len()
                ));
            }
            let cue_refs: Vec<(f64, f64, &str)> = speech
                .get(&id)
                .map(|s| {
                    s.cues
                        .iter()
                        .map(|c| (c.start.as_seconds(), c.end.as_seconds(), c.text.as_str()))
                        .collect()
                })
                .unwrap_or_default();
            let briefs = oc_media::brief_shots(&picture, &cue_refs);
            out.push_str(&compact_shots(&picture, &briefs));
            out
        }
        Inspect::GetMusic { media_id } => {
            let Some(row) = looks.get(&media_id.as_uuid()) else {
                return format!("no analysis for {}", media_id.as_uuid());
            };
            let music = row.raw.as_ref().and_then(|v| {
                serde_json::from_value::<oc_media::VisualDigest>(v.clone())
                    .ok()
                    .and_then(|d| d.music)
            });
            match music {
                Some(music) => oc_media::format_music(&music),
                None => format!("no beat grid for {}", media_id.as_uuid()),
            }
        }
        Inspect::FindShots {
            scale,
            camera,
            motion_dir,
            min_quality,
            subject,
            limit,
        } => find_shots(looks, scale, camera, motion_dir, min_quality, subject, limit),
        Inspect::ListCues { media_id } => {
            let id = media_id.as_uuid();
            let Some(s) = speech.get(&id) else {
                return format!("no cues for {id} yet");
            };
            format!(
                "cues {} words={} hook@{:.1}s\n{}",
                s.cues.len(),
                s.words,
                s.hook_in.as_seconds(),
                format_cues(&s.cues, 200)
            )
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
            source_in,
            kind,
            mode,
        } => {
            if let Some(row) = media.iter().find(|r| r.id == media_id.as_uuid()) {
                let (row_kind, row_dur) = spec_from_row(row);
                let remain = if source_in.as_ticks() > 0 && row_dur.as_ticks() > source_in.as_ticks()
                {
                    oc_core::Duration::from_ticks(row_dur.as_ticks() - source_in.as_ticks())
                } else {
                    row_dur
                };
                Op::PlaceMedia {
                    media_id,
                    track_id,
                    start,
                    duration: if duration.as_ticks() > 0 {
                        duration
                    } else {
                        remain
                    },
                    source_in,
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
                    source_in,
                    kind,
                    mode,
                }
            }
        }
        Op::Assemble {
            items,
            style,
            target_seconds,
        } => {
            let target = target_seconds.unwrap_or(45.0);
            let items = if items.is_empty() {
                media
                    .iter()
                    .rev()
                    .map(|row| {
                        item_from_row(row, speech.get(&row.id), looks.get(&row.id), target)
                    })
                    .collect()
            } else {
                items
                    .into_iter()
                    .map(|item| {
                        if let Some(row) = media.iter().find(|r| r.id == item.media_id.as_uuid()) {
                            item_from_row(row, speech.get(&row.id), looks.get(&row.id), target)
                        } else {
                            item
                        }
                    })
                    .collect()
            };
            Op::Assemble {
                items,
                style,
                target_seconds: Some(target),
            }
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
    target_seconds: f64,
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
        if duration.as_seconds() >= 20.0 && s.cues.len() >= 3 {
            let cues: Vec<_> = s
                .cues
                .iter()
                .map(|c| (c.start, c.end, c.text.as_str()))
                .collect();
            item.excerpts = pick_reel_excerpts(&cues, target_seconds);
        }
    }
    if let Some(l) = look {
        item.look = l.look.clone();
        item.motion = l.motion as f32;
        item.scenes = l.scenes.max(0) as u32;
    }
    item
}

fn compact_shots(picture: &[oc_media::ShotLook], briefs: &[oc_media::ShotBrief]) -> String {
    if briefs.is_empty() && picture.is_empty() {
        return String::new();
    }
    let mut hidden = 0;
    let mut out = String::new();
    let rows = if briefs.is_empty() {
        picture.len()
    } else {
        briefs.len()
    };
    for i in 0..rows {
        let start = briefs.get(i).map(|b| b.start).unwrap_or(picture[i].start);
        let end = briefs.get(i).map(|b| b.end).unwrap_or(picture[i].end);
        let shot = picture.iter().find(|s| (s.start - start).abs() < 0.4);
        let card = shot.and_then(|s| s.card.as_ref());
        if let Some(card) = card {
            if card.quality > 0 && card.quality < 5 {
                hidden += 1;
                continue;
            }
            let color = card.palette.first().map(String::as_str).unwrap_or("");
            let subject = shot.map(|s| s.subject.as_str()).unwrap_or("");
            let speech = briefs.get(i).map(|b| b.text.as_str()).unwrap_or("");
            let role = briefs.get(i).map(|b| b.role.as_str()).unwrap_or("silence");
            out.push_str(&format!(
                "{start:.1}-{end:.1} {} {subject} {} {} \"{}\" q{} {color} {role}",
                card.scale, card.camera, card.motion_dir, card.action, card.quality
            ));
            if !speech.is_empty() && role != "silence" {
                out.push(' ');
                out.push('"');
                out.push_str(&speech.chars().take(48).collect::<String>());
                out.push('"');
            }
            out.push('\n');
        } else if let Some(brief) = briefs.get(i) {
            let text = brief.text.chars().take(48).collect::<String>();
            out.push_str(&format!(
                "{:.1}-{:.1} {} {} {}\n",
                brief.start, brief.end, brief.look, brief.role.as_str(), text
            ));
        }
    }
    if hidden > 0 {
        out.push_str(&format!("{hidden} shots under q5 hidden\n"));
    }
    out
}

fn find_shots(
    looks: &HashMap<Uuid, oc_db::AnalysisRow>,
    scale: Option<String>,
    camera: Option<String>,
    motion_dir: Option<String>,
    min_quality: Option<u8>,
    subject: Option<String>,
    limit: usize,
) -> String {
    let mut lines = Vec::new();
    for (id, row) in looks {
        let shots = shot_looks(row);
        for shot in shots {
            let card = shot.card.as_ref();
            if let Some(want) = scale.as_deref() {
                let got = card.map(|c| c.scale.as_str()).unwrap_or(shot.look.as_str());
                if !got.eq_ignore_ascii_case(want) {
                    continue;
                }
            }
            if let Some(want) = camera.as_deref() {
                if card.map(|c| c.camera.as_str()) != Some(want) {
                    continue;
                }
            }
            if let Some(want) = motion_dir.as_deref() {
                if card.map(|c| c.motion_dir.as_str()) != Some(want) {
                    continue;
                }
            }
            if let Some(min) = min_quality {
                if card.map(|c| c.quality).unwrap_or(10) < min {
                    continue;
                }
            }
            if let Some(want) = subject.as_deref() {
                if !shot.subject.eq_ignore_ascii_case(want) {
                    continue;
                }
            }
            let scale = card.map(|c| c.scale.as_str()).unwrap_or(shot.look.as_str());
            lines.push(format!(
                "{id} {:.1}-{:.1} {scale} {}",
                shot.start, shot.end, shot.subject
            ));
            if lines.len() >= limit {
                return lines.join("\n");
            }
        }
    }
    if lines.is_empty() {
        "no shots matched".into()
    } else {
        lines.join("\n")
    }
}

async fn apply_submitted_plan(
    db: &Db,
    project_id: Uuid,
    name: &str,
    arguments: &serde_json::Value,
    media: &[oc_db::MediaRow],
    speech: &HashMap<Uuid, Speech>,
    looks: &HashMap<Uuid, oc_db::AnalysisRow>,
) -> Result<String, String> {
    let mut project = oc_db::get_project(db, project_id)
        .await
        .map_err(|e| e.to_string())?;
    let plan = if name == "revise_edit" {
        let current = project
            .timeline
            .edit_plan
            .clone()
            .ok_or_else(|| "no plan yet — call submit_edit first".to_string())?;
        let changes = arguments
            .get("changes")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        oc_core::revise_plan(&current, &changes)?
    } else {
        oc_core::plan_from_value(arguments)?
    };
    let windows = source_windows(media, speech, looks);
    let beats = plan
        .music_id
        .and_then(|id| music_beats(looks.get(&id.as_uuid())))
        .unwrap_or_default();
    let lines: Vec<oc_core::SpokenLine> = spoken(speech)
        .into_iter()
        .map(|s| oc_core::SpokenLine {
            media: s.media,
            start: s.start,
            end: s.end,
            text: s.text,
        })
        .collect();
    let covers: Vec<oc_core::CoverShot> = windows
        .iter()
        .filter(|w| w.silent && w.end - w.start >= 1.0)
        .map(|w| oc_core::CoverShot {
            media: w.media,
            start: w.start,
            end: w.end,
        })
        .collect();
    let notes = oc_core::build_plan(&mut project.timeline, &plan, &windows, &beats, &lines, &covers)?;
    oc_db::save_timeline(db, project_id, &project.timeline)
        .await
        .map_err(|e| e.to_string())?;
    let spoken = spoken(speech);
    let facts = oc_core::ReviewFacts {
        beats,
        has_music: plan.music_id.is_some(),
        ..oc_core::ReviewFacts::default()
    };
    let review = oc_core::review_with(&project.timeline, &spoken, "", &facts);
    Ok(format!("{}\n{}", notes.join("\n"), review.text))
}

fn source_windows(
    media: &[oc_db::MediaRow],
    speech: &HashMap<Uuid, Speech>,
    looks: &HashMap<Uuid, oc_db::AnalysisRow>,
) -> Vec<oc_core::SourceWindow> {
    let mut windows = Vec::new();
    for row in media {
        let (_, dur) = spec_from_row(row);
        let id = oc_core::MediaId::from_uuid(row.id);
        let shots = looks.get(&row.id).map(shot_looks).unwrap_or_default();
        if shots.is_empty() {
            if let Some(s) = speech.get(&row.id) {
                for cue in &s.cues {
                    windows.push(oc_core::SourceWindow {
                        media: id,
                        start: cue.start.as_seconds(),
                        end: cue.end.as_seconds(),
                        duration: dur.as_seconds(),
                        look: String::new(),
                        silent: false,
                    });
                }
            }
            windows.push(oc_core::SourceWindow {
                media: id,
                start: 0.0,
                end: 0.0,
                duration: dur.as_seconds().max(0.1),
                look: String::new(),
                silent: false,
            });
        } else {
            for shot in shots {
                let silent = !range_has_speech(speech, row.id, shot.start, shot.end);
                windows.push(oc_core::SourceWindow {
                    media: id,
                    start: shot.start,
                    end: shot.end,
                    duration: dur.as_seconds().max(shot.end),
                    look: shot.look.clone(),
                    silent,
                });
            }
        }
    }
    windows
}

fn range_has_speech(speech: &HashMap<Uuid, Speech>, id: Uuid, start: f64, end: f64) -> bool {
    speech.get(&id).is_some_and(|s| {
        s.cues.iter().any(|c| {
            c.end.as_seconds() > start + 0.2 && c.start.as_seconds() < end - 0.2
        })
    })
}

fn music_beats(row: Option<&oc_db::AnalysisRow>) -> Option<Vec<f64>> {
    let row = row?;
    let digest = serde_json::from_value::<oc_media::VisualDigest>(row.raw.clone()?).ok()?;
    Some(digest.music?.beats)
}

fn format_cues(cues: &[CueBrief], limit: usize) -> String {
    let mut out = String::new();
    for cue in cues.iter().take(limit) {
        out.push_str(&format!(
            "{:.1}-{:.1}  {}\n",
            cue.start.as_seconds(),
            cue.end.as_seconds(),
            cue.text.replace('\n', " ")
        ));
    }
    if cues.len() > limit {
        out.push_str(&format!("… {} more cues\n", cues.len() - limit));
    }
    out
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
                "    clip {} media={} start={:.2}s dur={:.2}s src_in={:.2}s\n",
                clip.id,
                media,
                clip.start.as_seconds(),
                clip.duration.as_seconds(),
                clip.source_in.as_seconds()
            ));
        }
    }
    if tl.tracks.iter().all(|t| t.clips.is_empty()) {
        out.push_str("(no clips yet)\n");
    }
    out
}
