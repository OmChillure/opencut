//! Groq returns OpenAI Whisper verbose JSON.

use crate::{Cue, Transcript};
use oc_time::Time;
use serde::Deserialize;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcribe::punctuate_transcript;

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
