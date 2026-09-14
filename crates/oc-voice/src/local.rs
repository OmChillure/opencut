//! Free local STT: ffmpeg extracts audio, Whisper turns it into text.
//! No API key. ffmpeg is required; Whisper is any of:
//! `whisper` (openai-whisper), `whisper-cli` / `whisper.cpp`.

use crate::{Cue, Transcript};
use oc_time::Time;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use thiserror::Error;
use tokio::process::Command;

#[derive(Debug, Error)]
pub enum LocalSttError {
    #[error("ffmpeg not found — install ffmpeg (free) to extract audio")]
    NoFfmpeg,
    #[error(
        "no local Whisper found. Install one (free, runs on this machine):\n  \
         pip install -U openai-whisper\n  \
         or build whisper.cpp and put `whisper-cli` on PATH"
    )]
    NoWhisper,
    #[error("ffmpeg failed: {0}")]
    Ffmpeg(String),
    #[error("whisper failed: {0}")]
    Whisper(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Transcribe media bytes on this machine. Pulls audio with ffmpeg, then Whisper.
pub async fn transcribe_local(bytes: &[u8], filename: &str) -> Result<Transcript, LocalSttError> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("oc-stt-{stamp}"));
    tokio::fs::create_dir_all(&dir).await?;
    let ext = Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .filter(|e| e.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or("bin");
    let input = dir.join(format!("in.{ext}"));
    let wav = dir.join("audio.wav");
    tokio::fs::write(&input, bytes).await?;

    extract_wav(&input, &wav).await?;
    let transcript = run_whisper(&wav, &dir).await?;
    let _ = tokio::fs::remove_dir_all(&dir).await;
    Ok(transcript)
}

async fn extract_wav(input: &Path, wav: &Path) -> Result<(), LocalSttError> {
    let ffmpeg = which("ffmpeg").ok_or(LocalSttError::NoFfmpeg)?;
    let out = Command::new(ffmpeg)
        .args([
            "-y",
            "-i",
            &path_str(input),
            "-vn",
            "-ac",
            "1",
            "-ar",
            "16000",
            "-c:a",
            "pcm_s16le",
        ])
        .arg(wav)
        .output()
        .await?;
    if !out.status.success() {
        return Err(LocalSttError::Ffmpeg(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    if !wav.exists() {
        return Err(LocalSttError::Ffmpeg("no wav written".into()));
    }
    Ok(())
}

async fn run_whisper(wav: &Path, dir: &Path) -> Result<Transcript, LocalSttError> {
    if let Some(bin) = which("whisper") {
        return whisper_openai(&bin, wav, dir).await;
    }
    if let Some(bin) = which("whisper-cli").or_else(|| which("whisper.cpp")) {
        return whisper_cpp(&bin, wav, dir).await;
    }
    Err(LocalSttError::NoWhisper)
}

async fn whisper_openai(bin: &Path, wav: &Path, dir: &Path) -> Result<Transcript, LocalSttError> {
    let model = std::env::var("WHISPER_MODEL").unwrap_or_else(|_| "tiny".into());
    let out = Command::new(bin)
        .args([
            &path_str(wav),
            "--model",
            &model,
            "--output_format",
            "json",
            "--output_dir",
            &path_str(dir),
            "--fp16",
            "False",
            "--verbose",
            "False",
        ])
        .output()
        .await?;
    if !out.status.success() {
        return Err(LocalSttError::Whisper(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    let json_path = dir.join("audio.json");
    let raw = tokio::fs::read_to_string(&json_path).await.map_err(|_| {
        LocalSttError::Whisper(format!("expected {}", json_path.display()))
    })?;
    parse_openai_whisper(&raw).ok_or_else(|| LocalSttError::Whisper("bad whisper json".into()))
}

async fn whisper_cpp(bin: &Path, wav: &Path, dir: &Path) -> Result<Transcript, LocalSttError> {
    let prefix = dir.join("out");
    let mut cmd = Command::new(bin);
    cmd.args(["-f", &path_str(wav), "-oj", "-of", &path_str(&prefix)]);
    if let Ok(model) = std::env::var("WHISPER_CPP_MODEL") {
        cmd.args(["-m", &model]);
    }
    let out = cmd.output().await?;
    if !out.status.success() {
        return Err(LocalSttError::Whisper(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    let json_path = PathBuf::from(format!("{}.json", prefix.display()));
    let raw = tokio::fs::read_to_string(&json_path).await.map_err(|_| {
        LocalSttError::Whisper(format!("expected {}", json_path.display()))
    })?;
    parse_whisper_cpp(&raw).ok_or_else(|| LocalSttError::Whisper("bad whisper.cpp json".into()))
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn path_str(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[derive(Deserialize)]
struct OpenAiJson {
    #[serde(default)]
    text: String,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    segments: Vec<OpenAiSeg>,
}

#[derive(Deserialize)]
struct OpenAiSeg {
    #[serde(default)]
    start: f64,
    #[serde(default)]
    end: f64,
    #[serde(default)]
    text: String,
}

#[derive(Deserialize)]
struct CppJson {
    #[serde(default)]
    transcription: Vec<CppSeg>,
}

#[derive(Deserialize)]
struct CppSeg {
    #[serde(default)]
    text: String,
    #[serde(default)]
    offsets: Option<CppOffsets>,
}

#[derive(Deserialize)]
struct CppOffsets {
    #[serde(default)]
    from: u64,
    #[serde(default)]
    to: u64,
}

pub(crate) fn parse_openai_whisper(raw: &str) -> Option<Transcript> {
    let parsed: OpenAiJson = serde_json::from_str(raw).ok()?;
    let cues: Vec<Cue> = parsed
        .segments
        .into_iter()
        .filter(|s| !s.text.trim().is_empty())
        .map(|s| Cue {
            start: Time::from_seconds(s.start),
            end: Time::from_seconds(s.end.max(s.start)),
            text: s.text.trim().to_string(),
            speaker: None,
        })
        .collect();
    let full_text = if parsed.text.trim().is_empty() {
        cues.iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        parsed.text.trim().to_string()
    };
    Some(Transcript {
        language: parsed.language,
        full_text,
        cues,
    })
}

pub(crate) fn parse_whisper_cpp(raw: &str) -> Option<Transcript> {
    let parsed: CppJson = serde_json::from_str(raw).ok()?;
    let cues: Vec<Cue> = parsed
        .transcription
        .into_iter()
        .filter(|s| !s.text.trim().is_empty())
        .map(|s| {
            let (start, end) = s
                .offsets
                .map(|o| (o.from as f64 / 1000.0, o.to as f64 / 1000.0))
                .unwrap_or((0.0, 0.0));
            Cue {
                start: Time::from_seconds(start),
                end: Time::from_seconds(end.max(start)),
                text: s.text.trim().to_string(),
                speaker: None,
            }
        })
        .collect();
    let full_text = cues
        .iter()
        .map(|c| c.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    Some(Transcript {
        language: None,
        full_text,
        cues,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_openai_segments() {
        let raw = r#"{
            "text": "hello world",
            "language": "en",
            "segments": [
                {"start": 0.0, "end": 1.2, "text": " hello"},
                {"start": 1.2, "end": 2.0, "text": " world"}
            ]
        }"#;
        let t = parse_openai_whisper(raw).unwrap();
        assert_eq!(t.cues.len(), 2);
        assert_eq!(t.cues[0].text, "hello");
        assert!((t.cues[1].start.as_seconds() - 1.2).abs() < 1e-6);
    }

    #[test]
    fn parses_whisper_cpp() {
        let raw = r#"{
            "transcription": [
                {"text": " hi there", "offsets": {"from": 0, "to": 1500}}
            ]
        }"#;
        let t = parse_whisper_cpp(raw).unwrap();
        assert_eq!(t.cues[0].text, "hi there");
        assert!((t.cues[0].end.as_seconds() - 1.5).abs() < 1e-6);
    }
}