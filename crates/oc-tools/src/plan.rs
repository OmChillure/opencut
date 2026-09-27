//! Turn a model plan into timeline ops. The model does not place each clip itself.

use crate::ops::{apply, Op};
use oc_time::{Duration, Time};
use oc_timeline::{
    ClipKind, EditPlan, EditSlot, MediaId, Timeline, TrackKind, TransitionKind, UndoStack,
};

#[derive(Clone, Debug)]
pub struct SourceWindow {
    pub media: MediaId,
    pub start: f64,
    pub end: f64,
    pub duration: f64,
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
    if !plan.grade.is_identity() {
        let styled = apply(
            timeline,
            &mut undo,
            Op::StyleClips {
                clip_ids: Vec::new(),
                all: true,
                grade: Some(plan.grade),
                fx: None,
                transition: None,
                transition_seconds: None,
            },
        )
        .map_err(err)?;
        notes.push(styled.note);
    }
    for (id, slot) in &ids {
        if let Some(speed) = slot.speed {
            let _ = apply(
                timeline,
                &mut undo,
                Op::SetSpeed {
                    clip_id: *id,
                    speed,
                },
            );
        }
        if let Some(scale) = slot.end_scale {
            let _ = apply(
                timeline,
                &mut undo,
                Op::SetMove {
                    clip_id: *id,
                    end_x: 0.0,
                    end_y: 0.0,
                    end_scale: scale,
                    ease: slot.ease.unwrap_or_default(),
                },
            );
        }
        if let Some(kind) = slot.transition.as_deref() {
            let _ = apply(
                timeline,
                &mut undo,
                Op::SetTransition {
                    clip_id: *id,
                    kind: TransitionKind::from_key(kind),
                    duration: slot.transition_duration,
                },
            );
        }
    }
    if plan.music_id.is_some() {
        let _ = apply(timeline, &mut undo, Op::Duck { amount: 0.65 });
        notes.push("ducked music under speech".into());
    }
    timeline.edit_plan = Some(plan.clone());
    Ok(notes)
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
