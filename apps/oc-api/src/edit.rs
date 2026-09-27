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

/// One-pass sentence cutter. The chat no longer calls this: the model cuts with tools.
#[allow(dead_code)]
pub(crate) async fn cut_short_now(db: &Db, project_id: Uuid, request: &str) -> Option<String> {
    if !oc_core::wants_picture_finish(request) || oc_core::asks_for_judgment(request) {
        return None;
    }
    let mut project = oc_db::get_project(db, project_id).await.ok()?;
    let has_picture = project.timeline.tracks.iter().any(|t| {
        t.kind == TrackKind::Video && t.clips.iter().any(|c| !c.disabled)
    });
    if has_picture && oc_core::revises_existing_cut(request) {
        return None;
    }
    let media_rows = oc_db::list_media(db, project_id).await.ok()?;
    let transcripts = oc_db::list_transcripts_for_project(db, project_id)
        .await
        .ok()?;
    let speech = speech_by_media(&transcripts);
    let looks = look_by_media(
        &oc_db::list_analysis_for_project(db, project_id)
            .await
            .unwrap_or_default(),
    );
    let beats = piece_beats(&media_rows, &speech, &looks);
    let target = asked_seconds(request).unwrap_or(60.0);
    let picks = oc_core::choose_piece(&beats, request, target);
    if picks.iter().all(|p| p.cover) {
        return None;
    }
    let mut undo = UndoStack::new();
    apply(&mut project.timeline, &mut undo, Op::ClearTimeline).ok()?;
    let mut at = 0.0;
    for pick in picks.iter().filter(|p| !p.cover) {
        apply(
            &mut project.timeline,
            &mut undo,
            Op::PlaceMedia {
                media_id: pick.media,
                track_id: None,
                start: Time::from_seconds(pick.at),
                duration: oc_core::Duration::from_seconds(pick.duration),
                source_in: Time::from_seconds(pick.source_in),
                kind: TrackKind::Video,
                mode: oc_core::TimelineEditMode::Normal,
            },
        )
        .ok()?;
        at = pick.at + pick.duration;
    }
    for pick in picks.iter().filter(|p| p.cover) {
        apply(
            &mut project.timeline,
            &mut undo,
            Op::Cover {
                media_id: pick.media,
                at: Time::from_seconds(pick.at),
                source_in: Time::from_seconds(pick.source_in),
                duration: oc_core::Duration::from_seconds(pick.duration),
            },
        )
        .ok()?;
    }
    close_program_gaps(&mut project.timeline);
    oc_db::save_timeline(db, project_id, &project.timeline)
        .await
        .ok()?;
    let note = apply_finish(db, project_id, request, true).await.ok()?;
    let sources = picks.iter().filter(|p| !p.cover).map(|p| p.media).collect::<std::collections::HashSet<_>>().len();
    Some(format!(
        "Cut a {at:.0}s short from the strongest lines and shots across {sources} source(s). {note}"
    ))
}

fn piece_beats(
    media: &[oc_db::MediaRow],
    speech: &HashMap<Uuid, Speech>,
    looks: &HashMap<Uuid, oc_db::AnalysisRow>,
) -> Vec<oc_core::SourceBeat> {
    let mut beats = Vec::new();
    for row in media {
        if row.content_type.starts_with("image/") {
            continue;
        }
        let media_id = MediaId::from_uuid(row.id);
        let picture = looks.get(&row.id).map(shot_looks).unwrap_or_default();
        let cue_refs: Vec<(f64, f64, &str)> = speech
            .get(&row.id)
            .map(|s| {
                s.cues
                    .iter()
                    .map(|c| (c.start.as_seconds(), c.end.as_seconds(), c.text.as_str()))
                    .collect()
            })
            .unwrap_or_default();
        let briefs = oc_media::brief_shots(&picture, &cue_refs);
        if briefs.is_empty() {
            for (start, end, text) in cue_refs {
                beats.push(oc_core::SourceBeat {
                    media: media_id,
                    start,
                    end,
                    look: String::new(),
                    subject: String::new(),
                    role: "speech".into(),
                    text: text.to_string(),
                });
            }
            continue;
        }
        for brief in briefs {
            beats.push(oc_core::SourceBeat {
                media: media_id,
                start: brief.start,
                end: brief.end,
                look: brief.look,
                subject: brief.subject,
                role: brief.role.as_str().into(),
                text: brief.text,
            });
        }
    }
    beats
}

fn asked_seconds(request: &str) -> Option<f64> {
    let lower = request.to_ascii_lowercase();
    let mut num = String::new();
    for ch in lower.chars() {
        if ch.is_ascii_digit() {
            num.push(ch);
        } else if !num.is_empty() {
            break;
        }
    }
    let n: f64 = num.parse().ok()?;
    let rest = lower.split_once(&num)?.1.trim_start();
    if rest.starts_with("min") {
        Some(n * 60.0)
    } else if rest.starts_with("sec") || rest.starts_with('s') {
        Some(n)
    } else {
        None
    }
}

/// Grade, punch-in, fades, captions, vertical frame, and a cover when the cut is a reel.
pub(crate) async fn apply_finish(
    db: &Db,
    project_id: Uuid,
    request: &str,
    force: bool,
) -> Result<String, String> {
    if !force && !oc_core::wants_picture_finish(request) {
        return Ok(String::new());
    }
    let mut project = oc_db::get_project(db, project_id)
        .await
        .map_err(|e| e.to_string())?;
    close_program_gaps(&mut project.timeline);
    if oc_core::already_finished(&project.timeline) {
        oc_db::save_timeline(db, project_id, &project.timeline)
            .await
            .map_err(|e| e.to_string())?;
        return Ok(String::new());
    }
    let transcripts = oc_db::list_transcripts_for_project(db, project_id)
        .await
        .map_err(|e| e.to_string())?;
    let speech = speech_by_media(&transcripts);
    let looks = look_by_media(
        &oc_db::list_analysis_for_project(db, project_id)
            .await
            .unwrap_or_default(),
    );
    let lines = spoken_lines(&speech);
    let covers = cover_shots(&looks, &speech);
    let ops = oc_core::finish_reel(&project.timeline, &lines, &covers, request);
    if ops.is_empty() {
        return Ok(String::new());
    }
    let mut undo = UndoStack::new();
    let mut notes = Vec::new();
    for op in ops {
        let applied = apply(&mut project.timeline, &mut undo, op).map_err(|e| e.to_string())?;
        notes.push(applied.note);
    }
    oc_db::save_timeline(db, project_id, &project.timeline)
        .await
        .map_err(|e| e.to_string())?;
    tracing::info!(project = %project_id, ops = notes.len(), "picture finish");
    Ok(format!("finish: {}", notes.join("; ")))
}

/// Pull later picture back so a hole cannot skip the playhead from one clip to the next.
fn close_program_gaps(timeline: &mut Timeline) {
    for track in &mut timeline.tracks {
        if track.hidden || track.kind != TrackKind::Video {
            continue;
        }
        let mut order: Vec<usize> = (0..track.clips.len())
            .filter(|&i| {
                !track.clips[i].disabled
                    && matches!(track.clips[i].kind, oc_core::ClipKind::Video { .. })
            })
            .collect();
        order.sort_by(|&a, &b| track.clips[a].start.cmp(&track.clips[b].start));
        let mut cursor = order
            .first()
            .map(|&i| track.clips[i].start)
            .unwrap_or(Time::ZERO);
        for i in order {
            if track.clips[i].start.as_seconds() > cursor.as_seconds() + 0.08 {
                track.clips[i].start = cursor;
            }
            cursor = track.clips[i].end();
        }
    }
}

fn spoken_lines(speech: &HashMap<Uuid, Speech>) -> Vec<oc_core::SpokenLine> {
    speech
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
        .collect()
}

fn cover_shots(
    looks: &HashMap<Uuid, oc_db::AnalysisRow>,
    speech: &HashMap<Uuid, Speech>,
) -> Vec<oc_core::CoverShot> {
    let mut out = Vec::new();
    for (id, row) in looks {
        let picture = shot_looks(row);
        let cues: Vec<(f64, f64, &str)> = speech
            .get(id)
            .map(|s| {
                s.cues
                    .iter()
                    .map(|c| (c.start.as_seconds(), c.end.as_seconds(), c.text.as_str()))
                    .collect()
            })
            .unwrap_or_default();
        for brief in oc_media::brief_shots(&picture, &cues) {
            if brief.role != oc_media::ShotRole::Silence || brief.end - brief.start < 1.0 {
                continue;
            }
            out.push(oc_core::CoverShot {
                media: MediaId::from_uuid(*id),
                start: brief.start,
                end: brief.end,
            });
        }
    }
    out
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
        let card = picture.iter().find(|s| (s.start - start).abs() < 0.4).and_then(|s| s.card.as_ref());
        if let Some(card) = card {
            if card.quality > 0 && card.quality < 5 {
                hidden += 1;
                continue;
            }
            let color = card.palette.first().map(String::as_str).unwrap_or("");
            let speech = briefs.get(i).map(|b| b.text.as_str()).unwrap_or("");
            let role = briefs.get(i).map(|b| b.role.as_str()).unwrap_or("silence");
            out.push_str(&format!(
                "{start:.1}-{end:.1} {} {} {} \"{}\" q{} {color} {role}",
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
    let notes = oc_core::build_plan(&mut project.timeline, &plan, &windows, &beats)?;
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
                    });
                }
            }
            windows.push(oc_core::SourceWindow {
                media: id,
                start: 0.0,
                end: 0.0,
                duration: dur.as_seconds().max(0.1),
            });
        } else {
            for shot in shots {
                windows.push(oc_core::SourceWindow {
                    media: id,
                    start: shot.start,
                    end: shot.end,
                    duration: dur.as_seconds().max(shot.end),
                });
            }
        }
    }
    windows
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
