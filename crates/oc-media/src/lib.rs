mod beats;
mod label;
mod shots;
mod vision;

pub use beats::{MusicAnalysis, MusicSection, detect_beats, format_music};
pub use label::{
    ShotStill, apply_shot_reply, grade_from_value, grade_prompt, grades_from_reply,
    shot_label_prompt, shot_stills, watched_grade,
};
pub use shots::{ShotBrief, ShotRole, brief_shots, format_shot_list};
pub use vision::{
    ShotCard, ShotLook, VisualDigest, analyze_local, analyze_path, grab_jpeg, picture_ranges,
};

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
pub fn object_key(kind: ObjectKind, project: ProjectId, media: MediaId, filename: &str) -> String {
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

/// The export object the worker uploads: `exports/{project}/{project}/{file}`.
#[must_use]
pub fn export_object_key(project: ProjectId, filename: &str) -> String {
    object_key(
        ObjectKind::Export,
        project,
        MediaId::from_uuid(project.as_uuid()),
        filename,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_key_uses_the_kind_prefix_and_strips_unsafe_characters() {
        let project = ProjectId::new();
        let media = MediaId::new();
        let key = object_key(ObjectKind::Raw, project, media, "my clip (1).mp4");
        assert_eq!(key, format!("raw/{project}/{media}/my_clip__1_.mp4"));
        assert!(object_key(ObjectKind::Export, project, media, "out.mp4").starts_with("exports/"));
        assert!(object_key(ObjectKind::Audio, project, media, "a.wav").ends_with("/a.wav"));
        let export = export_object_key(
            project,
            "00000000-0000-0000-0000-000000000000-youtube-1080.mp4",
        );
        assert_eq!(
            export,
            format!(
                "exports/{project}/{project}/00000000-0000-0000-0000-000000000000-youtube-1080.mp4"
            )
        );
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("ffprobe failed: {0}")]
    Probe(String),
    #[error("ffmpeg failed: {0}")]
    Ffmpeg(String),
}
