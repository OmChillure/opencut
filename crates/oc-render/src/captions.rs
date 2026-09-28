use oc_timeline::{ClipKind, Timeline};
use std::fmt::Write;

#[derive(Clone, Debug, PartialEq)]
pub struct BurnedCue {
    pub start: f64,
    pub end: f64,
    pub text: String,
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
                cues.push((
                    clip.start.as_seconds() + cue.start.as_seconds(),
                    clip.start.as_seconds() + cue.end.as_seconds(),
                    cue.text.clone(),
                ));
            }
        }
    }
    if cues.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    for track in &timeline.tracks {
        if track.kind != oc_timeline::TrackKind::Video || track.muted || track.hidden {
            continue;
        }
        for clip in &track.clips {
            if clip.disabled || !matches!(clip.kind, ClipKind::Video { .. }) {
                continue;
            }
            let tl0 = clip.start.as_seconds();
            let tl1 = clip.end().as_seconds();
            let src0 = clip.source_in.as_seconds();
            let src1 = src0 + clip.duration.as_seconds() * f64::from(clip.speed.max(0.01));
            for (c0, c1, text) in &cues {
                let (c0, c1) = (*c0, *c1);
                if c1 > tl0 + 0.05 && c0 < tl1 - 0.05 {
                    let a = c0.max(tl0);
                    let b = c1.min(tl1);
                    if b - a >= 0.05 {
                        out.push(BurnedCue {
                            start: a,
                            end: b,
                            text: text.clone(),
                        });
                        continue;
                    }
                }
                let a = c0.max(src0);
                let b = c1.min(src1);
                if b - a < 0.05 {
                    continue;
                }
                out.push(BurnedCue {
                    start: clip.start.as_seconds() + (a - src0),
                    end: clip.start.as_seconds() + (b - src0),
                    text: text.clone(),
                });
            }
        }
    }
    out.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));
    out.dedup_by(|a, b| (a.start - b.start).abs() < 0.05 && a.text == b.text);
    shorten_cues(out)
}

/// A caption stays two short lines. A full sentence is split across the cue.
fn shorten_cues(cues: Vec<BurnedCue>) -> Vec<BurnedCue> {
    let mut out = Vec::new();
    for cue in cues {
        let words: Vec<&str> = cue.text.split_whitespace().collect();
        if words.len() <= 7 {
            out.push(cue);
            continue;
        }
        let groups: Vec<&[&str]> = words.chunks(6).collect();
        let span = (cue.end - cue.start).max(0.4);
        let step = span / groups.len() as f64;
        for (i, group) in groups.iter().enumerate() {
            let i = i as f64;
            out.push(BurnedCue {
                start: cue.start + step * i,
                end: cue.start + step * (i + 1.0),
                text: group.join(" "),
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
                        },
                        CaptionCue {
                            start: Time::from_seconds(80.0),
                            end: Time::from_seconds(82.0),
                            text: "too late".into(),
                            speaker: None,
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
}
