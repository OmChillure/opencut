mod local;

use oc_time::Time;
use reqwest::multipart::{Form, Part};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use local::{LocalSttError, transcribe_local};

const DEFAULT_BASE: &str = "https://api.sarvam.ai";

#[derive(Debug, Error)]
pub enum SarvamError {
    #[error("missing SARVAM_API_KEY")]
    MissingKey,
    #[error("sarvam http {status}: {body}")]
    Http { status: u16, body: String },
    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),
    #[error("sarvam: {0}")]
    Message(String),
}

#[derive(Clone)]
pub struct Sarvam {
    http: reqwest::Client,
    api_key: String,
    base: String,
}

impl Sarvam {
    pub fn from_env() -> Result<Self, SarvamError> {
        let api_key = std::env::var("SARVAM_API_KEY")
            .ok()
            .filter(|s| !s.is_empty())
            .ok_or(SarvamError::MissingKey)?;
        let base = std::env::var("SARVAM_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE.to_string());
        Ok(Self::new(api_key, base))
    }

    #[must_use]
    pub fn new(api_key: impl Into<String>, base: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            api_key: api_key.into(),
            base: base.into().trim_end_matches('/').to_string(),
        }
    }

    /// REST STT. Files should be ≤ 30s. Use [`Self::start_batch_job`] for long cuts.
    pub async fn transcribe(
        &self,
        audio: Vec<u8>,
        filename: &str,
        language: Option<&str>,
    ) -> Result<Transcript, SarvamError> {
        let mut form = Form::new()
            .part(
                "file",
                Part::bytes(audio)
                    .file_name(filename.to_string())
                    .mime_str("application/octet-stream")
                    .map_err(|e| SarvamError::Message(e.to_string()))?,
            )
            .text("model", "saaras:v3")
            .text("mode", "verbatim")
            .text("with_timestamps", "true");
        if let Some(lang) = language {
            form = form.text("language_code", lang.to_string());
        }
        let res = self
            .http
            .post(format!("{}/speech-to-text", self.base))
            .header("api-subscription-key", &self.api_key)
            .multipart(form)
            .send()
            .await?;
        let status = res.status();
        let body = res.text().await?;
        if !status.is_success() {
            return Err(SarvamError::Http {
                status: status.as_u16(),
                body,
            });
        }
        let parsed: TranscribeResponse = serde_json::from_str(&body)
            .map_err(|e| SarvamError::Message(format!("decode: {e}; body={body}")))?;
        Ok(parsed.into_transcript())
    }

    pub async fn start_batch_job(
        &self,
        language: Option<&str>,
    ) -> Result<BatchJob, SarvamError> {
        #[derive(Serialize)]
        struct Body<'a> {
            model: &'a str,
            mode: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            language_code: Option<&'a str>,
            with_diarization: bool,
        }
        let res = self
            .http
            .post(format!("{}/speech-to-text/job/v1", self.base))
            .header("api-subscription-key", &self.api_key)
            .json(&Body {
                model: "saaras:v3",
                mode: "verbatim",
                language_code: language,
                with_diarization: true,
            })
            .send()
            .await?;
        parse_json(res).await
    }

    pub async fn synthesize(
        &self,
        text: &str,
        language_code: &str,
        speaker: Option<&str>,
    ) -> Result<Vec<u8>, SarvamError> {
        #[derive(Serialize)]
        struct Body<'a> {
            text: &'a str,
            target_language_code: &'a str,
            model: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            speaker: Option<&'a str>,
        }
        let res = self
            .http
            .post(format!("{}/text-to-speech", self.base))
            .header("api-subscription-key", &self.api_key)
            .json(&Body {
                text,
                target_language_code: language_code,
                model: "bulbul:v3",
                speaker,
            })
            .send()
            .await?;
        let status = res.status();
        let bytes = res.bytes().await?;
        if !status.is_success() {
            return Err(SarvamError::Http {
                status: status.as_u16(),
                body: String::from_utf8_lossy(&bytes).into_owned(),
            });
        }
        // API may return JSON { audios: ["base64..."] } or raw audio.
        if let Ok(json) = serde_json::from_slice::<TtsJson>(&bytes)
            && let Some(first) = json.audios.into_iter().next()
        {
            return decode_b64(&first);
        }
        Ok(bytes.to_vec())
    }
}

async fn parse_json<T: for<'de> Deserialize<'de>>(
    res: reqwest::Response,
) -> Result<T, SarvamError> {
    let status = res.status();
    let body = res.text().await?;
    if !status.is_success() {
        return Err(SarvamError::Http {
            status: status.as_u16(),
            body,
        });
    }
    serde_json::from_str(&body).map_err(|e| SarvamError::Message(format!("decode: {e}; body={body}")))
}

fn decode_b64(s: &str) -> Result<Vec<u8>, SarvamError> {
    use_base64(s).map_err(SarvamError::Message)
}

fn use_base64(s: &str) -> Result<Vec<u8>, String> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let s = s.trim().as_bytes();
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut buf = 0u32;
    let mut n = 0;
    for &c in s {
        if c == b'=' {
            break;
        }
        let Some(v) = val(c) else {
            continue;
        };
        buf = (buf << 6) | u32::from(v);
        n += 6;
        if n >= 8 {
            n -= 8;
            out.push((buf >> n) as u8);
        }
    }
    Ok(out)
}

#[derive(Debug, Deserialize)]
struct TtsJson {
    #[serde(default)]
    audios: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct TranscribeResponse {
    #[serde(default)]
    transcript: String,
    #[serde(default)]
    language_code: Option<String>,
    #[serde(default)]
    timestamps: Option<Timestamps>,
    #[serde(default)]
    diarized_transcript: Option<Diarized>,
}

#[derive(Debug, Deserialize)]
struct Timestamps {
    #[serde(default)]
    words: Vec<String>,
    #[serde(default)]
    start_time_seconds: Vec<f64>,
    #[serde(default)]
    end_time_seconds: Vec<f64>,
}

#[derive(Debug, Deserialize)]
struct Diarized {
    #[serde(default)]
    entries: Vec<DiarizedEntry>,
}

#[derive(Debug, Deserialize)]
struct DiarizedEntry {
    #[serde(default)]
    transcript: String,
    #[serde(default)]
    start_time_seconds: f64,
    #[serde(default)]
    end_time_seconds: f64,
    #[serde(default)]
    speaker_id: Option<String>,
}

impl TranscribeResponse {
    fn into_transcript(self) -> Transcript {
        let mut cues = Vec::new();
        if let Some(d) = self.diarized_transcript {
            for e in d.entries {
                cues.push(Cue {
                    start: Time::from_seconds(e.start_time_seconds),
                    end: Time::from_seconds(e.end_time_seconds),
                    text: e.transcript,
                    speaker: e.speaker_id,
                });
            }
        } else if let Some(ts) = self.timestamps {
            for i in 0..ts.words.len() {
                let start = ts.start_time_seconds.get(i).copied().unwrap_or(0.0);
                let end = ts.end_time_seconds.get(i).copied().unwrap_or(start);
                cues.push(Cue {
                    start: Time::from_seconds(start),
                    end: Time::from_seconds(end),
                    text: ts.words[i].clone(),
                    speaker: None,
                });
            }
        }
        if cues.is_empty() && !self.transcript.is_empty() {
            cues.push(Cue {
                start: Time::ZERO,
                end: Time::from_seconds(1.0),
                text: self.transcript.clone(),
                speaker: None,
            });
        }
        Transcript {
            language: self.language_code,
            full_text: self.transcript,
            cues,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Transcript {
    pub language: Option<String>,
    pub full_text: String,
    pub cues: Vec<Cue>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Cue {
    pub start: Time,
    pub end: Time,
    pub text: String,
    pub speaker: Option<String>,
}

impl Cue {
    #[must_use]
    pub fn into_timeline(self) -> oc_timeline::CaptionCue {
        oc_timeline::CaptionCue {
            start: self.start,
            end: self.end,
            text: self.text,
            speaker: self.speaker,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct BatchJob {
    #[serde(default)]
    pub job_id: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
}

impl BatchJob {
    #[must_use]
    pub fn id(&self) -> Option<&str> {
        self.job_id.as_deref().or(self.id.as_deref())
    }
}
