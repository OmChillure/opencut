//! Silent clips from the signed-in grok subscription. No API key.

use serde_json::Value;

const SIGN_IN: &str = "Sign in with `grok`. B-roll and motion design use that subscription.";
const EXPIRED: &str = "Grok sign-in expired. Run `grok` and try again.";

/// One silent mp4. `requested` is seconds. Returns the file bytes and its duration.
pub async fn imagine_clip(
    prompt: &str,
    requested: u32,
    aspect: &str,
) -> Result<(Vec<u8>, f64), String> {
    let token = session_token().await?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(90))
        .build()
        .map_err(|e| e.to_string())?;
    let started = client
        .post("https://api.x.ai/v1/videos/generations")
        .bearer_auth(&token)
        .json(&serde_json::json!({
            "model": "grok-imagine-video-1.5",
            "prompt": prompt,
            "duration": requested,
            "aspect_ratio": aspect,
            "resolution": "480p",
            "generate_audio": false,
        }))
        .send()
        .await
        .map_err(|e| format!("video request failed: {e}"))?;
    if started.status().as_u16() == 401 {
        return Err(EXPIRED.into());
    }
    if !started.status().is_success() {
        let status = started.status();
        let body = started.text().await.unwrap_or_default();
        return Err(format!("video request {status}: {}", clip_body(&body)));
    }
    let started: Value = started.json().await.map_err(|e| e.to_string())?;
    let request_id = started
        .get("request_id")
        .and_then(Value::as_str)
        .ok_or_else(|| "video request returned no request_id".to_string())?
        .to_string();
    let mut video_url = None;
    let mut seconds = f64::from(requested);
    for _ in 0..60 {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        let polled = client
            .get(format!("https://api.x.ai/v1/videos/{request_id}"))
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| format!("video poll failed: {e}"))?;
        if polled.status().as_u16() == 401 {
            return Err(EXPIRED.into());
        }
        if !polled.status().is_success() {
            let status = polled.status();
            let body = polled.text().await.unwrap_or_default();
            return Err(format!("video poll {status}: {}", clip_body(&body)));
        }
        let body: Value = polled.json().await.map_err(|e| e.to_string())?;
        match body
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("pending")
        {
            "done" => {
                video_url = body
                    .pointer("/video/url")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                if let Some(dur) = body.pointer("/video/duration").and_then(Value::as_f64) {
                    if dur > 0.2 {
                        seconds = dur;
                    }
                }
                break;
            }
            "expired" => return Err("video request expired".into()),
            "failed" => {
                let detail = body
                    .get("error")
                    .or_else(|| body.get("message"))
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "failed".into());
                return Err(format!("video generation failed: {}", clip_body(&detail)));
            }
            _ => {}
        }
    }
    let video_url = video_url.ok_or_else(|| "video generation timed out".to_string())?;
    let bytes = client
        .get(&video_url)
        .send()
        .await
        .map_err(|e| format!("video download failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("video download failed: {e}"))?
        .bytes()
        .await
        .map_err(|e| format!("video download failed: {e}"))?;
    if bytes.len() < 32 {
        return Err("video download was empty".into());
    }
    Ok((bytes.to_vec(), seconds))
}

async fn session_token() -> Result<String, String> {
    if let Some(token) = crate::local_auth::grok_access_token() {
        return Ok(token);
    }
    if !crate::local_auth::grok_logged_in() {
        return Err(SIGN_IN.into());
    }
    let bin = std::env::var("OPENCUT_GROK_ACP").unwrap_or_else(|_| "grok".into());
    let status = tokio::process::Command::new(bin)
        .args(["models"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map_err(|e| format!("could not refresh the grok sign-in: {e}"))?;
    if !status.success() {
        return Err(EXPIRED.into());
    }
    crate::local_auth::grok_access_token().ok_or_else(|| EXPIRED.into())
}

fn clip_body(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.chars().count() <= 300 {
        return trimmed.to_string();
    }
    trimmed.chars().take(300).collect()
}
