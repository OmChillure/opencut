mod shots;
mod subject;
mod vision;

pub use shots::{ShotBrief, ShotRole, brief_shots, format_shot_list};
pub use vision::{ShotLook, VisualDigest, analyze_local, analyze_path, picture_ranges};

use oc_time::{Duration, FrameRate};
use oc_timeline::{MediaId, ProjectId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaStatus {
    Uploading,
    Ready,
    Transcribing,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MediaProbe {
    pub duration: Duration,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub frame_rate: Option<FrameRate>,
    pub has_video: bool,
    pub has_audio: bool,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectKind {
    Raw,
    Proxy,
    Audio,
    Export,
}

impl ObjectKind {
    #[must_use]
    pub fn prefix(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::Proxy => "proxy",
            Self::Audio => "audio",
            Self::Export => "exports",
        }
    }
}

#[must_use]
pub fn object_key(
    kind: ObjectKind,
    project: ProjectId,
    media: MediaId,
    filename: &str,
) -> String {
    let safe: String = filename
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{}/{project}/{media}/{safe}", kind.prefix())
}

#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("ffprobe failed: {0}")]
    Probe(String),
    #[error("ffmpeg failed: {0}")]
    Ffmpeg(String),
}
