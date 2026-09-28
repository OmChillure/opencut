//! Turn a model plan into timeline ops. The model does not place each clip itself.
//! Rust then grades, covers jumps, lays music, and writes captions.

use crate::finish::{self, CoverShot, SpokenLine};
use crate::ops::{apply, Op};
use oc_time::{Duration, Time};
use oc_timeline::{
    AspectRatio, CaptionStyle, ClipKind, EditPlan, EditSlot, MediaId,
    Timeline, TrackKind, TransitionKind, UndoStack,
};

#[derive(Clone, Debug)]
pub struct SourceWindow {
    pub media: MediaId,
    pub start: f64,
    pub end: f64,
    pub duration: f64,
    /// Shot look from the analysis (`dark`, `wide`, `close`, …). Empty when unknown.
    pub look: String,
    /// True when this window has no speech. Used to cover a jump.
    pub silent: bool,
}

pub fn plan_from_value(value: &serde_json::Value) -> Result<EditPlan, String> {
    let mut value = value.clone();
    if let Some(obj) = value.as_object_mut() {
        for key in ["slots", "grade", "changes"] {
            let Some(raw) = obj.get(key).and_then(|v| v.as_str()) else {
                continue;
            };
            let parsed: serde_json::Value = serde_json::from_str(raw)
                .map_err(|e| format!("submit_edit {key}: {e}"))?;
            obj.insert(key.to_string(), parsed);
        }
    }
    serde_json::from_value(value).map_err(|e| format!("submit_edit: {e}"))
}

pub fn build_plan(
    timeline: &mut Timeline,
    plan: &EditPlan,
    windows: &[SourceWindow],
    beats: &[f64],
    lines: &[SpokenLine],
    covers: &[CoverShot],
) -> Result<Vec<String>, String> {
    check_slots(&plan.slots, windows)?;
    let mut undo = UndoStack::new();
    let mut notes = Vec::new();
    let cleared = apply(timeline, &mut undo, Op::ClearTimeline).map_err(err)?;
    notes.push(cleared.note);
    let mut at = 0.0;
    let mut ids = Vec::new();
    for slot in &plan.slots {
        let placed = apply(
            timeline,
            &mut undo,
            Op::PlaceMedia {
                media_id: slot.media_id,
                track_id: None,
                start: Time::from_seconds(at),
                duration: Duration::from_seconds(slot.duration.max(0.2)),
                source_in: Time::from_seconds(slot.source_in.max(0.0)),
                kind: TrackKind::Video,
                mode: crate::ops::TimelineEditMode::Normal,
            },
        )
        .map_err(err)?;
        notes.push(placed.note.clone());
        if let Some(id) = last_video(timeline) {
            ids.push((id, slot.clone()));
        }
        at += slot.duration.max(0.2);
    }
    if !beats.is_empty() {
        let snapped = apply(
            timeline,
            &mut undo,
            Op::SnapCuts {
                tolerance_frames: 2,
                beats: beats.to_vec(),
            },
        )
        .map_err(err)?;
        notes.push(snapped.note);
    }
    for (id, slot) in &ids {
        if let Some(speed) = slot.speed {
            apply(
                timeline,
                &mut undo,
                Op::SetSpeed {
                    clip_id: *id,
                    speed,
                },
            )
            .map_err(err)?;
        }
        if let Some(scale) = slot.end_scale {
            apply(
                timeline,
                &mut undo,
                Op::SetMove {
                    clip_id: *id,
                    end_x: 0.0,
                    end_y: 0.0,
                    end_scale: scale,
                    ease: slot.ease.unwrap_or_default(),
                },
            )
            .map_err(err)?;
        }
        if let Some(kind) = slot.transition.as_deref() {
            apply(
                timeline,
                &mut undo,
                Op::SetTransition {
                    clip_id: *id,
                    kind: TransitionKind::from_key(kind),
                    duration: slot.transition_duration,
                },
            )
            .map_err(err)?;
        }
    }
    notes.extend(finish_picture(timeline, &mut undo, plan, &ids, lines, covers)?);
    timeline.edit_plan = Some(plan.clone());
    Ok(notes)
}

fn finish_picture(
    timeline: &mut Timeline,
    undo: &mut UndoStack,
    plan: &EditPlan,
    ids: &[(oc_timeline::ClipId, EditSlot)],
    lines: &[SpokenLine],
    covers: &[CoverShot],
) -> Result<Vec<String>, String> {
    let mut notes = Vec::new();
    let mut graded = 0;
    for (id, slot) in ids {
        let grade = slot.grade.unwrap_or(plan.grade);
        if !grade.is_identity() {
            apply(timeline, undo, Op::SetGrade { clip_id: *id, grade }).map_err(err)?;
            graded += 1;
        }
        if let Some(fx) = slot.fx {
            if !fx.is_identity() {
                apply(timeline, undo, Op::SetFx { clip_id: *id, fx }).map_err(err)?;
            }
        }
        let fade_in = slot.fade_in.unwrap_or(0.0);
        let fade_out = slot.fade_out.unwrap_or(0.0);
        if fade_in > 0.0 || fade_out > 0.0 {
            apply(
                timeline,
                undo,
                Op::SetFade {
                    clip_id: *id,
                    fade_in: Duration::from_seconds(fade_in),
                    fade_out: Duration::from_seconds(fade_out),
                },
            )
            .map_err(err)?;
        }
    }
    if graded > 0 {
        notes.push(format!("graded {graded} clips from the plan"));
    }

    let mut covered = 0;
    for (index, (id, slot)) in ids.iter().enumerate().skip(1) {
        if slot.cover != Some(true) {
            continue;
        }
        let Some((_, clip)) = timeline.find_clip(*id) else { continue };
        let clip = clip.clone();
        let Some((_, prev)) = timeline.find_clip(ids[index - 1].0) else { continue };
        let prev = prev.clone();
        let Some(cover) = finish::pick_cover(covers, &prev, &clip) else {
            notes.push(format!("slot {index} asked for a cover and none was free"));
            continue;
        };
        let dur = 0.9_f64.min(cover.end - cover.start);
        if dur < 0.4 {
            continue;
        }
        let at = (clip.start.as_seconds() - dur * 0.5).max(prev.start.as_seconds());
        if apply(
            timeline,
            undo,
            Op::Cover {
                media_id: cover.media,
                at: Time::from_seconds(at),
                source_in: Time::from_seconds(cover.start),
                duration: Duration::from_seconds(dur),
            },
        )
        .is_ok()
        {
            covered += 1;
        }
    }
    if covered > 0 {
        notes.push(format!("covered {covered} jump(s)"));
    }

    if let Some(music) = plan.music_id {
        let end = finish::program_clips(timeline)
            .iter()
            .map(|c| c.end().as_seconds())
            .fold(0.0_f64, f64::max);
        if end > 0.2 {
            let placed = apply(
                timeline,
                undo,
                Op::PlaceMedia {
                    media_id: music,
                    track_id: None,
                    start: Time::ZERO,
                    duration: Duration::from_seconds(end),
                    source_in: Time::ZERO,
                    kind: TrackKind::Audio,
                    mode: crate::ops::TimelineEditMode::Normal,
                },
            )
            .map_err(err)?;
            notes.push(placed.note);
            if let Some(volume) = plan.music_volume {
                if let Some(clip) = timeline.tracks.iter().flat_map(|t| t.clips.iter()).rev().find(|c| {
                    c.media_id == Some(music) && matches!(c.kind, ClipKind::Audio { .. })
                }) {
                    let id = clip.id;
                    if let Some(clip) = timeline.clip_mut(id) {
                        if let ClipKind::Audio { volume: level, ducked } = &mut clip.kind {
                            *level = volume.clamp(0.0, 1.0);
                            *ducked = volume < 0.99;
                        }
                    }
                    notes.push(format!("music volume {volume:.2}"));
                }
            }
        }
    }

    if plan.captions || !lines.is_empty() {
        let cues = finish::mapped_cues(&finish::program_clips(timeline), lines);
        if !cues.is_empty() {
            let added = apply(
                timeline,
                undo,
                Op::AddCaptions {
                    style: CaptionStyle::Stacked,
                    cues,
                },
            )
            .map_err(err)?;
            notes.push(added.note);
        }
    }

    if let Some(aspect) = aspect_of(plan) {
        let framed = apply(timeline, undo, Op::Reframe { aspect }).map_err(err)?;
        notes.push(framed.note);
    }
    timeline.letterbox = plan.letterbox;
    if timeline.letterbox {
        notes.push("letterbox on".into());
    }
    Ok(notes)
}

fn aspect_of(plan: &EditPlan) -> Option<AspectRatio> {
    match plan.aspect.to_ascii_lowercase().as_str() {
        "vertical" | "9:16" | "reel" | "portrait" => Some(AspectRatio::Vertical),
        "square" | "1:1" => Some(AspectRatio::Square),
        _ => None,
    }
}

pub fn revise_plan(plan: &EditPlan, changes: &[serde_json::Value]) -> Result<EditPlan, String> {
    let mut next = plan.clone();
    for change in changes {
        let index = change
            .get("slot")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| "revise_edit change needs slot".to_string())? as usize;
        let slot = next
            .slots
            .get_mut(index)
            .ok_or_else(|| format!("slot {index} is not in the plan"))?;
        if let Some(v) = change.get("source_in").and_then(|v| v.as_f64()) {
            slot.source_in = v;
        }
        if let Some(v) = change.get("duration").and_then(|v| v.as_f64()) {
            slot.duration = v;
        }
        if let Some(v) = change.get("transition").and_then(|v| v.as_str()) {
            slot.transition = Some(v.to_string());
        }
        if let Some(v) = change.get("transition_duration").and_then(|v| v.as_f64()) {
            slot.transition_duration = Some(v);
        }
        if let Some(v) = change.get("speed").and_then(|v| v.as_f64()) {
            slot.speed = Some(v as f32);
        }
        if let Some(v) = change.get("end_scale").and_then(|v| v.as_f64()) {
            slot.end_scale = Some(v as f32);
        }
        if let Some(v) = change.get("why").and_then(|v| v.as_str()) {
            slot.why = v.to_string();
        }
        if let Some(v) = change.get("grade") {
            if let Ok(grade) = serde_json::from_value(v.clone()) {
                slot.grade = Some(grade);
            }
        }
        if let Some(v) = change.get("fx") {
            if let Ok(fx) = serde_json::from_value(v.clone()) {
                slot.fx = Some(fx);
            }
        }
        if let Some(v) = change.get("fade_in").and_then(|v| v.as_f64()) {
            slot.fade_in = Some(v);
        }
        if let Some(v) = change.get("fade_out").and_then(|v| v.as_f64()) {
            slot.fade_out = Some(v);
        }
        if let Some(v) = change.get("cover").and_then(|v| v.as_bool()) {
            slot.cover = Some(v);
        }
        if let Some(v) = change.get("music_volume").and_then(|v| v.as_f64()) {
            next.music_volume = Some(v as f32);
        }
        if let Some(v) = change.get("captions").and_then(|v| v.as_bool()) {
            next.captions = v;
        }
        if let Some(v) = change.get("letterbox").and_then(|v| v.as_bool()) {
            next.letterbox = v;
        }
        if let Some(media) = change.get("media_id").and_then(|v| v.as_str()) {
            let id = uuid::Uuid::parse_str(media).map_err(|_| format!("bad media id {media}"))?;
            slot.media_id = MediaId::from_uuid(id);
        }
    }
    Ok(next)
}

fn check_slots(slots: &[EditSlot], windows: &[SourceWindow]) -> Result<(), String> {
    if slots.is_empty() {
        return Err("submit_edit needs at least one slot".into());
    }
    let mut bad = Vec::new();
    for (index, slot) in slots.iter().enumerate() {
        let rows: Vec<_> = windows.iter().filter(|w| w.media == slot.media_id).collect();
        if rows.is_empty() {
            bad.push(format!("slot {index} uses a file that is not in the bin"));
            continue;
        }
        let file_end = rows.iter().map(|w| w.duration).fold(0.0, f64::max);
        if slot.source_in < -0.05 || slot.source_in + 0.2 > file_end + 0.2 {
            bad.push(format!(
                "slot {index} source {:.2}s is outside the file ({file_end:.1}s)",
                slot.source_in
            ));
            continue;
        }
        let known_shots = rows.iter().any(|w| w.end > w.start + 0.05);
        if known_shots {
            let inside = rows.iter().any(|w| {
                slot.source_in + 0.05 >= w.start - 0.15 && slot.source_in <= w.end + 0.15
            });
            if !inside {
                bad.push(format!(
                    "slot {index} source {:.2}s is not on a shot or a spoken line",
                    slot.source_in
                ));
            }
        }
    }
    if bad.is_empty() {
        Ok(())
    } else {
        Err(bad.join("\n"))
    }
}

fn last_video(timeline: &Timeline) -> Option<oc_timeline::ClipId> {
    timeline
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Video)
        .flat_map(|t| t.clips.iter())
        .filter(|c| matches!(c.kind, ClipKind::Video { .. }))
        .max_by_key(|c| c.start)
        .map(|c| c.id)
}

fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use oc_timeline::{Grade, Lut};

    fn window(media: MediaId, start: f64, end: f64, look: &str, silent: bool) -> SourceWindow {
        SourceWindow {
            media,
            start,
            end,
            duration: 80.0,
            look: look.into(),
            silent,
        }
    }

    fn slot(media: MediaId, source_in: f64) -> EditSlot {
        EditSlot {
            media_id: media,
            source_in,
            duration: 4.0,
            transition: None,
            transition_duration: None,
            speed: None,
            end_scale: None,
            ease: None,
            grade: None,
            fx: None,
            fade_in: None,
            fade_out: None,
            cover: None,
            why: String::new(),
        }
    }

    #[test]
    fn plan_applies_only_the_grade_mix_and_cover_the_model_set() {
        let picture = MediaId::new();
        let music = MediaId::new();
        let mut timeline = Timeline::default();
        let mut first = slot(picture, 2.0);
        first.grade = Some(Grade {
            exposure: 0.04,
            lut: Lut::Film,
            ..Grade::default()
        });
        first.fade_in = Some(0.4);
        let mut second = slot(picture, 40.0);
        second.transition = Some("dissolve".into());
        second.transition_duration = Some(0.4);
        second.cover = Some(true);
        let plan = EditPlan {
            style: "cinematic".into(),
            aspect: String::new(),
            letterbox: false,
            music_id: Some(music),
            music_volume: Some(0.4),
            captions: true,
            grade: Grade::default(),
            slots: vec![first, second],
        };
        let windows = vec![
            window(picture, 0.0, 10.0, "bright-wide", false),
            window(picture, 12.0, 20.0, "dark", true),
            window(picture, 38.0, 50.0, "close", false),
            window(music, 0.0, 0.0, "", false),
        ];
        let lines = vec![SpokenLine {
            media: picture,
            start: 2.2,
            end: 5.0,
            text: "The city opens up".into(),
        }];
        let covers = vec![CoverShot {
            media: picture,
            start: 12.0,
            end: 16.0,
        }];
        let notes = build_plan(&mut timeline, &plan, &windows, &[], &lines, &covers).unwrap();
        let joined = notes.join("\n");
        assert!(!timeline.letterbox, "{joined}");
        let pictures: Vec<_> = timeline
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Video && t.name != "GFX")
            .flat_map(|t| t.clips.iter())
            .filter(|c| matches!(c.kind, ClipKind::Video { .. }))
            .collect();
        assert_eq!(pictures.len(), 2);
        assert_eq!(pictures[0].look.grade.lut, Lut::Film);
        assert!(pictures[1].look.grade.is_identity());
        assert!(pictures.iter().all(|c| c.look.move_to.is_none()));
        assert!(pictures[0].look.fade_in.as_seconds() > 0.0);
        assert_eq!(pictures[1].look.fade_in.as_seconds(), 0.0);
        assert!(pictures[1].look.transition == TransitionKind::Dissolve);
        let cover = timeline.tracks.iter().any(|t| t.name == "GFX" && !t.clips.is_empty());
        assert!(cover, "{joined}");
        let ducked = timeline.tracks.iter().any(|t| {
            t.kind == TrackKind::Audio
                && t.clips.iter().any(|c| {
                    c.media_id == Some(music)
                        && matches!(c.kind, ClipKind::Audio { volume, ducked: true } if (volume - 0.4).abs() < 0.01)
                })
        });
        assert!(ducked, "{joined}");
        let captioned = timeline.tracks.iter().any(|t| {
            t.clips.iter().any(|c| match &c.kind {
                ClipKind::Caption { cues, .. } => cues.iter().any(|cue| cue.text.contains("city")),
                _ => false,
            })
        });
        assert!(captioned, "{joined}");
    }

    #[test]
    fn speech_is_captioned_even_when_the_plan_omits_the_flag() {
        let picture = MediaId::new();
        let mut timeline = Timeline::default();
        let plan = EditPlan {
            style: String::new(),
            aspect: String::new(),
            letterbox: false,
            music_id: None,
            music_volume: None,
            captions: false,
            grade: Grade::default(),
            slots: vec![slot(picture, 2.0)],
        };
        let lines = vec![SpokenLine {
            media: picture,
            start: 2.2,
            end: 5.0,
            text: "The city opens up".into(),
        }];
        let windows = vec![window(picture, 0.0, 10.0, "talk", false)];
        build_plan(&mut timeline, &plan, &windows, &[], &lines, &[]).unwrap();
        let captioned = timeline.tracks.iter().any(|t| {
            t.clips.iter().any(|c| match &c.kind {
                ClipKind::Caption { cues, .. } => cues.iter().any(|cue| cue.text.contains("city")),
                _ => false,
            })
        });
        assert!(captioned);
    }
}
