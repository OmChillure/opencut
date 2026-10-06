use crate::edit::{self, hydrate_op, look_by_media, speech_by_media};
use crate::state::AppState;
use axum::Json;
use axum::body::Body;
use axum::extract::{FromRequestParts, Path, Query, State};
use axum::http::request::Parts;
use axum::http::{StatusCode, header};
use axum::response::Response;
use futures_util::StreamExt;
use oc_core::time::TICKS_PER_SECOND;
use oc_core::{
    MediaId, Op, Project, ProjectId, Timeline, UndoStack, apply, is_director_request, mcp_tools,
    review_cut,
};
use oc_media::{ObjectKind, object_key};
use oc_providers::{ChatEvent, ChatTurn, LlmReply};
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
            oc_db::DbError::BadUser => Self::new(StatusCode::BAD_REQUEST, "sign in with an email"),
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

/// The signed-in email. Fetch sends `x-opencut-user`. A media or export URL may use `?user=`.
pub struct SignedIn(pub String);

impl<S> FromRequestParts<S> for SignedIn
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get("x-opencut-user")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        let from_query = query_param(parts.uri.query().unwrap_or(""), "user");
        let raw = if header.is_empty() {
            from_query.as_str()
        } else {
            header
        };
        let email = oc_db::normalize_user_email(raw)
            .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "sign in with an email"))?;
        Ok(SignedIn(email))
    }
}

fn query_param(query: &str, key: &str) -> String {
    for pair in query.split('&') {
        let Some((name, value)) = pair.split_once('=') else {
            continue;
        };
        if name == key {
            return percent_decode(value);
        }
    }
    String::new()
}

fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        out.push(if bytes[index] == b'+' {
            b' '
        } else {
            bytes[index]
        });
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

async fn owned(state: &AppState, id: Uuid, email: &str) -> Result<(), ApiError> {
    oc_db::require_project_owner(&state.db, id, email).await?;
    Ok(())
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
    SignedIn(email): SignedIn,
    Json(body): Json<CreateProjectBody>,
) -> ApiResult<Json<Project>> {
    let name = body.name.unwrap_or_else(|| "Untitled".into());
    Ok(Json(oc_db::create_project(&state.db, &name, &email).await?))
}

pub async fn list_projects(
    State(state): State<AppState>,
    SignedIn(email): SignedIn,
) -> ApiResult<Json<Vec<oc_db::ProjectRow>>> {
    Ok(Json(oc_db::list_projects(&state.db, &email).await?))
}

pub async fn get_project(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    SignedIn(email): SignedIn,
) -> ApiResult<Json<Project>> {
    owned(&state, id, &email).await?;
    Ok(Json(oc_db::get_project(&state.db, id).await?))
}

#[derive(Deserialize)]
pub struct UpdateProjectBody {
    pub name: String,
}

pub async fn update_project(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    SignedIn(email): SignedIn,
    Json(body): Json<UpdateProjectBody>,
) -> ApiResult<Json<Project>> {
    owned(&state, id, &email).await?;
    let name = body.name.trim();
    if name.is_empty() {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "name required"));
    }
    Ok(Json(oc_db::rename_project(&state.db, id, name).await?))
}

#[derive(Serialize)]
pub struct DeleteProjectResponse {
    pub ok: bool,
    pub r2_deleted: u32,
}

pub async fn delete_project(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    SignedIn(email): SignedIn,
) -> ApiResult<Json<DeleteProjectResponse>> {
    owned(&state, id, &email).await?;
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
    SignedIn(email): SignedIn,
    Json(body): Json<ApplyOpsBody>,
) -> ApiResult<Json<ApplyOpsResponse>> {
    owned(&state, id, &email).await?;
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

pub async fn chat(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    SignedIn(email): SignedIn,
    Json(body): Json<ChatBody>,
) -> Result<Response, ApiError> {
    owned(&state, id, &email).await?;
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
        let pending = jobs.len() - futures(jobs, state).await;
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
        if tx.is_closed() {
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
        tokio::select! {
            _ = tx.closed() => return,
            _ = tokio::time::sleep(Duration::from_secs(1)) => {}
        }
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
    let spoken = edit::spoken(&speech);
    let director = oc_core::is_director_request(request);
    if !director {
        return review_cut(&timeline, &spoken, request);
    }
    let media = oc_db::list_media(&state.db, id).await.unwrap_or_default();
    let looks = look_by_media(
        &oc_db::list_analysis_for_project(&state.db, id)
            .await
            .unwrap_or_default(),
    );
    let has_music = timeline
        .edit_plan
        .as_ref()
        .and_then(|p| p.music_id)
        .is_some();
    let facts = edit::review_facts(&timeline, &media, &looks, Vec::new(), has_music);
    oc_core::review_with(&timeline, &spoken, request, &facts)
}

fn tool_finished(value: &serde_json::Value) -> bool {
    value.get("type").and_then(|v| v.as_str()) == Some("tool")
        && value.get("status").and_then(|v| v.as_str()) == Some("done")
}

fn attach_timeline(value: &mut serde_json::Value, timeline: &Timeline) {
    if let Ok(timeline) = serde_json::to_value(timeline) {
        value["timeline"] = timeline;
    }
}

async fn push(tx: &mpsc::Sender<String>, value: serde_json::Value) {
    let _ = tx.send(line(&value)).await;
}

async fn forward_events(
    mut rx: mpsc::Receiver<ChatEvent>,
    tx: mpsc::Sender<String>,
    db: oc_db::Db,
    project_id: Uuid,
) {
    while let Some(ev) = rx.recv().await {
        let Ok(mut value) = serde_json::to_value(&ev) else {
            continue;
        };
        // MCP writes the timeline before ACP reports the tool done. Attach it
        // so the page paints the cut while the reply is still running.
        if tool_finished(&value) {
            if let Ok(project) = oc_db::get_project(&db, project_id).await {
                attach_timeline(&mut value, &project.timeline);
            }
        }
        let _ = tx.send(line(&value)).await;
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
        let understood = media
            .iter()
            .any(|m| speech.contains_key(&m.id) || looks.contains_key(&m.id));
        let needs_scan = media.iter().any(|m| {
            if m.content_type.starts_with("image/") {
                return false;
            }
            let stale = looks.get(&m.id).is_some_and(|l| {
                (l.has_video && crate::edit::shot_looks(l).is_empty())
                    || crate::edit::look_needs_vision(l)
            });
            stale || (!speech.contains_key(&m.id) && !looks.contains_key(&m.id))
        });
        if needs_scan {
            push(
                &tx,
                serde_json::json!({
                    "type": "status",
                    "text": "Watching clips before the cut (shot list, then words)"
                }),
            )
            .await;
            let mut jobs = Vec::new();
            for row in &media {
                if row.content_type.starts_with("image/") {
                    continue;
                }
                let stale_look = looks.get(&row.id).is_some_and(|l| {
                    (l.has_video && crate::edit::shot_looks(l).is_empty())
                        || crate::edit::look_needs_vision(l)
                });
                if (speech.contains_key(&row.id) || looks.contains_key(&row.id)) && !stale_look {
                    continue;
                }
                let on_disk = oc_db::local_media_path(&row.r2_key).is_some_and(|p| p.is_file())
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
    let mut system = format!("Project '{}'.", project.name);
    if let Some(guide) = oc_providers::style_guide(&last_user) {
        system.push_str("\n\n");
        system.push_str(&guide);
    }
    let mut turns = body.messages;
    let mut text = String::new();
    let (ev_tx, ev_rx) = mpsc::channel::<ChatEvent>(64);
    let pump = tokio::spawn(forward_events(ev_rx, tx.clone(), state.db.clone(), id));
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
    let mut session = oc_providers::DirectorSession::open(
        &body.provider,
        &body.model,
        &mcp_servers,
        Some(ev_tx.clone()),
    )
    .await
    .map_err(|e| e.to_string())?;
    let mut message =
        oc_providers::opening_prompt(&system, &turns, &tools, !mcp_servers.is_empty());
    let mut note_rounds = 0_u32;
    let mut seen: Vec<oc_providers::PromptImage> = Vec::new();
    for turn_i in 0..8 {
        if tx.is_closed() {
            drop(ev_tx);
            let _ = pump.await;
            return Ok(());
        }
        let reply = tokio::select! {
            _ = tx.closed() => {
                drop(ev_tx);
                let _ = pump.await;
                return Ok(());
            }
            reply = async {
                let show = seen.as_slice();
                if session.remembers() {
                    session.turn(&message, show, Some(&ev_tx)).await
                } else {
                    oc_providers::complete_stream(
                        &body.provider,
                        &body.model,
                        &system,
                        &turns,
                        &tools,
                        Some(ev_tx.clone()),
                        &mcp_servers,
                        show,
                    )
                    .await
                }
            } => reply,
        };
        seen.clear();
        let reply = match reply {
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
                let director = oc_core::is_director_request(&last_user);
                let note_blocks = review.notes && director && note_rounds < 2;
                if (review.issues || note_blocks) && turn_i + 1 < 8 {
                    if note_blocks {
                        note_rounds += 1;
                    }
                    tracing::info!(project = %id, "cut review rejected a finished reply");
                    let follow = format!(
                        "{}\nThose fix: lines are still open. note: lines matter for two rounds. \
                         Correct them with tools. Do not describe the cut as done.",
                        review.text
                    );
                    turns.push(ChatTurn {
                        role: "assistant".into(),
                        content: t,
                    });
                    turns.push(ChatTurn {
                        role: "user".into(),
                        content: follow.clone(),
                    });
                    message = oc_providers::followup_prompt(&follow);
                    continue;
                }
                text = t;
                if director && !review.issues {
                    if let Ok(fresh) = oc_db::get_project(&state.db, id).await {
                        let preset = export_preset(&fresh.timeline);
                        match edit::queue_export(&state.db, id, preset).await {
                            Ok(()) => notes.push(format!("export queued {}", preset.label())),
                            Err(err) => {
                                tracing::error!(project = %id, "export queue failed: {err}")
                            }
                        }
                    }
                }
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
                        None,
                    )
                    .await;
                    tracing::info!(tool = %call.name, "host tool");
                    let (ok, result) = if call.name == "see" {
                        see_host(&state, id, &call.arguments, &mut seen).await
                    } else {
                        match edit::call_tool(&state.db, id, &call.name, call.arguments.clone())
                            .await
                        {
                            Ok(out) => {
                                if !out.starts_with("bin:")
                                    && !out.starts_with("Current timeline")
                                    && !out.starts_with("media ")
                                {
                                    notes.push(out.clone());
                                }
                                batch.push_str(&compact_tool(&call.name, &out));
                                batch.push('\n');
                                (true, out)
                            }
                            Err(err) => {
                                let msg = format!("tool error: {err}");
                                batch.push_str(&msg);
                                batch.push('\n');
                                (false, msg)
                            }
                        }
                    };
                    if call.name == "see" {
                        batch.push_str(&compact_tool("see", &result));
                        batch.push('\n');
                        if ok {
                            notes.push(result.clone());
                        }
                    }
                    let timeline = oc_db::get_project(&state.db, id)
                        .await
                        .ok()
                        .map(|p| p.timeline);
                    emit_host_tool(
                        &tx,
                        &tool_id,
                        &call.name,
                        call.arguments,
                        Some(result),
                        if ok { "done" } else { "error" },
                        timeline.as_ref(),
                    )
                    .await;
                }
                let logged = format!("tools\n{batch}");
                turns.push(ChatTurn {
                    role: "assistant".into(),
                    content: logged.clone(),
                });
                let review = fresh_review(&state, id, &last_user).await;
                let follow = format!(
                    "{logged}\n{review}\n\
                     If a line starts with \"fix:\", correct it with tools. \
                     note: lines matter for two rounds in a full edit. \
                     If the cut matches the request, reply in 2–4 sentences.",
                    review = review.text
                );
                turns.push(ChatTurn {
                    role: "user".into(),
                    content: follow.clone(),
                });
                message = oc_providers::followup_prompt(&follow);
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

async fn see_host(
    state: &AppState,
    project_id: Uuid,
    arguments: &serde_json::Value,
    seen: &mut Vec<oc_providers::PromptImage>,
) -> (bool, String) {
    let media = arguments
        .get("media_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let at = arguments.get("at").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let Ok(media_id) = Uuid::parse_str(media) else {
        return (false, format!("tool error: bad media id {media}"));
    };
    match edit::see_frame(&state.db, state.r2.as_ref(), project_id, media_id, at).await {
        Ok(frame) => {
            let caption = frame.caption.clone();
            seen.push(frame);
            (true, caption)
        }
        Err(err) => (false, format!("tool error: {err}")),
    }
}

fn export_preset(timeline: &Timeline) -> oc_core::ExportPreset {
    if timeline.height > timeline.width + 32 {
        oc_core::ExportPreset::Vertical1080
    } else if (timeline.width as i32 - timeline.height as i32).unsigned_abs() < 32 {
        oc_core::ExportPreset::Square1080
    } else {
        oc_core::ExportPreset::Youtube1080
    }
}

fn compact_tool(name: &str, result: &str) -> String {
    let short: String = result
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("ok")
        .chars()
        .take(140)
        .collect();
    format!("{name}: {short}")
}

async fn emit_host_tool(
    tx: &mpsc::Sender<String>,
    id: &str,
    name: &str,
    args: serde_json::Value,
    result: Option<String>,
    status: &str,
    timeline: Option<&oc_core::Timeline>,
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
            "timeline": timeline,
        }),
    )
    .await;
}

async fn finish_chat(tx: &mpsc::Sender<String>, text: &str, notes: &[String], timeline: &Timeline) {
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
    SignedIn(email): SignedIn,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> ApiResult<Json<serde_json::Value>> {
    owned(&state, id, &email).await?;
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
    SignedIn(email): SignedIn,
    Json(body): Json<RegisterMediaBody>,
) -> ApiResult<Json<serde_json::Value>> {
    owned(&state, id, &email).await?;
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
    SignedIn(email): SignedIn,
    Json(body): Json<UploadBody>,
) -> ApiResult<Json<UploadResponse>> {
    owned(&state, id, &email).await?;
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
    SignedIn(email): SignedIn,
) -> ApiResult<Json<Vec<MediaOut>>> {
    owned(&state, id, &email).await?;
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
        let guessed =
            oc_db::local_media_path(&oc_db::local_media_key(project_id, media_id, "media.bin"));
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
    SignedIn(email): SignedIn,
    headers: axum::http::HeaderMap,
) -> ApiResult<Response> {
    owned(&state, id, &email).await?;
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

fn latest_export(id: Uuid) -> Result<std::path::PathBuf, ApiError> {
    let dir = std::env::var("OPENCUT_EXPORT_DIR").unwrap_or_else(|_| "data/exports".into());
    let prefix = id.to_string();
    let entries = std::fs::read_dir(&dir)
        .map_err(|_| ApiError::new(StatusCode::NOT_FOUND, "no export yet"))?;
    let mut best: Option<(std::time::SystemTime, std::path::PathBuf)> = None;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with(&prefix) || !name.ends_with(".mp4") || name.starts_with('.') {
            continue;
        }
        if !mp4_has_moov(&entry.path()) {
            continue;
        }
        let modified = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .unwrap_or(std::time::UNIX_EPOCH);
        if best.as_ref().is_none_or(|(t, _)| modified > *t) {
            best = Some((modified, entry.path()));
        }
    }
    best.map(|(_, path)| path)
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "no export yet"))
}

pub async fn head_export(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    SignedIn(email): SignedIn,
) -> ApiResult<Response> {
    owned(&state, id, &email).await?;
    let path = latest_export(id)?;
    let len = std::fs::metadata(&path)
        .map_err(|_| ApiError::new(StatusCode::NOT_FOUND, "no export yet"))?
        .len();
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "video/mp4")
        .header(header::CONTENT_LENGTH, len)
        .header(header::CACHE_CONTROL, "no-store")
        .header(header::ACCEPT_RANGES, "bytes")
        .body(Body::empty())
        .unwrap_or_else(|_| Response::new(Body::empty())))
}

pub async fn get_export(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    SignedIn(email): SignedIn,
    headers: axum::http::HeaderMap,
) -> ApiResult<Response> {
    owned(&state, id, &email).await?;
    let path = latest_export(id)?;
    serve_local_file(&path, "video/mp4", headers.get(header::RANGE))
        .await
        .map(with_no_store)
}

fn with_no_store(mut response: Response) -> Response {
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}

/// A file ffmpeg is still writing has `ftyp` and an `mdat` that runs to EOF.
/// `moov` is only there after the encode finishes. Serving before that plays
/// the first GOP and then goes blank, and a download has no playable stream.
fn mp4_has_moov(path: &std::path::Path) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return false,
    };
    let len = match file.metadata() {
        Ok(meta) => meta.len(),
        Err(_) => return false,
    };
    let mut pos = 0u64;
    while pos + 8 <= len {
        if file.seek(SeekFrom::Start(pos)).is_err() {
            return false;
        }
        let mut header = [0u8; 8];
        if file.read_exact(&mut header).is_err() {
            return false;
        }
        let size32 = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
        if &header[4..8] == b"moov" {
            return true;
        }
        let box_size = if size32 == 1 {
            let mut large = [0u8; 8];
            if file.read_exact(&mut large).is_err() {
                return false;
            }
            u64::from_be_bytes(large)
        } else if size32 == 0 {
            return false;
        } else {
            u64::from(size32)
        };
        if box_size < 8 || pos.saturating_add(box_size) > len {
            return false;
        }
        pos += box_size;
    }
    false
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
    if let Some(range) = range
        .and_then(|v| v.to_str().ok())
        .and_then(parse_byte_range)
    {
        let (start, end) = range;
        let start = start.min(len.saturating_sub(1));
        let end = end
            .unwrap_or(len.saturating_sub(1))
            .min(len.saturating_sub(1))
            .max(start);
        let slice = data[start as usize..=end as usize].to_vec();
        return Ok(Response::builder()
            .status(StatusCode::PARTIAL_CONTENT)
            .header(header::CONTENT_TYPE, ctype)
            .header(header::ACCEPT_RANGES, "bytes")
            .header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{len}"))
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
    let end = if b.is_empty() {
        None
    } else {
        Some(b.parse().ok()?)
    };
    Some((start, end))
}

#[derive(Deserialize)]
pub struct PatchMediaBody {
    pub duration_seconds: Option<f64>,
}

pub async fn patch_media(
    State(state): State<AppState>,
    Path((id, media_id)): Path<(Uuid, Uuid)>,
    SignedIn(email): SignedIn,
    Json(body): Json<PatchMediaBody>,
) -> ApiResult<Json<serde_json::Value>> {
    owned(&state, id, &email).await?;
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
    SignedIn(email): SignedIn,
) -> ApiResult<Json<serde_json::Value>> {
    owned(&state, id, &email).await?;
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

pub async fn generate_captions(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    SignedIn(email): SignedIn,
) -> ApiResult<Json<serde_json::Value>> {
    owned(&state, id, &email).await?;
    let (timeline, note) = edit::place_captions(&state.db, id)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, e))?;
    Ok(Json(serde_json::json!({
        "note": note,
        "timeline": timeline,
    })))
}

pub async fn transcribe_media(
    State(state): State<AppState>,
    Path((id, media_id)): Path<(Uuid, Uuid)>,
    SignedIn(email): SignedIn,
) -> ApiResult<Json<serde_json::Value>> {
    owned(&state, id, &email).await?;
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

#[derive(Deserialize)]
pub(crate) struct UserQuery {
    user: String,
}

#[derive(Deserialize)]
pub(crate) struct CreateChatBody {
    user: String,
    title: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct SaveChatBody {
    user: String,
    #[serde(default)]
    messages: Vec<oc_db::ChatMessageInput>,
}

#[derive(Serialize)]
pub(crate) struct ChatDetail {
    #[serde(flatten)]
    chat: oc_db::ChatRow,
    messages: Vec<oc_db::ChatMessageInput>,
}

pub async fn list_chats(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    SignedIn(email): SignedIn,
    Query(query): Query<UserQuery>,
) -> ApiResult<Json<Vec<oc_db::ChatRow>>> {
    owned(&state, id, &email).await?;
    Ok(Json(oc_db::list_chats(&state.db, id, &query.user).await?))
}

pub async fn create_chat(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    SignedIn(email): SignedIn,
    Json(body): Json<CreateChatBody>,
) -> ApiResult<Json<oc_db::ChatRow>> {
    owned(&state, id, &email).await?;
    let chat = oc_db::create_chat(&state.db, id, &body.user, body.title.as_deref()).await?;
    tracing::info!(project = %id, chat = %chat.id, "chat created");
    Ok(Json(chat))
}

pub async fn get_chat(
    State(state): State<AppState>,
    Path((id, chat_id)): Path<(Uuid, Uuid)>,
    SignedIn(email): SignedIn,
    Query(query): Query<UserQuery>,
) -> ApiResult<Json<ChatDetail>> {
    owned(&state, id, &email).await?;
    let (chat, messages) = oc_db::get_chat(&state.db, id, chat_id, &query.user).await?;
    Ok(Json(ChatDetail { chat, messages }))
}

pub async fn save_chat_messages(
    State(state): State<AppState>,
    Path((id, chat_id)): Path<(Uuid, Uuid)>,
    SignedIn(email): SignedIn,
    Json(body): Json<SaveChatBody>,
) -> ApiResult<Json<oc_db::ChatRow>> {
    owned(&state, id, &email).await?;
    let chat =
        oc_db::save_chat_messages(&state.db, id, chat_id, &body.user, &body.messages).await?;
    tracing::info!(project = %id, chat = %chat_id, n = body.messages.len(), "chat saved");
    Ok(Json(chat))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_finished_tool_carries_the_timeline_before_the_reply_ends() {
        let mut event = serde_json::json!({
            "type": "tool",
            "name": "submit_edit",
            "status": "done",
            "result": "placed 3 clips"
        });
        assert!(tool_finished(&event));
        let mut timeline = Timeline::default();
        timeline.width = 1920;
        timeline.height = 1080;
        attach_timeline(&mut event, &timeline);
        assert_eq!(event["timeline"]["width"], 1920);
        assert!(event["timeline"]["tracks"].is_array());

        let pending = serde_json::json!({
            "type": "tool",
            "name": "list_bin",
            "status": "pending"
        });
        assert!(!tool_finished(&pending));
    }

    fn box_bytes(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let size = (8 + payload.len()) as u32;
        let mut out = size.to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(payload);
        out
    }

    #[test]
    fn an_unfinished_mp4_is_not_served() {
        let dir = std::env::temp_dir().join("oc-moov-check");
        std::fs::create_dir_all(&dir).unwrap();
        let done = dir.join("done.mp4");
        let writing = dir.join("writing.mp4");
        let decoy = dir.join("decoy.mp4");
        let mut finished = box_bytes(b"ftyp", b"isom");
        finished.extend(box_bytes(b"moov", b"mvhd"));
        finished.extend(box_bytes(b"mdat", b"frames"));
        std::fs::write(&done, &finished).unwrap();
        // size 0 means the box runs to EOF, which is how ffmpeg leaves mdat.
        let mut partial = box_bytes(b"ftyp", b"isom");
        partial.extend_from_slice(&0u32.to_be_bytes());
        partial.extend_from_slice(b"mdat");
        partial.extend_from_slice(b"moov-is-not-a-box-here");
        std::fs::write(&writing, &partial).unwrap();
        let mut bait = box_bytes(b"ftyp", b"isom");
        bait.extend(box_bytes(b"free", b"moov"));
        std::fs::write(&decoy, &bait).unwrap();
        assert!(mp4_has_moov(&done));
        assert!(!mp4_has_moov(&writing));
        assert!(!mp4_has_moov(&decoy));
    }
}
