use crate::edit::{self, hydrate_op, look_by_media, speech_by_media};
use crate::state::AppState;
use axum::Json;
use axum::body::Body;
use axum::extract::{FromRequestParts, Path, Query, State};
use axum::http::request::Parts;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use futures_util::StreamExt;
use oc_core::time::TICKS_PER_SECOND;
use oc_core::{
    MediaId, Op, Project, ProjectId, Timeline, UndoStack, apply, call_changes_timeline,
    follow_after_text, follow_after_tools, is_director_request, review_cut, tools_for_request,
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
            oc_db::DbError::WrongPassword => Self::new(StatusCode::UNAUTHORIZED, "wrong password"),
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

/// The signed-in email. Fetch sends `x-opencut-token`. A media or export URL may use `?token=`.
pub struct SignedIn(pub String);

/// Header wins. `?user=` is not a credential.
pub(crate) fn presented_token(header: &str, query: &str) -> String {
    let header = header.trim();
    if header.is_empty() {
        query_param(query, "token")
    } else {
        header.to_string()
    }
}

impl FromRequestParts<AppState> for SignedIn {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get("x-opencut-token")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        let token = presented_token(header, parts.uri.query().unwrap_or(""));
        if token.is_empty() {
            return Err(ApiError::new(StatusCode::UNAUTHORIZED, "sign in"));
        }
        match oc_db::email_for_session(&state.db, &token).await {
            Ok(email) => Ok(SignedIn(email)),
            Err(oc_db::DbError::NotFound) => {
                Err(ApiError::new(StatusCode::UNAUTHORIZED, "sign in"))
            }
            Err(other) => Err(other.into()),
        }
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

const HOST_TURNS: u32 = 32;
const PROVIDER_RESUMES: u32 = 2;

const RESUME_STATUS: &str = "The edit stopped. Continuing from the timeline.";
const RESUME_FOLLOW: &str = "The previous session stopped before it finished. Continue the same piece from the timeline above. Do not start over. Do not repeat a step that is already on the timeline. Finish the open work, then reply in 2–4 sentences.";
const UNFINISHED_EDIT: &str = "The edit is on the timeline and was not finished, so nothing was exported. Send continue to keep going.";
const OPEN_FIXES_HOLD: &str =
    "Open fixes are still on the cut, so nothing was exported. Send continue to keep going.";

fn provider_stopped(err: &str) -> bool {
    let lower = err.to_ascii_lowercase();
    if lower.contains("cannot edit without")
        || lower.contains("auth login")
        || lower.contains("unknown provider")
        || lower.contains("adapter missing")
    {
        return false;
    }
    lower.contains("acp cli closed")
        || lower.contains("acp prompt:")
        || lower.contains("broken pipe")
        || lower.contains("connection reset")
        || lower.contains("unexpected eof")
        || lower.contains("unexpected end of file")
        || lower.contains("early eof")
}

fn next_unchanged_repeats(
    changed: bool,
    batch: &str,
    last_batch: Option<&str>,
    repeats: u32,
) -> u32 {
    if changed || last_batch.is_none_or(|prev| prev != batch) {
        0
    } else {
        repeats.saturating_add(1)
    }
}

fn hold_open_fixes(reply: &str) -> String {
    format!("{}\n\n{}", reply.trim_end(), OPEN_FIXES_HOLD)
}

fn unfinished_with_notes(notes: &[String]) -> String {
    let mut text = UNFINISHED_EDIT.to_string();
    if !notes.is_empty() {
        text.push_str("\n\n");
        text.push_str(&notes.join(" · "));
    }
    text
}

async fn close_chat_events(ev_tx: mpsc::Sender<ChatEvent>, pump: tokio::task::JoinHandle<()>) {
    drop(ev_tx);
    let _ = pump.await;
}

async fn reopen_director(
    state: &AppState,
    id: Uuid,
    provider: &str,
    model: &str,
    mcp_servers: &[serde_json::Value],
    events: &mpsc::Sender<ChatEvent>,
    project: &mut Project,
    media: &mut Vec<oc_db::MediaRow>,
    speech: &std::collections::HashMap<Uuid, crate::edit::Speech>,
    looks: &std::collections::HashMap<Uuid, oc_db::AnalysisRow>,
    turns: &mut Vec<ChatTurn>,
    system: &mut String,
) -> Result<oc_providers::DirectorSession, String> {
    *project = oc_db::get_project(&state.db, id)
        .await
        .map_err(|err| err.to_string())?;
    match oc_db::list_media(&state.db, id).await {
        Ok(rows) => *media = rows,
        Err(err) => tracing::warn!(project = %id, "resume kept the earlier bin: {err}"),
    }
    *system = format!("Project '{}'.\n\n", project.name);
    system.push_str(&edit::opening_inventory(
        &project.timeline,
        media,
        speech,
        looks,
    ));
    turns.push(ChatTurn {
        role: "user".into(),
        content: RESUME_FOLLOW.into(),
    });
    oc_providers::DirectorSession::open(provider, model, mcp_servers, Some(events.clone()))
        .await
        .map_err(|err| err.to_string())
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
    let mut media = oc_db::list_media(&state.db, id)
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
            if m.content_type.starts_with("image/") || !oc_db::is_r2_object_key(&m.r2_key) {
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
                if !oc_db::is_r2_object_key(&row.r2_key) {
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
                        "text": "This clip stays in the browser that imported it. The shot list runs for a file stored on the server."
                    }),
                )
                .await;
            }
        }
    }
    // Understand may have finished during the wait. Reload so the opening
    // inventory is not the snapshot from before the shot list existed.
    let speech = match oc_db::list_transcripts_for_project(&state.db, id).await {
        Ok(rows) => speech_by_media(&rows),
        Err(err) => {
            tracing::warn!(project = %id, "opening inventory kept the earlier speech: {err}");
            speech
        }
    };
    let looks = match oc_db::list_analysis_for_project(&state.db, id).await {
        Ok(rows) => look_by_media(&rows),
        Err(err) => {
            tracing::warn!(project = %id, "opening inventory kept the earlier shot list: {err}");
            looks
        }
    };
    let tools = tools_for_request(&last_user);
    let piece = is_director_request(&last_user);
    let mut system = format!("Project '{}'.\n\n", project.name);
    system.push_str(&edit::opening_inventory(
        &project.timeline,
        &media,
        &speech,
        &looks,
    ));
    let mut turns = body.messages;
    let mut text = String::new();
    let mut queued_export: Option<(String, bool)> = None;
    let (ev_tx, ev_rx) = mpsc::channel::<ChatEvent>(64);
    let pump = tokio::spawn(forward_events(ev_rx, tx.clone(), state.db.clone(), id));
    let tool_allow = if piece {
        Some(
            tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>()
                .join(","),
        )
    } else {
        None
    };
    let mcp_servers = crate::mcp::builtin_mcp_acp(&id.to_string(), tool_allow.as_deref())
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
    let mut seen: Vec<oc_providers::PromptImage> = Vec::new();
    let mut turn_i: u32 = 0;
    let mut resumes = 0u32;
    let mut last_batch: Option<String> = None;
    let mut unchanged_repeats = 0u32;
    let mut answered = false;
    while turn_i < HOST_TURNS {
        if tx.is_closed() {
            close_chat_events(ev_tx, pump).await;
            return Ok(());
        }
        let reply = tokio::select! {
            _ = tx.closed() => {
                close_chat_events(ev_tx, pump).await;
                return Ok(());
            }
            reply = session.turn(&message, &seen, Some(&ev_tx)) => reply,
        };
        seen.clear();
        let reply = match reply {
            Ok(reply) => reply,
            Err(err) => {
                let err = err.to_string();
                tracing::error!(project = %id, "provider failed: {err}");
                if tx.is_closed() {
                    close_chat_events(ev_tx, pump).await;
                    return Ok(());
                }
                if !provider_stopped(&err) || resumes >= PROVIDER_RESUMES {
                    close_chat_events(ev_tx, pump).await;
                    return Err(err);
                }
                resumes += 1;
                push(
                    &tx,
                    serde_json::json!({
                        "type": "status",
                        "text": RESUME_STATUS
                    }),
                )
                .await;
                session = match reopen_director(
                    &state,
                    id,
                    &body.provider,
                    &body.model,
                    &mcp_servers,
                    &ev_tx,
                    &mut project,
                    &mut media,
                    &speech,
                    &looks,
                    &mut turns,
                    &mut system,
                )
                .await
                {
                    Ok(session) => session,
                    Err(err) => {
                        close_chat_events(ev_tx, pump).await;
                        return Err(err);
                    }
                };
                message =
                    oc_providers::opening_prompt(&system, &turns, &tools, !mcp_servers.is_empty());
                tracing::info!(project = %id, resumes, "provider resumed from the timeline");
                continue;
            }
        };
        turn_i += 1;
        match reply {
            LlmReply::Text(t) => {
                let review = fresh_review(&state, id, &last_user).await;
                if review.issues {
                    if turn_i < HOST_TURNS {
                        tracing::info!(project = %id, "cut review rejected a finished reply");
                        let follow = follow_after_text(&review.text, piece);
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
                    tracing::info!(project = %id, "open fixes remain, export held");
                    answered = true;
                    text = hold_open_fixes(&t);
                    break;
                }
                answered = true;
                text = t;
                if piece {
                    if let Ok(fresh) = oc_db::get_project(&state.db, id).await {
                        let preset = export_preset(&fresh.timeline);
                        let rows = oc_db::list_media(&state.db, id).await.unwrap_or_default();
                        if sources_need_browser(&fresh.timeline, &rows) {
                            queued_export = Some((preset.label().to_string(), true));
                            notes.push(
                                "This browser sends the clips, then the finished video is stored on Cloudflare."
                                    .into(),
                            );
                        } else {
                            match edit::queue_export(&state.db, id, preset).await {
                                Ok(()) => {
                                    queued_export = Some((preset.label().to_string(), false));
                                    notes.push(format!(
                                        "export queued {}. The finished video is stored on Cloudflare.",
                                        preset.label()
                                    ));
                                }
                                Err(err) => {
                                    tracing::error!(project = %id, "export queue failed: {err}")
                                }
                            }
                        }
                    }
                }
                break;
            }
            LlmReply::Tools(calls) => {
                let mut batch = String::new();
                let mut changed = false;
                for (i, call) in calls.into_iter().enumerate() {
                    if call_changes_timeline(&call.name) {
                        changed = true;
                    }
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
                    } else if call.name == "watch" {
                        watch_host(&state, id, &call.arguments, &mut seen).await
                    } else {
                        match edit::call_tool(&state.db, id, &call.name, call.arguments.clone())
                            .await
                        {
                            Ok(out) => {
                                if call.name != "edit_skill"
                                    && !out.starts_with("bin:")
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
                    if call.name == "see" || call.name == "watch" {
                        batch.push_str(&compact_tool(call.name.as_str(), &result));
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
                unchanged_repeats = next_unchanged_repeats(
                    changed,
                    &batch,
                    last_batch.as_deref(),
                    unchanged_repeats,
                );
                last_batch = Some(batch.clone());
                if unchanged_repeats >= 2 {
                    tracing::info!(project = %id, "same tool batch left the timeline unchanged");
                    break;
                }
                let logged = format!("tools\n{batch}");
                turns.push(ChatTurn {
                    role: "assistant".into(),
                    content: logged.clone(),
                });
                let follow = if changed {
                    let review = fresh_review(&state, id, &last_user).await;
                    follow_after_tools(&logged, &review.text, true, piece)
                } else {
                    follow_after_tools(&logged, "", false, piece)
                };
                turns.push(ChatTurn {
                    role: "user".into(),
                    content: follow.clone(),
                });
                message = oc_providers::followup_prompt(&follow);
            }
        }
    }
    close_chat_events(ev_tx, pump).await;
    // MCP + call_tool already persist. Reload so we don't clobber those edits.
    if let Ok(fresh) = oc_db::get_project(&state.db, id).await {
        project = fresh;
    }
    if !answered {
        tracing::info!(project = %id, turns = turn_i, "host stopped before the piece finished");
        text = unfinished_with_notes(&notes);
    }
    tracing::info!(
        project = %id,
        notes = notes.len(),
        chars = text.len(),
        "chat done"
    );
    let export_preset = queued_export.as_ref().map(|(name, _)| name.as_str());
    let export_handoff = queued_export.as_ref().is_some_and(|(_, handoff)| *handoff);
    finish_chat(
        &tx,
        &text,
        &notes,
        &project.timeline,
        export_preset,
        export_handoff,
    )
    .await;
    Ok(())
}

async fn watch_host(
    state: &AppState,
    project_id: Uuid,
    arguments: &serde_json::Value,
    seen: &mut Vec<oc_providers::PromptImage>,
) -> (bool, String) {
    let media = arguments
        .get("media_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let start = arguments
        .get("start")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let end = arguments
        .get("end")
        .and_then(|v| v.as_f64())
        .unwrap_or(start);
    let Ok(media_id) = Uuid::parse_str(media) else {
        return (false, format!("tool error: bad media id {media}"));
    };
    match edit::watch_range(
        &state.db,
        state.r2.as_ref(),
        project_id,
        media_id,
        start,
        end,
    )
    .await
    {
        Ok(watched) => {
            let text = watched.text.clone();
            for frame in watched.frames {
                if seen.len() >= 8 {
                    break;
                }
                seen.push(frame);
            }
            (true, text)
        }
        Err(err) => (false, format!("tool error: {err}")),
    }
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
            let mut caption = frame.caption.clone();
            if seen.len() >= 8 {
                caption.push_str(" (frame held; this turn already has 8 frames)");
            } else {
                seen.push(frame);
            }
            (true, caption)
        }
        Err(err) => (false, format!("tool error: {err}")),
    }
}

fn export_preset(timeline: &Timeline) -> oc_core::ExportPreset {
    oc_core::ExportPreset::for_frame(timeline.width, timeline.height)
}

fn compact_tool(name: &str, result: &str) -> String {
    if name == "edit_skill" {
        return format!("{name}: {result}");
    }
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

async fn finish_chat(
    tx: &mpsc::Sender<String>,
    text: &str,
    notes: &[String],
    timeline: &Timeline,
    export_preset: Option<&str>,
    export_handoff: bool,
) {
    let mut body = serde_json::json!({
        "type": "done",
        "text": text,
        "notes": notes,
        "timeline": timeline,
    });
    if let Some(preset) = export_preset {
        body["export_preset"] = serde_json::Value::String(preset.to_string());
        body["export_handoff"] = serde_json::Value::Bool(export_handoff);
    }
    push(tx, body).await;
}

/// True when a queued render would skip a clip because its bytes are not on Cloudflare.
fn sources_need_browser(timeline: &Timeline, rows: &[oc_db::MediaRow]) -> bool {
    timeline.source_media_ids().into_iter().any(|id| {
        rows.iter()
            .find(|row| row.id == id.as_uuid())
            .is_none_or(|row| !oc_db::is_r2_object_key(&row.r2_key))
    })
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

/// Hold source bytes for one render. The media row stays a browser clip: no R2 key,
/// no status change, and no transcript job.
pub async fn put_media_spool(
    State(state): State<AppState>,
    Path((id, media_id)): Path<(Uuid, Uuid)>,
    SignedIn(email): SignedIn,
    body: axum::body::Bytes,
) -> ApiResult<Json<serde_json::Value>> {
    owned(&state, id, &email).await?;
    if body.is_empty() {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "empty clip"));
    }
    let media = oc_db::get_media(&state.db, media_id).await?;
    if media.project_id != id {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "media not in project"));
    }
    oc_db::write_render_spool(id, media_id, &media.filename, &body)
        .await
        .map_err(|err| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("could not hold the clip for render: {err}"),
            )
        })?;
    tracing::info!(project = %id, media = %media_id, bytes = body.len(), "spooled for render");
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// Byte size of the temporary copy, when this browser has already sent it.
/// The file stays out of Cloudflare. A missing copy is 404.
pub async fn head_media_spool(
    State(state): State<AppState>,
    Path((id, media_id)): Path<(Uuid, Uuid)>,
    SignedIn(email): SignedIn,
) -> ApiResult<Response> {
    owned(&state, id, &email).await?;
    let media = oc_db::get_media(&state.db, media_id).await?;
    if media.project_id != id {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "media not in project"));
    }
    let path = oc_db::render_spool_file(id, media_id, &media.filename);
    let meta = tokio::fs::metadata(&path)
        .await
        .map_err(|_| ApiError::new(StatusCode::NOT_FOUND, "no temporary copy"))?;
    if !meta.is_file() || meta.len() <= 32 {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "no temporary copy"));
    }
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_LENGTH, meta.len().to_string())
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::empty())
        .unwrap_or_else(|_| Response::new(Body::empty())))
}

#[derive(Serialize)]
pub(crate) struct SpeechOut {
    stored: bool,
    cues: i64,
}

/// Whether this clip already has a transcript row. The picture stays in the browser.
pub async fn get_media_speech(
    State(state): State<AppState>,
    Path((id, media_id)): Path<(Uuid, Uuid)>,
    SignedIn(email): SignedIn,
) -> ApiResult<Json<SpeechOut>> {
    owned(&state, id, &email).await?;
    let media = speech_media(&state, id, media_id).await?;
    speech_out(&state, &media).await
}

/// Read this browser's clip once and store the transcript. The media key stays
/// `workspace/`. A picture is also kept as a temporary spool so see and watch can
/// read a frame. That copy is not stored on Cloudflare.
pub async fn put_media_speech(
    State(state): State<AppState>,
    Path((id, media_id)): Path<(Uuid, Uuid)>,
    SignedIn(email): SignedIn,
    body: axum::body::Bytes,
) -> ApiResult<Json<SpeechOut>> {
    owned(&state, id, &email).await?;
    let media = speech_media(&state, id, media_id).await?;
    if oc_db::has_transcript(&state.db, media_id)
        .await
        .map_err(|err| ApiError::new(StatusCode::BAD_GATEWAY, err.to_string()))?
    {
        return speech_out(&state, &media).await;
    }
    if body.is_empty() {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "empty clip"));
    }
    if !media.content_type.starts_with("audio/") {
        if let Err(err) = oc_db::write_render_spool(id, media_id, &media.filename, &body).await {
            tracing::warn!(project = %id, media = %media_id, "look copy was not kept: {err}");
        }
    }
    let transcript = transcript_from_bytes(&body).await.map_err(|err| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            format!("could not read speech: {err}"),
        )
    })?;
    let cues = transcript.cues.len() as i64;
    save_transcript_row(&state.db, media_id, &transcript)
        .await
        .map_err(|err| ApiError::new(StatusCode::BAD_GATEWAY, err.to_string()))?;
    tracing::info!(project = %id, media = %media_id, cues, "speech stored");
    Ok(Json(SpeechOut { stored: true, cues }))
}

async fn speech_media(
    state: &AppState,
    project_id: Uuid,
    media_id: Uuid,
) -> Result<oc_db::MediaRow, ApiError> {
    let media = oc_db::get_media(&state.db, media_id).await?;
    if media.project_id != project_id {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "media not in project"));
    }
    if media.content_type.starts_with("image/") {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "images are not transcribed",
        ));
    }
    Ok(media)
}

async fn speech_out(
    state: &AppState,
    media: &oc_db::MediaRow,
) -> Result<Json<SpeechOut>, ApiError> {
    let stored = oc_db::has_transcript(&state.db, media.id).await?;
    let cues = if stored {
        oc_db::count_transcript_cues(&state.db, media.id).await?
    } else {
        0
    };
    Ok(Json(SpeechOut { stored, cues }))
}

async fn transcript_from_bytes(bytes: &[u8]) -> Result<oc_voice::Transcript, String> {
    let dir = std::env::temp_dir().join(format!("oc-speech-{}", Uuid::now_v7()));
    let _guard = PosterDir(dir.clone());
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|err| err.to_string())?;
    let input = dir.join("source.bin");
    tokio::fs::write(&input, bytes)
        .await
        .map_err(|err| err.to_string())?;
    match oc_voice::transcribe_path(&input).await {
        Ok(transcript) => Ok(transcript),
        Err(err) if ffmpeg_has_no_audio(&err.to_string()) => Ok(oc_voice::Transcript {
            language: None,
            full_text: String::new(),
            cues: Vec::new(),
        }),
        Err(err) => Err(err.to_string()),
    }
}

fn ffmpeg_has_no_audio(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("does not contain any stream")
        || message.contains("output file does not contain")
        || message.contains("no wav written")
}

async fn save_transcript_row(
    db: &oc_db::Db,
    media_id: Uuid,
    transcript: &oc_voice::Transcript,
) -> Result<(), oc_db::DbError> {
    let raw = serde_json::to_value(transcript).unwrap_or_else(|_| serde_json::json!({}));
    let owned: Vec<(i64, i64, String, Option<String>)> = transcript
        .cues
        .iter()
        .map(|cue| {
            (
                cue.start.as_ticks(),
                cue.end.as_ticks(),
                cue.text.clone(),
                cue.speaker.clone(),
            )
        })
        .collect();
    let refs: Vec<(i64, i64, &str, Option<&str>)> = owned
        .iter()
        .map(|(start, end, text, speaker)| (*start, *end, text.as_str(), speaker.as_deref()))
        .collect();
    oc_db::insert_transcript(
        db,
        media_id,
        transcript.language.as_deref(),
        &transcript.full_text,
        &raw,
        &refs,
    )
    .await?;
    Ok(())
}

/// One JPEG for a clip this browser cannot decode. The upload is a temp file and
/// is deleted before this returns. It is not a spool, not `OPENCUT_MEDIA_DIR`, and not R2.
pub async fn post_media_poster(
    State(state): State<AppState>,
    Path((id, media_id)): Path<(Uuid, Uuid)>,
    SignedIn(email): SignedIn,
    body: axum::body::Bytes,
) -> ApiResult<Response> {
    owned(&state, id, &email).await?;
    if body.is_empty() {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "empty clip"));
    }
    let media = oc_db::get_media(&state.db, media_id).await?;
    if media.project_id != id {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "media not in project"));
    }
    let jpeg = poster_from_bytes(&body).await.map_err(|err| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("could not make a still: {err}"),
        )
    })?;
    tracing::info!(
        project = %id,
        media = %media_id,
        bytes = body.len(),
        jpeg = jpeg.len(),
        "poster"
    );
    Ok((
        [
            (header::CONTENT_TYPE, "image/jpeg"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        jpeg,
    )
        .into_response())
}

async fn poster_from_bytes(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.is_empty() {
        return Err("empty clip".into());
    }
    let dir = std::env::temp_dir().join(format!("oc-poster-{}", Uuid::now_v7()));
    let _guard = PosterDir(dir.clone());
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|err| err.to_string())?;
    let input = dir.join("source.bin");
    tokio::fs::write(&input, bytes)
        .await
        .map_err(|err| err.to_string())?;
    let jpeg =
        match tokio::time::timeout(Duration::from_secs(60), oc_media::poster_jpeg(&input)).await {
            Ok(Ok(bytes)) => bytes,
            Ok(Err(err)) => return Err(err.to_string()),
            Err(_) => return Err("still took too long".into()),
        };
    if jpeg.len() < 32 || jpeg[0] != 0xFF || jpeg[1] != 0xD8 {
        return Err("poster was not a jpeg".into());
    }
    Ok(jpeg)
}

/// Deletes the temp upload when the request ends, including on error.
struct PosterDir(std::path::PathBuf);

impl Drop for PosterDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
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
        let play_url = media_play_url(id, row.id, &row.r2_key);
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

fn media_play_url(project_id: Uuid, media_id: Uuid, key: &str) -> Option<String> {
    if oc_db::is_r2_object_key(key) {
        Some(media_file_url(project_id, media_id))
    } else {
        None
    }
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
    let Some(r2) = &state.r2 else {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "R2 is not configured",
        ));
    };
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
    Ok(key)
}

pub async fn get_media_file(
    State(state): State<AppState>,
    Path((id, media_id)): Path<(Uuid, Uuid)>,
    SignedIn(email): SignedIn,
    _headers: axum::http::HeaderMap,
) -> ApiResult<Response> {
    owned(&state, id, &email).await?;
    let media = oc_db::get_media(&state.db, media_id).await?;
    if media.project_id != id {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "media not in project"));
    }
    if oc_db::is_r2_object_key(&media.r2_key) {
        let Some(r2) = &state.r2 else {
            return Err(ApiError::new(
                StatusCode::BAD_GATEWAY,
                "R2 is not configured",
            ));
        };
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
    Err(ApiError::new(
        StatusCode::NOT_FOUND,
        "this clip stays in the browser that imported it",
    ))
}

/// Worker clocks and Postgres can disagree by a few seconds.
const EXPORT_CLOCK_SLACK_MS: u64 = 5_000;

#[derive(Deserialize, Default)]
pub(crate) struct ExportQuery {
    #[serde(default)]
    preset: String,
    #[serde(default)]
    probe: String,
}

fn export_probe_requested(raw: &str) -> bool {
    matches!(raw, "1" | "true" | "yes")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ExportStamp {
    modified_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExportChoice {
    Wait,
    Remote,
}

struct RemoteExport {
    key: String,
    bytes: u64,
}

fn export_preset_wire(preset: oc_core::ExportPreset) -> String {
    serde_json::to_value(preset)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn parse_export_preset(raw: &str) -> Option<oc_core::ExportPreset> {
    oc_core::ExportPreset::parse(raw)
}

/// What the export page should do before the file exists.
fn export_probe_state(file_ready: bool, job_status: Option<&str>) -> &'static str {
    if file_ready {
        "ready"
    } else if job_status == Some("failed") {
        "failed"
    } else if matches!(job_status, Some("queued" | "running" | "done")) {
        "pending"
    } else {
        "idle"
    }
}

fn export_is_current(modified_ms: u64, job_started_ms: Option<i64>) -> bool {
    match job_started_ms {
        Some(started) if started > 0 => {
            modified_ms.saturating_add(EXPORT_CLOCK_SLACK_MS) >= started as u64
        }
        _ => true,
    }
}

/// A file older than this preset's latest export job is the previous render.
fn choose_export(remote: Option<ExportStamp>, job_started_ms: Option<i64>) -> ExportChoice {
    if remote.is_some_and(|stamp| export_is_current(stamp.modified_ms, job_started_ms)) {
        ExportChoice::Remote
    } else {
        ExportChoice::Wait
    }
}

async fn requested_preset(
    state: &AppState,
    id: Uuid,
    raw: &str,
) -> Result<oc_core::ExportPreset, ApiError> {
    if raw.is_empty() {
        let project = oc_db::get_project(&state.db, id).await?;
        return Ok(export_preset(&project.timeline));
    }
    parse_export_preset(raw)
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "unknown export preset"))
}

async fn locate_export(
    state: &AppState,
    id: Uuid,
    preset: oc_core::ExportPreset,
) -> Result<RemoteExport, ApiError> {
    let started = oc_db::latest_export_job_ms(&state.db, id, &export_preset_wire(preset)).await?;
    let filename = preset.file_name(id);
    let key = oc_media::export_object_key(ProjectId::from_uuid(id), &filename);
    let (remote, head_failed) = match &state.r2 {
        Some(r2) => match r2.stat_object(&key).await {
            Ok(found) => (found, false),
            Err(err) => {
                tracing::warn!(project = %id, "export object: {err}");
                (None, true)
            }
        },
        None => (None, false),
    };
    let bytes = remote.map(|item| item.bytes).unwrap_or(0);
    let stamp = remote.map(|item| ExportStamp {
        modified_ms: item.modified_ms,
    });
    match choose_export(stamp, started) {
        ExportChoice::Remote => Ok(RemoteExport { key, bytes }),
        ExportChoice::Wait if head_failed => Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "export store is unavailable",
        )),
        ExportChoice::Wait => Err(ApiError::new(StatusCode::NOT_FOUND, "no export yet")),
    }
}

fn export_head(len: u64) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "video/mp4")
        .header(header::CONTENT_LENGTH, len)
        .header(header::CACHE_CONTROL, "no-store")
        .header(header::ACCEPT_RANGES, "bytes")
        .body(Body::empty())
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

pub async fn head_export(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(query): Query<ExportQuery>,
    SignedIn(email): SignedIn,
) -> ApiResult<Response> {
    owned(&state, id, &email).await?;
    let preset = requested_preset(&state, id, &query.preset).await?;
    let export = locate_export(&state, id, preset).await?;
    Ok(export_head(export.bytes))
}

pub async fn get_export(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(query): Query<ExportQuery>,
    SignedIn(email): SignedIn,
) -> ApiResult<Response> {
    owned(&state, id, &email).await?;
    let preset = requested_preset(&state, id, &query.preset).await?;
    if export_probe_requested(&query.probe) {
        return export_probe(&state, id, preset).await;
    }
    let export = locate_export(&state, id, preset).await?;
    let Some(r2) = &state.r2 else {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "no export yet"));
    };
    let url = r2
        .presign_get(&export.key, Duration::from_secs(6 * 3600))
        .await
        .map_err(|err| ApiError::new(StatusCode::BAD_GATEWAY, err.to_string()))?;
    Ok(Response::builder()
        .status(StatusCode::TEMPORARY_REDIRECT)
        .header(header::LOCATION, url)
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::empty())
        .unwrap_or_else(|_| Response::new(Body::empty())))
}

async fn export_probe(
    state: &AppState,
    id: Uuid,
    preset: oc_core::ExportPreset,
) -> ApiResult<Response> {
    let located = locate_export(state, id, preset).await;
    let file_ready = located.is_ok();
    if let Err(err) = located {
        if err.status != StatusCode::NOT_FOUND {
            return Err(err);
        }
    }
    let wire = export_preset_wire(preset);
    let job = oc_db::latest_export_status(&state.db, id, &wire).await?;
    let status = job.as_ref().map(|(status, _)| status.as_str());
    let state_name = export_probe_state(file_ready, status);
    let error = if state_name == "failed" {
        job.and_then(|(_, error)| error.filter(|text| !text.is_empty()))
            .unwrap_or_else(|| "render failed".into())
    } else {
        String::new()
    };
    Ok(Json(serde_json::json!({
        "state": state_name,
        "error": error,
    }))
    .into_response())
}

#[derive(Deserialize)]
pub struct PatchMediaBody {
    pub duration_seconds: Option<f64>,
    pub width: Option<i32>,
    pub height: Option<i32>,
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
    if let (Some(width), Some(height)) = (body.width, body.height) {
        if width > 1 && height > 1 {
            oc_db::set_media_frame(&state.db, media_id, width, height).await?;
        }
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// What to do after the bytes are stored. Images are ready. Everything else is watched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UploadFollowUp {
    Ready,
    Transcribe,
}

pub(crate) fn upload_follow_up(
    key: &str,
    content_type: &str,
) -> Result<UploadFollowUp, &'static str> {
    if !oc_db::is_r2_object_key(key) {
        return Err("this clip stays in the browser that imported it");
    }
    if content_type.starts_with("image/") {
        Ok(UploadFollowUp::Ready)
    } else {
        Ok(UploadFollowUp::Transcribe)
    }
}

pub async fn complete_upload(
    State(state): State<AppState>,
    Path((id, media_id)): Path<(Uuid, Uuid)>,
    SignedIn(email): SignedIn,
) -> ApiResult<Json<serde_json::Value>> {
    owned(&state, id, &email).await?;
    let media = oc_db::get_media(&state.db, media_id).await?;
    if media.project_id != id {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "media not in project"));
    }
    match upload_follow_up(&media.r2_key, &media.content_type) {
        Err(message) => Err(ApiError::new(StatusCode::CONFLICT, message)),
        Ok(UploadFollowUp::Ready) => {
            oc_db::set_media_status(&state.db, media_id, "ready").await?;
            Ok(Json(serde_json::json!({ "status": "ready" })))
        }
        Ok(UploadFollowUp::Transcribe) => {
            oc_db::enqueue_job(
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
            Ok(Json(serde_json::json!({ "status": "transcribing" })))
        }
    }
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
    match upload_follow_up(&media.r2_key, &media.content_type) {
        Err(message) => return Err(ApiError::new(StatusCode::CONFLICT, message)),
        Ok(UploadFollowUp::Ready) => {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "images are not transcribed",
            ));
        }
        Ok(UploadFollowUp::Transcribe) => {}
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
pub(crate) struct CreateChatBody {
    title: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct SaveChatBody {
    #[serde(default)]
    messages: Vec<oc_db::ChatMessageInput>,
}

#[derive(Deserialize)]
pub struct SessionBody {
    pub email: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct SessionResponse {
    pub email: String,
    pub token: String,
}

pub async fn open_session(
    State(state): State<AppState>,
    Json(body): Json<SessionBody>,
) -> ApiResult<Json<SessionResponse>> {
    let (email, token) = oc_db::open_session(&state.db, &body.email, &body.password).await?;
    Ok(Json(SessionResponse { email, token }))
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
) -> ApiResult<Json<Vec<oc_db::ChatRow>>> {
    owned(&state, id, &email).await?;
    Ok(Json(oc_db::list_chats(&state.db, id, &email).await?))
}

pub async fn create_chat(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    SignedIn(email): SignedIn,
    Json(body): Json<CreateChatBody>,
) -> ApiResult<Json<oc_db::ChatRow>> {
    owned(&state, id, &email).await?;
    let chat = oc_db::create_chat(&state.db, id, &email, body.title.as_deref()).await?;
    tracing::info!(project = %id, chat = %chat.id, "chat created");
    Ok(Json(chat))
}

pub async fn get_chat(
    State(state): State<AppState>,
    Path((id, chat_id)): Path<(Uuid, Uuid)>,
    SignedIn(email): SignedIn,
) -> ApiResult<Json<ChatDetail>> {
    owned(&state, id, &email).await?;
    let (chat, messages) = oc_db::get_chat(&state.db, id, chat_id, &email).await?;
    Ok(Json(ChatDetail { chat, messages }))
}

pub async fn save_chat_messages(
    State(state): State<AppState>,
    Path((id, chat_id)): Path<(Uuid, Uuid)>,
    SignedIn(email): SignedIn,
    Json(body): Json<SaveChatBody>,
) -> ApiResult<Json<oc_db::ChatRow>> {
    owned(&state, id, &email).await?;
    let chat = oc_db::save_chat_messages(&state.db, id, chat_id, &email, &body.messages).await?;
    tracing::info!(project = %id, chat = %chat_id, n = body.messages.len(), "chat saved");
    Ok(Json(chat))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clip_with_no_audio_is_a_stored_silence() {
        assert!(ffmpeg_has_no_audio(
            "ffmpeg failed: Output file does not contain any stream"
        ));
        assert!(ffmpeg_has_no_audio("no wav written"));
        assert!(!ffmpeg_has_no_audio("groq stt http: connection timed out"));
    }

    #[test]
    fn upload_complete_waits_for_an_r2_object() {
        assert_eq!(
            upload_follow_up("workspace/abc", "video/mp4"),
            Err("this clip stays in the browser that imported it")
        );
        assert!(upload_follow_up("local/clip.mp4", "video/mp4").is_err());
        assert!(upload_follow_up("", "video/mp4").is_err());
        assert_eq!(
            upload_follow_up("raw/p/m/a.mp4", "video/mp4").unwrap(),
            UploadFollowUp::Transcribe
        );
        assert_eq!(
            upload_follow_up("raw/p/m/a.png", "image/png").unwrap(),
            UploadFollowUp::Ready
        );
    }

    #[test]
    fn a_poster_upload_is_deleted_with_the_request() {
        let dir = std::env::temp_dir().join(format!("oc-poster-{}", Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("source.bin"), b"clip-bytes").unwrap();
        {
            let _guard = PosterDir(dir.clone());
            assert!(dir.join("source.bin").exists());
        }
        assert!(!dir.exists());
    }

    #[tokio::test]
    async fn a_poster_of_an_uploaded_clip_is_a_jpeg_and_the_file_is_removed() {
        let ready = std::process::Command::new("ffmpeg")
            .arg("-version")
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false);
        if !ready {
            return;
        }
        let dir = std::env::temp_dir().join(format!("oc-poster-src-{}", Uuid::now_v7()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let file = dir.join("sample.mp4");
        let made = tokio::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=160x120:rate=15:duration=1.2",
                "-pix_fmt",
                "yuv420p",
                "-c:v",
                "libx264",
                &file.to_string_lossy(),
            ])
            .output()
            .await
            .unwrap();
        assert!(
            made.status.success(),
            "{}",
            String::from_utf8_lossy(&made.stderr)
        );
        let bytes = tokio::fs::read(&file).await.unwrap();
        let _ = tokio::fs::remove_dir_all(&dir).await;
        let jpeg = poster_from_bytes(&bytes).await.expect("poster");
        assert!(jpeg.len() > 400 && jpeg[0] == 0xFF && jpeg[1] == 0xD8);
    }

    #[test]
    fn a_browser_clip_has_no_server_play_url() {
        let project = Uuid::now_v7();
        let media = Uuid::now_v7();
        assert_eq!(media_play_url(project, media, "workspace/abc"), None);
        assert_eq!(media_play_url(project, media, "local/clip.mp4"), None);
        assert_eq!(media_play_url(project, media, ""), None);
        let remote = media_play_url(project, media, "raw/p/m/a.mp4").expect("r2 url");
        assert!(remote.contains(&media.to_string()), "{remote}");
        assert!(!remote.starts_with("blob:"), "{remote}");
    }

    #[test]
    fn a_browser_timeline_waits_for_this_browser_before_export() {
        let project = Uuid::now_v7();
        let media = Uuid::now_v7();
        let row: oc_db::MediaRow = serde_json::from_value(serde_json::json!({
            "id": media,
            "project_id": project,
            "r2_key": format!("workspace/{media}"),
            "filename": "take.mp4",
            "content_type": "video/mp4",
            "status": "browser",
            "created_at": "2026-10-08T00:00:00Z",
        }))
        .unwrap();
        let mut timeline = Timeline::default();
        let track = timeline.first_track(oc_core::TrackKind::Video).unwrap().id;
        timeline
            .add_clip(
                track,
                oc_core::Clip {
                    id: oc_core::ClipId::new(),
                    media_id: Some(oc_core::MediaId::from_uuid(media)),
                    kind: oc_core::ClipKind::Video {
                        transform: oc_core::Transform::default(),
                    },
                    start: oc_core::Time::ZERO,
                    duration: oc_core::Duration::from_seconds(2.0),
                    source_in: oc_core::Time::ZERO,
                    speed: 1.0,
                    group_id: None,
                    link_id: None,
                    disabled: false,
                    look: oc_core::ClipLook::default(),
                },
            )
            .unwrap();
        assert!(sources_need_browser(&timeline, &[row.clone()]));
        let mut remote = row;
        remote.r2_key = format!("raw/{project}/{media}/take.mp4");
        assert!(!sources_need_browser(&timeline, &[remote]));
        assert!(sources_need_browser(&timeline, &[]));
    }

    #[test]
    fn an_export_probe_stays_idle_until_a_job_exists() {
        assert_eq!(export_probe_state(true, Some("failed")), "ready");
        assert_eq!(export_probe_state(false, None), "idle");
        assert_eq!(export_probe_state(false, Some("queued")), "pending");
        assert_eq!(export_probe_state(false, Some("running")), "pending");
        assert_eq!(export_probe_state(false, Some("failed")), "failed");
        assert!(export_probe_requested("1"));
        assert!(!export_probe_requested(""));
    }

    #[test]
    fn a_session_reads_the_token_and_ignores_the_email() {
        assert_eq!(presented_token("", "user=ada@studio.com&token=abc"), "abc");
        assert_eq!(
            presented_token("header-token", "user=ada@studio.com"),
            "header-token"
        );
        assert_eq!(presented_token("", "user=ada@studio.com"), "");
    }

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

    #[test]
    fn query_email_is_percent_decoded() {
        assert_eq!(query_param("user=a%40b.com&x=1", "user"), "a@b.com");
        assert_eq!(query_param("user=a+b", "user"), "a b");
        assert_eq!(query_param("other=1", "user"), "");
        assert_eq!(percent_decode("%2"), "%2");
    }

    #[test]
    fn an_export_file_is_named_for_its_preset() {
        let id = Uuid::nil();
        assert_eq!(
            oc_core::ExportPreset::Youtube1080.file_name(id),
            "00000000-0000-0000-0000-000000000000-youtube-1080.mp4"
        );
        assert_ne!(
            oc_core::ExportPreset::Youtube1080.file_name(id),
            oc_core::ExportPreset::Vertical1080.file_name(id)
        );
        let wire = export_preset_wire(oc_core::ExportPreset::Vertical1080);
        assert_eq!(
            wire,
            serde_json::to_value(oc_core::ExportPreset::Vertical1080)
                .unwrap()
                .as_str()
                .unwrap()
        );
        assert_eq!(
            parse_export_preset(&wire),
            Some(oc_core::ExportPreset::Vertical1080)
        );
        assert_eq!(
            parse_export_preset("youtube-1080"),
            Some(oc_core::ExportPreset::Youtube1080)
        );
        assert_eq!(
            parse_export_preset("square_1080"),
            Some(oc_core::ExportPreset::Square1080)
        );
        assert_eq!(parse_export_preset("nope"), None);
        assert_eq!(
            parse_export_preset("classic-1080"),
            Some(oc_core::ExportPreset::Classic1080)
        );
        assert_eq!(
            oc_core::ExportPreset::for_frame(1440, 1080),
            oc_core::ExportPreset::Classic1080
        );
        let mut vertical = oc_core::Timeline::default();
        vertical.width = 1080;
        vertical.height = 1920;
        assert_eq!(
            export_preset(&vertical),
            oc_core::ExportPreset::Vertical1080
        );
    }

    #[test]
    fn a_current_cloudflare_file_is_served_and_an_older_render_is_not() {
        let started = 1_000_000_i64;
        let old = ExportStamp {
            modified_ms: (started as u64) - 60_000,
        };
        let fresh = ExportStamp {
            modified_ms: (started as u64) + 1_000,
        };
        let within_slack = ExportStamp {
            modified_ms: (started as u64) - 1_000,
        };
        let older_than_slack = ExportStamp {
            modified_ms: (started as u64) - EXPORT_CLOCK_SLACK_MS - 1,
        };
        assert_eq!(choose_export(None, Some(started)), ExportChoice::Wait);
        assert_eq!(choose_export(Some(old), Some(started)), ExportChoice::Wait);
        assert_eq!(
            choose_export(Some(fresh), Some(started)),
            ExportChoice::Remote
        );
        assert_eq!(
            choose_export(Some(within_slack), Some(started)),
            ExportChoice::Remote
        );
        assert_eq!(
            choose_export(Some(older_than_slack), Some(started)),
            ExportChoice::Wait
        );
        assert_eq!(choose_export(Some(old), None), ExportChoice::Remote);
    }

    #[test]
    fn a_dead_provider_resumes_and_a_login_failure_does_not() {
        assert!(HOST_TURNS > 8);
        assert!(PROVIDER_RESUMES >= 1);
        assert!(provider_stopped("ACP CLI closed"));
        assert!(provider_stopped("ACP prompt: process exited"));
        assert!(provider_stopped("Broken pipe (os error 32)"));
        assert!(provider_stopped("Connection reset by peer (os error 104)"));
        assert!(provider_stopped("unexpected end of file"));
        assert!(provider_stopped("early eof"));
        assert!(!provider_stopped(
            "ACP CLI closed\nClaude cannot edit without claude-agent-acp"
        ));
        assert!(!provider_stopped(
            "Codex cannot edit without codex-acp. Install codex-acp."
        ));
        assert!(!provider_stopped(
            "Run `claude auth login` so we can use the local Claude store."
        ));
        assert!(!provider_stopped("unknown provider potato"));
        assert!(!provider_stopped("ACP adapter missing."));
        assert!(!provider_stopped("tool error: media not found"));
        let held = hold_open_fixes("The grade still jumps.\n");
        assert!(held.contains("The grade still jumps."));
        assert!(held.contains(OPEN_FIXES_HOLD));
        assert!(!held.contains("export queued"));
    }

    #[test]
    fn an_unchanged_tool_batch_stops_on_the_second_repeat() {
        let batch = "see: frame";
        let mut last: Option<String> = None;
        let mut repeats = 0;
        repeats = next_unchanged_repeats(false, batch, last.as_deref(), repeats);
        last = Some(batch.into());
        assert_eq!(repeats, 0);
        repeats = next_unchanged_repeats(false, batch, last.as_deref(), repeats);
        assert_eq!(repeats, 1);
        repeats = next_unchanged_repeats(false, batch, last.as_deref(), repeats);
        assert_eq!(repeats, 2);
        assert_eq!(
            next_unchanged_repeats(true, batch, last.as_deref(), repeats),
            0
        );
        assert_eq!(
            next_unchanged_repeats(false, "watch: line", last.as_deref(), repeats),
            0
        );
        let text = unfinished_with_notes(&["see: frame".into()]);
        assert!(text.starts_with(UNFINISHED_EDIT));
        assert!(text.contains("see: frame"));
        assert!(!text.contains("export queued"));
    }
}
