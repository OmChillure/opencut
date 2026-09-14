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

#[derive(Deserialize)]
pub struct ChatReply {
    pub text: String,
    #[serde(default)]
    pub notes: Vec<String>,
    pub timeline: Timeline,
}

pub async fn chat(
    project_id: &str,
    provider: &str,
    model: &str,
    messages: &[(bool, String)],
) -> Result<ChatReply, String> {
    let body = serde_json::json!({
        "provider": provider,
        "model": model,
        "messages": messages.iter().map(|(user, text)| serde_json::json!({
            "role": if *user { "user" } else { "assistant" },
            "content": text,
        })).collect::<Vec<_>>(),
    });
    reqwest::Client::new()
        .post(format!("{API}/v1/projects/{project_id}/chat"))
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())
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
