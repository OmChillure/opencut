//! Turn the ranges from `analyze_path` into shot cards.
//!
//! This module grabs one JPEG per range and parses the label JSON.
//! The signed-in Claude, Grok, or Codex CLI writes that JSON. No API key.

use crate::vision::{ShotCard, ShotLook};
use serde_json::Value;
use std::path::Path;

/// One frame waiting for the local subscription to label.
#[derive(Clone, Debug)]
pub struct ShotStill {
    pub index: usize,
    pub caption: String,
    pub jpeg: Vec<u8>,
}

/// JPEGs for ranges that do not have a card yet.
pub async fn shot_stills(input: &Path, shots: &[ShotLook]) -> Result<Vec<ShotStill>, String> {
    let pending: Vec<usize> = shots
        .iter()
        .enumerate()
        .filter(|(_, shot)| shot.card.is_none())
        .map(|(i, _)| i)
        .collect();
    if pending.is_empty() {
        return Ok(Vec::new());
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("oc-card-{stamp}"));
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| format!("could not make a temp dir: {e}"))?;
    let mut frames = Vec::new();
    for i in pending {
        let shot = &shots[i];
        let at = (shot.start + shot.end) * 0.5;
        let dest = dir.join(format!("s{i}.jpg"));
        if crate::vision::grab_jpeg(input, at, &dest).await.is_err() {
            continue;
        }
        let Ok(bytes) = tokio::fs::read(&dest).await else {
            continue;
        };
        if bytes.len() < 32 {
            continue;
        }
        frames.push(ShotStill {
            index: i,
            caption: format!("Shot {i}, source {:.1}-{:.1}s", shot.start, shot.end),
            jpeg: bytes,
        });
    }
    let _ = tokio::fs::remove_dir_all(&dir).await;
    if frames.is_empty() {
        return Err("no frames to label".into());
    }
    Ok(frames)
}

#[must_use]
pub fn shot_label_prompt() -> &'static str {
    "You label video shots for an editor. Reply with one JSON array only, no markdown. \
     One object per image, in the same order. Do not call tools. Fields: \
     i (the shot index named on that image), \
     scale (wide, medium, close, or detail), \
     camera (static, pan, tilt, handheld, push, or pull), \
     motion_dir (none, l2r, r2l, toward, or away), \
     subject (person, product, street, screen, interior, landscape, or object), \
     action (at most 12 words, what is happening), \
     mood (one word), \
     palette (up to 3 hex colors), \
     quality (integer 1-10, 10 is a usable in-focus frame), \
     best (0 to 1, where in this shot the frame sits), \
     grade (from this frame: exposure, contrast, saturation, temperature, lift, gamma, gain, \
     each from -1 to 1 where 0 leaves it unchanged, and lut: none, film, cool, warm, teal_orange, or mono). \
     Grade only to correct this frame. A clear, well-exposed frame is all zeros and lut none. \
     Do not add warm, cool, film, or teal to a frame that is already fine, and do not raise \
     exposure, contrast, saturation, lift, gamma, or gain on it. A real correction stays between \
     -0.08 and 0.08. Shots of the same place and light get the same correction so skin and white \
     walls match. Use mono only when the frame is already black and white."
}

/// Ask for a grade when a shot was never graded while it was watched.
#[must_use]
pub fn grade_prompt() -> &'static str {
    "You grade video frames for an editor. Reply with one JSON array only, no markdown. \
     One object per image, in the same order. Do not call tools. Fields: \
     i (the shot index named on that image), \
     grade: { exposure, contrast, saturation, temperature, lift, gamma, gain, lut }. \
     Each number is from -1 to 1. 0 leaves that control unchanged. \
     lut is none, film, cool, warm, teal_orange, or mono. \
     Grade only to correct the frame. A clear, well-exposed frame is all zeros and lut none. \
     Do not add warm, cool, film, or teal to a frame that is already fine, and do not raise \
     exposure, contrast, saturation, lift, gamma, or gain on it. A real correction stays between \
     -0.08 and 0.08. Shots of the same place and light get the same correction so skin and white \
     walls match. Use mono only when the frame is already black and white."
}

/// Write cards from the model's text. Ranges the reply skips get the coarse look.
pub fn apply_shot_reply(
    shots: &mut [ShotLook],
    stills: &[ShotStill],
    text: &str,
) -> Result<(), String> {
    let parsed = parse_cards(text)?;
    let frames: Vec<(usize, Vec<u8>)> = stills.iter().map(|s| (s.index, Vec::new())).collect();
    assign_cards(shots, &frames, &parsed);
    for still in stills {
        let Some(shot) = shots.get_mut(still.index) else {
            continue;
        };
        if shot.card.is_none() {
            shot.card = Some(fallback_card(shot));
        }
    }
    Ok(())
}

struct Parsed {
    i: usize,
    scale: String,
    camera: String,
    motion_dir: String,
    action: String,
    mood: String,
    palette: Vec<String>,
    quality: u8,
    subject: String,
    best: f64,
    grade: Option<oc_timeline::Grade>,
}

fn assign_cards(shots: &mut [ShotLook], frames: &[(usize, Vec<u8>)], parsed: &[Parsed]) {
    let frame_ids: Vec<usize> = frames.iter().map(|(i, _)| *i).collect();
    let ids_match =
        parsed.len() == frame_ids.len() && parsed.iter().all(|card| frame_ids.contains(&card.i));
    let pairs: Vec<(usize, &Parsed)> = if ids_match {
        parsed.iter().map(|card| (card.i, card)).collect()
    } else if parsed.len() == frame_ids.len() {
        frame_ids.into_iter().zip(parsed.iter()).collect()
    } else {
        parsed
            .iter()
            .filter(|card| shots.get(card.i).is_some())
            .map(|card| (card.i, card))
            .collect()
    };
    for (i, card) in pairs {
        let Some(shot) = shots.get_mut(i) else {
            continue;
        };
        let span = (shot.end - shot.start).max(0.0);
        let best = if (0.0..=1.0).contains(&card.best) {
            shot.start + card.best * span
        } else if card.best >= shot.start && card.best <= shot.end {
            card.best
        } else {
            shot.start + span * 0.5
        };
        let subject = normalize_subject(&card.subject);
        if !subject.is_empty() {
            shot.subject = subject;
        }
        shot.card = Some(ShotCard {
            scale: normalize_scale(&card.scale),
            camera: normalize_token(&card.camera),
            motion_dir: normalize_motion(&card.motion_dir),
            action: card.action.trim().chars().take(80).collect(),
            mood: card.mood.trim().chars().take(32).collect(),
            palette: card.palette.iter().take(3).cloned().collect(),
            quality: if card.quality == 0 {
                0
            } else {
                card.quality.clamp(1, 10)
            },
            best_moment: best,
            grade: card.grade,
        });
    }
}

fn fallback_card(shot: &ShotLook) -> ShotCard {
    ShotCard {
        scale: shot.look.clone(),
        motion_dir: "none".into(),
        best_moment: (shot.start + shot.end) * 0.5,
        ..ShotCard::default()
    }
}

fn parse_cards(text: &str) -> Result<Vec<Parsed>, String> {
    let json = extract_array(text)?;
    let value: Value = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    let items = value.as_array().ok_or_else(|| "not an array".to_string())?;
    if items.is_empty() {
        return Err("empty label".into());
    }
    let mut out = Vec::new();
    for (idx, item) in items.iter().enumerate() {
        out.push(Parsed {
            i: item
                .get("i")
                .and_then(Value::as_u64)
                .map(|n| n as usize)
                .unwrap_or(idx),
            scale: string_field(item, "scale"),
            camera: string_field(item, "camera"),
            motion_dir: string_field(item, "motion_dir"),
            action: string_field(item, "action"),
            mood: string_field(item, "mood"),
            palette: item
                .get("palette")
                .and_then(Value::as_array)
                .map(|colors| {
                    colors
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            quality: item
                .get("quality")
                .and_then(|v| v.as_u64().or_else(|| v.as_f64().map(|n| n.round() as u64)))
                .unwrap_or(0) as u8,
            subject: string_field(item, "subject"),
            best: item.get("best").and_then(Value::as_f64).unwrap_or(0.5),
            grade: item.get("grade").and_then(grade_from_value),
        });
    }
    Ok(out)
}

fn string_field(item: &Value, key: &str) -> String {
    item.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string()
}

fn extract_array(text: &str) -> Result<String, String> {
    let trimmed = text.trim();
    let fenced = trimmed
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    if fenced.starts_with('[') {
        return Ok(fenced.to_string());
    }
    if let (Some(start), Some(end)) = (fenced.find('['), fenced.rfind(']')) {
        if end > start {
            return Ok(fenced[start..=end].to_string());
        }
    }
    Err("no json array".into())
}

fn normalize_token(raw: &str) -> String {
    raw.trim().to_ascii_lowercase().replace([' ', '_'], "")
}

fn normalize_scale(raw: &str) -> String {
    match normalize_token(raw).as_str() {
        "wide" | "ws" | "long" | "establishing" => "wide".into(),
        "medium" | "ms" | "mid" => "medium".into(),
        "close" | "cu" | "closeup" | "ecu" => "close".into(),
        "detail" | "macro" | "insert" => "detail".into(),
        other => other.to_string(),
    }
}

fn normalize_motion(raw: &str) -> String {
    let t = normalize_token(raw);
    match t.as_str() {
        "l2r" | "lefttoright" | "ltr" | "rightward" => "l2r".into(),
        "r2l" | "righttoleft" | "rtl" | "leftward" => "r2l".into(),
        "toward" | "in" | "pushin" => "toward".into(),
        "away" | "out" | "pullout" => "away".into(),
        "none" | "static" | "" => "none".into(),
        other => other.to_string(),
    }
}

/// The grade on the shot that overlaps this source range, when the model wrote one.
#[must_use]
pub fn watched_grade(shots: &[ShotLook], start: f64, end: f64) -> Option<oc_timeline::Grade> {
    let mut best: Option<(f64, oc_timeline::Grade)> = None;
    for shot in shots {
        let Some(grade) = shot.card.as_ref().and_then(|card| card.grade) else {
            continue;
        };
        let overlap = (shot.end.min(end) - shot.start.max(start)).max(0.0);
        if overlap <= 0.0 {
            continue;
        }
        if best.map(|(have, _)| overlap > have).unwrap_or(true) {
            best = Some((overlap, grade));
        }
    }
    best.map(|(_, grade)| grade)
}

/// A grade object from a model that looked at a frame. `None` when the value is not an object.
#[must_use]
pub fn grade_from_value(value: &Value) -> Option<oc_timeline::Grade> {
    let obj = value.as_object()?;
    Some(oc_timeline::Grade {
        exposure: grade_number(obj, "exposure"),
        contrast: grade_number(obj, "contrast"),
        saturation: grade_number(obj, "saturation"),
        temperature: grade_number(obj, "temperature"),
        lift: grade_number(obj, "lift"),
        gamma: grade_number(obj, "gamma"),
        gain: grade_number(obj, "gain"),
        lut: grade_lut(obj.get("lut").and_then(Value::as_str).unwrap_or("none")),
        cube: None,
    })
}

/// Grades from a JSON array. Each item is `{ "i", "grade" }`. Items without a grade are skipped.
pub fn grades_from_reply(text: &str) -> Result<Vec<(usize, oc_timeline::Grade)>, String> {
    let json = extract_array(text)?;
    let value: Value = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    let items = value.as_array().ok_or_else(|| "not an array".to_string())?;
    let mut out = Vec::new();
    for (idx, item) in items.iter().enumerate() {
        let Some(grade) = item.get("grade").and_then(grade_from_value) else {
            continue;
        };
        let i = item
            .get("i")
            .and_then(Value::as_u64)
            .map(|n| n as usize)
            .unwrap_or(idx);
        out.push((i, grade));
    }
    if out.is_empty() {
        return Err("no grades".into());
    }
    Ok(out)
}

fn grade_number(obj: &serde_json::Map<String, Value>, key: &str) -> f32 {
    obj.get(key)
        .and_then(|value| {
            value.as_f64().or_else(|| {
                value
                    .as_str()
                    .and_then(|text| text.trim().parse::<f64>().ok())
            })
        })
        .unwrap_or(0.0)
        .clamp(-1.0, 1.0) as f32
}

fn grade_lut(raw: &str) -> oc_timeline::Lut {
    let key = raw.trim().to_ascii_lowercase().replace([' ', '-'], "_");
    match key.as_str() {
        "film" => oc_timeline::Lut::Film,
        "cool" | "cold" => oc_timeline::Lut::Cool,
        "warm" => oc_timeline::Lut::Warm,
        "teal_orange" | "tealorange" | "teal" => oc_timeline::Lut::TealOrange,
        "mono" | "bw" | "black_white" | "black_and_white" => oc_timeline::Lut::Mono,
        _ => oc_timeline::Lut::None,
    }
}

fn normalize_subject(raw: &str) -> String {
    match normalize_token(raw).as_str() {
        "person" | "people" | "face" | "talkinghead" => "person".into(),
        "product" => "product".into(),
        "street" | "city" | "outdoor" => "street".into(),
        "screen" | "monitor" | "phone" => "screen".into(),
        "interior" | "room" | "indoor" => "interior".into(),
        "landscape" | "nature" | "sky" => "landscape".into(),
        "object" => "object".into(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shot(start: f64, end: f64) -> ShotLook {
        ShotLook {
            start,
            end,
            look: "wide".into(),
            subject: String::new(),
            motion: 0.0,
            card: None,
        }
    }

    fn still(index: usize) -> ShotStill {
        ShotStill {
            index,
            caption: format!("Shot {index}"),
            jpeg: Vec::new(),
        }
    }

    #[test]
    fn a_fenced_reply_fills_scale_subject_and_motion() {
        let text = "```json\n[{\"i\":0,\"scale\":\"CU\",\"camera\":\"handheld\",\"motion_dir\":\"left to right\",\"subject\":\"person\",\"action\":\"talks to camera\",\"mood\":\"calm\",\"palette\":[\"#112233\"],\"quality\":8,\"best\":0.25}]\n```";
        let mut shots = vec![shot(10.0, 14.0)];
        apply_shot_reply(&mut shots, &[still(0)], text).unwrap();
        assert_eq!(shots[0].subject, "person");
        let card = shots[0].card.as_ref().unwrap();
        assert_eq!(card.scale, "close");
        assert_eq!(card.motion_dir, "l2r");
        assert_eq!(card.quality, 8);
        assert!((card.best_moment - 11.0).abs() < 1e-6);
        assert!(card.grade.is_none());
    }

    #[test]
    fn a_label_keeps_the_grade_the_model_chose_for_that_frame() {
        let text =
            r#"[{"i":0,"scale":"wide","grade":{"exposure":0.2,"saturation":"0.1","lut":"cool"}}]"#;
        let mut shots = vec![shot(0.0, 4.0)];
        apply_shot_reply(&mut shots, &[still(0)], text).unwrap();
        let grade = shots[0].card.as_ref().unwrap().grade.unwrap();
        assert_eq!(grade.lut, oc_timeline::Lut::Cool);
        assert!((grade.exposure - 0.2).abs() < 1e-4);
        assert!((grade.saturation - 0.1).abs() < 1e-4);
        assert!(grade.contrast.abs() < 1e-4);
    }

    #[test]
    fn grades_follow_the_frame_index_and_a_split_palette_name() {
        let text = r#"[{"i":1,"grade":{"lut":"teal-orange","saturation":2.5}},{"i":0,"grade":{"lut":"warm","exposure":-0.4}}]"#;
        let grades = grades_from_reply(text).unwrap();
        assert_eq!(grades[0].0, 1);
        assert_eq!(grades[0].1.lut, oc_timeline::Lut::TealOrange);
        assert!((grades[0].1.saturation - 1.0).abs() < 1e-4);
        assert_eq!(grades[1].1.lut, oc_timeline::Lut::Warm);
        assert!((grades[1].1.exposure + 0.4).abs() < 1e-4);
    }

    #[test]
    fn the_grade_comes_from_the_shot_the_range_sits_on() {
        let mut dark = shot(0.0, 4.0);
        dark.card = Some(ShotCard {
            grade: Some(oc_timeline::Grade {
                exposure: 0.2,
                lut: oc_timeline::Lut::Cool,
                ..oc_timeline::Grade::default()
            }),
            ..ShotCard::default()
        });
        let mut day = shot(4.0, 8.0);
        day.card = Some(ShotCard {
            grade: Some(oc_timeline::Grade {
                lut: oc_timeline::Lut::Warm,
                ..oc_timeline::Grade::default()
            }),
            ..ShotCard::default()
        });
        let plain = shot(8.0, 12.0);
        let shots = vec![dark, day, plain];
        let cool = watched_grade(&shots, 1.0, 3.0).unwrap();
        assert_eq!(cool.lut, oc_timeline::Lut::Cool);
        let warm = watched_grade(&shots, 5.0, 7.0).unwrap();
        assert_eq!(warm.lut, oc_timeline::Lut::Warm);
        assert!(watched_grade(&shots, 9.0, 11.0).is_none());
    }

    #[test]
    fn a_partial_reply_labels_the_named_shot_only() {
        let text = r#"[{"i":1,"scale":"wide","quality":6}]"#;
        let mut shots = vec![shot(0.0, 2.0), shot(2.0, 5.0)];
        shots[0].look = "close".into();
        apply_shot_reply(&mut shots, &[still(0), still(1)], text).unwrap();
        assert_eq!(shots[1].card.as_ref().unwrap().scale, "wide");
        assert_eq!(shots[0].card.as_ref().unwrap().scale, "close");
    }
}
