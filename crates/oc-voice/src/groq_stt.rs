//! Groq Whisper — free tier, no credit card.
//! ~8 hours of audio/day (`whisper-large-v3-turbo`).

use crate::Transcript;
use crate::parse::parse_openai_whisper;
use reqwest::multipart::{Form, Part};
use std::path::Path;
use std::time::Instant;

const DEFAULT_BASE: &str = "https://api.groq.com/openai/v1";
const MODEL: &str = "whisper-large-v3-turbo";

pub fn configured() -> bool {
    std::env::var("GROQ_API_KEY")
        .ok()
        .is_some_and(|k| !k.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_follows_the_env_key() {
        let set = std::env::var("GROQ_API_KEY")
            .ok()
            .is_some_and(|key| !key.trim().is_empty());
        assert_eq!(configured(), set);
    }
}

pub async fn transcribe_wav(wav: &Path) -> Result<Transcript, String> {
    let key = std::env::var("GROQ_API_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty())
        .ok_or_else(|| "GROQ_API_KEY not set".to_string())?;
    let base = std::env::var("GROQ_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE.into());
    let bytes = tokio::fs::read(wav)
        .await
        .map_err(|e| format!("read wav: {e}"))?;
    let t0 = Instant::now();
    tracing::info!(bytes = bytes.len(), model = MODEL, "groq whisper upload");

    let mut form = Form::new()
        .text("model", MODEL)
        .text("response_format", "verbose_json")
        .text("temperature", "0");
    if let Ok(lang) = std::env::var("OPENCUT_STT_LANG") {
        let lang = lang.trim().to_string();
        if !lang.is_empty() {
            form = form.text("language", lang);
        }
    }
    form = form.part(
        "file",
        Part::bytes(bytes)
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .map_err(|e| e.to_string())?,
    );

    let url = format!("{}/audio/transcriptions", base.trim_end_matches('/'));
    let res = reqwest::Client::new()
        .post(url)
        .bearer_auth(key.trim())
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("groq stt http: {e}"))?;
    let status = res.status();
    let body = res.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!(
            "groq stt {status}: {}",
            body.chars().take(400).collect::<String>()
        ));
    }
    let t = parse_openai_whisper(&body).ok_or_else(|| format!("groq stt decode: {body}"))?;
    tracing::info!(
        ms = t0.elapsed().as_millis(),
        words = t.full_text.split_whitespace().count(),
        cues = t.cues.len(),
        "groq whisper done (free)"
    );
    Ok(t)
}
