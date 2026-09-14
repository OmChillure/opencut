use crate::bind;
use crate::{WorkspaceSave, api};
use dioxus::prelude::*;
use oc_core::{ClipId, Duration, Intent, Op, Time, TrackId, UndoStack, apply, parse_intent};
use uuid::Uuid;

pub fn run_ops(mut save: WorkspaceSave, ops: Vec<Op>) -> Result<Vec<String>, String> {
    if ops.is_empty() {
        return Ok(Vec::new());
    }
    let mut timeline = save.engine.peek().clone();
    let mut undo = UndoStack::new();
    let mut notes = Vec::new();
    for op in ops.clone() {
        let applied = apply(&mut timeline, &mut undo, op).map_err(|e| e.to_string())?;
        notes.push(applied.note);
    }
    save.engine.set(timeline.clone());
    save.tracks.set(bind::tracks_from_timeline(&timeline));
    enqueue_persist(save, ops);
    Ok(notes)
}

fn enqueue_persist(mut save: WorkspaceSave, ops: Vec<Op>) {
    save.persist_q.write().push(ops);
    pump_persist(save);
}

fn pump_persist(mut save: WorkspaceSave) {
    if *save.persist_busy.peek() {
        return;
    }
    let Some(ops) = save.persist_q.write().first().cloned() else {
        return;
    };
    save.persist_q.write().remove(0);
    save.persist_busy.set(true);
    let pid = save.project_id.peek().clone();
    spawn(async move {
        if !pid.is_empty() {
            if let Ok(next) = api::apply_ops(&pid, ops).await {
                save.engine.set(next);
            }
        }
        save.persist_busy.set(false);
        pump_persist(save);
    });
}

pub fn split_at(save: WorkspaceSave, track_id: &str, at: f64) -> Result<Vec<String>, String> {
    let at = Time::from_seconds(at);
    let tl = save.engine.peek();
    let clip_id = match parse_track_id(track_id).and_then(|id| tl.clip_at(id, at)) {
        Some(id) => id,
        None => tl
            .clip_at_any(at)
            .ok_or_else(|| "no clip at the playhead".to_string())?,
    };
    drop(tl);
    run_ops(save, vec![Op::Split { clip_id, at }])
}

pub fn merge_at(save: WorkspaceSave, track_id: &str, at: f64) -> Result<Vec<String>, String> {
    let at = Time::from_seconds(at);
    let tl = save.engine.peek();
    let clip_id = match parse_track_id(track_id).and_then(|id| tl.clip_at(id, at)) {
        Some(id) => id,
        None => tl
            .clip_at_any(at)
            .ok_or_else(|| "no clip at the playhead".to_string())?,
    };
    drop(tl);
    run_ops(save, vec![Op::Merge { clip_id }])
}

pub fn lift_at(save: WorkspaceSave, track_id: &str, at: f64) -> Result<Vec<String>, String> {
    let clip_id = clip_at(save, track_id, at)?;
    run_ops(save, vec![Op::RemoveClip { clip_id }])
}

pub fn trim_start_at(save: WorkspaceSave, track_id: &str, at: f64) -> Result<Vec<String>, String> {
    let at_t = Time::from_seconds(at);
    let clip_id = clip_at(save, track_id, at)?;
    let clip = save
        .engine
        .peek()
        .find_clip(clip_id)
        .ok_or_else(|| "no clip at the playhead".to_string())?
        .1
        .clone();
    let end = clip.end();
    if at_t >= end {
        return Err("playhead is past the clip".into());
    }
    run_ops(
        save,
        vec![Op::Trim {
            clip_id,
            start: at_t,
            duration: end - at_t,
        }],
    )
}

pub fn trim_end_at(save: WorkspaceSave, track_id: &str, at: f64) -> Result<Vec<String>, String> {
    let at_t = Time::from_seconds(at);
    let clip_id = clip_at(save, track_id, at)?;
    let clip = save
        .engine
        .peek()
        .find_clip(clip_id)
        .ok_or_else(|| "no clip at the playhead".to_string())?
        .1
        .clone();
    if at_t <= clip.start {
        return Err("playhead is before the clip".into());
    }
    run_ops(
        save,
        vec![Op::Trim {
            clip_id,
            start: clip.start,
            duration: at_t - clip.start,
        }],
    )
}

fn clip_at(save: WorkspaceSave, track_id: &str, at: f64) -> Result<ClipId, String> {
    let at = Time::from_seconds(at);
    let tl = save.engine.peek();
    match parse_track_id(track_id).and_then(|id| tl.clip_at(id, at)) {
        Some(id) => Ok(id),
        None => tl
            .clip_at_any(at)
            .ok_or_else(|| "no clip at the playhead".to_string()),
    }
}

pub fn delete_at(save: WorkspaceSave, track_id: &str, at: f64) -> Result<Vec<String>, String> {
    let at = Time::from_seconds(at);
    let tl = save.engine.peek();
    let clip_id = match parse_track_id(track_id).and_then(|id| tl.clip_at(id, at)) {
        Some(id) => id,
        None => tl
            .clip_at_any(at)
            .ok_or_else(|| "no clip at the playhead".to_string())?,
    };
    drop(tl);
    run_ops(save, vec![Op::RippleDelete { clip_id }])
}

pub fn trim_clip(save: WorkspaceSave, clip_id: &str, start: f64, duration: f64) -> Result<Vec<String>, String> {
    let clip_id = parse_clip_id(clip_id).ok_or_else(|| "bad clip id".to_string())?;
    run_ops(
        save,
        vec![Op::Trim {
            clip_id,
            start: Time::from_seconds(start),
            duration: Duration::from_seconds(duration),
        }],
    )
}

pub fn run_intent(
    save: WorkspaceSave,
    text: &str,
    track_id: &str,
    at: f64,
) -> Result<Vec<String>, String> {
    match parse_intent(text) {
        Intent::Split => split_at(save, track_id, at),
        Intent::Merge => merge_at(save, track_id, at),
        Intent::Delete => delete_at(save, track_id, at),
        Intent::TrimIn | Intent::TrimOut => {
            Err("drag a clip edge to trim, or split first".into())
        }
        Intent::Slip | Intent::Roll | Intent::Slide | Intent::Stretch => Err(
            "select that tool and drag the clip".into(),
        ),
        Intent::SplitAll => split_all_at(save, at),
        Intent::DetachAudio => detach_audio_at(save, track_id, at),
        Intent::Marker => add_marker_at(save, at, "Marker"),
        Intent::Direct => Ok(Vec::new()),
        Intent::Unknown => Err(
            "I can split, merge, delete, split all, detach audio, or add a marker."
                .into(),
        ),
    }
}

pub fn split_all_at(save: WorkspaceSave, at: f64) -> Result<Vec<String>, String> {
    run_ops(
        save,
        vec![Op::SplitAll {
            at: Time::from_seconds(at),
        }],
    )
}

pub fn detach_audio_at(save: WorkspaceSave, track_id: &str, at: f64) -> Result<Vec<String>, String> {
    let clip_id = clip_at(save, track_id, at)?;
    run_ops(save, vec![Op::DetachAudio { clip_id }])
}

pub fn add_marker_at(save: WorkspaceSave, at: f64, name: &str) -> Result<Vec<String>, String> {
    run_ops(
        save,
        vec![Op::AddMarker {
            time: Time::from_seconds(at),
            name: name.into(),
            color: 0,
        }],
    )
}

pub fn mark_in_at(save: WorkspaceSave, at: f64) -> Result<Vec<String>, String> {
    run_ops(
        save,
        vec![Op::SetMarkIn {
            time: Some(Time::from_seconds(at)),
        }],
    )
}

pub fn mark_out_at(save: WorkspaceSave, at: f64) -> Result<Vec<String>, String> {
    run_ops(
        save,
        vec![Op::SetMarkOut {
            time: Some(Time::from_seconds(at)),
        }],
    )
}

pub fn insert_space_at(save: WorkspaceSave, track_id: &str, at: f64) -> Result<Vec<String>, String> {
    run_ops(
        save,
        vec![Op::InsertSpace {
            track_id: parse_track_id(track_id),
            at: Time::from_seconds(at),
            amount: Duration::from_seconds(1.0),
        }],
    )
}

pub fn delete_space_at(save: WorkspaceSave, track_id: &str, at: f64) -> Result<Vec<String>, String> {
    run_ops(
        save,
        vec![Op::DeleteSpace {
            track_id: parse_track_id(track_id),
            at: Time::from_seconds(at),
        }],
    )
}

pub fn group_at(save: WorkspaceSave, track_id: &str, at: f64) -> Result<Vec<String>, String> {
    let clip_id = clip_at(save, track_id, at)?;
    let next = {
        let tl = save.engine.peek();
        let Some((track, clip)) = tl.find_clip(clip_id) else {
            return Err("no clip at the playhead".into());
        };
        track
            .clips
            .iter()
            .find(|c| c.start >= clip.end())
            .map(|c| c.id)
    };
    let Some(next) = next else {
        return Err("need a following clip to group".into());
    };
    run_ops(
        save,
        vec![Op::Group {
            clip_ids: vec![clip_id, next],
        }],
    )
}

pub fn ungroup_at(save: WorkspaceSave, track_id: &str, at: f64) -> Result<Vec<String>, String> {
    let clip_id = clip_at(save, track_id, at)?;
    run_ops(save, vec![Op::Ungroup { clip_id }])
}

pub fn link_at(save: WorkspaceSave, track_id: &str, at: f64) -> Result<Vec<String>, String> {
    let at_t = Time::from_seconds(at);
    let ids: Vec<ClipId> = save
        .engine
        .peek()
        .tracks
        .iter()
        .filter_map(|track| track.clip_at(at_t).map(|c| c.id))
        .collect();
    let _ = track_id;
    if ids.len() < 2 {
        return Err("need a video and audio clip at the playhead".into());
    }
    run_ops(save, vec![Op::Link { clip_ids: ids }])
}

pub fn unlink_at(save: WorkspaceSave, track_id: &str, at: f64) -> Result<Vec<String>, String> {
    let clip_id = clip_at(save, track_id, at)?;
    run_ops(save, vec![Op::Unlink { clip_id }])
}

pub fn multicam_at(save: WorkspaceSave, track_id: &str, at: f64) -> Result<Vec<String>, String> {
    let track_id = parse_track_id(track_id).ok_or_else(|| "bad track id".to_string())?;
    run_ops(
        save,
        vec![Op::MulticamCut {
            track_id,
            at: Time::from_seconds(at),
        }],
    )
}

fn parse_track_id(raw: &str) -> Option<TrackId> {
    Uuid::parse_str(raw.trim()).ok().map(TrackId::from_uuid)
}

fn parse_clip_id(raw: &str) -> Option<ClipId> {
    Uuid::parse_str(raw.trim()).ok().map(ClipId::from_uuid)
}
