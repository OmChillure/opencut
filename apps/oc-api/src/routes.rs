use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use oc_core::{
    AssembleItem, AssembleStyle, MediaId, Op, Project, ProjectId, Time, Timeline, TrackKind,
    UndoStack,
    apply, is_director_request, mcp_tools, op_from_mcp,
};
use oc_core::time::TICKS_PER_SECOND;
use oc_providers::{ChatTurn, LlmReply};
use oc_media::{ObjectKind, object_key};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use uuid::Uuid;

type ApiResult<T> = Result<T, ApiError>;

pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

impl From<oc_db::DbError> for ApiError {
    fn from(value: oc_db::DbError) -> Self {
        match value {
            oc_db::DbError::NotFound => Self::new(StatusCode::NOT_FOUND, "not found"),
            other => Self::new(StatusCode::INTERNAL_SERVER_ERROR, other.to_string()),
        }
    }
}

impl axum::response::IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let body = serde_json::json!({ "error": self.message });
        (self.status, Json(body)).into_response()
    }
}

pub async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": true }))
}

#[derive(Deserialize)]
pub struct CreateProjectBody {
    pub name: Option<String>,
}

pub async fn create_project(
    State(state): State<AppState>,
    Json(body): Json<CreateProjectBody>,
) -> ApiResult<Json<Project>> {
    let name = body.name.unwrap_or_else(|| "Untitled".into());
    Ok(Json(oc_db::create_project(&state.db, &name).await?))
}

pub async fn list_projects(
    State(state): State<AppState>,
) -> ApiResult<Json<Vec<oc_db::ProjectRow>>> {
    Ok(Json(oc_db::list_projects(&state.db).await?))
}

pub async fn get_project(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Project>> {
    Ok(Json(oc_db::get_project(&state.db, id).await?))
}

#[derive(Deserialize)]
pub struct UpdateProjectBody {
    pub name: String,
}

pub async fn update_project(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateProjectBody>,
) -> ApiResult<Json<Project>> {
    let name = body.name.trim();
    if name.is_empty() {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "name required"));
    }
    Ok(Json(
        oc_db::rename_project(&state.db, id, name).await?,
    ))
}

#[derive(Serialize)]
pub struct DeleteProjectResponse {
    pub ok: bool,
    pub r2_deleted: u32,
}

pub async fn delete_project(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<DeleteProjectResponse>> {
    let keys = oc_db::delete_project(&state.db, id).await?;
    let mut r2_deleted = 0u32;
    if let Some(r2) = &state.r2 {
        for key in &keys {
            match r2.delete_object(key).await {
                Ok(()) => r2_deleted += 1,
                Err(err) => tracing::warn!(key, "{err}"),
            }
        }
        for kind in [
            ObjectKind::Raw,
            ObjectKind::Proxy,
            ObjectKind::Audio,
            ObjectKind::Export,
        ] {
            let prefix = format!("{}/{id}/", kind.prefix());
            match r2.delete_prefix(&prefix).await {
                Ok(n) => r2_deleted += n,
                Err(err) => tracing::warn!(prefix, "{err}"),
            }
        }
    }
    Ok(Json(DeleteProjectResponse {
        ok: true,
        r2_deleted,
    }))
}

#[derive(Deserialize)]
pub struct ApplyOpsBody {
    pub ops: Vec<Op>,
}

#[derive(Serialize)]
pub struct ApplyOpsResponse {
    pub timeline: Timeline,
    pub notes: Vec<String>,
}

pub async fn apply_ops(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<ApplyOpsBody>,
) -> ApiResult<Json<ApplyOpsResponse>> {
    let mut project = oc_db::get_project(&state.db, id).await?;
    let media = oc_db::list_media(&state.db, id).await?;
    let transcripts = oc_db::list_transcripts_for_project(&state.db, id).await?;
    let speech = speech_by_media(&transcripts);
    let mut undo = UndoStack::new();
    let mut notes = Vec::new();
    for op in body.ops {
        let op = hydrate_op(op, &media, &speech);
        match apply(&mut project.timeline, &mut undo, op) {
            Ok(applied) => notes.push(applied.note),
            Err(err) => return Err(ApiError::new(StatusCode::BAD_REQUEST, err.to_string())),
        }
    }
    oc_db::save_timeline(&state.db, id, &project.timeline).await?;
    Ok(Json(ApplyOpsResponse {
        timeline: project.timeline,
        notes,
    }))
}

pub async fn list_ai_providers() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "providers": oc_providers::catalog() }))
}

#[derive(Deserialize)]
pub struct ChatBody {
    pub provider: String,
    pub model: String,
    pub messages: Vec<ChatTurn>,
}

#[derive(Serialize)]
pub struct ChatResponse {
    pub text: String,
    pub notes: Vec<String>,
    pub timeline: Timeline,
}

pub async fn chat(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<ChatBody>,
) -> ApiResult<Json<ChatResponse>> {
    let mut project = oc_db::get_project(&state.db, id).await?;
    let media = oc_db::list_media(&state.db, id).await?;
    let transcripts = oc_db::list_transcripts_for_project(&state.db, id).await?;
    let speech = speech_by_media(&transcripts);
    let mut notes = Vec::new();
    let last_user = body
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| m.content.as_str())
        .unwrap_or("");
    if is_director_request(last_user) {
        if media.is_empty() {
            return Ok(Json(ChatResponse {
                text: "Import some clips first, then ask me to make the video.".into(),
                notes: Vec::new(),
                timeline: project.timeline,
            }));
        }
        let heard = media.iter().any(|m| speech.contains_key(&m.id));
        let needs_listen = media.iter().any(|m| {
            !m.content_type.starts_with("image/")
                && !m.content_type.starts_with("audio/")
                && !speech.contains_key(&m.id)
        });
        if needs_listen {
            for row in &media {
                if row.content_type.starts_with("image/") {
                    continue;
                }
                if speech.contains_key(&row.id) {
                    continue;
                }
                let _ = oc_db::enqueue_job(
                    &state.db,
                    "transcribe",
                    serde_json::json!({
                        "project_id": id,
                        "media_id": row.id,
                        "r2_key": row.r2_key,
                    }),
                )
                .await;
            }
            if !heard {
                oc_db::save_timeline(&state.db, id, &project.timeline).await?;
                return Ok(Json(ChatResponse {
                    text: "Listening to your clips (speech-to-text). Ask again in a few seconds and I’ll cut the short from what was said — not from the file names.".into(),
                    notes: Vec::new(),
                    timeline: project.timeline,
                }));
            }
        }
        let mut undo = UndoStack::new();
        let op = hydrate_op(
            Op::Assemble {
                items: Vec::new(),
                style: AssembleStyle::Vlog,
            },
            &media,
            &speech,
        );
        match apply(&mut project.timeline, &mut undo, op) {
            Ok(applied) => notes.push(applied.note),
            Err(err) => {
                return Ok(Json(ChatResponse {
                    text: format!("Couldn't cut the short: {err}"),
                    notes: Vec::new(),
                    timeline: project.timeline,
                }));
            }
        }
    }
    let tools = mcp_tools();
    let directed = !notes.is_empty();
    let system = if directed {
        format!(
            "You are OpenCut, an AI video director. A short is already cut on the timeline \
             from the imported bin. Do not assemble again unless the user asks for a redo.\n\
             Explain the cut in 2-4 short sentences (hook, A-roll, B-roll, music). \
             Only emit TOOL lines if they asked for a specific change.\n\
             Project '{}'.\n\n{}\n\n{}",
            project.name,
            media_brief(&media, &speech),
            timeline_brief(&project.timeline)
        )
    } else {
        format!(
            "You are OpenCut, an AI video director. You decide order from what is SAID and shown, \
             never from filenames. Never ask the user to order clips. If they want a video/vlog/short, \
             call assemble with style=vlog and omit media_ids.\n\
             Times are seconds. Use media ids from the bin.\n\
             Project '{}'.\n\n{}\n\n{}",
            project.name,
            media_brief(&media, &speech),
            timeline_brief(&project.timeline)
        )
    };
    let mut turns = body.messages;
    let mut text = String::new();
    for _ in 0..8 {
        let reply = match oc_providers::complete(
            &body.provider,
            &body.model,
            &system,
            &turns,
            &tools,
        )
        .await
        {
            Ok(reply) => reply,
            Err(err) if directed => {
                text = format!(
                    "Cut a short from your clips and laid it on the timeline. Play it. ({err})"
                );
                break;
            }
            Err(err) => {
                return Err(ApiError::new(StatusCode::BAD_GATEWAY, err.to_string()));
            }
        };
        match reply {
            LlmReply::Text(t) => {
                text = t;
                break;
            }
            LlmReply::Tools(calls) => {
                let mut batch = String::new();
                let mut undo = UndoStack::new();
                for call in calls {
                    match op_from_mcp(&call) {
                        Ok(op) => match apply(&mut project.timeline, &mut undo, hydrate_op(op, &media, &speech)) {
                            Ok(applied) => {
                                notes.push(applied.note.clone());
                                batch.push_str(&applied.note);
                                batch.push('\n');
                            }
                            Err(err) => batch.push_str(&format!("tool error: {err}\n")),
                        },
                        Err(err) => batch.push_str(&format!("bad tool {}: {err}\n", call.name)),
                    }
                }
                turns.push(ChatTurn {
                    role: "assistant".into(),
                    content: format!("Called tools:\n{batch}"),
                });
                turns.push(ChatTurn {
                    role: "user".into(),
                    content: "Done. Reply briefly with what you changed.".into(),
                });
            }
        }
    }
    oc_db::save_timeline(&state.db, id, &project.timeline).await?;
    if text.is_empty() && !notes.is_empty() {
        text = notes.join(" · ");
    }
    Ok(Json(ChatResponse {
        text,
        notes,
        timeline: project.timeline,
    }))
}

struct Speech {
    words: u32,
    speech_seconds: f64,
    hook_in: Time,
    text: String,
}

fn speech_by_media(rows: &[oc_db::TranscriptCueRow]) -> std::collections::HashMap<Uuid, Speech> {
    use std::collections::HashMap;
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

fn media_brief(
    rows: &[oc_db::MediaRow],
    speech: &std::collections::HashMap<Uuid, Speech>,
) -> String {
    if rows.is_empty() {
        return "Media bin: (empty — ask the user to import clips first)\n".into();
    }
    let mut out = String::from(
        "Media bin (understand from SPEECH, not filenames):\n",
    );
    for row in rows {
        let (kind, dur) = spec_from_row(row);
        match speech.get(&row.id) {
            Some(s) if s.words > 0 => {
                let excerpt: String = s.text.chars().take(180).collect();
                out.push_str(&format!(
                    "- id={}  {:?}  {:.1}s  SPEECH words={}  hook@{:.1}s  \"{}\"\n",
                    row.id,
                    kind,
                    dur.as_seconds(),
                    s.words,
                    s.hook_in.as_seconds(),
                    excerpt.replace('\n', " ")
                ));
            }
            _ => {
                out.push_str(&format!(
                    "- id={}  {:?}  {:.1}s  (no speech — treat as B-roll / music / still)\n",
                    row.id,
                    kind,
                    dur.as_seconds()
                ));
            }
        }
    }
    out
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

fn hydrate_op(
    op: Op,
    media: &[oc_db::MediaRow],
    speech: &std::collections::HashMap<Uuid, Speech>,
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
                    .map(|row| item_from_row(row, speech.get(&row.id)))
                    .collect()
            } else {
                items
                    .into_iter()
                    .map(|item| {
                        if let Some(row) = media.iter().find(|r| r.id == item.media_id.as_uuid()) {
                            item_from_row(row, speech.get(&row.id))
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

fn item_from_row(row: &oc_db::MediaRow, speech: Option<&Speech>) -> AssembleItem {
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
    item
}

fn timeline_brief(tl: &Timeline) -> String {
    let mut out = String::from("Current timeline:\n");
    for track in &tl.tracks {
        out.push_str(&format!("- track {} ({:?}) {}\n", track.id, track.kind, track.name));
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

#[derive(Deserialize)]
pub struct UploadBody {
    pub filename: String,
    pub content_type: Option<String>,
}

#[derive(Serialize)]
pub struct UploadResponse {
    pub media_id: Uuid,
    pub key: String,
    pub upload_url: Option<String>,
}

pub async fn request_upload(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<UploadBody>,
) -> ApiResult<Json<UploadResponse>> {
    let _ = oc_db::get_project(&state.db, id).await?;
    let media_id = MediaId::new();
    let project_id = ProjectId::from_uuid(id);
    let content_type = body
        .content_type
        .unwrap_or_else(|| "application/octet-stream".into());
    let key = object_key(ObjectKind::Raw, project_id, media_id, &body.filename);
    oc_db::insert_media(
        &state.db,
        id,
        media_id.as_uuid(),
        &key,
        &body.filename,
        &content_type,
    )
    .await?;
    let upload_url = if let Some(r2) = &state.r2 {
        Some(
            r2.presign_put(&key, &content_type, Duration::from_secs(3600))
                .await
                .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, e.to_string()))?,
        )
    } else {
        None
    };
    Ok(Json(UploadResponse {
        media_id: media_id.as_uuid(),
        key,
        upload_url,
    }))
}

#[derive(Serialize)]
pub struct MediaOut {
    pub id: Uuid,
    pub project_id: Uuid,
    pub filename: String,
    pub content_type: String,
    pub duration_ticks: Option<i64>,
    pub status: String,
    pub play_url: Option<String>,
}

pub async fn list_media(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<MediaOut>>> {
    let rows = oc_db::list_media(&state.db, id).await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let play_url = media_play_url(&state, &row.r2_key).await;
        out.push(MediaOut {
            id: row.id,
            project_id: row.project_id,
            filename: row.filename,
            content_type: row.content_type,
            duration_ticks: row.duration_ticks,
            status: row.status,
            play_url,
        });
    }
    Ok(Json(out))
}

async fn media_play_url(state: &AppState, key: &str) -> Option<String> {
    let base = std::env::var("R2_PUBLIC_BASE_URL").ok().filter(|s| !s.is_empty());
    if let Some(base) = base {
        return Some(format!("{}/{key}", base.trim_end_matches('/')));
    }
    if let Some(r2) = &state.r2 {
        return r2
            .presign_get(key, Duration::from_secs(6 * 3600))
            .await
            .ok();
    }
    None
}

#[derive(Deserialize)]
pub struct PatchMediaBody {
    pub duration_seconds: Option<f64>,
}

pub async fn patch_media(
    State(state): State<AppState>,
    Path((id, media_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<PatchMediaBody>,
) -> ApiResult<Json<serde_json::Value>> {
    let media = oc_db::get_media(&state.db, media_id).await?;
    if media.project_id != id {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "media not in project"));
    }
    if let Some(seconds) = body.duration_seconds {
        if seconds.is_finite() && seconds > 0.0 {
            let ticks = (seconds * TICKS_PER_SECOND as f64).round() as i64;
            oc_db::set_media_duration(&state.db, media_id, ticks).await?;
        }
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn complete_upload(
    State(state): State<AppState>,
    Path((id, media_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<serde_json::Value>> {
    let media = oc_db::get_media(&state.db, media_id).await?;
    oc_db::set_media_status(&state.db, media_id, "ready").await?;
    if !media.content_type.starts_with("image/") {
        let _ = oc_db::enqueue_job(
            &state.db,
            "transcribe",
            serde_json::json!({
                "project_id": id,
                "media_id": media_id,
                "r2_key": media.r2_key,
            }),
        )
        .await;
        oc_db::set_media_status(&state.db, media_id, "transcribing").await?;
    }
    Ok(Json(serde_json::json!({ "status": "ready" })))
}

pub async fn transcribe_media(
    State(state): State<AppState>,
    Path((id, media_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<serde_json::Value>> {
    let media = oc_db::get_media(&state.db, media_id).await?;
    if media.project_id != id {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "media not in project"));
    }
    let job = oc_db::enqueue_job(
        &state.db,
        "transcribe",
        serde_json::json!({
            "project_id": id,
            "media_id": media_id,
            "r2_key": media.r2_key,
        }),
    )
    .await?;
    oc_db::set_media_status(&state.db, media_id, "transcribing").await?;
    Ok(Json(serde_json::json!({ "job_id": job })))
}
