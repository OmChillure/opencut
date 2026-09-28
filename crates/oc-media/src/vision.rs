//! Always-on local look: ffmpeg scene cuts + cheap frame stats. No API.

use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::process::Command;

const W: usize = 160;
const H: usize = 90;

/// One picture range inside a source. `look` is this shot, not the whole file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShotLook {
    pub start: f64,
    pub end: f64,
    pub look: String,
    /// What is in frame: person, product, street, screen, interior, landscape, object.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub subject: String,
    #[serde(default)]
    pub motion: f32,
    /// Filled by one vision batch at import. Missing when there is no API key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card: Option<ShotCard>,
}

/// Compact look from one vision batch. Cached on the analysis row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShotCard {
    #[serde(default)]
    pub scale: String,
    #[serde(default)]
    pub camera: String,
    #[serde(default)]
    pub motion_dir: String,
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub mood: String,
    #[serde(default)]
    pub palette: Vec<String>,
    #[serde(default)]
    pub quality: u8,
    #[serde(default)]
    pub best_moment: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VisualDigest {
    pub look: String,
    pub motion: f32,
    pub scenes: u32,
    pub brightness: f32,
    pub colorful: bool,
    pub has_video: bool,
    pub has_audio: bool,
    /// Scene ranges with a look each. Missing on rows saved before shot lists.
    #[serde(default)]
    pub shots: Vec<ShotLook>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub music: Option<crate::MusicAnalysis>,
}

impl Default for VisualDigest {
    fn default() -> Self {
        Self {
            look: "unknown".into(),
            motion: 0.0,
            scenes: 0,
            brightness: 0.0,
            colorful: false,
            has_video: false,
            has_audio: false,
            shots: Vec::new(),
            music: None,
        }
    }
}

impl VisualDigest {
    #[must_use]
    pub fn brief(&self) -> String {
        if !self.has_video {
            return "audio-only".into();
        }
        format!(
            "{}  motion={:.2}  scenes={}  {}",
            self.look,
            self.motion,
            self.scenes,
            if self.colorful { "colorful" } else { "flat" }
        )
    }
}

pub async fn analyze_local(bytes: &[u8], filename: &str) -> Result<VisualDigest, crate::MediaError> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("oc-look-in-{stamp}"));
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| crate::MediaError::Ffmpeg(e.to_string()))?;
    let ext = Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .filter(|e| e.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or("bin");
    let input = dir.join(format!("in.{ext}"));
    tokio::fs::write(&input, bytes)
        .await
        .map_err(|e| crate::MediaError::Ffmpeg(e.to_string()))?;
    let digest = analyze_path(&input).await;
    let _ = tokio::fs::remove_dir_all(&dir).await;
    digest
}

/// Scene cuts plus one look per kept range. Reads `input` in place.
pub async fn analyze_path(input: &Path) -> Result<VisualDigest, crate::MediaError> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("oc-look-{stamp}"));
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| crate::MediaError::Ffmpeg(e.to_string()))?;

    let probe = probe(input).await.unwrap_or_default();
    if !probe.has_video {
        let _ = tokio::fs::remove_dir_all(&dir).await;
        let end = probe.duration_s.max(0.0);
        let look = if probe.has_audio {
            "audio-only"
        } else {
            "unknown"
        };
        return Ok(VisualDigest {
            look: look.into(),
            has_audio: probe.has_audio,
            shots: if end > 0.05 {
                vec![ShotLook {
                    start: 0.0,
                    end,
                    look: look.into(),
                    subject: String::new(),
                    motion: 0.0,
                    card: None,
                }]
            } else {
                Vec::new()
            },
            ..VisualDigest::default()
        });
    }

    let scene_times = scene_cuts(input).await.unwrap_or_default();
    let duration = probe.duration_s.max(1.0);
    let ranges = picture_ranges(&scene_times, duration);

    let mut frames = Vec::new();
    let mut shots = Vec::new();
    for (i, (start, end, motion)) in ranges.iter().enumerate() {
        let at = (start + end) * 0.5;
        let raw = dir.join(format!("f{i}.rgb"));
        let stats = if grab_rgb(input, at, &raw).await.is_ok() {
            tokio::fs::read(&raw)
                .await
                .ok()
                .and_then(|bytes| FrameStats::from_rgb(&bytes))
        } else {
            None
        };
        let look = stats
            .as_ref()
            .map(|s| look_label(s, *motion))
            .unwrap_or("unknown");
        if let Some(stats) = stats {
            frames.push(stats);
        }
        shots.push(ShotLook {
            start: *start,
            end: *end,
            look: look.into(),
            subject: String::new(),
            motion: *motion,
            card: None,
        });
    }

    let mut digest = fold_frames(frames, scene_times.len(), duration, probe.has_audio);
    digest.shots = shots;
    let _ = tokio::fs::remove_dir_all(&dir).await;
    Ok(digest)
}

/// Merge scene cuts into at most 24 ranges, none shorter than 1.2s.
/// Each tuple is `(start, end, motion)` where motion is absorbed cuts / duration.
pub fn picture_ranges(cuts: &[f64], duration: f64) -> Vec<(f64, f64, f32)> {
    let duration = if duration.is_finite() {
        duration.max(0.04)
    } else {
        0.04
    };
    struct Span {
        start: f64,
        end: f64,
        cuts: u32,
    }
    let mut spans = Vec::new();
    let mut prev = 0.0;
    for t in cuts {
        if !t.is_finite() || *t <= prev + 0.05 || *t >= duration - 0.05 {
            continue;
        }
        spans.push(Span {
            start: prev,
            end: *t,
            cuts: 0,
        });
        prev = *t;
    }
    spans.push(Span {
        start: prev,
        end: duration,
        cuts: 0,
    });

    fn absorb(into: &mut Span, gone: &Span) {
        into.cuts += gone.cuts + 1;
    }

    let min = 1.2;
    let mut i = 0;
    while i < spans.len() {
        let len = spans[i].end - spans[i].start;
        if len < min && spans.len() > 1 {
            if i == 0 {
                let gone = spans.remove(0);
                spans[0].start = gone.start;
                absorb(&mut spans[0], &gone);
            } else {
                let gone = spans.remove(i);
                spans[i - 1].end = gone.end;
                absorb(&mut spans[i - 1], &gone);
            }
            continue;
        }
        i += 1;
    }
    while spans.len() > 24 {
        let idx = spans
            .iter()
            .enumerate()
            .min_by(|a, b| {
                let la = a.1.end - a.1.start;
                let lb = b.1.end - b.1.start;
                la.partial_cmp(&lb).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(i, _)| i)
            .unwrap_or(0);
        if idx == 0 {
            let gone = spans.remove(0);
            spans[0].start = gone.start;
            absorb(&mut spans[0], &gone);
        } else {
            let gone = spans.remove(idx);
            spans[idx - 1].end = gone.end;
            absorb(&mut spans[idx - 1], &gone);
        }
    }
    spans
        .into_iter()
        .map(|s| {
            let dur = (s.end - s.start).max(0.05);
            let motion = (s.cuts as f32 / dur as f32).min(4.0);
            (s.start, s.end, motion)
        })
        .collect()
}

#[derive(Default)]
struct Probe {
    duration_s: f64,
    has_video: bool,
    has_audio: bool,
}

async fn probe(input: &Path) -> Result<Probe, crate::MediaError> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration:stream=codec_type",
            "-of",
            "json",
            &path_str(input),
        ])
        .output()
        .await
        .map_err(|e| crate::MediaError::Probe(e.to_string()))?;
    if !out.status.success() {
        return Err(crate::MediaError::Probe(
            String::from_utf8_lossy(&out.stderr).into(),
        ));
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    let duration_s = v
        .pointer("/format/duration")
        .and_then(|x| x.as_str())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);
    let mut has_video = false;
    let mut has_audio = false;
    if let Some(streams) = v.get("streams").and_then(|s| s.as_array()) {
        for s in streams {
            match s.get("codec_type").and_then(|t| t.as_str()) {
                Some("video") => has_video = true,
                Some("audio") => has_audio = true,
                _ => {}
            }
        }
    }
    Ok(Probe {
        duration_s,
        has_video,
        has_audio,
    })
}

async fn scene_cuts(input: &Path) -> Result<Vec<f64>, crate::MediaError> {
    // Low-res, 2 fps. A full-frame scene pass is what made a long file feel stuck.
    let out = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-i",
            &path_str(input),
            "-vf",
            "scale=320:-1,fps=2,select='gt(scene,0.30)',showinfo",
            "-an",
            "-f",
            "null",
            "-",
        ])
        .output()
        .await
        .map_err(|e| crate::MediaError::Probe(e.to_string()))?;
    let text = String::from_utf8_lossy(&out.stderr);
    let mut times = Vec::new();
    for line in text.lines() {
        let Some(rest) = line.split("pts_time:").nth(1) else {
            continue;
        };
        let num: String = rest
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        if let Ok(t) = num.parse::<f64>() {
            if t > 0.05 && t < 86_400.0 {
                times.push(t);
            }
        }
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    times.dedup();
    Ok(times)
}

/// One JPEG at `at` seconds. 384px wide keeps the frame readable and the vision tokens small.
pub async fn grab_jpeg(input: &Path, at: f64, dest: &Path) -> Result<(), crate::MediaError> {
    let out = Command::new("ffmpeg")
        .args([
            "-y",
            "-ss",
            &format!("{at:.3}"),
            "-i",
            &path_str(input),
            "-frames:v",
            "1",
            "-vf",
            "scale=384:-2",
            "-q:v",
            "8",
            &path_str(dest),
        ])
        .output()
        .await
        .map_err(|e| crate::MediaError::Ffmpeg(e.to_string()))?;
    if !out.status.success() || !dest.exists() {
        return Err(crate::MediaError::Ffmpeg("no jpeg".into()));
    }
    Ok(())
}

async fn grab_rgb(input: &Path, at: f64, dest: &Path) -> Result<(), crate::MediaError> {
    let out = Command::new("ffmpeg")
        .args([
            "-y",
            "-ss",
            &format!("{at:.3}"),
            "-i",
            &path_str(input),
            "-frames:v",
            "1",
            "-vf",
            &format!("scale={W}:{H}"),
            "-pix_fmt",
            "rgb24",
            "-f",
            "rawvideo",
            &path_str(dest),
        ])
        .output()
        .await
        .map_err(|e| crate::MediaError::Ffmpeg(e.to_string()))?;
    if !out.status.success() || !dest.exists() {
        return Err(crate::MediaError::Ffmpeg("no frame".into()));
    }
    Ok(())
}

struct FrameStats {
    luma: f32,
    sat: f32,
    edges: f32,
    center_luma: f32,
}

impl FrameStats {
    fn from_rgb(buf: &[u8]) -> Option<Self> {
        if buf.len() < W * H * 3 {
            return None;
        }
        let mut luma_sum = 0.0f32;
        let mut sat_sum = 0.0f32;
        let mut center = 0.0f32;
        let mut center_n = 0u32;
        let mut lum = vec![0f32; W * H];
        for y in 0..H {
            for x in 0..W {
                let i = (y * W + x) * 3;
                let r = buf[i] as f32;
                let g = buf[i + 1] as f32;
                let b = buf[i + 2] as f32;
                let yv = 0.299 * r + 0.587 * g + 0.114 * b;
                lum[y * W + x] = yv;
                luma_sum += yv;
                let mx = r.max(g).max(b);
                let mn = r.min(g).min(b);
                sat_sum += if mx > 1.0 { (mx - mn) / mx } else { 0.0 };
                if x > W / 3 && x < 2 * W / 3 && y > H / 3 && y < 2 * H / 3 {
                    center += yv;
                    center_n += 1;
                }
            }
        }
        let n = (W * H) as f32;
        let mut edge = 0.0f32;
        for y in 1..H - 1 {
            for x in 1..W - 1 {
                let gx = lum[y * W + x + 1] - lum[y * W + x - 1];
                let gy = lum[(y + 1) * W + x] - lum[(y - 1) * W + x];
                edge += (gx * gx + gy * gy).sqrt();
            }
        }
        Some(Self {
            luma: luma_sum / n,
            sat: sat_sum / n,
            edges: edge / n,
            center_luma: if center_n > 0 {
                center / center_n as f32
            } else {
                0.0
            },
        })
    }
}

fn look_label(s: &FrameStats, motion: f32) -> &'static str {
    let colorful = s.sat > 0.22;
    if s.luma < 38.0 {
        "dark"
    } else if s.edges > 28.0 && s.sat < 0.18 {
        "graphic"
    } else if (s.center_luma - s.luma).abs() < 12.0 && s.edges < 12.0 {
        "wide"
    } else if s.edges > 16.0 && s.center_luma > s.luma {
        "close"
    } else if colorful && s.luma > 90.0 {
        "bright-wide"
    } else if motion > 0.6 {
        "action"
    } else {
        "interior"
    }
}

fn fold_frames(frames: Vec<FrameStats>, scene_n: usize, duration: f64, has_audio: bool) -> VisualDigest {
    if frames.is_empty() {
        return VisualDigest {
            look: "unknown".into(),
            has_video: true,
            has_audio,
            scenes: scene_n as u32,
            ..VisualDigest::default()
        };
    }
    let n = frames.len() as f32;
    let luma = frames.iter().map(|f| f.luma).sum::<f32>() / n;
    let sat = frames.iter().map(|f| f.sat).sum::<f32>() / n;
    let edges = frames.iter().map(|f| f.edges).sum::<f32>() / n;
    let center = frames.iter().map(|f| f.center_luma).sum::<f32>() / n;
    let motion = (scene_n as f32 / duration as f32).min(4.0);
    let colorful = sat > 0.22;
    let fake = FrameStats {
        luma,
        sat,
        edges,
        center_luma: center,
    };
    VisualDigest {
        look: look_label(&fake, motion).into(),
        motion,
        scenes: scene_n as u32,
        brightness: luma / 255.0,
        colorful,
        has_video: true,
        has_audio,
        shots: Vec::new(),
        music: None,
    }
}

fn path_str(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_dark_frame() {
        let buf = vec![10u8; W * H * 3];
        let s = FrameStats::from_rgb(&buf).unwrap();
        assert!(s.luma < 20.0);
        assert_eq!(look_label(&s, 0.0), "dark");
    }

    #[test]
    fn picture_ranges_keep_scene_times_and_cap() {
        let cuts = [2.0, 3.0, 10.0, 10.4, 30.0];
        let ranges = picture_ranges(&cuts, 40.0);
        assert!(ranges.len() <= 24);
        assert!((ranges[0].0 - 0.0).abs() < 1e-6);
        assert!((ranges.last().unwrap().1 - 40.0).abs() < 1e-6);
        for w in ranges.windows(2) {
            assert!((w[0].1 - w[1].0).abs() < 1e-6);
            assert!(w[0].1 - w[0].0 >= 1.2 - 1e-6);
        }
        let tight: Vec<f64> = (1..40).map(|i| i as f64 * 2.0).collect();
        let capped = picture_ranges(&tight, 90.0);
        assert_eq!(capped.len(), 24);
        assert!((capped[0].0).abs() < 1e-6);
        assert!((capped.last().unwrap().1 - 90.0).abs() < 1e-6);
    }

    #[test]
    fn old_digest_json_has_no_shots() {
        let raw = r#"{"look":"wide","motion":0.1,"scenes":2,"brightness":0.4,"colorful":false,"has_video":true,"has_audio":true}"#;
        let d: VisualDigest = serde_json::from_str(raw).unwrap();
        assert!(d.shots.is_empty());
        assert_eq!(d.look, "wide");
    }
}