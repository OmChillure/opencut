//! Speech. Groq Whisper writes the transcript. ffmpeg extracts the wav.

mod groq_stt;
mod parse;
mod punctuate;
mod transcribe;

use oc_time::Time;
use serde::{Deserialize, Serialize};

pub use groq_stt::configured as groq_stt_configured;
pub use transcribe::{SttError, transcribe_path};

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
            place: oc_timeline::CaptionPlace::Bottom,
            font: oc_timeline::CaptionFont::Sans,
            effect: oc_timeline::CaptionEffect::None,
        }
    }
}
