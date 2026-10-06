//! ffmpeg extracts audio. Groq Whisper writes the transcript.

use crate::Transcript;
use crate::groq_stt;
use crate::punctuate::restore_punctuation;
use std::path::Path;
use std::time::Instant;
use thiserror::Error;
use tokio::process::Command;

#[derive(Debug, Error)]
pub enum SttError {
    #[error("ffmpeg not found — install ffmpeg (free) to extract audio")]
    NoFfmpeg,
    #[error("ffmpeg failed: {0}")]
    Ffmpeg(String),
    #[error("groq whisper failed: {0}")]
    Groq(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Transcribe a file already on disk. Does not delete `input`.
pub async fn transcribe_path(input: &Path) -> Result<Transcript, SttError> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("oc-stt-{stamp}"));
    tokio::fs::create_dir_all(&dir).await?;
    let wav = dir.join("audio.wav");
    tracing::info!(file = %input.display(), "stt extract audio");
    let t0 = Instant::now();
    let extracted = extract_wav(input, &wav).await;
    if let Err(err) = extracted {
        let _ = tokio::fs::remove_dir_all(&dir).await;
        return Err(err);
    }
    tracing::info!(ms = t0.elapsed().as_millis(), "stt wav ready");
    let t1 = Instant::now();
    let transcript = match groq_stt::transcribe_wav(&wav).await {
        Ok(t) => {
            tracing::info!("stt via groq whisper");
            Ok(t)
        }
        Err(err) => Err(SttError::Groq(err)),
    };
    let _ = tokio::fs::remove_dir_all(&dir).await;
    let transcript = punctuate_transcript(transcript?);
    tracing::info!(
        ms = t1.elapsed().as_millis(),
        words = transcript.full_text.split_whitespace().count(),
        cues = transcript.cues.len(),
        "stt done"
    );
    Ok(transcript)
}

pub(crate) fn punctuate_transcript(mut t: Transcript) -> Transcript {
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

async fn extract_wav(input: &Path, wav: &Path) -> Result<(), SttError> {
    let ffmpeg = which("ffmpeg").ok_or(SttError::NoFfmpeg)?;
    let out = Command::new(ffmpeg)
        .args([
            "-y",
            "-i",
            &input.to_string_lossy(),
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
        return Err(SttError::Ffmpeg(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    if !wav.exists() {
        return Err(SttError::Ffmpeg("no wav written".into()));
    }
    Ok(())
}

fn which(name: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}
