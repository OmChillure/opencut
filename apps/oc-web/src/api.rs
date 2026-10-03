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
    let resp: ProvidersResp = reqwest::Client::new()
        .get(format!("{API}/v1/ai/providers"))
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
}

pub async fn register_media(
    project_id: &str,
    media_id: &str,
    filename: &str,
    content_type: &str,
    duration: f64,
) -> Result<(), String> {
    reqwest::Client::new()
        .post(format!("{API}/v1/projects/{project_id}/media"))
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

pub async fn put_media_bytes(
    project_id: &str,
    media_id: &str,
    content_type: &str,
    bytes: Vec<u8>,
) -> Result<(), String> {
    let res = reqwest::Client::new()
        .put(format!("{API}/v1/projects/{project_id}/media/{media_id}/bytes"))
        .header("content-type", content_type)
        .body(bytes)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        return Err(format!("upload {status}: {body}"));
    }
    Ok(())
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

pub async fn list_chats(project_id: &str, user: &str) -> Result<Vec<ChatSummary>, String> {
    let url = format!(
        "{API}/v1/projects/{project_id}/chats?user={}",
        user_query(user)
    );
    get_json(&url).await
}

pub async fn create_chat(project_id: &str, user: &str) -> Result<ChatSummary, String> {
    reqwest::Client::new()
        .post(format!("{API}/v1/projects/{project_id}/chats"))
        .json(&serde_json::json!({ "user": user }))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())
}

pub async fn get_chat(project_id: &str, chat_id: &str, user: &str) -> Result<ChatDetail, String> {
    let url = format!(
        "{API}/v1/projects/{project_id}/chats/{chat_id}?user={}",
        user_query(user)
    );
    get_json(&url).await
}

pub async fn save_chat(
    project_id: &str,
    chat_id: &str,
    user: &str,
    messages: &[StoredMsg],
) -> Result<ChatSummary, String> {
    reqwest::Client::new()
        .put(format!(
            "{API}/v1/projects/{project_id}/chats/{chat_id}/messages"
        ))
        .json(&serde_json::json!({ "user": user, "messages": messages }))
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
        let raw = reqwest::Client::new()
            .post(url)
            .header("content-type", "application/json")
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
    err.as_string()
        .unwrap_or_else(|| format!("{err:?}"))
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
    let rows: Vec<ApiProject> = reqwest::Client::new()
        .get(format!("{API}/v1/projects"))
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
    let _project: Project = reqwest::Client::new()
        .patch(format!("{API}/v1/projects/{id}"))
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
    let resp: Resp = reqwest::Client::new()
        .post(format!("{API}/v1/projects/{id}/ops"))
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
    let resp: Resp = reqwest::Client::new()
        .post(format!("{API}/v1/projects/{id}/captions"))
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
    reqwest::Client::new()
        .patch(format!("{API}/v1/projects/{project_id}/media/{media_id}"))
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
            {
                let id = value_to_id(row.id);
                let play = row
                    .play_url
                    .filter(|u| !u.is_empty() && !u.starts_with("blob:"))
                    .unwrap_or_else(|| media_file_url(project_id, &id));
                bind::media_from_api(
                    &id,
                    row.filename,
                    &row.content_type,
                    row.duration_ticks,
                    Some(play),
                )
            }
        })
        .collect())
}

async fn get_json<T: for<'de> Deserialize<'de>>(url: &str) -> Result<T, String> {
    reqwest::Client::new()
        .get(url)
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
    let res = reqwest::Client::new()
        .delete(format!("{API}/v1/projects/{id}"))
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
    let row: ApiProject = reqwest::Client::new()
        .post(format!("{API}/v1/projects"))
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

#[derive(Deserialize)]
struct UploadResponse {
    media_id: serde_json::Value,
    upload_url: Option<String>,
}

pub async fn upload_media(
    project_id: &str,
    filename: &str,
    content_type: &str,
    bytes: Vec<u8>,
) -> Result<String, String> {
    let client = reqwest::Client::new();
    let pending = client
        .post(format!("{API}/v1/projects/{project_id}/media/upload"))
        .json(&serde_json::json!({
            "filename": filename,
            "content_type": content_type,
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !pending.status().is_success() {
        let status = pending.status();
        let body = pending.text().await.unwrap_or_default();
        return Err(format!(
            "upload {status} for project {project_id}: {body}"
        ));
    }
    let res: UploadResponse = pending.json().await.map_err(|e| e.to_string())?;
    let media_id = value_to_id(res.media_id);
    if let Some(url) = res.upload_url {
        if client
            .put(&url)
            .header("content-type", content_type)
            .body(bytes.clone())
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .is_ok()
        {
            let _ = client
                .post(format!(
                    "{API}/v1/projects/{project_id}/media/{media_id}/complete"
                ))
                .send()
                .await;
            return Ok(media_id);
        }
    }
    put_media_bytes(project_id, &media_id, content_type, bytes).await?;
    Ok(media_id)
}

pub fn media_file_url(project_id: &str, media_id: &str) -> String {
    format!("{API}/v1/projects/{project_id}/media/{media_id}/file")
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
}
