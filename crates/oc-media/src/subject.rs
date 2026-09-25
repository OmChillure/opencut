//! One vision pass over shot mid-frames. Optional: Groq, then xAI. No key → subjects stay empty.

use crate::vision::ShotLook;
use std::path::Path;

const SUBJECTS: &[&str] = &[
    "person",
    "people",
    "product",
    "street",
    "screen",
    "interior",
    "landscape",
    "object",
];

pub async fn label_subjects(input: &Path, shots: &mut [ShotLook]) -> Result<(), String> {
    if shots.is_empty() {
        return Ok(());
    }
    let Some(endpoint) = vision_endpoint() else {
        tracing::info!("no GROQ_API_KEY or XAI_API_KEY — shot subjects left blank");
        return Ok(());
    };
    let mut jpeg = Vec::new();
    for (i, shot) in shots.iter().enumerate() {
        let at = (shot.start + shot.end) * 0.5;
        let path = std::env::temp_dir().join(format!(
            "oc-subj-{}-{i}.jpg",
            std::process::id()
        ));
        if grab_jpeg(input, at, &path).await.is_err() {
            continue;
        }
        if let Ok(bytes) = tokio::fs::read(&path).await {
            if bytes.len() > 32 {
                jpeg.push((i, bytes));
            }
        }
        let _ = tokio::fs::remove_file(&path).await;
    }
    if jpeg.is_empty() {
        return Err("no frames for subject look".into());
    }
    for chunk in jpeg.chunks(4) {
        match ask(&endpoint, chunk).await {
            Ok(lines) => {
                for (idx, subject) in lines {
                    if let Some(shot) = shots.get_mut(idx) {
                        shot.subject = subject;
                    }
                }
            }
            Err(e) => tracing::warn!("vision batch: {e}"),
        }
    }
    Ok(())
}

struct Endpoint {
    url: String,
    key: String,
    model: String,
}

fn vision_endpoint() -> Option<Endpoint> {
    if let Some(key) = nonempty("GROQ_API_KEY") {
        return Some(Endpoint {
            url: "https://api.groq.com/openai/v1/chat/completions".into(),
            key,
            model: std::env::var("OPENCUT_VISION_MODEL")
                .unwrap_or_else(|_| "meta-llama/llama-4-scout-17b-16e-instruct".into()),
        });
    }
    if let Some(key) = nonempty("XAI_API_KEY") {
        let base = std::env::var("XAI_BASE_URL").unwrap_or_else(|_| "https://api.x.ai/v1".into());
        return Some(Endpoint {
            url: format!("{}/chat/completions", base.trim_end_matches('/')),
            key,
            model: std::env::var("OPENCUT_VISION_MODEL").unwrap_or_else(|_| "grok-2-vision-1212".into()),
        });
    }
    None
}

fn nonempty(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

async fn grab_jpeg(input: &Path, at: f64, dest: &Path) -> Result<(), ()> {
    let out = tokio::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-ss",
            &format!("{at:.3}"),
            "-i",
            &input.to_string_lossy(),
            "-frames:v",
            "1",
            "-vf",
            "scale=320:-1",
            &dest.to_string_lossy(),
        ])
        .output()
        .await
        .map_err(|_| ())?;
    if out.status.success() && dest.exists() {
        Ok(())
    } else {
        Err(())
    }
}

async fn ask(endpoint: &Endpoint, frames: &[(usize, Vec<u8>)]) -> Result<Vec<(usize, String)>, String> {
    let mut content = vec![serde_json::json!({
        "type": "text",
        "text": format!(
            "You label video frames for an editor. Images are in order, indexes {}.\n\
             For each image write one line: INDEX subject\n\
             subject is exactly one of: {}.\n\
             person = a human is the subject. product = an object being shown. street = outdoors, road, city.\n\
             screen = a display. interior = a room. landscape = scenery. object = a thing, no person.\n\
             people = more than one person. No other words.",
            frames.iter().map(|(i, _)| i.to_string()).collect::<Vec<_>>().join(", "),
            SUBJECTS.join(", ")
        )
    })];
    for (_, bytes) in frames {
        let url = format!("data:image/jpeg;base64,{}", b64(bytes));
        content.push(serde_json::json!({
            "type": "image_url",
            "image_url": { "url": url }
        }));
    }
    let body = serde_json::json!({
        "model": endpoint.model,
        "temperature": 0,
        "messages": [{ "role": "user", "content": content }]
    });
    let res = reqwest::Client::new()
        .post(&endpoint.url)
        .bearer_auth(&endpoint.key)
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = res.status();
    let raw = res.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("vision {status}: {}", raw.chars().take(240).collect::<String>()));
    }
    let v: serde_json::Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    let text = v
        .pointer("/choices/0/message/content")
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string();
    Ok(parse_subjects(&text))
}

pub fn parse_subjects(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let Some(idx) = parts.next().and_then(|s| s.trim_end_matches(['.', ':']).parse().ok()) else {
            continue;
        };
        let Some(word) = parts.next() else { continue };
        let word = word.trim_matches(|c: char| !c.is_ascii_alphabetic()).to_ascii_lowercase();
        if SUBJECTS.contains(&word.as_str()) {
            out.push((idx, word));
        }
    }
    out
}

fn b64(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        let n = ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8) | data[i + 2] as u32;
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(T[((n >> 6) & 63) as usize] as char);
        out.push(T[(n & 63) as usize] as char);
        i += 3;
    }
    if i < data.len() {
        let b0 = data[i] as u32;
        let b1 = if i + 1 < data.len() { data[i + 1] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8);
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        if i + 1 < data.len() {
            out.push(T[((n >> 6) & 63) as usize] as char);
        } else {
            out.push('=');
        }
        out.push('=');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_subject_lines() {
        let got = parse_subjects("0 person\n1 street\n2 not-a-thing\n3 product extra");
        assert_eq!(
            got,
            vec![(0, "person".into()), (1, "street".into()), (3, "product".into())]
        );
    }
}
