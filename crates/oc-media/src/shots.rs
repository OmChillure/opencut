//! Join picture ranges with speech cues into a director shot list.

use crate::vision::ShotLook;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShotRole {
    Speech,
    Silence,
    Filler,
}

impl ShotRole {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Speech => "speech",
            Self::Silence => "silence",
            Self::Filler => "filler",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ShotBrief {
    pub start: f64,
    pub end: f64,
    pub look: String,
    pub subject: String,
    pub role: ShotRole,
    pub text: String,
}

const GAP: f64 = 0.8;
const TEXT_MAX: usize = 160;

/// Picture ranges tiled across the file, split where speech pauses.
/// A cue belongs to the range that contains its midpoint.
pub fn brief_shots(picture: &[ShotLook], cues: &[(f64, f64, &str)]) -> Vec<ShotBrief> {
    let mut out = Vec::new();
    let last = picture.len().saturating_sub(1);
    for (i, shot) in picture.iter().enumerate() {
        if shot.end - shot.start < 0.05 {
            continue;
        }
        let inside: Vec<(f64, f64, &str)> = cues
            .iter()
            .copied()
            .filter(|(s, e, _)| {
                let mid = (s + e) * 0.5;
                mid >= shot.start - 0.001 && (mid < shot.end - 0.001 || (i == last && mid <= shot.end + 0.001))
            })
            .collect();
        if inside.is_empty() {
            push_silence(&mut out, shot.start, shot.end, &shot.look, &shot.subject);
            continue;
        }
        let mut groups: Vec<Vec<(f64, f64, &str)>> = Vec::new();
        for cue in inside {
            if let Some(g) = groups.last_mut() {
                let prev_end = g.last().map(|c| c.1).unwrap_or(cue.0);
                if cue.0 - prev_end < GAP {
                    g.push(cue);
                    continue;
                }
            }
            groups.push(vec![cue]);
        }
        let mut cursor = shot.start;
        for g in &groups {
            let beat_start = g.first().map(|c| c.0).unwrap_or(cursor).max(shot.start);
            let beat_end = g.last().map(|c| c.1).unwrap_or(beat_start).min(shot.end);
            if beat_start - cursor >= GAP {
                push_silence(&mut out, cursor, beat_start, &shot.look, &shot.subject);
            }
            let texts: Vec<&str> = g.iter().map(|c| c.2).collect();
            let speech: Vec<&str> = texts.iter().copied().filter(|t| !is_filler(t)).collect();
            let (role, body) = if speech.is_empty() {
                (ShotRole::Filler, texts.join(" "))
            } else {
                (ShotRole::Speech, speech.join(" "))
            };
            let start = beat_start.min(beat_end);
            let end = beat_end.max(start + 0.05).min(shot.end);
            if end - start >= 0.15 {
                out.push(ShotBrief {
                    start,
                    end,
                    look: shot.look.clone(),
                    subject: shot.subject.clone(),
                    role,
                    text: clip_text(&body),
                });
            }
            cursor = end;
        }
        if shot.end - cursor >= GAP {
            push_silence(&mut out, cursor, shot.end, &shot.look, &shot.subject);
        }
    }
    out
}

pub fn format_shot_list(shots: &[ShotBrief], limit: usize) -> String {
    if shots.is_empty() {
        return String::new();
    }
    let shown = shots.len().min(limit);
    let mut out = format!("shots {}\n", shots.len());
    for shot in shots.iter().take(shown) {
        out.push_str(&format!("{:.1}-{:.1}  {}", shot.start, shot.end, shot.look));
        if !shot.subject.is_empty() {
            out.push(' ');
            out.push_str(&shot.subject);
        }
        out.push(' ');
        out.push_str(shot.role.as_str());
        if !shot.text.is_empty() && shot.role != ShotRole::Silence {
            out.push_str("  ");
            out.push_str(&shot.text);
        }
        out.push('\n');
    }
    if shots.len() > shown {
        out.push_str(&format!("… {} more shots\n", shots.len() - shown));
    }
    out
}

fn push_silence(out: &mut Vec<ShotBrief>, start: f64, end: f64, look: &str, subject: &str) {
    if end - start < GAP {
        return;
    }
    out.push(ShotBrief {
        start,
        end,
        look: look.to_string(),
        subject: subject.to_string(),
        role: ShotRole::Silence,
        text: String::new(),
    });
}

fn clip_text(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= TEXT_MAX {
        return flat;
    }
    let cut: String = flat.chars().take(TEXT_MAX).collect();
    format!("{cut}…")
}

fn is_filler(text: &str) -> bool {
    let t = text.trim().to_ascii_lowercase();
    if t.is_empty() {
        return true;
    }
    t.split_whitespace().all(|w| {
        matches!(
            w.trim_matches(|c: char| !c.is_alphanumeric()),
            "um" | "uh" | "uhm" | "hmm" | "mm" | "yeah" | "yep" | "ok" | "okay" | "so" | "like"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shot(start: f64, end: f64, look: &str) -> ShotLook {
        ShotLook {
            start,
            end,
            look: look.into(),
            subject: String::new(),
            motion: 0.0,
            card: None,
        }
    }

    #[test]
    fn splits_speech_silence_and_filler_inside_one_look() {
        let picture = vec![shot(0.0, 20.0, "close")];
        let cues = [
            (1.0, 3.0, "what if we just left tonight"),
            (4.2, 4.8, "um"),
            (8.0, 10.0, "the train was already gone"),
        ];
        let shots = brief_shots(&picture, &cues);
        assert!(shots.iter().any(|s| s.role == ShotRole::Speech && s.text.contains("left tonight")));
        assert!(shots.iter().any(|s| s.role == ShotRole::Filler));
        assert!(shots.iter().any(|s| s.role == ShotRole::Silence && s.start >= 4.0 && s.end <= 8.1));
        assert!(shots.iter().all(|s| s.look == "close"));
    }

    #[test]
    fn silent_range_is_one_silence_shot() {
        let picture = vec![shot(0.0, 6.0, "wide"), shot(6.0, 9.0, "action")];
        let shots = brief_shots(&picture, &[]);
        assert_eq!(shots.len(), 2);
        assert_eq!(shots[0].role, ShotRole::Silence);
        assert_eq!(shots[1].look, "action");
    }

    #[test]
    fn cue_midpoint_stays_in_its_scene() {
        let picture = vec![shot(0.0, 5.0, "wide"), shot(5.0, 10.0, "close")];
        let cues = [(4.0, 4.8, "wide line"), (6.0, 7.0, "close line")];
        let shots = brief_shots(&picture, &cues);
        let wide = shots.iter().find(|s| s.text.contains("wide")).unwrap();
        let close = shots.iter().find(|s| s.text.contains("close")).unwrap();
        assert_eq!(wide.look, "wide");
        assert_eq!(close.look, "close");
    }
}
