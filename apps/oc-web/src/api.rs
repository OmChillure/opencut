use crate::bind;
use crate::media::MediaItem;
use oc_core::{Op, Project, Timeline};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize)]
pub struct AiModel {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AiProvider {
    pub id: String,
    pub name: String,
    pub connected: bool,
    #[serde(default)]
    pub login_hint: String,
    pub models: Vec<AiModel>,
}

#[derive(Deserialize)]
struct ProvidersResp {
    providers: Vec<AiProvider>,
}

pub async fn list_ai_providers() -> Result<Vec<AiProvider>, String> {
    let resp: ProvidersResp = authed(reqwest::Client::new().get(format!("{API}/v1/ai/providers")))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    Ok(resp.providers)
}

#[derive(Clone, Debug, Deserialize)]
pub struct ChatReply {
    pub text: String,
    #[serde(default)]
    pub notes: Vec<String>,
    pub timeline: Timeline,
    /// Set when a finished director cut should become a Cloudflare video.
    #[serde(default)]
    pub export_preset: String,
    /// The browser must send its clips and then queue the export.
    #[serde(default)]
    pub export_handoff: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ChatStreamEvent {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub args: serde_json::Value,
    #[serde(default)]
    pub result: Option<String>,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub notes: Vec<String>,
    #[serde(default)]
    pub timeline: Option<Timeline>,
    #[serde(default)]
    pub export_preset: String,
    #[serde(default)]
    pub export_handoff: bool,
}

pub async fn register_media(
    project_id: &str,
    media_id: &str,
    filename: &str,
    content_type: &str,
    duration: f64,
) -> Result<(), String> {
    authed(reqwest::Client::new().post(format!("{API}/v1/projects/{project_id}/media")))
        .json(&serde_json::json!({
            "id": media_id,
            "filename": filename,
            "content_type": content_type,
            "duration_seconds": duration,
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Register the clip, and keep the bytes in this browser. Nothing is sent to Cloudflare.
pub async fn store_imported(
    project_id: &str,
    media_id: &str,
    filename: &str,
    content_type: &str,
    bytes: Vec<u8>,
) -> Result<(), String> {
    crate::hold::save_file(project_id, media_id, filename, content_type, &bytes).await?;
    register_media(project_id, media_id, filename, content_type, 0.0).await
}

/// A server play URL. A blob, or an empty URL, is not one: the picture stays in the browser.
#[must_use]
pub fn remote_play_url(play_url: Option<String>) -> Option<String> {
    play_url.filter(|url| !url.is_empty() && !url.starts_with("blob:"))
}

/// Keep a blob the browser already has. A server URL fills in only when this
/// tab has no picture yet, which is an older clip that was stored on the server.
#[must_use]
pub fn keep_play_url(existing: &str, remote: &str) -> String {
    if existing.starts_with("blob:") || remote.is_empty() {
        return existing.to_string();
    }
    if existing.is_empty() {
        return remote.to_string();
    }
    existing.to_string()
}

thread_local! {
    static CHAT_STOP: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static CHAT_GEN: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    static CHAT_ABORT: std::cell::RefCell<Option<(u32, web_sys::AbortController)>> =
        std::cell::RefCell::new(None);
}

/// Starts one chat turn. A later stop only applies to this generation.
pub fn begin_chat() -> u32 {
    CHAT_STOP.with(|cell| cell.set(false));
    CHAT_GEN.with(|cell| {
        let next = cell.get().wrapping_add(1);
        cell.set(next);
        next
    })
}

pub fn chat_generation() -> u32 {
    CHAT_GEN.with(|cell| cell.get())
}

/// True while `turn` is still the turn on screen and the user has not stopped it.
pub fn chat_current(turn: u32) -> bool {
    chat_generation() == turn && !chat_stopped()
}

pub fn request_chat_stop() {
    CHAT_STOP.with(|cell| cell.set(true));
    abort_chat_stream();
}

pub fn chat_stopped() -> bool {
    CHAT_STOP.with(|cell| cell.get())
}

fn abort_chat_stream() {
    #[cfg(target_arch = "wasm32")]
    CHAT_ABORT.with(|slot| {
        if let Some((_, ctrl)) = slot.borrow().as_ref() {
            ctrl.abort();
        }
    });
}

#[cfg(target_arch = "wasm32")]
fn bind_chat_abort(turn: u32, ctrl: web_sys::AbortController) {
    CHAT_ABORT.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some((_, prev)) = slot.take() {
            if prev != ctrl {
                prev.abort();
            }
        }
        *slot = Some((turn, ctrl));
    });
}

#[cfg(target_arch = "wasm32")]
fn clear_chat_abort(turn: u32) {
    CHAT_ABORT.with(|slot| {
        let same = slot
            .borrow()
            .as_ref()
            .is_some_and(|(bound, _)| *bound == turn);
        if same {
            *slot.borrow_mut() = None;
        }
    });
}

#[cfg(target_arch = "wasm32")]
struct ChatAbortLease(u32);

#[cfg(target_arch = "wasm32")]
impl Drop for ChatAbortLease {
    fn drop(&mut self) {
        clear_chat_abort(self.0);
    }
}

pub async fn chat_stream(
    project_id: &str,
    provider: &str,
    model: &str,
    messages: &[(bool, String)],
    turn: u32,
    mut on_event: impl FnMut(ChatStreamEvent),
) -> Result<ChatReply, String> {
    let body = serde_json::json!({
        "provider": provider,
        "model": model,
        "messages": messages.iter().map(|(user, text)| serde_json::json!({
            "role": if *user { "user" } else { "assistant" },
            "content": text,
        })).collect::<Vec<_>>(),
    });
    let mut reply = ChatReply {
        text: String::new(),
        notes: Vec::new(),
        timeline: Timeline::default(),
        export_preset: String::new(),
        export_handoff: false,
    };
    let mut saw_done = false;
    let url = format!("{API}/v1/projects/{project_id}/chat");
    read_ndjson(&url, &body.to_string(), turn, |line| {
        if !chat_current(turn) {
            return Err("stopped".into());
        }
        let ev: ChatStreamEvent = serde_json::from_str(line).map_err(|e| e.to_string())?;
        if ev.kind == "done" {
            reply.text = ev.text.clone();
            reply.notes = ev.notes.clone();
            reply.export_preset = ev.export_preset.clone();
            reply.export_handoff = ev.export_handoff;
            if let Some(tl) = ev.timeline.clone() {
                reply.timeline = tl;
            }
            saw_done = true;
        }
        if ev.kind == "error" && !ev.text.is_empty() {
            return Err(ev.text.clone());
        }
        on_event(ev);
        Ok(())
    })
    .await?;
    if !saw_done && reply.text.is_empty() {
        return Err("chat stream ended without a reply".into());
    }
    Ok(reply)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChatSummary {
    pub id: String,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredMsg {
    pub role: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub tool_id: String,
    #[serde(default)]
    pub tool_name: String,
    #[serde(default)]
    pub tool_status: String,
    #[serde(default)]
    pub tool_args: String,
    #[serde(default)]
    pub tool_result: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ChatDetail {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub messages: Vec<StoredMsg>,
}

fn user_query(user: &str) -> String {
    let mut out = String::with_capacity(user.len());
    for b in user.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn signed_token() -> Option<String> {
    crate::auth::current_token().filter(|token| !token.is_empty())
}

fn authed(builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    match signed_token() {
        Some(token) => builder.header("x-opencut-token", token),
        None => builder,
    }
}

/// A video element cannot send a header. File and export URLs carry `?token=`.
pub(crate) fn token_query(url: &str, token: Option<&str>) -> String {
    if url.is_empty() || url.starts_with("blob:") || url.contains("token=") {
        return url.to_string();
    }
    let ours = url.contains("/v1/projects/")
        && ((url.contains("/media/") && url.contains("/file")) || url.contains("/export"));
    if !ours {
        return url.to_string();
    }
    let Some(token) = token.filter(|token| !token.is_empty()) else {
        return url.to_string();
    };
    let join = if url.contains('?') { '&' } else { '?' };
    format!("{url}{join}token={}", user_query(token))
}

fn with_token(url: &str) -> String {
    token_query(url, signed_token().as_deref())
}

pub fn export_file_url(project_id: &str, preset: &str) -> String {
    with_token(&format!("{API}{}", export_query(project_id, preset)))
}

fn export_query(project_id: &str, preset: &str) -> String {
    format!("/v1/projects/{project_id}/export?preset={preset}")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExportWatch {
    Ready,
    Pending,
    Idle,
    Failed(String),
}

pub async fn watch_export(project_id: &str, preset: &str) -> Result<ExportWatch, String> {
    #[derive(Deserialize)]
    struct Body {
        state: String,
        #[serde(default)]
        error: String,
    }
    let body: Body = get_json(&format!(
        "{API}/v1/projects/{project_id}/export?preset={preset}&probe=1"
    ))
    .await?;
    Ok(match body.state.as_str() {
        "ready" => ExportWatch::Ready,
        "pending" => ExportWatch::Pending,
        "failed" => ExportWatch::Failed(if body.error.is_empty() {
            "The render failed.".into()
        } else {
            body.error
        }),
        _ => ExportWatch::Idle,
    })
}

/// Send this browser's timeline clips to the render spool. Server clips are left alone.
/// Nothing is written to Cloudflare here.
pub async fn send_browser_sources(project_id: &str) -> Result<(), String> {
    let project = get_project(project_id).await?;
    let media = list_media(project_id).await?;
    let held = crate::hold::files_for_project(project_id).await?;
    let clips = project
        .timeline
        .source_media_ids()
        .into_iter()
        .map(|id| {
            let id = id.to_string();
            let item = media.iter().find(|item| item.id == id);
            oc_core::timeline::BrowserHold {
                id: id.clone(),
                name: item
                    .map(|item| item.name.clone())
                    .unwrap_or_else(|| id.clone()),
                on_server: item.is_some_and(|item| !item.url.is_empty()),
                in_browser: held.iter().any(|file| file.id == id),
            }
        })
        .collect::<Vec<_>>();
    let ids = oc_core::timeline::browser_spool_ids(&clips)?;
    for id in ids {
        let file = held
            .iter()
            .find(|file| file.id == id)
            .ok_or_else(|| format!("These clips are not in this browser: {id}"))?;
        spool_media(project_id, &file.id, file.bytes.clone()).await?;
    }
    Ok(())
}

async fn spool_media(project_id: &str, media_id: &str, bytes: Vec<u8>) -> Result<(), String> {
    let res = authed(
        reqwest::Client::new()
            .put(format!(
                "{API}/v1/projects/{project_id}/media/{media_id}/spool"
            ))
            .header("content-type", "application/octet-stream"),
    )
    .body(bytes)
    .send()
    .await
    .map_err(|e| e.to_string())?;
    if res.status().is_success() {
        return Ok(());
    }
    Err(api_error_message(res).await)
}

async fn api_error_message(res: reqwest::Response) -> String {
    #[derive(Deserialize)]
    struct ApiErr {
        error: String,
    }
    let status = res.status();
    let text = res.text().await.unwrap_or_default();
    serde_json::from_str::<ApiErr>(&text)
        .map(|body| body.error)
        .unwrap_or_else(|_| {
            if text.trim().is_empty() {
                format!("HTTP {}", status.as_u16())
            } else {
                text
            }
        })
}

pub async fn open_session(email: &str, password: &str) -> Result<(String, String), String> {
    #[derive(Deserialize)]
    struct SessionResp {
        email: String,
        token: String,
    }
    #[derive(Deserialize)]
    struct ApiErr {
        error: String,
    }
    let res = reqwest::Client::new()
        .post(format!("{API}/v1/session"))
        .json(&serde_json::json!({ "email": email, "password": password }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = res.status();
    let text = res.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        let msg = serde_json::from_str::<ApiErr>(&text)
            .map(|body| body.error)
            .unwrap_or_else(|_| {
                if text.trim().is_empty() {
                    format!("HTTP {}", status.as_u16())
                } else {
                    text
                }
            });
        return Err(msg);
    }
    let body: SessionResp = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    if body.token.is_empty() {
        return Err("sign in failed".into());
    }
    Ok((body.email, body.token))
}

pub async fn list_chats(project_id: &str) -> Result<Vec<ChatSummary>, String> {
    get_json(&format!("{API}/v1/projects/{project_id}/chats")).await
}

pub async fn create_chat(project_id: &str) -> Result<ChatSummary, String> {
    authed(reqwest::Client::new().post(format!("{API}/v1/projects/{project_id}/chats")))
        .json(&serde_json::json!({}))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())
}

pub async fn get_chat(project_id: &str, chat_id: &str) -> Result<ChatDetail, String> {
    get_json(&format!("{API}/v1/projects/{project_id}/chats/{chat_id}")).await
}

pub async fn save_chat(
    project_id: &str,
    chat_id: &str,
    messages: &[StoredMsg],
) -> Result<ChatSummary, String> {
    authed(reqwest::Client::new().put(format!(
        "{API}/v1/projects/{project_id}/chats/{chat_id}/messages"
    )))
    .json(&serde_json::json!({ "messages": messages }))
    .send()
    .await
    .map_err(|e| e.to_string())?
    .error_for_status()
    .map_err(|e| e.to_string())?
    .json()
    .await
    .map_err(|e| e.to_string())
}

async fn read_ndjson(
    url: &str,
    json_body: &str,
    turn: u32,
    on_line: impl FnMut(&str) -> Result<(), String>,
) -> Result<(), String> {
    #[cfg(target_arch = "wasm32")]
    {
        wasm_read_ndjson(url, json_body, turn, on_line).await
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut on_line = on_line;
        if !chat_current(turn) {
            return Err("stopped".into());
        }
        let raw = authed(
            reqwest::Client::new()
                .post(url)
                .header("content-type", "application/json"),
        )
        .body(json_body.to_string())
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())?;
        for line in raw.lines() {
            let line = line.trim();
            if !line.is_empty() {
                on_line(line)?;
            }
        }
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
async fn wasm_read_ndjson(
    url: &str,
    json_body: &str,
    turn: u32,
    mut on_line: impl FnMut(&str) -> Result<(), String>,
) -> Result<(), String> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{ReadableStreamDefaultReader, Request, RequestInit, RequestMode, Response};

    let ctrl = web_sys::AbortController::new().map_err(js_err)?;
    let opts = RequestInit::new();
    opts.set_method("POST");
    opts.set_mode(RequestMode::Cors);
    opts.set_body(&wasm_bindgen::JsValue::from_str(json_body));
    opts.set_signal(Some(&ctrl.signal()));
    bind_chat_abort(turn, ctrl);
    let _lease = ChatAbortLease(turn);
    if !chat_current(turn) {
        return Err("stopped".into());
    }
    let request = Request::new_with_str_and_init(url, &opts).map_err(js_err)?;
    request
        .headers()
        .set("content-type", "application/json")
        .map_err(js_err)?;
    if let Some(token) = signed_token() {
        request
            .headers()
            .set("x-opencut-token", &token)
            .map_err(js_err)?;
    }
    let window = web_sys::window().ok_or_else(|| "no window".to_string())?;
    let resp = match JsFuture::from(window.fetch_with_request(&request)).await {
        Ok(resp) => resp,
        Err(err) => {
            if !chat_current(turn) {
                return Err("stopped".into());
            }
            return Err(js_err(err));
        }
    };
    let resp: Response = resp.dyn_into().map_err(|_| "bad response".to_string())?;
    if !resp.ok() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let Some(body) = resp.body() else {
        return Err("empty body".into());
    };
    let reader: ReadableStreamDefaultReader = body
        .get_reader()
        .dyn_into()
        .map_err(|_| "stream reader".to_string())?;
    let mut pending = String::new();
    loop {
        if !chat_current(turn) {
            let _ = reader.cancel();
            return Err("stopped".into());
        }
        let next = match JsFuture::from(reader.read()).await {
            Ok(next) => next,
            Err(err) => {
                if !chat_current(turn) {
                    return Err("stopped".into());
                }
                return Err(js_err(err));
            }
        };
        let done = js_sys::Reflect::get(&next, &"done".into())
            .ok()
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        if !done {
            let value = js_sys::Reflect::get(&next, &"value".into()).map_err(js_err)?;
            if !value.is_undefined() && !value.is_null() {
                let arr = js_sys::Uint8Array::new(&value);
                let mut bytes = vec![0u8; arr.length() as usize];
                arr.copy_to(&mut bytes);
                pending.push_str(&String::from_utf8_lossy(&bytes));
            }
        }
        while let Some(idx) = pending.find('\n') {
            let line = pending[..idx].trim().to_string();
            pending = pending[idx + 1..].to_string();
            if !line.is_empty() {
                on_line(&line)?;
            }
        }
        if done {
            break;
        }
    }
    let tail = pending.trim().to_string();
    if !tail.is_empty() {
        on_line(&tail)?;
    }
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn js_err(err: wasm_bindgen::JsValue) -> String {
    err.as_string().unwrap_or_else(|| format!("{err:?}"))
}

const API: &str = "http://127.0.0.1:8787";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectSummary {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub updated_at: Option<String>,
}

#[derive(Deserialize)]
struct ApiProject {
    id: serde_json::Value,
    name: String,
    #[serde(default)]
    updated_at: Option<String>,
}

impl ApiProject {
    fn into_summary(self) -> ProjectSummary {
        ProjectSummary {
            id: value_to_id(self.id),
            name: self.name,
            updated_at: self.updated_at,
        }
    }
}

fn value_to_id(value: serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s,
        other => other.to_string().trim_matches('"').to_string(),
    }
}

pub async fn list_projects() -> Result<Vec<ProjectSummary>, String> {
    let rows: Vec<ApiProject> = authed(reqwest::Client::new().get(format!("{API}/v1/projects")))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    Ok(rows.into_iter().map(ApiProject::into_summary).collect())
}

pub async fn get_project(id: &str) -> Result<Project, String> {
    get_json(&format!("{API}/v1/projects/{id}")).await
}

pub async fn rename_project(id: &str, name: &str) -> Result<(), String> {
    let _project: Project = authed(reqwest::Client::new().patch(format!("{API}/v1/projects/{id}")))
        .json(&serde_json::json!({ "name": name }))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn apply_ops(id: &str, ops: Vec<Op>) -> Result<Timeline, String> {
    #[derive(Serialize)]
    struct Body {
        ops: Vec<Op>,
    }
    #[derive(Deserialize)]
    struct Resp {
        timeline: Timeline,
    }
    let resp: Resp = authed(reqwest::Client::new().post(format!("{API}/v1/projects/{id}/ops")))
        .json(&Body { ops })
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    Ok(resp.timeline)
}

pub async fn generate_captions(id: &str) -> Result<(Timeline, String), String> {
    #[derive(Deserialize)]
    struct Resp {
        timeline: Timeline,
        note: String,
    }
    let resp: Resp =
        authed(reqwest::Client::new().post(format!("{API}/v1/projects/{id}/captions")))
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
    Ok((resp.timeline, resp.note))
}

pub async fn save_timeline(id: &str, timeline: Timeline) -> Result<Timeline, String> {
    apply_ops(id, vec![Op::SetTimeline { timeline }]).await
}

pub async fn patch_media_duration(
    project_id: &str,
    media_id: &str,
    seconds: f64,
) -> Result<(), String> {
    authed(
        reqwest::Client::new().patch(format!("{API}/v1/projects/{project_id}/media/{media_id}")),
    )
    .json(&serde_json::json!({ "duration_seconds": seconds }))
    .send()
    .await
    .map_err(|e| e.to_string())?
    .error_for_status()
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn list_media(project_id: &str) -> Result<Vec<MediaItem>, String> {
    #[derive(Deserialize)]
    struct Row {
        id: serde_json::Value,
        filename: String,
        content_type: String,
        #[serde(default)]
        duration_ticks: Option<i64>,
        #[serde(default)]
        play_url: Option<String>,
    }
    let rows: Vec<Row> = get_json(&format!("{API}/v1/projects/{project_id}/media")).await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let id = value_to_id(row.id);
            let play = remote_play_url(row.play_url).map(|url| with_token(&url));
            bind::media_from_api(
                &id,
                row.filename,
                &row.content_type,
                row.duration_ticks,
                play,
            )
        })
        .collect())
}

async fn get_json<T: for<'de> Deserialize<'de>>(url: &str) -> Result<T, String> {
    authed(reqwest::Client::new().get(url))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())
}

pub async fn delete_project(id: &str) -> Result<(), String> {
    let res = authed(reqwest::Client::new().delete(format!("{API}/v1/projects/{id}")))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if res.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(());
    }
    res.error_for_status().map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn create_project(name: &str) -> Result<ProjectSummary, String> {
    let row: ApiProject = authed(reqwest::Client::new().post(format!("{API}/v1/projects")))
        .json(&serde_json::json!({ "name": name }))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    Ok(row.into_summary())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stopped_turn_does_not_keep_the_composer() {
        let first = begin_chat();
        assert!(chat_current(first));
        request_chat_stop();
        assert!(chat_stopped());
        assert!(!chat_current(first));
        let second = begin_chat();
        assert_ne!(first, second);
        assert!(chat_current(second));
        assert!(!chat_current(first));
        assert!(!chat_stopped());
    }

    #[test]
    fn a_browser_clip_has_no_server_play_url() {
        assert_eq!(remote_play_url(None), None);
        assert_eq!(remote_play_url(Some(String::new())), None);
        assert_eq!(remote_play_url(Some("blob:http://local/1".into())), None);
        assert_eq!(
            remote_play_url(Some("https://cdn.example/a.mp4".into())).as_deref(),
            Some("https://cdn.example/a.mp4")
        );
    }

    #[test]
    fn a_blob_is_not_replaced_by_a_server_url() {
        assert_eq!(keep_play_url("blob:1", "https://cdn/a.mp4"), "blob:1");
        assert_eq!(keep_play_url("blob:1", ""), "blob:1");
        assert_eq!(keep_play_url("", "https://cdn/a.mp4"), "https://cdn/a.mp4");
        assert_eq!(keep_play_url("", ""), "");
        assert_eq!(keep_play_url("https://old", "https://new"), "https://old");
    }

    #[test]
    fn export_url_names_the_preset() {
        assert_eq!(
            export_query("abc", "vertical-1080"),
            "/v1/projects/abc/export?preset=vertical-1080"
        );
        assert_ne!(
            export_query("abc", "youtube-1080"),
            export_query("abc", "square-1080")
        );
    }

    #[test]
    fn file_urls_carry_the_token_not_the_email() {
        let file = "http://127.0.0.1:8787/v1/projects/p/media/m/file";
        assert_eq!(
            token_query(file, Some("abc")),
            "http://127.0.0.1:8787/v1/projects/p/media/m/file?token=abc"
        );
        let export = "http://127.0.0.1:8787/v1/projects/p/export?preset=youtube-1080";
        let with = token_query(export, Some("tok en"));
        assert!(with.contains("token=tok%20en"));
        assert!(!with.contains("user="));
        assert_eq!(token_query(file, None), file);
        assert_eq!(token_query("blob:http://x", Some("abc")), "blob:http://x");
        assert_eq!(
            token_query("http://127.0.0.1:8787/v1/projects", Some("abc")),
            "http://127.0.0.1:8787/v1/projects"
        );
        assert_eq!(
            token_query(&format!("{file}?token=kept"), Some("abc")),
            format!("{file}?token=kept")
        );
    }
}
