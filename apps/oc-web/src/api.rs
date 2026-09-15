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
    reqwest::Client::new()
        .put(format!("{API}/v1/projects/{project_id}/media/{media_id}/bytes"))
        .header("content-type", content_type)
        .body(bytes)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn chat(
    project_id: &str,
    provider: &str,
    model: &str,
    messages: &[(bool, String)],
) -> Result<ChatReply, String> {
    chat_stream(project_id, provider, model, messages, |_| {}).await
}

pub async fn chat_stream(
    project_id: &str,
    provider: &str,
    model: &str,
    messages: &[(bool, String)],
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
    read_ndjson(&url, &body.to_string(), |line| {
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

async fn read_ndjson(
    url: &str,
    json_body: &str,
    mut on_line: impl FnMut(&str) -> Result<(), String>,
) -> Result<(), String> {
    #[cfg(target_arch = "wasm32")]
    {
        wasm_read_ndjson(url, json_body, on_line).await
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
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
    mut on_line: impl FnMut(&str) -> Result<(), String>,
) -> Result<(), String> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{ReadableStreamDefaultReader, Request, RequestInit, RequestMode, Response};

    let opts = RequestInit::new();
    opts.set_method("POST");
    opts.set_mode(RequestMode::Cors);
    opts.set_body(&wasm_bindgen::JsValue::from_str(json_body));
    let request = Request::new_with_str_and_init(url, &opts).map_err(js_err)?;
    request
        .headers()
        .set("content-type", "application/json")
        .map_err(js_err)?;
    let window = web_sys::window().ok_or_else(|| "no window".to_string())?;
    let resp = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(js_err)?;
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
        let next = JsFuture::from(reader.read()).await.map_err(js_err)?;
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
            bind::media_from_api(
                &value_to_id(row.id),
                row.filename,
                &row.content_type,
                row.duration_ticks,
                row.play_url,
            )
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
    let res: UploadResponse = client
        .post(format!("{API}/v1/projects/{project_id}/media/upload"))
        .json(&serde_json::json!({
            "filename": filename,
            "content_type": content_type,
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let media_id = value_to_id(res.media_id);
    if let Some(url) = res.upload_url {
        client
            .put(url)
            .header("content-type", content_type)
            .body(bytes)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?;
        client
            .post(format!(
                "{API}/v1/projects/{project_id}/media/{media_id}/complete"
            ))
            .send()
            .await
            .ok();
    }
    Ok(media_id)
}
