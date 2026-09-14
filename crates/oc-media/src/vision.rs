//! Always-on local look: ffmpeg scene cuts + cheap frame stats. No API.

use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::process::Command;

const W: usize = 160;
const H: usize = 90;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VisualDigest {
    pub look: String,
    pub motion: f32,
    pub scenes: u32,
    pub brightness: f32,
    pub colorful: bool,
    pub has_video: bool,
    pub has_audio: bool,
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
    let dir = std::env::temp_dir().join(format!("oc-look-{stamp}"));
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

    let probe = probe(&input).await.unwrap_or_default();
    if !probe.has_video {
        let _ = tokio::fs::remove_dir_all(&dir).await;
        return Ok(VisualDigest {
            look: if probe.has_audio {
                "audio-only".into()
            } else {
                "unknown".into()
            },
            has_audio: probe.has_audio,
            ..VisualDigest::default()
        });
    }

    let scene_times = scene_cuts(&input).await.unwrap_or_default();
    let duration = probe.duration_s.max(1.0);
    let mut samples: Vec<f64> = scene_times.iter().take(6).copied().collect();
    for p in [0.12, 0.38, 0.62, 0.88] {
        samples.push(duration * p);
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    samples.dedup_by(|a, b| (*a - *b).abs() < 0.35);
    samples.truncate(8);

    let mut frames = Vec::new();
    for (i, t) in samples.iter().enumerate() {
        let raw = dir.join(format!("f{i}.rgb"));
        if grab_rgb(&input, *t, &raw).await.is_ok() {
            if let Ok(bytes) = tokio::fs::read(&raw).await {
                if let Some(stats) = FrameStats::from_rgb(&bytes) {
                    frames.push(stats);
                }
            }
        }
    }

    let digest = fold_frames(frames, scene_times.len(), duration, probe.has_audio);
    let _ = tokio::fs::remove_dir_all(&dir).await;
    Ok(digest)
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
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_frames",
            "-of",
            "csv=p=0",
            "-f",
            "lavfi",
            &format!(
                "movie={},select=gt(scene\\,0.28)",
                path_str(input).replace('\\', "\\\\").replace(':', "\\:")
            ),
        ])
        .output()
        .await
        .map_err(|e| crate::MediaError::Probe(e.to_string()))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut times = Vec::new();
    for line in text.lines() {
        // frame,video,0,1,... pts_time is often field 5 or we scan for a float
        for part in line.split(',') {
            if let Ok(t) = part.parse::<f64>() {
                if t > 0.05 && t < 86_400.0 {
                    times.push(t);
                    break;
                }
            }
        }
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    times.dedup();
    Ok(times)
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
    let look = if luma < 38.0 {
        "dark"
    } else if edges > 28.0 && sat < 0.18 {
        "graphic"
    } else if (center - luma).abs() < 12.0 && edges < 12.0 {
        "wide"
    } else if edges > 16.0 && center > luma {
        "close"
    } else if colorful && luma > 90.0 {
        "bright-wide"
    } else if motion > 0.6 {
        "action"
    } else {
        "interior"
    };
    VisualDigest {
        look: look.into(),
        motion,
        scenes: scene_n as u32,
        brightness: luma / 255.0,
        colorful,
        has_video: true,
        has_audio,
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
    }
}