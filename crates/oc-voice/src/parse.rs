//! Groq returns OpenAI Whisper verbose JSON.

use crate::{Cue, Transcript};
use oc_time::Time;
use serde::Deserialize;

/// A pause this long is a new line. A shorter one stays in the sentence.
const WORD_GAP_SECS: f64 = 0.70;

#[derive(Deserialize)]
struct OpenAiJson {
    #[serde(default)]
    text: String,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    segments: Vec<OpenAiSeg>,
    #[serde(default)]
    words: Vec<OpenAiWord>,
}

#[derive(Deserialize)]
struct OpenAiSeg {
    #[serde(default)]
    start: f64,
    #[serde(default)]
    end: f64,
    #[serde(default)]
    text: String,
    #[serde(default)]
    words: Vec<OpenAiWord>,
}

#[derive(Clone, Deserialize)]
struct OpenAiWord {
    #[serde(default)]
    word: String,
    #[serde(default)]
    start: f64,
    #[serde(default)]
    end: f64,
}

pub(crate) fn parse_openai_whisper(raw: &str) -> Option<Transcript> {
    let parsed: OpenAiJson = serde_json::from_str(raw).ok()?;
    let mut words = parsed.words.clone();
    if words.is_empty() {
        words = parsed.segments.iter().flat_map(|seg| seg.words.clone()).collect();
    }
    let from_words = cues_from_words(&words);
    let cues = if from_words.is_empty() {
        parsed
            .segments
            .iter()
            .filter(|s| !s.text.trim().is_empty())
            .map(|s| Cue {
                start: Time::from_seconds(s.start),
                end: Time::from_seconds(s.end.max(s.start)),
                text: s.text.trim().to_string(),
                speaker: None,
            })
            .collect()
    } else {
        from_words
    };
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

/// Cue ends are the last word, not the segment padded out to the file.
fn cues_from_words(words: &[OpenAiWord]) -> Vec<Cue> {
    let mut words: Vec<&OpenAiWord> = words
        .iter()
        .filter(|word| !word.word.trim().is_empty() && word.end > word.start + 0.005)
        .collect();
    words.sort_by(|a, b| {
        a.start
            .partial_cmp(&b.start)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut cues = Vec::new();
    let mut group: Vec<&OpenAiWord> = Vec::new();
    for word in words {
        if let Some(prev) = group.last() {
            if word.start - prev.end >= WORD_GAP_SECS {
                push_word_cue(&mut cues, &group);
                group.clear();
            }
        }
        group.push(word);
    }
    push_word_cue(&mut cues, &group);
    cues
}

fn push_word_cue(cues: &mut Vec<Cue>, group: &[&OpenAiWord]) {
    let Some(first) = group.first() else {
        return;
    };
    let Some(last) = group.last() else {
        return;
    };
    let text = group
        .iter()
        .map(|word| word.word.trim())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if text.is_empty() {
        return;
    }
    cues.push(Cue {
        start: Time::from_seconds(first.start.max(0.0)),
        end: Time::from_seconds(last.end.max(first.start)),
        text,
        speaker: None,
    });
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
    fn a_word_timestamp_ends_before_the_silent_tail() {
        let raw = r#"{
            "text": "food and refreshments are on the house",
            "language": "en",
            "segments": [
                {"start": 13.68, "end": 17.54, "text": " Food and refreshments are on the house."}
            ],
            "words": [
                {"word": " Food", "start": 13.68, "end": 14.00},
                {"word": " and", "start": 14.02, "end": 14.20},
                {"word": " refreshments", "start": 14.22, "end": 14.90},
                {"word": " are", "start": 14.92, "end": 15.05},
                {"word": " on", "start": 15.08, "end": 15.20},
                {"word": " the", "start": 15.22, "end": 15.32},
                {"word": " house.", "start": 15.34, "end": 15.60}
            ]
        }"#;
        let t = parse_openai_whisper(raw).unwrap();
        assert_eq!(t.cues.len(), 1);
        assert!((t.cues[0].end.as_seconds() - 15.60).abs() < 0.02, "{:?}", t.cues[0].end);
        assert!(t.cues[0].end.as_seconds() < 16.0);
        assert!(t.cues[0].text.contains("house"));
    }

    #[test]
    fn a_pause_between_words_starts_a_new_line() {
        let raw = r#"{
            "text": "register now see you there",
            "segments": [],
            "words": [
                {"word": " register", "start": 1.00, "end": 1.40},
                {"word": " now", "start": 1.42, "end": 1.70},
                {"word": " see", "start": 2.60, "end": 2.80},
                {"word": " you", "start": 2.82, "end": 3.00}
            ]
        }"#;
        let t = parse_openai_whisper(raw).unwrap();
        assert_eq!(t.cues.len(), 2, "{:?}", t.cues);
        assert!((t.cues[0].end.as_seconds() - 1.70).abs() < 0.02);
        assert!((t.cues[1].start.as_seconds() - 2.60).abs() < 0.02);
        assert_eq!(t.cues[1].text, "see you");
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
