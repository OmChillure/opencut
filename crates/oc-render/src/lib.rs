//! Bake an OpenCut timeline to a file with ffmpeg.
//!
//! `compile` builds the filter graph (testable without encoding).
//! `render` runs ffmpeg and writes an MP4.

mod captions;
mod graph;

pub use captions::{captions_for_cut, to_srt, BurnedCue};
pub use graph::{compile, Compiled};

use oc_timeline::{MediaId, Timeline};
use oc_tools::ExportPreset;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Debug)]
pub struct MediaSource {
    pub id: MediaId,
    pub path: PathBuf,
    pub has_video: bool,
    pub has_audio: bool,
}

#[derive(Clone, Debug)]
pub struct RenderRequest {
    pub timeline: Timeline,
    pub media: HashMap<MediaId, MediaSource>,
    pub output: PathBuf,
    pub preset: ExportPreset,
}

#[derive(Clone, Debug)]
pub struct RenderResult {
    pub output: PathBuf,
    pub width: u32,
    pub height: u32,
    pub filter: String,
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("nothing to render")]
    Empty,
    #[error("ffmpeg is not on PATH")]
    NoFfmpeg,
    #[error("unknown media {0}")]
    UnknownMedia(MediaId),
    #[error("missing media file {0}")]
    MissingMedia(String),
    #[error("ffmpeg failed: {0}")]
    Ffmpeg(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[must_use]
pub fn still_input(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .as_deref(),
        Some("png" | "jpg" | "jpeg" | "webp" | "gif")
    )
}

#[must_use]
pub fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[must_use]
pub fn font_path() -> Option<String> {
    const CANDIDATES: &[&str] = &[
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
    ];
    CANDIDATES
        .iter()
        .map(PathBuf::from)
        .find(|p| p.is_file())
        .map(|p| p.to_string_lossy().into_owned())
}

pub fn render(req: &RenderRequest) -> Result<RenderResult, RenderError> {
    if !ffmpeg_available() {
        return Err(RenderError::NoFfmpeg);
    }
    let parent = req
        .output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let work = parent.join(format!(
        ".oc-render-{}",
        req.output
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("out")
    ));
    std::fs::create_dir_all(&work)?;
    let compiled = compile(&req.timeline, &req.media, req.preset, &work)?;
    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-y").arg("-hide_banner").arg("-loglevel").arg("error");
    for input in &compiled.inputs {
        if still_input(input) {
            cmd.arg("-loop")
                .arg("1")
                .arg("-framerate")
                .arg(format!("{:.3}", compiled.fps));
        }
        cmd.arg("-i").arg(input);
    }
    cmd.arg("-filter_complex")
        .arg(&compiled.filter)
        .arg("-map")
        .arg(format!("[{}]", compiled.video_label));
    if let Some(a) = &compiled.audio_label {
        cmd.arg("-map").arg(format!("[{a}]"));
        cmd.arg("-c:a").arg("aac").arg("-b:a").arg("192k").arg("-shortest");
    }
    cmd.arg("-c:v")
        .arg("libx264")
        .arg("-pix_fmt")
        .arg("yuv420p")
        .arg("-preset")
        .arg("veryfast")
        .arg("-crf")
        .arg("20")
        .arg("-movflags")
        .arg("+faststart")
        .arg(&req.output);
    let out = cmd.output()?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(RenderError::Ffmpeg(stderr.chars().take(2000).collect()));
    }
    let _ = std::fs::remove_dir_all(&work);
    Ok(RenderResult {
        output: req.output.clone(),
        width: compiled.width,
        height: compiled.height,
        filter: compiled.filter,
    })
}
