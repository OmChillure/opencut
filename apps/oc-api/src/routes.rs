use crate::edit::{self, hydrate_op, look_by_media, speech_by_media};
use crate::state::AppState;
use axum::Json;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::Response;
use futures_util::StreamExt;
use oc_core::{
    MediaId, Op, Project, ProjectId, Timeline, UndoStack, apply, is_director_request, mcp_tools,
    review_cut,
};
use oc_core::time::TICKS_PER_SECOND;
use oc_providers::{ChatEvent, ChatTurn, LlmReply};
use oc_media::{ObjectKind, object_key};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
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
        for key in keys.iter().filter(|key| oc_db::is_r2_object_key(key)) {
            match r2.delete_object(key).await {
                Ok(()) => r2_deleted += 1,
                Err(err) => tracing::warn!(key, "R2 delete skipped: {err}"),
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
    let looks = look_by_media(
        &oc_db::list_analysis_for_project(&state.db, id)
            .await
            .unwrap_or_default(),
    );
    let mut undo = UndoStack::new();
    let mut notes = Vec::new();
    let mut exports = Vec::new();
    for op in body.ops {
        if let Op::Export { preset } = &op {
            exports.push(*preset);
        }
        let op = hydrate_op(op, &media, &speech, &looks);
        match apply(&mut project.timeline, &mut undo, op) {
            Ok(applied) => notes.push(applied.note),
            Err(err) => return Err(ApiError::new(StatusCode::BAD_REQUEST, err.to_string())),
        }
    }
    oc_db::save_timeline(&state.db, id, &project.timeline).await?;
    for preset in exports {
        if let Err(err) = edit::queue_export(&state.db, id, preset).await {
            notes.push(format!("export queue failed: {err}"));
        } else {
            notes.push(format!("export job queued ({preset:?})"));
        }
    }
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
#[allow(dead_code)]
pub struct ChatResponse {
    pub text: String,
    pub notes: Vec<String>,
    pub timeline: Timeline,
}

pub async fn chat(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<ChatBody>,
) -> Result<Response, ApiError> {
    tracing::info!(
        project = %id,
        provider = %body.provider,
        model = %body.model,
        messages = body.messages.len(),
        "chat request"
    );
    let (tx, rx) = mpsc::channel::<String>(64);
    let _ = tx.try_send(line(&serde_json::json!({
        "type": "status",
        "text": format!("Starting {} · {}", body.provider, body.model),
    })));
    tokio::spawn(async move {
        if let Err(err) = run_chat(state, id, body, tx.clone()).await {
            tracing::error!(project = %id, "chat failed: {err}");
            let _ = tx
                .send(line(&serde_json::json!({
                    "type": "error",
                    "text": err,
                })))
                .await;
        }
    });
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx)
        .map(|chunk| Ok::<_, std::convert::Infallible>(chunk));
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/x-ndjson")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from_stream(stream))
        .expect("ndjson response"))
}

fn line(value: &serde_json::Value) -> String {
    format!("{value}\n")
}

async fn wait_for_understand(state: &AppState, tx: &mpsc::Sender<String>, jobs: &[Uuid]) {
    let secs = std::env::var("OPENCUT_UNDERSTAND_WAIT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|n: &u64| *n > 0)
        .unwrap_or(180);
    let deadline = Instant::now() + Duration::from_secs(secs);
    let mut last_note = Instant::now();
    loop {
        let pending = jobs.len()
            - futures(jobs, state).await;
        if pending == 0 {
            push(
                tx,
                serde_json::json!({"type":"status","text":"Shot list is ready."}),
            )
            .await;
            return;
        }
        if Instant::now() >= deadline {
            push(
                tx,
                serde_json::json!({
                    "type": "status",
                    "text": "Shot list is still running. Cutting with what is ready — ask again if the bin has no shots yet."
                }),
            )
            .await;
            return;
        }
        if last_note.elapsed() >= Duration::from_secs(8) {
            push(
                tx,
                serde_json::json!({
                    "type": "status",
                    "text": format!("Still watching clips ({pending} left) before cutting.")
                }),
            )
            .await;
            last_note = Instant::now();
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

async fn futures(jobs: &[Uuid], state: &AppState) -> usize {
    let mut done = 0;
    for id in jobs {
        if oc_db::job_finished(&state.db, *id).await.unwrap_or(false) {
            done += 1;
        }
    }
    done
}

async fn fresh_review(state: &AppState, id: Uuid, request: &str) -> oc_core::CutReview {
    let timeline = oc_db::get_project(&state.db, id)
        .await
        .map(|p| p.timeline)
        .unwrap_or_default();
    let rows = oc_db::list_transcripts_for_project(&state.db, id)
        .await
        .unwrap_or_default();
    let speech = speech_by_media(&rows);
    review_cut(&timeline, &edit::spoken(&speech), request)
}

async fn push(tx: &mpsc::Sender<String>, value: serde_json::Value) {
    let _ = tx.send(line(&value)).await;
}

async fn forward_events(mut rx: mpsc::Receiver<ChatEvent>, tx: mpsc::Sender<String>) {
    while let Some(ev) = rx.recv().await {
        if let Ok(value) = serde_json::to_value(&ev) {
            let _ = tx.send(line(&value)).await;
        }
    }
}

async fn run_chat(
    state: AppState,
    id: Uuid,
    body: ChatBody,
    tx: mpsc::Sender<String>,
) -> Result<(), String> {
    let mut project = oc_db::get_project(&state.db, id)
        .await
        .map_err(|e| e.to_string())?;
    let media = oc_db::list_media(&state.db, id)
        .await
        .map_err(|e| e.to_string())?;
    let transcripts = oc_db::list_transcripts_for_project(&state.db, id)
        .await
        .map_err(|e| e.to_string())?;
    let speech = speech_by_media(&transcripts);
    let looks = oc_db::list_analysis_for_project(&state.db, id)
        .await
        .unwrap_or_default();
    let looks = look_by_media(&looks);
    let mut notes = Vec::new();
    let last_user = body
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| m.content.clone())
        .unwrap_or_default();
    tracing::info!(
        project = %id,
        media = media.len(),
        transcripts = transcripts.len(),
        looks = looks.len(),
        director = is_director_request(&last_user),
        user = %last_user.chars().take(120).collect::<String>(),
        "chat context"
    );
    if !media.is_empty() {
        let understood = media.iter().any(|m| {
            speech.contains_key(&m.id) || looks.contains_key(&m.id)
        });
        let needs_scan = media.iter().any(|m| {
            if m.content_type.starts_with("image/") {
                return false;
            }
            let stale = looks.get(&m.id).is_some_and(|l| {
                l.has_video && crate::edit::shot_looks(l).is_empty()
            });
            stale || (!speech.contains_key(&m.id) && !looks.contains_key(&m.id))
        });
        if needs_scan {
            push(
                &tx,
                serde_json::json!({"type":"status","text":"Watching clips before the cut (shot list, then words)"}),
            )
            .await;
            let mut jobs = Vec::new();
            for row in &media {
                if row.content_type.starts_with("image/") {
                    continue;
                }
                let stale_look = looks.get(&row.id).is_some_and(|l| {
                    l.has_video && crate::edit::shot_looks(l).is_empty()
                });
                if (speech.contains_key(&row.id) || looks.contains_key(&row.id)) && !stale_look
                {
                    continue;
                }
                let on_disk = oc_db::local_media_path(&row.r2_key)
                    .is_some_and(|p| p.is_file())
                    || std::path::Path::new(&row.r2_key).is_file();
                if !oc_db::is_r2_object_key(&row.r2_key) && !on_disk {
                    continue;
                }
                if let Ok(job) = oc_db::enqueue_job(
                    &state.db,
                    "transcribe",
                    serde_json::json!({
                        "project_id": id,
                        "media_id": row.id,
                        "r2_key": row.r2_key,
                    }),
                )
                .await
                {
                    jobs.push(job);
                }
            }
            if !jobs.is_empty() {
                wait_for_understand(&state, &tx, &jobs).await;
            } else if !understood {
                push(
                    &tx,
                    serde_json::json!({
                        "type": "status",
                        "text": "No file on disk to watch yet. Re-import the clip."
                    }),
                )
                .await;
            }
        }
    }
    let tools = mcp_tools();
    let system = format!(
        "You are the picture editor for project '{}'. Do what the user asked — \
         reel, trim, recut, captions, silence, whatever. There is no default cut. \
         Call list_bin and list_timeline first. For a long file, call get_media. \
         It returns a shot list: start-end, look, subject (person, product, street, …), \
         speech|silence|filler, and the words. Place excerpts on those times. \
         Drop filler and long silence unless asked to keep them. \
         After tools, a cut review lists fix: lines (length, late hook, jump cut, stacked talk). \
         Fix those with tools before you say the cut is done. Do not assume the \
         bin is empty. Never describe an edit you did not make with tools.",
        project.name
    );
    let mut turns = body.messages;
    let mut text = String::new();
    let (ev_tx, ev_rx) = mpsc::channel::<ChatEvent>(64);
    let pump = tokio::spawn(forward_events(ev_rx, tx.clone()));
    let mcp_servers = crate::mcp::builtin_mcp_acp(&id.to_string())
        .into_iter()
        .collect::<Vec<_>>();
    tracing::info!(
        project = %id,
        mcp = mcp_servers.len(),
        tools = tools.len(),
        "starting provider"
    );
    if mcp_servers.is_empty() {
        tracing::warn!(project = %id, "no opencut MCP server — TOOL-line fallback only");
    }
    for turn_i in 0..8 {
        let reply = match oc_providers::complete_stream(
            &body.provider,
            &body.model,
            &system,
            &turns,
            &tools,
            Some(ev_tx.clone()),
            &mcp_servers,
        )
        .await
        {
            Ok(reply) => reply,
            Err(err) => {
                tracing::error!(project = %id, "provider failed: {err}");
                drop(ev_tx);
                let _ = pump.await;
                return Err(err.to_string());
            }
        };
        match reply {
            LlmReply::Text(t) => {
                let review = fresh_review(&state, id, &last_user).await;
                if review.issues && turn_i + 1 < 8 {
                    tracing::info!(project = %id, "cut review rejected a finished reply");
                    turns.push(ChatTurn {
                        role: "assistant".into(),
                        content: t,
                    });
                    turns.push(ChatTurn {
                        role: "user".into(),
                        content: format!(
                            "{}\nThose fix: lines are still open. Correct them with tools. \
                             Do not describe the cut as done.",
                            review.text
                        ),
                    });
                    continue;
                }
                text = t;
                break;
            }
            LlmReply::Tools(calls) => {
                let mut batch = String::new();
                for (i, call) in calls.into_iter().enumerate() {
                    let tool_id = format!("host-{turn_i}-{i}-{}", call.name);
                    emit_host_tool(
                        &tx,
                        &tool_id,
                        &call.name,
                        call.arguments.clone(),
                        None,
                        "pending",
                    )
                    .await;
                    tracing::info!(tool = %call.name, "host tool");
                    let (ok, result) = match edit::call_tool(
                        &state.db,
                        id,
                        &call.name,
                        call.arguments.clone(),
                    )
                    .await
                    {
                        Ok(out) => {
                            if !out.starts_with("bin:")
                                && !out.starts_with("Current timeline")
                                && !out.starts_with("media ")
                            {
                                notes.push(out.clone());
                            }
                            batch.push_str(&out);
                            batch.push('\n');
                            (true, out)
                        }
                        Err(err) => {
                            let msg = format!("tool error: {err}");
                            batch.push_str(&msg);
                            batch.push('\n');
                            (false, msg)
                        }
                    };
                    emit_host_tool(
                        &tx,
                        &tool_id,
                        &call.name,
                        call.arguments,
                        Some(result),
                        if ok { "done" } else { "error" },
                    )
                    .await;
                }
                turns.push(ChatTurn {
                    role: "assistant".into(),
                    content: format!("Called tools:\n{batch}"),
                });
                let review = fresh_review(&state, id, &last_user).await;
                turns.push(ChatTurn {
                    role: "user".into(),
                    content: format!(
                        "Tool results above.\n{review}\n\
                         If a line starts with \"fix:\", correct it with tools unless the user asked for it. \
                         If the cut matches the request, reply in 2–4 sentences.",
                        review = review.text
                    ),
                });
            }
        }
    }
    drop(ev_tx);
    let _ = pump.await;
    // MCP + call_tool already persist. Reload so we don't clobber those edits.
    if let Ok(fresh) = oc_db::get_project(&state.db, id).await {
        project = fresh;
    }
    if text.is_empty() && !notes.is_empty() {
        text = notes.join(" · ");
    }
    tracing::info!(
        project = %id,
        notes = notes.len(),
        chars = text.len(),
        "chat done"
    );
    finish_chat(&tx, &text, &notes, &project.timeline).await;
    Ok(())
}

async fn emit_host_tool(
    tx: &mpsc::Sender<String>,
    id: &str,
    name: &str,
    args: serde_json::Value,
    result: Option<String>,
    status: &str,
) {
    push(
        tx,
        serde_json::json!({
            "type": "tool",
            "id": id,
            "name": name,
            "args": args,
            "result": result,
            "status": status,
        }),
    )
    .await;
}

async fn finish_chat(
    tx: &mpsc::Sender<String>,
    text: &str,
    notes: &[String],
    timeline: &Timeline,
) {
    push(
        tx,
        serde_json::json!({
            "type": "done",
            "text": text,
            "notes": notes,
            "timeline": timeline,
        }),
    )
    .await;
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

#[derive(Deserialize)]
pub struct RegisterMediaBody {
    pub id: Option<Uuid>,
    pub filename: String,
    pub content_type: Option<String>,
    pub duration_seconds: Option<f64>,
}

pub async fn put_media_bytes(
    State(state): State<AppState>,
    Path((id, media_id)): Path<(Uuid, Uuid)>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> ApiResult<Json<serde_json::Value>> {
    let media = oc_db::get_media(&state.db, media_id).await?;
    if media.project_id != id {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "media not in project"));
    }
    let ctype = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or(&media.content_type)
        .to_string();
    let key = store_media_bytes(&state, id, media_id, &media, &ctype, body.to_vec()).await?;
    oc_db::set_media_r2_key(&state.db, media_id, &key).await?;
    if !ctype.starts_with("image/") {
        let _ = oc_db::enqueue_job(
            &state.db,
            "transcribe",
            serde_json::json!({
                "project_id": id,
                "media_id": media_id,
                "r2_key": key,
            }),
        )
        .await;
        oc_db::set_media_status(&state.db, media_id, "transcribing").await?;
    }
    Ok(Json(serde_json::json!({ "ok": true, "key": key })))
}

pub async fn register_media(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<RegisterMediaBody>,
) -> ApiResult<Json<serde_json::Value>> {
    let _ = oc_db::get_project(&state.db, id).await?;
    let media_id = body.id.unwrap_or_else(Uuid::now_v7);
    let ctype = body
        .content_type
        .unwrap_or_else(|| "application/octet-stream".into());
    let ticks = body
        .duration_seconds
        .filter(|s| s.is_finite() && *s > 0.0)
        .map(|s| (s * TICKS_PER_SECOND as f64).round() as i64);
    oc_db::upsert_workspace_media(&state.db, id, media_id, &body.filename, &ctype, ticks).await?;
    Ok(Json(serde_json::json!({ "media_id": media_id })))
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
        let play_url = media_play_url(id, row.id, &row.r2_key).await;
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

async fn media_play_url(project_id: Uuid, media_id: Uuid, key: &str) -> Option<String> {
    if oc_db::is_r2_object_key(key)
        || oc_db::is_local_media_key(key)
        || oc_db::local_media_path(key).is_some_and(|p| p.is_file())
    {
        return Some(media_file_url(project_id, media_id));
    }
    if key.starts_with("workspace/") {
        let guessed = oc_db::local_media_path(&oc_db::local_media_key(
            project_id,
            media_id,
            "media.bin",
        ));
        if guessed.is_some_and(|p| p.is_file()) {
            return Some(media_file_url(project_id, media_id));
        }
    }
    None
}

fn media_file_url(project_id: Uuid, media_id: Uuid) -> String {
    let base = std::env::var("API_PUBLIC_URL").unwrap_or_else(|_| "http://127.0.0.1:8787".into());
    format!(
        "{}/v1/projects/{project_id}/media/{media_id}/file",
        base.trim_end_matches('/')
    )
}

async fn store_media_bytes(
    state: &AppState,
    project_id: Uuid,
    media_id: Uuid,
    media: &oc_db::MediaRow,
    content_type: &str,
    bytes: Vec<u8>,
) -> Result<String, ApiError> {
    if let Some(r2) = &state.r2 {
        let key = if oc_db::is_r2_object_key(&media.r2_key) {
            media.r2_key.clone()
        } else {
            object_key(
                ObjectKind::Raw,
                ProjectId::from_uuid(project_id),
                MediaId::from_uuid(media_id),
                &media.filename,
            )
        };
        r2.put_bytes(&key, bytes, content_type)
            .await
            .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, e.to_string()))?;
        return Ok(key);
    }
    let key = oc_db::local_media_key(project_id, media_id, &media.filename);
    let path = oc_db::local_media_path(&key)
        .ok_or_else(|| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "bad local key"))?;
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }
    tokio::fs::write(&path, bytes)
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(key)
}

pub async fn get_media_file(
    State(state): State<AppState>,
    Path((id, media_id)): Path<(Uuid, Uuid)>,
    headers: axum::http::HeaderMap,
) -> ApiResult<Response> {
    let media = oc_db::get_media(&state.db, media_id).await?;
    if media.project_id != id {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "media not in project"));
    }
    if oc_db::is_local_media_key(&media.r2_key) {
        if let Some(path) = oc_db::local_media_path(&media.r2_key) {
            return serve_local_file(&path, &media.content_type, headers.get(header::RANGE)).await;
        }
    }
    if oc_db::is_r2_object_key(&media.r2_key) {
        if let Some(r2) = &state.r2 {
            let url = r2
                .presign_get(&media.r2_key, Duration::from_secs(6 * 3600))
                .await
                .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, e.to_string()))?;
            return Ok(Response::builder()
                .status(StatusCode::TEMPORARY_REDIRECT)
                .header(header::LOCATION, url)
                .body(Body::empty())
                .unwrap_or_else(|_| Response::new(Body::empty())));
        }
    }
    // workspace/ rows: still try the local file we may have written
    let fallback = oc_db::local_media_key(id, media_id, &media.filename);
    if let Some(path) = oc_db::local_media_path(&fallback) {
        if path.is_file() {
            return serve_local_file(&path, &media.content_type, headers.get(header::RANGE)).await;
        }
    }
    Err(ApiError::new(
        StatusCode::NOT_FOUND,
        "media file is not stored — re-import the clip",
    ))
}

async fn serve_local_file(
    path: &std::path::Path,
    content_type: &str,
    range: Option<&axum::http::HeaderValue>,
) -> ApiResult<Response> {
    let data = tokio::fs::read(path)
        .await
        .map_err(|_| ApiError::new(StatusCode::NOT_FOUND, "media file missing"))?;
    let len = data.len() as u64;
    let ctype = if content_type.is_empty() {
        "application/octet-stream"
    } else {
        content_type
    };
    if let Some(range) = range.and_then(|v| v.to_str().ok()).and_then(parse_byte_range) {
        let (start, end) = range;
        let start = start.min(len.saturating_sub(1));
        let end = end.unwrap_or(len.saturating_sub(1)).min(len.saturating_sub(1)).max(start);
        let slice = data[start as usize..=end as usize].to_vec();
        return Ok(Response::builder()
            .status(StatusCode::PARTIAL_CONTENT)
            .header(header::CONTENT_TYPE, ctype)
            .header(header::ACCEPT_RANGES, "bytes")
            .header(
                header::CONTENT_RANGE,
                format!("bytes {start}-{end}/{len}"),
            )
            .header(header::CONTENT_LENGTH, slice.len())
            .body(Body::from(slice))
            .unwrap_or_else(|_| Response::new(Body::empty())));
    }
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, ctype)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_LENGTH, len)
        .body(Body::from(data))
        .unwrap_or_else(|_| Response::new(Body::empty())))
}

fn parse_byte_range(raw: &str) -> Option<(u64, Option<u64>)> {
    let spec = raw.strip_prefix("bytes=")?;
    let spec = spec.split(',').next()?.trim();
    let (a, b) = spec.split_once('-')?;
    let start = if a.is_empty() { 0 } else { a.parse().ok()? };
    let end = if b.is_empty() { None } else { Some(b.parse().ok()?) };
    Some((start, end))
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
