//! Free local STT: ffmpeg extracts audio, Whisper turns it into text.
//! No API key. ffmpeg is required; Whisper is any of:
//! `whisper` (openai-whisper), `whisper-cli` / `whisper.cpp`.

use crate::punctuate::restore_punctuation;
use crate::{Cue, Transcript};
use oc_time::Time;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Instant;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::Mutex;
use tokio::time::{Duration, timeout};

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
    tokio::fs::write(&input, bytes).await?;
    let result = transcribe_path(&input).await;
    let _ = tokio::fs::remove_dir_all(&dir).await;
    result
}

/// Transcribe a file already on disk. Does not delete `input`.
pub async fn transcribe_path(input: &Path) -> Result<Transcript, LocalSttError> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("oc-stt-{stamp}"));
    tokio::fs::create_dir_all(&dir).await?;
    let wav = dir.join("audio.wav");
    tracing::info!(file = %input.display(), "stt extract audio");
    let t0 = Instant::now();
    extract_wav(input, &wav).await?;
    tracing::info!(ms = t0.elapsed().as_millis(), "stt wav ready");
    let t1 = Instant::now();
    let transcript = transcribe_cloud_or_local(&wav, &dir).await?;
    let transcript = punctuate_transcript(transcript);
    tracing::info!(
        ms = t1.elapsed().as_millis(),
        words = transcript.full_text.split_whitespace().count(),
        cues = transcript.cues.len(),
        "stt whisper done"
    );
    let _ = tokio::fs::remove_dir_all(&dir).await;
    Ok(transcript)
}

async fn transcribe_cloud_or_local(wav: &Path, dir: &Path) -> Result<Transcript, LocalSttError> {
    if crate::groq_stt::configured() {
        match crate::groq_stt::transcribe_wav(wav).await {
            Ok(t) => {
                tracing::info!("stt via groq whisper (free tier)");
                return Ok(t);
            }
            Err(e) => tracing::warn!("groq stt failed: {e}"),
        }
    }
    if crate::grok_stt::configured() {
        match crate::grok_stt::transcribe_wav(wav).await {
            Ok(t) => {
                tracing::info!("stt via grok ($0.10/hour)");
                return Ok(t);
            }
            Err(e) => tracing::warn!("grok stt failed: {e}"),
        }
    }
    run_whisper(wav, dir).await
}

fn punctuate_transcript(mut t: Transcript) -> Transcript {
    t.cues = t
        .cues
        .into_iter()
        .map(|mut c| {
            c.text = restore_punctuation(&c.text);
            c
        })
        .collect();
    t.full_text = restore_punctuation(&t.full_text);
    t
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
    if let Ok(Some(t)) = whisper_faster(wav).await {
        return Ok(t);
    }
    if let Some(bin) = which("whisper") {
        return whisper_openai(&bin, wav, dir).await;
    }
    if let Some(bin) = which("whisper-cli").or_else(|| which("whisper.cpp")) {
        return whisper_cpp(&bin, wav, dir).await;
    }
    Err(LocalSttError::NoWhisper)
}

fn whisper_timeout() -> Duration {
    let secs = std::env::var("WHISPER_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(180u64)
        .max(15);
    Duration::from_secs(secs)
}

async fn whisper_faster(wav: &Path) -> Result<Option<Transcript>, LocalSttError> {
    match timeout(whisper_timeout(), whisper_faster_inner(wav)).await {
        Ok(ok) => ok,
        Err(_) => {
            tracing::error!(
                secs = whisper_timeout().as_secs(),
                "whisper timed out — skip this clip"
            );
            reset_daemon().await;
            Err(LocalSttError::Whisper(format!(
                "timed out after {}s",
                whisper_timeout().as_secs()
            )))
        }
    }
}

async fn whisper_faster_inner(wav: &Path) -> Result<Option<Transcript>, LocalSttError> {
    if let Some(t) = daemon_transcribe(wav).await? {
        return Ok(Some(t));
    }
    let Ok(script) = whisper_script() else {
        return Ok(None);
    };
    let Ok(py) = whisper_python() else {
        return Ok(None);
    };
    tracing::warn!("whisper daemon unavailable — one-shot (slow, reloads model)");
    let out = Command::new(py).arg(&script).arg(wav).output().await?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        tracing::error!(stderr = %err.trim(), "whisper one-shot failed");
        return Ok(None);
    }
    let raw = String::from_utf8_lossy(&out.stdout);
    Ok(parse_openai_whisper(&raw))
}

struct WhisperDaemon {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<tokio::process::ChildStdout>,
}

static DAEMON: OnceLock<Mutex<Option<WhisperDaemon>>> = OnceLock::new();

fn daemon_lock() -> &'static Mutex<Option<WhisperDaemon>> {
    DAEMON.get_or_init(|| Mutex::new(None))
}

async fn reset_daemon() {
    let mut slot = daemon_lock().lock().await;
    if let Some(mut d) = slot.take() {
        let _ = d.child.start_kill();
    }
}

async fn daemon_transcribe(wav: &Path) -> Result<Option<Transcript>, LocalSttError> {
    let mut slot = daemon_lock().lock().await;
    if slot.is_none() {
        *slot = spawn_daemon().await?;
    }
    let Some(d) = slot.as_mut() else {
        return Ok(None);
    };
    let line = format!("{}\n", path_str(wav));
    if d.stdin.write_all(line.as_bytes()).await.is_err() || d.stdin.flush().await.is_err() {
        tracing::warn!("whisper daemon stdin closed — restart");
        let _ = d.child.start_kill();
        *slot = None;
        return Ok(None);
    }
    let mut reply = String::new();
    match d.stdout.read_line(&mut reply).await {
        Ok(0) => {
            tracing::warn!("whisper daemon stdout closed — restart");
            let _ = d.child.start_kill();
            *slot = None;
            Ok(None)
        }
        Ok(_) => {
            if let Some(err) = serde_json::from_str::<serde_json::Value>(&reply)
                .ok()
                .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
            {
                return Err(LocalSttError::Whisper(err));
            }
            Ok(parse_openai_whisper(&reply))
        }
        Err(e) => Err(LocalSttError::Whisper(e.to_string())),
    }
}

async fn spawn_daemon() -> Result<Option<WhisperDaemon>, LocalSttError> {
    let Ok(script) = whisper_script() else {
        return Ok(None);
    };
    let Ok(py) = whisper_python() else {
        return Ok(None);
    };
    tracing::info!(script = %script.display(), py = %py.display(), "start whisper daemon");
    let mut child = Command::new(py)
        .arg(&script)
        .arg("--serve")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let stdin = child.stdin.take().ok_or(LocalSttError::NoWhisper)?;
    let stdout = BufReader::new(child.stdout.take().ok_or(LocalSttError::NoWhisper)?);
    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if !line.trim().is_empty() {
                    tracing::info!(whisper = %line, "whisper");
                }
            }
        });
    }
    Ok(Some(WhisperDaemon {
        child,
        stdin,
        stdout,
    }))
}

fn whisper_script() -> Result<PathBuf, LocalSttError> {
    if let Ok(p) = std::env::var("WHISPER_SCRIPT") {
        let path = PathBuf::from(p);
        if path.is_file() {
            return Ok(path);
        }
    }
    let candidates = [
        PathBuf::from("scripts/faster_whisper_transcribe.py"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/faster_whisper_transcribe.py"),
    ];
    for p in candidates {
        if p.is_file() {
            return Ok(p);
        }
    }
    Err(LocalSttError::NoWhisper)
}

fn whisper_python() -> Result<PathBuf, LocalSttError> {
    if let Ok(p) = std::env::var("WHISPER_PYTHON") {
        let path = PathBuf::from(p);
        if path.is_file() {
            return Ok(path);
        }
    }
    for name in [
        "/tmp/oc-whisper-venv/bin/python",
        "python3",
        "python",
    ] {
        if let Some(p) = which(name) {
            return Ok(p);
        }
        let path = PathBuf::from(name);
        if path.is_file() {
            return Ok(path);
        }
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

    #[test]
    fn punctuates_after_parse() {
        let raw = r#"{
            "text": "and so my fellow Americans, ask not what your country can do for you, ask what you can do for your country",
            "language": "en",
            "segments": []
        }"#;
        let t = punctuate_transcript(parse_openai_whisper(raw).unwrap());
        assert!(t.full_text.contains("And so,"), "{}", t.full_text);
        assert!(t.full_text.contains('—'), "{}", t.full_text);
        assert!(t.full_text.ends_with('.'), "{}", t.full_text);
    }
}