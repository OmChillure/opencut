//! Grok Speech-to-Text (xAI). $0.10/hour batch. Optional — needs XAI_API_KEY.

use crate::{Cue, Transcript};
use oc_time::Time;
use reqwest::multipart::{Form, Part};
use serde::Deserialize;
use std::path::Path;
use std::time::Instant;

const DEFAULT_BASE: &str = "https://api.x.ai/v1";

#[derive(Debug, Deserialize)]
struct SttResponse {
    #[serde(default)]
    text: String,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    words: Vec<SttWord>,
}

#[derive(Debug, Deserialize)]
struct SttWord {
    #[serde(default)]
    text: String,
    #[serde(default)]
    start: f64,
    #[serde(default)]
    end: f64,
    #[serde(default)]
    speaker: Option<i32>,
}

pub fn configured() -> bool {
    std::env::var("XAI_API_KEY")
        .ok()
        .is_some_and(|k| !k.trim().is_empty())
}

pub async fn transcribe_wav(wav: &Path) -> Result<Transcript, String> {
    let key = std::env::var("XAI_API_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty())
        .ok_or_else(|| "XAI_API_KEY not set".to_string())?;
    let base = std::env::var("XAI_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE.into());
    let bytes = tokio::fs::read(wav)
        .await
        .map_err(|e| format!("read wav: {e}"))?;
    let t0 = Instant::now();
    tracing::info!(bytes = bytes.len(), "grok stt upload");

    let mut form = Form::new();
    if let Ok(lang) = std::env::var("OPENCUT_STT_LANG") {
        let lang = lang.trim().to_string();
        if !lang.is_empty() {
            form = form.text("language", lang).text("format", "true");
        }
    }
    form = form.part(
        "file",
        Part::bytes(bytes)
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .map_err(|e| e.to_string())?,
    );

    let url = format!("{}/stt", base.trim_end_matches('/'));
    let res = reqwest::Client::new()
        .post(url)
        .bearer_auth(key.trim())
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("grok stt http: {e}"))?;
    let status = res.status();
    let body = res.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("grok stt {status}: {}", body.chars().take(400).collect::<String>()));
    }
    let parsed: SttResponse =
        serde_json::from_str(&body).map_err(|e| format!("grok stt decode: {e}; body={body}"))?;
    let t = into_transcript(parsed);
    tracing::info!(
        ms = t0.elapsed().as_millis(),
        words = t.full_text.split_whitespace().count(),
        cues = t.cues.len(),
        "grok stt done"
    );
    Ok(t)
}

fn into_transcript(resp: SttResponse) -> Transcript {
    let cues = words_to_cues(&resp.words);
    let full_text = if resp.text.trim().is_empty() {
        cues.iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        resp.text.trim().to_string()
    };
    Transcript {
        language: resp.language,
        full_text,
        cues,
    }
}

fn words_to_cues(words: &[SttWord]) -> Vec<Cue> {
    let mut cues = Vec::new();
    let mut buf: Vec<&SttWord> = Vec::new();
    for w in words {
        if w.text.trim().is_empty() {
            continue;
        }
        if let Some(prev) = buf.last() {
            let gap = w.start - prev.end;
            let speaker_changed = w.speaker != prev.speaker;
            let long = w.end - buf[0].start >= 2.4;
            let punct = prev.text.ends_with(['.', '?', '!']);
            if speaker_changed || gap > 0.45 || (long && punct) {
                cues.push(flush(&buf));
                buf.clear();
            }
        }
        buf.push(w);
    }
    if !buf.is_empty() {
        cues.push(flush(&buf));
    }
    cues
}

fn flush(words: &[&SttWord]) -> Cue {
    let start = words.first().map(|w| w.start).unwrap_or(0.0);
    let end = words.last().map(|w| w.end).unwrap_or(start);
    let speaker = words.first().and_then(|w| w.speaker).map(|s| format!("S{s}"));
    let text = words
        .iter()
        .map(|w| w.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    Cue {
        start: Time::from_seconds(start),
        end: Time::from_seconds(end.max(start)),
        text,
        speaker,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_words_on_gap() {
        let words = vec![
            SttWord {
                text: "Hello".into(),
                start: 0.0,
                end: 0.4,
                speaker: Some(0),
            },
            SttWord {
                text: "there.".into(),
                start: 0.4,
                end: 0.8,
                speaker: Some(0),
            },
            SttWord {
                text: "Next".into(),
                start: 2.0,
                end: 2.3,
                speaker: Some(0),
            },
        ];
        let cues = words_to_cues(&words);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].text, "Hello there.");
        assert_eq!(cues[1].text, "Next");
    }
}
