use oc_timeline::{CaptionEffect, CaptionFont, CaptionPlace, ClipKind, Timeline};
use std::fmt::Write;

#[derive(Clone, Debug, PartialEq)]
pub struct BurnedCue {
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub place: CaptionPlace,
    pub font: CaptionFont,
    pub effect: CaptionEffect,
}

/// Map transcript-style cues (source seconds) onto the *cut*.
///
/// Whisper writes source times. After assemble, picture clips only keep a
/// slice (`source_in` + duration). We emit a cue only where it overlaps that
/// slice, shifted onto the timeline.
#[must_use]
pub fn captions_for_cut(timeline: &Timeline) -> Vec<BurnedCue> {
    let mut cues = Vec::new();
    for track in &timeline.tracks {
        if track.muted || track.hidden {
            continue;
        }
        for clip in &track.clips {
            if clip.disabled {
                continue;
            }
            let ClipKind::Caption { cues: raw, .. } = &clip.kind else {
                continue;
            };
            for cue in raw {
                cues.push(BurnedCue {
                    start: clip.start.as_seconds() + cue.start.as_seconds(),
                    end: clip.start.as_seconds() + cue.end.as_seconds(),
                    text: cue.text.clone(),
                    place: cue.place,
                    font: cue.font,
                    effect: cue.effect,
                });
            }
        }
    }
    if cues.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut placed = vec![false; cues.len()];
    let program: Vec<&oc_timeline::Clip> = timeline
        .tracks
        .iter()
        .filter(|track| {
            track.kind == oc_timeline::TrackKind::Video
                && !track.muted
                && !track.hidden
                // Design stills and the corner window are overlays.
                && !matches!(track.name.as_str(), "Design" | "Front" | "GFX")
        })
        .flat_map(|track| track.clips.iter())
        .filter(|clip| !clip.disabled && matches!(clip.kind, ClipKind::Video { .. }))
        .collect();
    for clip in &program {
        let tl0 = clip.start.as_seconds();
        let tl1 = clip.end().as_seconds();
        for (i, cue) in cues.iter().enumerate() {
            if cue.end <= tl0 + 0.05 || cue.start >= tl1 - 0.05 {
                continue;
            }
            let a = cue.start.max(tl0);
            let b = cue.end.min(tl1);
            if b - a < 0.05 {
                continue;
            }
            out.push(BurnedCue {
                start: a,
                end: b,
                text: cue.text.clone(),
                place: cue.place,
                font: cue.font,
                effect: cue.effect,
            });
            placed[i] = true;
        }
    }
    // Cues still in source time (not already on the cut) map through the excerpt.
    for clip in &program {
        let src0 = clip.source_in.as_seconds();
        let src1 = src0 + clip.duration.as_seconds() * f64::from(clip.speed.max(0.01));
        for (i, cue) in cues.iter().enumerate() {
            if placed[i] {
                continue;
            }
            let a = cue.start.max(src0);
            let b = cue.end.min(src1);
            if b - a < 0.05 {
                continue;
            }
            out.push(BurnedCue {
                start: clip.start.as_seconds() + (a - src0),
                end: clip.start.as_seconds() + (b - src0),
                text: cue.text.clone(),
                place: cue.place,
                font: cue.font,
                effect: cue.effect,
            });
        }
    }
    out.sort_by(|a, b| {
        a.start
            .partial_cmp(&b.start)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out.dedup_by(|a, b| (a.start - b.start).abs() < 0.05 && a.text == b.text);
    shorten_cues(out)
}

/// A caption stays a short line. A long line is split into beats across the cue.
fn shorten_cues(cues: Vec<BurnedCue>) -> Vec<BurnedCue> {
    let mut out = Vec::new();
    for cue in cues {
        let span = (cue.end - cue.start).max(0.4);
        let beats = oc_timeline::caption_beats(&cue.text, span);
        if beats.len() <= 1 {
            let mut cue = cue;
            if let Some((text, _, _)) = beats.first() {
                if !text.is_empty() {
                    cue.text = text.clone();
                }
            }
            out.push(cue);
            continue;
        }
        for (text, start, end) in beats {
            out.push(BurnedCue {
                start: cue.start + span * start,
                end: cue.start + span * end,
                text,
                place: cue.place,
                font: cue.font,
                effect: cue.effect,
            });
        }
    }
    out
}

#[must_use]
pub fn to_srt(cues: &[BurnedCue]) -> String {
    let mut out = String::new();
    for (i, cue) in cues.iter().enumerate() {
        let _ = write!(
            out,
            "{}\n{} --> {}\n{}\n\n",
            i + 1,
            srt_time(cue.start),
            srt_time(cue.end),
            cue.text.trim()
        );
    }
    out
}

/// ASS with PlayRes equal to the frame, so Fontsize is in real pixels.
/// An SRT burned through libass uses a 288-line script and blows the type up.
/// Each line carries its own anchor, face, and effect.
#[must_use]
pub fn to_ass(cues: &[BurnedCue], width: u32, height: u32, letterbox: bool) -> String {
    let short = width.min(height).max(1);
    let font = (short / 22).clamp(36, 58);
    let margin_v = if letterbox {
        ((height as f32) * 0.12).round() as u32 + font
    } else {
        (height / 9).max(font * 2)
    };
    let mut out = format!(
        "[Script Info]\n\
         ScriptType: v4.00+\n\
         PlayResX: {width}\n\
         PlayResY: {height}\n\
         WrapStyle: 0\n\
         ScaledBorderAndShadow: yes\n\
         \n\
         [V4+ Styles]\n\
         Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
         Style: Default,DejaVu Sans,{font},&H00FFFFFF,&H000000FF,&H00000000,&H64000000,-1,0,0,0,100,100,0,0,1,3,0,2,72,72,{margin_v},1\n\
         \n\
         [Events]\n\
         Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n"
    );
    for cue in cues {
        let (x, y, an) = anchor(cue.place, width, height, letterbox);
        let effect = effect_prefix(cue.effect);
        let _ = writeln!(
            out,
            "Dialogue: 0,{},{},Default,,0,0,0,,{{\\an{an}\\pos({x},{y}){}{effect}}}{}",
            ass_time(cue.start),
            ass_time(cue.end),
            font_override(cue.font, short),
            dialogue_text(cue)
        );
    }
    out
}

fn anchor(place: CaptionPlace, width: u32, height: u32, letterbox: bool) -> (u32, u32, u8) {
    let h = u64::from(height.max(1));
    let y_num = match place {
        CaptionPlace::Top => 14,
        CaptionPlace::Middle => 46,
        CaptionPlace::Lower => 64,
        CaptionPlace::Bottom => {
            if letterbox {
                76
            } else {
                84
            }
        }
    };
    let y = (h * y_num / 100).clamp(48, h.saturating_sub(48)) as u32;
    let an = match place {
        CaptionPlace::Top => 8,
        CaptionPlace::Middle | CaptionPlace::Lower => 5,
        CaptionPlace::Bottom => 2,
    };
    (width / 2, y, an)
}

fn font_override(font: CaptionFont, short: u32) -> String {
    match font {
        CaptionFont::Sans => {
            let size = (short / 22).clamp(36, 58);
            format!("\\fnDejaVu Sans\\fs{size}\\b1")
        }
        CaptionFont::Display => {
            // A heavier band of type, still white, still inside the frame.
            let size = (short / 28).clamp(32, 52);
            format!("\\fnDejaVu Sans\\fs{size}\\b1")
        }
        CaptionFont::Serif => {
            let size = (short / 22).clamp(36, 58);
            format!("\\fnDejaVu Serif\\fs{size}\\i1")
        }
        CaptionFont::Mono => {
            let size = (short / 24).clamp(32, 52);
            format!("\\fnDejaVu Sans Mono\\fs{size}")
        }
    }
}

fn effect_prefix(effect: CaptionEffect) -> &'static str {
    match effect {
        CaptionEffect::Pop => "\\fscx62\\fscy62\\t(0,160,\\fscx100\\fscy100)\\fad(30,0)",
        CaptionEffect::Fade => "\\fad(90,340)",
        CaptionEffect::Typewriter | CaptionEffect::None => "",
    }
}

fn dialogue_text(cue: &BurnedCue) -> String {
    let body = cue.text.trim().to_string();
    if cue.effect == CaptionEffect::Typewriter {
        type_on(&body, (cue.end - cue.start).max(0.2))
    } else {
        ass_escape(&body)
    }
}

fn type_on(text: &str, duration: f64) -> String {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len().max(1) as f64;
    let window = (duration * 0.7).clamp(0.2, duration.max(0.2));
    let step = (window / n).clamp(0.03, 0.12);
    let mut out = String::new();
    for (i, ch) in chars.iter().enumerate() {
        let t0 = (i as f64 * step * 1000.0).round() as u32;
        let t1 = t0 + 40;
        out.push_str(&format!("{{\\alpha&HFF&\\t({t0},{t1},\\alpha&H00&)}}"));
        out.push_str(&escape_char(*ch));
    }
    out
}

fn ass_escape(text: &str) -> String {
    text.chars().map(escape_char).collect()
}

fn escape_char(c: char) -> String {
    match c {
        '\\' => "\\\\".into(),
        '{' => "\\{".into(),
        '}' => "\\}".into(),
        '\n' => "\\N".into(),
        _ => c.to_string(),
    }
}

fn ass_time(secs: f64) -> String {
    let cs = (secs.max(0.0) * 100.0).round() as u64;
    let h = cs / 360_000;
    let m = (cs / 6_000) % 60;
    let s = (cs / 100) % 60;
    let c = cs % 100;
    format!("{h}:{m:02}:{s:02}.{c:02}")
}

fn srt_time(secs: f64) -> String {
    let ms_total = (secs.max(0.0) * 1000.0).round() as u64;
    let h = ms_total / 3_600_000;
    let m = (ms_total / 60_000) % 60;
    let s = (ms_total / 1000) % 60;
    let ms = ms_total % 1000;
    format!("{h:02}:{m:02}:{s:02},{ms:03}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ass_type_is_sized_to_the_frame() {
        let ass = to_ass(
            &[BurnedCue {
                start: 0.0,
                end: 1.5,
                text: "something that matters.".into(),
                place: CaptionPlace::Bottom,
                font: CaptionFont::Sans,
                effect: CaptionEffect::None,
            }],
            1080,
            1920,
            false,
        );
        assert!(ass.contains("PlayResX: 1080\nPlayResY: 1920"), "{ass}");
        assert!(
            ass.contains("{\\an2\\pos(540,1612)\\fnDejaVu Sans\\fs49\\b1}something that matters."),
            "{ass}"
        );
    }

    #[test]
    fn a_middle_punch_pops_and_a_line_can_type_on_or_vanish() {
        let pop = to_ass(
            &[BurnedCue {
                start: 0.0,
                end: 1.2,
                text: "go".into(),
                place: CaptionPlace::Middle,
                font: CaptionFont::Display,
                effect: CaptionEffect::Pop,
            }],
            1080,
            1920,
            false,
        );
        assert!(pop.contains("\\an5\\pos(540,883)"), "{pop}");
        assert!(pop.contains("\\fs38"), "{pop}");
        assert!(!pop.contains("\\fs90"), "{pop}");
        assert!(pop.contains("\\fscx62\\fscy62"), "{pop}");
        assert!(pop.contains("}go"), "{pop}");
        assert!(!pop.contains("GO"), "{pop}");
        assert!(!pop.contains("&H006AE5FF"), "{pop}");
        let typed = to_ass(
            &[BurnedCue {
                start: 0.0,
                end: 1.0,
                text: "3 steps".into(),
                place: CaptionPlace::Lower,
                font: CaptionFont::Mono,
                effect: CaptionEffect::Typewriter,
            }],
            1080,
            1920,
            false,
        );
        assert!(typed.contains("\\alpha&HFF&"), "{typed}");
        assert!(typed.contains("DejaVu Sans Mono"), "{typed}");
        let vanish = to_ass(
            &[BurnedCue {
                start: 0.0,
                end: 2.0,
                text: "the long way".into(),
                place: CaptionPlace::Bottom,
                font: CaptionFont::Sans,
                effect: CaptionEffect::Fade,
            }],
            1080,
            1920,
            false,
        );
        assert!(vanish.contains("\\fad(90,340)"), "{vanish}");
    }
    use oc_time::{Duration, Time};
    use oc_timeline::{CaptionCue, CaptionStyle, Clip, ClipId, ClipKind, ClipLook, TrackKind};

    #[test]
    fn remaps_source_cues_onto_an_excerpt() {
        let mut tl = oc_timeline::Timeline::default();
        let v = tl.first_track(TrackKind::Video).unwrap().id;
        let mut clip = Clip {
            id: ClipId::new(),
            media_id: Some(oc_timeline::MediaId::new()),
            kind: ClipKind::Video {
                transform: Default::default(),
            },
            start: Time::from_seconds(0.0),
            duration: Duration::from_seconds(4.0),
            source_in: Time::from_seconds(40.0),
            speed: 1.0,
            group_id: None,
            link_id: None,
            disabled: false,
            look: ClipLook::default(),
        };
        tl.add_clip(v, clip.clone()).unwrap();
        clip.id = ClipId::new();
        let cap = tl.add_track(TrackKind::Caption, "Captions");
        tl.add_clip(
            cap,
            Clip {
                id: ClipId::new(),
                media_id: None,
                kind: ClipKind::Caption {
                    style: CaptionStyle::Stacked,
                    cues: vec![
                        CaptionCue {
                            start: Time::from_seconds(40.0),
                            end: Time::from_seconds(43.0),
                            text: "we left".into(),
                            speaker: None,
                            place: Default::default(),
                            font: Default::default(),
                            effect: Default::default(),
                        },
                        CaptionCue {
                            start: Time::from_seconds(80.0),
                            end: Time::from_seconds(82.0),
                            text: "too late".into(),
                            speaker: None,
                            place: Default::default(),
                            font: Default::default(),
                            effect: Default::default(),
                        },
                    ],
                },
                start: Time::ZERO,
                duration: Duration::from_seconds(120.0),
                source_in: Time::ZERO,
                speed: 1.0,
                group_id: None,
                link_id: None,
                disabled: false,
                look: ClipLook::default(),
            },
        )
        .unwrap();
        let burned = captions_for_cut(&tl);
        assert_eq!(burned.len(), 1, "{burned:?}");
        assert!((burned[0].start - 0.0).abs() < 1e-6);
        assert!((burned[0].end - 3.0).abs() < 1e-6);
        assert_eq!(burned[0].text, "we left");
    }

    #[test]
    fn burns_timeline_cues_on_the_cut() {
        let mut tl = oc_timeline::Timeline::default();
        let v = tl.first_track(TrackKind::Video).unwrap().id;
        tl.add_clip(
            v,
            Clip {
                id: ClipId::new(),
                media_id: Some(oc_timeline::MediaId::new()),
                kind: ClipKind::Video {
                    transform: Default::default(),
                },
                start: Time::from_seconds(0.0),
                duration: Duration::from_seconds(6.0),
                source_in: Time::from_seconds(40.0),
                speed: 1.0,
                group_id: None,
                link_id: None,
                disabled: false,
                look: ClipLook::default(),
            },
        )
        .unwrap();
        let cap = tl.add_track(TrackKind::Caption, "Captions");
        tl.add_clip(
            cap,
            Clip {
                id: ClipId::new(),
                media_id: None,
                kind: ClipKind::Caption {
                    style: CaptionStyle::Stacked,
                    cues: vec![CaptionCue {
                        start: Time::from_seconds(1.0),
                        end: Time::from_seconds(3.0),
                        text: "on the cut".into(),
                        speaker: None,
                        place: Default::default(),
                        font: Default::default(),
                        effect: Default::default(),
                    }],
                },
                start: Time::ZERO,
                duration: Duration::from_seconds(6.0),
                source_in: Time::ZERO,
                speed: 1.0,
                group_id: None,
                link_id: None,
                disabled: false,
                look: ClipLook::default(),
            },
        )
        .unwrap();
        let burned = captions_for_cut(&tl);
        assert_eq!(burned.len(), 1, "{burned:?}");
        assert!((burned[0].start - 1.0).abs() < 1e-6);
        assert!((burned[0].end - 3.0).abs() < 1e-6);
        assert_eq!(burned[0].text, "on the cut");
    }

    #[test]
    fn a_design_still_does_not_replay_the_opening_line() {
        let mut tl = oc_timeline::Timeline::default();
        let v = tl.first_track(TrackKind::Video).unwrap().id;
        tl.add_clip(
            v,
            Clip {
                id: ClipId::new(),
                media_id: Some(oc_timeline::MediaId::new()),
                kind: ClipKind::Video {
                    transform: Default::default(),
                },
                start: Time::ZERO,
                duration: Duration::from_seconds(8.0),
                source_in: Time::from_seconds(40.0),
                speed: 1.0,
                group_id: None,
                link_id: None,
                disabled: false,
                look: ClipLook::default(),
            },
        )
        .unwrap();
        let design = tl.add_track(TrackKind::Video, "Design");
        tl.add_clip(
            design,
            Clip {
                id: ClipId::new(),
                media_id: Some(oc_timeline::MediaId::new()),
                kind: ClipKind::Video {
                    transform: Default::default(),
                },
                start: Time::from_seconds(2.0),
                duration: Duration::from_seconds(4.0),
                source_in: Time::ZERO,
                speed: 1.0,
                group_id: None,
                link_id: None,
                disabled: false,
                look: ClipLook::default(),
            },
        )
        .unwrap();
        let cap = tl.add_track(TrackKind::Caption, "Captions");
        tl.add_clip(
            cap,
            Clip {
                id: ClipId::new(),
                media_id: None,
                kind: ClipKind::Caption {
                    style: CaptionStyle::Stacked,
                    cues: vec![CaptionCue {
                        start: Time::ZERO,
                        end: Time::from_seconds(2.0),
                        text: "learn the patterns".into(),
                        speaker: None,
                        place: Default::default(),
                        font: Default::default(),
                        effect: Default::default(),
                    }],
                },
                start: Time::ZERO,
                duration: Duration::from_seconds(8.0),
                source_in: Time::ZERO,
                speed: 1.0,
                group_id: None,
                link_id: None,
                disabled: false,
                look: ClipLook::default(),
            },
        )
        .unwrap();
        let burned = captions_for_cut(&tl);
        assert_eq!(burned.len(), 1, "{burned:?}");
        assert!((burned[0].start - 0.0).abs() < 1e-6);
        assert!((burned[0].end - 2.0).abs() < 1e-6);
    }

    #[test]
    fn a_later_line_is_not_replayed_inside_an_earlier_source_range() {
        // The flag excerpt is timeline 5–22.3 from source 56.5. The neckline
        // line is already placed at 52.2–58.2. Those numbers also sit inside
        // the flag's source range, so a second source pass would burn the
        // ending words over the flag.
        let mut tl = oc_timeline::Timeline::default();
        let v = tl.first_track(TrackKind::Video).unwrap().id;
        tl.add_clip(
            v,
            Clip {
                id: ClipId::new(),
                media_id: Some(oc_timeline::MediaId::new()),
                kind: ClipKind::Video {
                    transform: Default::default(),
                },
                start: Time::from_seconds(5.0),
                duration: Duration::from_seconds(17.3),
                source_in: Time::from_seconds(56.5),
                speed: 1.0,
                group_id: None,
                link_id: None,
                disabled: false,
                look: ClipLook::default(),
            },
        )
        .unwrap();
        tl.add_clip(
            v,
            Clip {
                id: ClipId::new(),
                media_id: Some(oc_timeline::MediaId::new()),
                kind: ClipKind::Video {
                    transform: Default::default(),
                },
                start: Time::from_seconds(52.2),
                duration: Duration::from_seconds(6.0),
                source_in: Time::from_seconds(355.5),
                speed: 1.0,
                group_id: None,
                link_id: None,
                disabled: false,
                look: ClipLook::default(),
            },
        )
        .unwrap();
        let cap = tl.add_track(TrackKind::Caption, "Captions");
        tl.add_clip(
            cap,
            Clip {
                id: ClipId::new(),
                media_id: None,
                kind: ClipKind::Caption {
                    style: CaptionStyle::Stacked,
                    cues: vec![
                        CaptionCue {
                            start: Time::from_seconds(5.0),
                            end: Time::from_seconds(7.2),
                            text: "be a bullish flag".into(),
                            speaker: None,
                            place: Default::default(),
                            font: Default::default(),
                            effect: Default::default(),
                        },
                        CaptionCue {
                            start: Time::from_seconds(52.2),
                            end: Time::from_seconds(58.2),
                            text: "is the important level though as".into(),
                            speaker: None,
                            place: Default::default(),
                            font: Default::default(),
                            effect: Default::default(),
                        },
                    ],
                },
                start: Time::ZERO,
                duration: Duration::from_seconds(58.2),
                source_in: Time::ZERO,
                speed: 1.0,
                group_id: None,
                link_id: None,
                disabled: false,
                look: ClipLook::default(),
            },
        )
        .unwrap();
        let burned = captions_for_cut(&tl);
        assert_eq!(
            burned.iter().filter(|cue| cue.text == "be a bullish flag").count(),
            1,
            "{burned:?}"
        );
        assert_eq!(burned[0].text, "be a bullish flag");
        assert!((burned[0].start - 5.0).abs() < 1e-6);
        assert_eq!(burned[1].text, "is the important level");
        assert!((burned[1].start - 52.2).abs() < 1e-6);
        assert_eq!(burned.last().unwrap().text, "though as");
        assert!(burned.last().unwrap().start > 52.2);
        assert!((burned.last().unwrap().end - 58.2).abs() < 1e-3);
    }
}
