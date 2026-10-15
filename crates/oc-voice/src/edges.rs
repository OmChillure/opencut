//! Pull a cue's end off a quiet hold. A breath under 0.7s stays.

use crate::Cue;
use oc_time::Time;
use std::path::Path;

/// Quiet longer than this at the end of a cue is a hold, not the last word.
const QUIET_GAP_SECS: f64 = 0.70;
const WINDOW_SECS: f64 = 0.10;
/// A pop after a long hold does not keep the tail.
const BLIP_SECS: f64 = 0.35;

/// Move each cue end back to the last loud window when the cue runs through a quiet hold.
pub(crate) fn snap_file(cues: &mut [Cue], wav: &Path) {
    let Ok(bytes) = std::fs::read(wav) else {
        return;
    };
    let Some((rate, samples)) = pcm16(&bytes) else {
        return;
    };
    let moved = snap_cue_ends(cues, &samples, rate);
    if moved > 0 {
        tracing::info!(cues = moved, "trimmed a quiet hold off the cue end");
    }
}

/// Returns how many cues moved. `sample_rate` is the PCM rate.
pub(crate) fn snap_cue_ends(cues: &mut [Cue], samples: &[i16], sample_rate: u32) -> usize {
    if sample_rate < 1000 || samples.is_empty() {
        return 0;
    }
    let window = ((f64::from(sample_rate) * WINDOW_SECS).round() as usize).max(1);
    if samples.len() < window * 8 {
        return 0;
    }
    let mut levels = Vec::with_capacity(samples.len() / window);
    let mut index = 0;
    while index + window <= samples.len() {
        let chunk = &samples[index..index + window];
        let energy = chunk
            .iter()
            .map(|sample| {
                let value = f64::from(*sample);
                value * value
            })
            .sum::<f64>()
            / chunk.len() as f64;
        let db = 20.0 * (energy.sqrt() / 32768.0 + 1e-12).log10();
        levels.push(db);
        index += window;
    }
    let Some(threshold) = speech_threshold(&levels) else {
        return 0;
    };
    let mut moved = 0;
    for cue in cues {
        let start = cue.start.as_seconds();
        let end = cue.end.as_seconds();
        if end - start < 0.35 {
            continue;
        }
        let Some(speech_end) =
            trailing_speech_end(start, end, &levels, WINDOW_SECS, threshold)
        else {
            continue;
        };
        if end - speech_end < QUIET_GAP_SECS || speech_end < start + 0.35 {
            continue;
        }
        cue.end = Time::from_seconds(speech_end);
        moved += 1;
    }
    moved
}

fn speech_threshold(levels: &[f64]) -> Option<f64> {
    if levels.len() < 8 {
        return None;
    }
    let mut sorted = levels.to_vec();
    sorted.sort_by(f64::total_cmp);
    let floor = sorted[sorted.len() / 5];
    let speech = sorted[sorted.len() * 3 / 4];
    if speech - floor < 10.0 {
        return None;
    }
    Some((speech + floor) * 0.5)
}

/// End of the last real speech inside `[start, end]`, when a quiet hold follows it.
fn trailing_speech_end(
    start: f64,
    end: f64,
    levels: &[f64],
    window: f64,
    threshold: f64,
) -> Option<f64> {
    let mut runs: Vec<(f64, f64)> = Vec::new();
    for (index, level) in levels.iter().enumerate() {
        let t0 = index as f64 * window;
        let t1 = t0 + window;
        if t1 <= start + 0.02 || t0 >= end - 0.02 || *level < threshold {
            continue;
        }
        if let Some(run) = runs.last_mut() {
            if t0 - run.1 <= window * 1.5 {
                run.1 = t1.min(end);
                continue;
            }
        }
        runs.push((t0.max(start), t1.min(end)));
    }
    let (run_start, mut run_end) = *runs.last()?;
    if run_end - run_start < BLIP_SECS {
        if let Some((_, prev_end)) = runs.iter().rev().nth(1) {
            if run_start - prev_end >= QUIET_GAP_SECS {
                run_end = *prev_end;
            }
        }
    }
    Some(run_end)
}

fn pcm16(bytes: &[u8]) -> Option<(u32, Vec<i16>)> {
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let mut index = 12usize;
    let mut rate = None;
    let mut channels = None;
    let mut bits = None;
    let mut data: Option<&[u8]> = None;
    while index + 8 <= bytes.len() {
        let id = &bytes[index..index + 4];
        let size = u32::from_le_bytes(bytes[index + 4..index + 8].try_into().ok()?) as usize;
        let body = index + 8;
        if body + size > bytes.len() {
            break;
        }
        if id == b"fmt " && size >= 16 {
            let format = u16::from_le_bytes(bytes[body..body + 2].try_into().ok()?);
            channels = Some(u16::from_le_bytes(bytes[body + 2..body + 4].try_into().ok()?));
            rate = Some(u32::from_le_bytes(bytes[body + 4..body + 8].try_into().ok()?));
            bits = Some(u16::from_le_bytes(
                bytes[body + 14..body + 16].try_into().ok()?,
            ));
            if format != 1 {
                return None;
            }
        } else if id == b"data" {
            data = Some(&bytes[body..body + size]);
        }
        index = body + size + (size % 2);
    }
    if channels != Some(1) || bits != Some(16) {
        return None;
    }
    let rate = rate?;
    let data = data?;
    let mut samples = Vec::with_capacity(data.len() / 2);
    for pair in data.chunks_exact(2) {
        samples.push(i16::from_le_bytes([pair[0], pair[1]]));
    }
    Some((rate, samples))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(db: f64, seconds: f64) -> Vec<i16> {
        let amp = 10f64.powf(db / 20.0) * 32767.0;
        let count = (16_000.0 * seconds).round() as usize;
        vec![amp.round() as i16; count]
    }

    fn cue(start: f64, end: f64) -> Cue {
        Cue {
            start: Time::from_seconds(start),
            end: Time::from_seconds(end),
            text: "the line".into(),
            speaker: None,
        }
    }

    #[test]
    fn a_quiet_hold_after_the_words_leaves_the_cue() {
        let mut samples = tone(-20.0, 4.0);
        samples.extend(tone(-50.0, 2.0));
        let mut cues = vec![cue(0.0, 6.0)];
        assert_eq!(snap_cue_ends(&mut cues, &samples, 16_000), 1);
        let end = cues[0].end.as_seconds();
        assert!((end - 4.0).abs() < 0.15, "{end}");
    }

    #[test]
    fn a_flat_take_and_a_short_breath_stay() {
        let flat = tone(-20.0, 6.0);
        let mut cues = vec![cue(0.0, 6.0)];
        assert_eq!(snap_cue_ends(&mut cues, &flat, 16_000), 0);
        assert!((cues[0].end.as_seconds() - 6.0).abs() < 0.01);

        let mut breath = tone(-20.0, 2.0);
        breath.extend(tone(-50.0, 0.4));
        breath.extend(tone(-20.0, 2.0));
        let mut cues = vec![cue(0.0, 4.4)];
        assert_eq!(snap_cue_ends(&mut cues, &breath, 16_000), 0);
    }

    #[test]
    fn a_pop_after_the_hold_does_not_keep_the_tail() {
        let mut samples = tone(-20.0, 3.0);
        samples.extend(tone(-50.0, 1.2));
        samples.extend(tone(-20.0, 0.2));
        let mut cues = vec![cue(0.0, 4.4)];
        assert_eq!(snap_cue_ends(&mut cues, &samples, 16_000), 1);
        let end = cues[0].end.as_seconds();
        assert!((end - 3.0).abs() < 0.15, "{end}");
    }

    #[test]
    fn an_earlier_line_keeps_its_end() {
        let mut samples = tone(-20.0, 4.0);
        samples.extend(tone(-50.0, 2.0));
        let mut cues = vec![cue(0.0, 3.2), cue(3.2, 6.0)];
        assert_eq!(snap_cue_ends(&mut cues, &samples, 16_000), 1);
        assert!((cues[0].end.as_seconds() - 3.2).abs() < 0.01);
        assert!((cues[1].end.as_seconds() - 4.0).abs() < 0.15);
    }
}
