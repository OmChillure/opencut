use crate::bind;
use crate::{WorkspaceSave, api};
use dioxus::prelude::*;
use oc_core::{
    ClipId, Duration, Fx, Grade, Graphic, Intent, Op, Time, TrackId, TransitionKind,
    UndoStack, apply, parse_intent,
};
use uuid::Uuid;

pub fn run_ops(mut save: WorkspaceSave, ops: Vec<Op>) -> Result<Vec<String>, String> {
    if ops.is_empty() {
        return Ok(Vec::new());
    }
    let mut timeline = save.engine.peek().clone();
    let mut undo = save.undo.peek().clone();
    let mut notes = Vec::new();
    for op in ops.clone() {
        let applied = apply(&mut timeline, &mut undo, op).map_err(|e| e.to_string())?;
        notes.push(applied.note);
    }
    save.undo.set(undo.clone());
    store_undo(&save.project_id.peek(), &undo);
    save.engine.set(timeline.clone());
    save.tracks.set(bind::tracks_from_timeline(&timeline));
    enqueue_persist(save, ops);
    Ok(notes)
}

fn undo_key(project: &str) -> String {
    format!("opencut-undo-{project}")
}

pub fn store_undo(project: &str, stack: &UndoStack) {
    if project.is_empty() {
        return;
    }
    let Some(win) = web_sys::window() else {
        return;
    };
    let Ok(Some(storage)) = win.local_storage() else {
        return;
    };
    if let Ok(json) = serde_json::to_string(stack) {
        let _ = storage.set_item(&undo_key(project), &json);
    }
}

pub fn load_undo(project: &str) -> UndoStack {
    let Some(win) = web_sys::window() else {
        return UndoStack::new();
    };
    let Ok(Some(storage)) = win.local_storage() else {
        return UndoStack::new();
    };
    storage
        .get_item(&undo_key(project))
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_else(UndoStack::new)
}

fn restore(mut save: WorkspaceSave, timeline: oc_core::Timeline, undo: UndoStack) {
    save.undo.set(undo.clone());
    store_undo(&save.project_id.peek(), &undo);
    save.engine.set(timeline.clone());
    save.tracks.set(bind::tracks_from_timeline(&timeline));
    enqueue_persist(save, vec![Op::SetTimeline { timeline }]);
}

/// Step back one named change. The history itself is kept, as in Kdenlive.
pub fn undo_edit(save: WorkspaceSave) -> Result<(), String> {
    let mut timeline = save.engine.peek().clone();
    let mut undo = save.undo.peek().clone();
    if !undo.undo(&mut timeline) {
        return Err("nothing to undo".into());
    }
    restore(save, timeline, undo);
    Ok(())
}

pub fn redo_edit(save: WorkspaceSave) -> Result<(), String> {
    let mut timeline = save.engine.peek().clone();
    let mut undo = save.undo.peek().clone();
    if !undo.redo(&mut timeline) {
        return Err("nothing to redo".into());
    }
    restore(save, timeline, undo);
    Ok(())
}

/// Jump the Undo History. `keep` is how many changes stay applied.
pub fn jump_history(save: WorkspaceSave, keep: usize) -> Result<(), String> {
    let mut timeline = save.engine.peek().clone();
    let mut undo = save.undo.peek().clone();
    if !undo.jump(&mut timeline, keep) && keep != undo.depth() {
        return Err("that step is not in the history".into());
    }
    restore(save, timeline, undo);
    Ok(())
}

pub fn clear_history(mut save: WorkspaceSave) {
    let mut undo = save.undo.peek().clone();
    undo.clear();
    save.undo.set(undo.clone());
    store_undo(&save.project_id.peek(), &undo);
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
    let at_t = Time::from_seconds(at);
    let tl = save.engine.peek();
    if let Some(id) = parse_track_id(track_id).and_then(|id| tl.clip_at(id, at_t)) {
        return Ok(id);
    }
    if let Some(id) = tl.clip_at_any(at_t) {
        return Ok(id);
    }
    drop(tl);
    // UI tracks can be ahead of a stale engine after a local drop.
    let tracks = save.tracks.peek();
    let from_ui = tracks
        .iter()
        .flat_map(|track| track.clips.iter().map(move |c| (track.id.as_str(), c)))
        .find(|(_, clip)| at + 1e-4 >= clip.start && at < clip.end())
        .and_then(|(_, clip)| parse_clip_id(&clip.id));
    from_ui.ok_or_else(|| {
        "no clip at the playhead — click the shot, then apply the mix".to_string()
    })
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

pub fn sync_engine_from_tracks(mut save: WorkspaceSave) {
    let engine = save.engine.peek().clone();
    let next = bind::timeline_from_tracks(
        &save.tracks.peek(),
        &engine,
        engine.width,
        engine.height,
    );
    save.engine.set(next);
}

pub fn selected_or_playhead(
    save: WorkspaceSave,
    selected: Option<&str>,
    track_id: &str,
    at: f64,
) -> Result<ClipId, String> {
    sync_engine_from_tracks(save);
    if let Some(raw) = selected {
        if let Some(id) = parse_clip_id(raw) {
            if save.engine.peek().find_clip(id).is_some() {
                return Ok(id);
            }
        }
    }
    clip_at(save, track_id, at)
}

pub fn set_transition_at(
    save: WorkspaceSave,
    selected: Option<&str>,
    track_id: &str,
    at: f64,
    kind: TransitionKind,
) -> Result<Vec<String>, String> {
    let clip_id = selected_or_playhead(save, selected, track_id, at)?;
    let mut notes = run_ops(save, vec![Op::SetTransition { clip_id, kind, duration: None }])?;
    let has_next = save.engine.peek().tracks.iter().any(|track| {
        let Some(i) = track.clips.iter().position(|c| c.id == clip_id) else {
            return false;
        };
        track
            .clips
            .get(i + 1)
            .is_some_and(|n| (n.start - track.clips[i].end()).as_seconds().abs() < 0.08)
    });
    if kind != TransitionKind::Cut && !has_next {
        notes.push(
            "applied — add or split a following shot so the mix has something to blend into"
                .into(),
        );
    } else if kind != TransitionKind::Cut {
        notes.push("play across the join to see it".into());
    }
    Ok(notes)
}

pub fn set_grade_at(
    save: WorkspaceSave,
    selected: Option<&str>,
    track_id: &str,
    at: f64,
    grade: Grade,
) -> Result<Vec<String>, String> {
    let clip_id = selected_or_playhead(save, selected, track_id, at)?;
    run_ops(save, vec![Op::SetGrade { clip_id, grade }])
}

pub fn set_fx_at(
    save: WorkspaceSave,
    selected: Option<&str>,
    track_id: &str,
    at: f64,
    fx: Fx,
) -> Result<Vec<String>, String> {
    let clip_id = selected_or_playhead(save, selected, track_id, at)?;
    run_ops(save, vec![Op::SetFx { clip_id, fx }])
}

pub fn set_fade_at(
    save: WorkspaceSave,
    selected: Option<&str>,
    track_id: &str,
    at: f64,
    fade: f64,
) -> Result<Vec<String>, String> {
    set_fade_ends(save, selected, track_id, at, fade, fade)
}

pub fn set_fade_ends(
    save: WorkspaceSave,
    selected: Option<&str>,
    track_id: &str,
    at: f64,
    fade_in: f64,
    fade_out: f64,
) -> Result<Vec<String>, String> {
    let clip_id = selected_or_playhead(save, selected, track_id, at)?;
    run_ops(
        save,
        vec![Op::SetFade {
            clip_id,
            fade_in: Duration::from_seconds(fade_in),
            fade_out: Duration::from_seconds(fade_out),
        }],
    )
}

pub fn set_volume_at(
    save: WorkspaceSave,
    selected: Option<&str>,
    track_id: &str,
    at: f64,
    volume: f32,
) -> Result<Vec<String>, String> {
    let clip_id = selected_or_playhead(save, selected, track_id, at)?;
    run_ops(save, vec![Op::SetVolume { clip_id, volume }])
}

pub fn add_graphic_at(
    save: WorkspaceSave,
    at: f64,
    graphic: Graphic,
    duration: f64,
) -> Result<Vec<String>, String> {
    run_ops(
        save,
        vec![Op::AddGraphic {
            graphic,
            start: Time::from_seconds(at),
            duration: Duration::from_seconds(duration),
            track_id: None,
        }],
    )
}

pub fn duck_at(save: WorkspaceSave) -> Result<Vec<String>, String> {
    run_ops(save, vec![Op::Duck { amount: 0.7 }])
}

pub fn set_mix(
    save: WorkspaceSave,
    track_id: Option<oc_core::TrackId>,
    mix: oc_core::Mix,
) -> Result<Vec<String>, String> {
    run_ops(save, vec![Op::SetMix { track_id, mix }])
}

pub fn set_curves_at(
    save: WorkspaceSave,
    selected: Option<&str>,
    track_id: &str,
    at: f64,
    curves: oc_core::Curves,
) -> Result<Vec<String>, String> {
    let clip_id = selected_or_playhead(save, selected, track_id, at)?;
    run_ops(save, vec![Op::SetCurves { clip_id, curves }])
}

pub fn set_mask_at(
    save: WorkspaceSave,
    selected: Option<&str>,
    track_id: &str,
    at: f64,
    mask: Option<oc_core::AlphaShape>,
) -> Result<Vec<String>, String> {
    let clip_id = selected_or_playhead(save, selected, track_id, at)?;
    run_ops(save, vec![Op::SetMask { clip_id, mask }])
}

pub fn set_speed_keys_at(
    save: WorkspaceSave,
    selected: Option<&str>,
    track_id: &str,
    at: f64,
    keys: Vec<oc_core::SpeedKey>,
) -> Result<Vec<String>, String> {
    let clip_id = selected_or_playhead(save, selected, track_id, at)?;
    run_ops(save, vec![Op::SetSpeedKeys { clip_id, keys }])
}

pub fn add_generator(
    save: WorkspaceSave,
    generator: oc_core::Generator,
    at: f64,
    duration: f64,
) -> Result<Vec<String>, String> {
    run_ops(
        save,
        vec![Op::AddGenerator {
            generator,
            at: Time::from_seconds(at),
            duration: Duration::from_seconds(duration),
        }],
    )
}

fn parse_track_id(raw: &str) -> Option<TrackId> {
    Uuid::parse_str(raw.trim()).ok().map(TrackId::from_uuid)
}

fn parse_clip_id(raw: &str) -> Option<ClipId> {
    Uuid::parse_str(raw.trim()).ok().map(ClipId::from_uuid)
}
