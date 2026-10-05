//! Local beat grid from PCM. No paid API. Sections are energy bands, not a genre model.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MusicSection {
    pub name: String,
    pub start: f64,
    pub end: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MusicAnalysis {
    pub bpm: f64,
    pub beats: Vec<f64>,
    pub downbeats: Vec<f64>,
    pub sections: Vec<MusicSection>,
    /// Mean absolute energy every 0.5s.
    pub energy: Vec<f32>,
}

/// Impulse / onset grid. `samples` are mono f32.
pub fn detect_beats(samples: &[f32], sample_rate: u32) -> MusicAnalysis {
    let rate = sample_rate.max(1) as f64;
    let hop = (sample_rate / 50).max(1) as usize;
    let mut flux = Vec::new();
    let mut prev = 0.0_f32;
    let mut i = 0;
    while i + hop <= samples.len() {
        let energy: f32 = samples[i..i + hop].iter().map(|s| s.abs()).sum::<f32>() / hop as f32;
        flux.push((energy - prev).max(0.0));
        prev = energy;
        i += hop;
    }
    let mean = flux.iter().copied().sum::<f32>() / flux.len().max(1) as f32;
    let var = flux.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / flux.len().max(1) as f32;
    let thresh = mean + var.sqrt() * 0.8;
    let mut beats = Vec::new();
    let mut last = -1.0;
    for (n, value) in flux.iter().enumerate() {
        let left = if n == 0 { 0.0 } else { flux[n - 1] };
        let right = flux.get(n + 1).copied().unwrap_or(0.0);
        if *value >= thresh && *value >= left && *value >= right {
            let t = n as f64 * hop as f64 / rate;
            if t - last > 0.18 {
                beats.push(t);
                last = t;
            }
        }
    }
    let bpm = if beats.len() >= 2 {
        let mut gaps: Vec<f64> = beats.windows(2).map(|w| w[1] - w[0]).collect();
        gaps.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mid = gaps[gaps.len() / 2];
        if mid > 0.05 { 60.0 / mid } else { 0.0 }
    } else {
        0.0
    };
    let downbeats: Vec<f64> = beats.iter().step_by(4).copied().collect();
    let step = (sample_rate as usize / 2).max(1);
    let mut energy = Vec::new();
    let mut at = 0;
    while at < samples.len() {
        let end = (at + step).min(samples.len());
        let mean = samples[at..end].iter().map(|s| s.abs()).sum::<f32>() / (end - at) as f32;
        energy.push(mean);
        at = end;
    }
    let sections = sections_from_energy(&energy, samples.len() as f64 / rate);
    MusicAnalysis {
        bpm,
        beats,
        downbeats,
        sections,
        energy,
    }
}

fn sections_from_energy(energy: &[f32], duration: f64) -> Vec<MusicSection> {
    if energy.is_empty() || duration <= 0.0 {
        return Vec::new();
    }
    let names = ["intro", "build", "drop", "outro"];
    let n = names.len();
    let mut out = Vec::new();
    for (i, name) in names.iter().enumerate() {
        let start = duration * i as f64 / n as f64;
        let end = duration * (i + 1) as f64 / n as f64;
        out.push(MusicSection {
            name: (*name).to_string(),
            start,
            end,
        });
    }
    out
}

pub fn format_music(music: &MusicAnalysis) -> String {
    let mut out = format!("bpm {:.0}\n", music.bpm);
    for section in &music.sections {
        out.push_str(&format!(
            "{} {:.1}-{:.1}\n",
            section.name, section.start, section.end
        ));
    }
    let shown: Vec<String> = music
        .beats
        .iter()
        .take(64)
        .map(|t| format!("{t:.2}"))
        .collect();
    out.push_str("beats ");
    out.push_str(&shown.join(","));
    if music.beats.len() > shown.len() {
        out.push_str(&format!(" …{} more", music.beats.len() - shown.len()));
    }
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::detect_beats;

    #[test]
    fn click_track_lands_on_the_beat() {
        let rate = 22_050_u32;
        let bpm = 120.0;
        let gap = 60.0 / bpm;
        let seconds = 4.0;
        let n = (rate as f64 * seconds) as usize;
        let mut samples = vec![0.0_f32; n];
        let mut t = 0.0;
        while t < seconds - 0.05 {
            let i = (t * rate as f64) as usize;
            for k in 0..40 {
                if i + k < samples.len() {
                    samples[i + k] = 1.0;
                }
            }
            t += gap;
        }
        let music = detect_beats(&samples, rate);
        let frame = 1.0 / 30.0;
        for expect in [0.0, gap, gap * 2.0, gap * 3.0] {
            let hit = music
                .beats
                .iter()
                .any(|found| (found - expect).abs() <= frame);
            assert!(hit, "missing beat at {expect}, got {:?}", music.beats);
        }
    }
}
