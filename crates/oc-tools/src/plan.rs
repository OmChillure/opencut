//! Turn a model plan into timeline ops. The model does not place each clip itself.
//! Rust then grades, covers jumps, lays music, and writes captions.

use crate::finish::{self, CoverShot, SpokenLine};
use crate::ops::{Op, apply};
use oc_time::{Duration, Time};
use oc_timeline::{
    AspectRatio, CaptionCue, CaptionMood, CaptionStyle, Clip, ClipKind, EditPlan, EditSlot,
    MediaId, Timeline, TrackKind, TransitionKind, UndoStack,
};

#[derive(Clone, Debug)]
pub struct SourceWindow {
    pub media: MediaId,
    pub start: f64,
    pub end: f64,
    pub duration: f64,
    /// Framing the caption seater reads (`close`, `wide`, `medium`, …).
    /// When the shot has no card, this is the pixel look instead.
    pub look: String,
    /// What is in frame (`person`, `product`, …). Empty when unknown.
    pub subject: String,
    /// True when this window has no speech. Used to cover a jump.
    pub silent: bool,
}

pub fn plan_from_value(value: &serde_json::Value) -> Result<EditPlan, String> {
    let mut value = value.clone();
    if let Some(obj) = value.as_object_mut() {
        if let Some(raw) = obj.get("caption_mood").and_then(|v| v.as_str()) {
            if raw.trim().is_empty() {
                obj.remove("caption_mood");
            } else {
                obj.insert(
                    "caption_mood".into(),
                    serde_json::Value::String(CaptionMood::parse(raw).as_str().into()),
                );
            }
        }
        if let Some(raw) = obj.get("caption_look").cloned() {
            match stored_caption_look(&raw) {
                Some(parsed) => {
                    obj.insert("caption_look".into(), parsed);
                }
                None => {
                    obj.remove("caption_look");
                }
            }
        }
        for key in ["slots", "grade", "changes"] {
            let Some(raw) = obj.get(key).and_then(|v| v.as_str()) else {
                continue;
            };
            let parsed: serde_json::Value =
                serde_json::from_str(raw).map_err(|e| format!("submit_edit {key}: {e}"))?;
            obj.insert(key.to_string(), parsed);
        }
    }
    serde_json::from_value(value).map_err(|e| format!("submit_edit: {e}"))
}

/// A caption note the model bothered to send. Prose that names nothing is dropped.
fn stored_caption_look(value: &serde_json::Value) -> Option<serde_json::Value> {
    let recipe = if let Some(raw) = value.as_str() {
        oc_timeline::CaptionRecipe::from_loose(raw)
    } else if value.is_object() {
        serde_json::from_value(value.clone()).ok()
    } else {
        None
    };
    let recipe = recipe.filter(|recipe| !recipe.is_blank());
    recipe.and_then(|recipe| serde_json::to_value(recipe).ok())
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
    let slots = seated_slots(&plan.slots, windows, lines);
    let (kept_clips, kept_looks) = snapshot_layers(timeline);
    let mut undo = UndoStack::new();
    let mut notes = Vec::new();
    let cleared = apply(timeline, &mut undo, Op::ClearTimeline).map_err(err)?;
    notes.push(cleared.note);
    let mut at = 0.0;
    let mut ids = Vec::new();
    for slot in &slots {
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
    notes.extend(finish_picture(
        timeline, &mut undo, plan, &ids, windows, lines, covers,
    )?);
    restore_layers(timeline, kept_clips, &kept_looks);
    timeline.edit_plan = Some(plan.clone());
    Ok(notes)
}

struct KeptClip {
    track_name: String,
    clip: Clip,
}

struct KeptLook {
    media: MediaId,
    source_in: f64,
    mask: Option<oc_timeline::AlphaShape>,
    curves: oc_timeline::Curves,
    crop: Option<oc_timeline::Crop>,
    stabilize: bool,
}

fn snapshot_layers(timeline: &Timeline) -> (Vec<KeptClip>, Vec<KeptLook>) {
    let mut clips = Vec::new();
    let mut looks = Vec::new();
    for track in &timeline.tracks {
        let overlay = matches!(track.name.as_str(), "Design" | "Front" | "GFX");
        for clip in &track.clips {
            if clip.disabled {
                continue;
            }
            let graphic = matches!(clip.kind, ClipKind::Graphic { .. });
            if overlay || graphic {
                let mut kept = clip.clone();
                kept.id = oc_timeline::ClipId::new();
                clips.push(KeptClip {
                    track_name: track.name.clone(),
                    clip: kept,
                });
            }
            if overlay || !matches!(clip.kind, ClipKind::Video { .. }) {
                continue;
            }
            let Some(media) = clip.media_id else {
                continue;
            };
            let look = &clip.look;
            if look.mask.is_none()
                && look.crop.is_none()
                && !look.stabilize
                && look.curves.is_identity()
            {
                continue;
            }
            looks.push(KeptLook {
                media,
                source_in: clip.source_in.as_seconds(),
                mask: look.mask,
                curves: look.curves.clone(),
                crop: look.crop,
                stabilize: look.stabilize,
            });
        }
    }
    (clips, looks)
}

fn restore_layers(timeline: &mut Timeline, clips: Vec<KeptClip>, looks: &[KeptLook]) {
    let end = timeline.duration().as_seconds();
    if end < 0.2 {
        return;
    }
    let mut pending = Vec::new();
    for mut kept in clips {
        let start = kept.clip.start.as_seconds();
        if start >= end - 0.05 {
            continue;
        }
        if kept.clip.end().as_seconds() > end {
            let dur = end - start;
            if dur < 0.2 {
                continue;
            }
            kept.clip.duration = Duration::from_seconds(dur);
        }
        if plan_cover_already(&kept, timeline) {
            continue;
        }
        pending.push(kept);
    }
    for kept in pending {
        let kind = kept.clip.kind.track_kind();
        let track_id = ensure_named(timeline, kind, &kept.track_name);
        let _ = timeline.add_clip(track_id, kept.clip);
    }
    for track in &mut timeline.tracks {
        if matches!(track.name.as_str(), "Design" | "Front" | "GFX") {
            continue;
        }
        for clip in &mut track.clips {
            if !matches!(clip.kind, ClipKind::Video { .. }) {
                continue;
            }
            let Some(media) = clip.media_id else {
                continue;
            };
            let src = clip.source_in.as_seconds();
            let Some(look) = looks
                .iter()
                .filter(|look| look.media == media)
                .min_by(|a, b| {
                    (a.source_in - src)
                        .abs()
                        .partial_cmp(&(b.source_in - src).abs())
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
            else {
                continue;
            };
            if (look.source_in - src).abs() > 0.35 {
                continue;
            }
            if clip.look.mask.is_none() {
                clip.look.mask = look.mask;
            }
            if clip.look.crop.is_none() {
                clip.look.crop = look.crop;
            }
            if look.stabilize {
                clip.look.stabilize = true;
            }
            if clip.look.curves.is_identity() && !look.curves.is_identity() {
                clip.look.curves = look.curves.clone();
            }
        }
    }
}

fn plan_cover_already(kept: &KeptClip, timeline: &Timeline) -> bool {
    if kept.track_name != "GFX" || !matches!(kept.clip.kind, ClipKind::Video { .. }) {
        return false;
    }
    let Some(media) = kept.clip.media_id else {
        return false;
    };
    let src = kept.clip.source_in.as_seconds();
    timeline
        .tracks
        .iter()
        .filter(|track| track.name == "GFX")
        .flat_map(|track| track.clips.iter())
        .any(|clip| clip.media_id == Some(media) && (clip.source_in.as_seconds() - src).abs() < 0.3)
}

fn ensure_named(timeline: &mut Timeline, kind: TrackKind, name: &str) -> oc_timeline::TrackId {
    if let Some(track) = timeline
        .tracks
        .iter()
        .find(|track| track.kind == kind && track.name == name)
    {
        return track.id;
    }
    timeline.add_track(kind, name)
}

fn finish_picture(
    timeline: &mut Timeline,
    undo: &mut UndoStack,
    plan: &EditPlan,
    ids: &[(oc_timeline::ClipId, EditSlot)],
    windows: &[SourceWindow],
    lines: &[SpokenLine],
    covers: &[CoverShot],
) -> Result<Vec<String>, String> {
    let mut notes = Vec::new();
    let mut graded = 0;
    for (id, slot) in ids.iter() {
        let grade = grade_for_slot(slot, plan.grade);
        if !grade.is_identity() {
            apply(
                timeline,
                undo,
                Op::SetGrade {
                    clip_id: *id,
                    grade,
                },
            )
            .map_err(err)?;
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
        let Some((_, clip)) = timeline.find_clip(*id) else {
            continue;
        };
        let clip = clip.clone();
        let Some((_, prev)) = timeline.find_clip(ids[index - 1].0) else {
            continue;
        };
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
                if let Some(clip) = timeline
                    .tracks
                    .iter()
                    .flat_map(|t| t.clips.iter())
                    .rev()
                    .find(|c| c.media_id == Some(music) && matches!(c.kind, ClipKind::Audio { .. }))
                {
                    let id = clip.id;
                    if let Some(clip) = timeline.clip_mut(id) {
                        if let ClipKind::Audio {
                            volume: level,
                            ducked,
                        } = &mut clip.kind
                        {
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
        let mut cues = finish::mapped_cues(&finish::program_clips(timeline), lines);
        if !cues.is_empty() {
            let recipe = caption_recipe(plan);
            let clips = finish::program_clips(timeline);
            let faces = cue_faces(&clips, windows, &cues);
            oc_timeline::dress_cues(&mut cues, &recipe, &faces);
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
            let label = if plan.caption_look.is_some() {
                "custom"
            } else {
                caption_mood(plan).as_str()
            };
            notes.push(format!("captions {label}"));
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

fn caption_mood(plan: &EditPlan) -> CaptionMood {
    if let Some(mood) = plan.caption_mood {
        return mood;
    }
    match plan.aspect.to_ascii_lowercase().as_str() {
        "vertical" | "9:16" | "reel" | "portrait" => CaptionMood::Kinetic,
        _ => CaptionMood::Clean,
    }
}

/// The mix for this cut. A composed look wins. A named mood only fills empty roles.
fn caption_recipe(plan: &EditPlan) -> oc_timeline::CaptionRecipe {
    if let Some(look) = &plan.caption_look {
        let mut recipe = look.clone();
        if recipe.base.is_none() {
            recipe.base = Some(caption_mood(plan));
        }
        return recipe;
    }
    caption_mood(plan).recipe()
}

/// True when the picture under that cue is a close person. Unknown stays true.
#[must_use]
pub fn cue_faces(clips: &[&Clip], windows: &[SourceWindow], cues: &[CaptionCue]) -> Vec<bool> {
    cues.iter()
        .map(|cue| face_at(clips, windows, cue.start.as_seconds()))
        .collect()
}

fn face_at(clips: &[&Clip], windows: &[SourceWindow], at: f64) -> bool {
    let Some(clip) = clips.iter().find(|clip| {
        let start = clip.start.as_seconds();
        let end = clip.end().as_seconds();
        at >= start - 0.02 && at < end - 0.02
    }) else {
        return true;
    };
    let Some(media) = clip.media_id else {
        return true;
    };
    let speed = if clip.speed.is_finite() && clip.speed > 0.0 {
        f64::from(clip.speed)
    } else {
        1.0
    };
    let src = clip.source_in.as_seconds() + (at - clip.start.as_seconds()).max(0.0) * speed;
    let hit = windows.iter().find(|window| {
        window.media == media
            && window.end > window.start + 0.05
            && src >= window.start - 0.05
            && src < window.end + 0.05
    });
    match hit {
        Some(window) => oc_timeline::shot_is_face(&window.look, &window.subject),
        None => true,
    }
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
            .ok_or_else(|| "revise_edit change needs slot".to_string())?
            as usize;
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
        if let Some(v) = change.get("caption_mood").and_then(|v| v.as_str()) {
            next.caption_mood = Some(CaptionMood::parse(v));
        }
        if let Some(v) = change.get("caption_look") {
            let recipe = if let Some(raw) = v.as_str() {
                oc_timeline::CaptionRecipe::from_loose(raw)
            } else {
                serde_json::from_value(v.clone()).ok()
            };
            if let Some(recipe) = recipe.filter(|recipe| !recipe.is_blank()) {
                next.caption_look = Some(recipe);
            }
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
        let rows: Vec<_> = windows
            .iter()
            .filter(|w| w.media == slot.media_id)
            .collect();
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
            let inside = rows
                .iter()
                .any(|w| slot.source_in + 0.05 >= w.start - 0.15 && slot.source_in <= w.end + 0.15);
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

fn seated_slots(
    slots: &[EditSlot],
    windows: &[SourceWindow],
    lines: &[SpokenLine],
) -> Vec<EditSlot> {
    let mut seated = slots.to_vec();
    for slot in &mut seated {
        let file_end = windows
            .iter()
            .filter(|window| window.media == slot.media_id)
            .map(|window| window.duration)
            .fold(0.0, f64::max);
        seat_on_speech(slot, lines, file_end);
    }
    seated
}

/// Pull a talking slot onto the lines it actually uses.
/// A chopped last word is extended. A tail into the next sentence, or dead air, is cut.
/// A silent slot is left where the model put it.
fn seat_on_speech(slot: &mut EditSlot, lines: &[SpokenLine], file_end: f64) {
    let speed = slot
        .speed
        .filter(|speed| speed.is_finite() && *speed > 0.05)
        .map(f64::from)
        .unwrap_or(1.0);
    let src_in = slot.source_in.max(0.0);
    let src_out = src_in + slot.duration.max(0.2) * speed;
    let mut same: Vec<&SpokenLine> = lines
        .iter()
        .filter(|line| {
            line.media == slot.media_id
                && line.end > line.start + 0.05
                && !crate::finish::is_filler(&line.text)
        })
        .collect();
    same.sort_by(|a, b| {
        a.start
            .partial_cmp(&b.start)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let kept: Vec<&&SpokenLine> = same
        .iter()
        .filter(|line| speech_kept(line.start, line.end, src_in, src_out))
        .collect();
    let Some(first) = kept.first().copied() else {
        return;
    };
    let last = *kept.last().expect("kept");
    let prev_end = same
        .iter()
        .rev()
        .find(|line| line.end <= first.start + 0.02)
        .map(|line| line.end);
    let gap_before = prev_end.map(|end| first.start - end).unwrap_or(1.0);
    let preroll = if gap_before >= 0.12 {
        0.08_f64.min(gap_before * 0.5)
    } else {
        0.0
    };
    let mut new_in = (first.start - preroll).max(0.0);
    let earliest = (src_in - 0.45).max(0.0);
    if new_in < earliest {
        new_in = earliest;
    }
    let next_start = same
        .iter()
        .find(|line| line.start >= last.end - 0.02)
        .map(|line| line.start);
    let gap_after = next_start.map(|start| start - last.end).unwrap_or(1.0);
    let tail = if gap_after < 0.12 {
        0.0
    } else {
        0.22_f64.min(gap_after * 0.5)
    };
    let mut new_out = last.end + tail;
    if new_out > src_out + 0.45 {
        new_out = src_out + 0.45;
    }
    if file_end > 0.2 {
        new_out = new_out.min(file_end);
        new_in = new_in.min((file_end - 0.4).max(0.0));
    }
    if new_out < new_in + 0.4 || new_in > last.end - 0.3 {
        return;
    }
    let speech_tail = (new_out - last.end).max(0.0);
    if let Some(fade) = slot.fade_out {
        if fade > speech_tail + 0.02 {
            slot.fade_out = (speech_tail >= 0.05).then_some(speech_tail);
        }
    }
    slot.source_in = new_in;
    slot.duration = (new_out - new_in) / speed;
}

fn speech_kept(start: f64, end: f64, src_in: f64, src_out: f64) -> bool {
    let overlap = (end.min(src_out) - start.max(src_in)).max(0.0);
    if overlap < 0.28 {
        return false;
    }
    let line_len = (end - start).max(0.05);
    let slot_len = (src_out - src_in).max(0.05);
    overlap >= line_len * 0.55 || overlap >= slot_len * 0.55
}

fn grade_for_slot(slot: &EditSlot, shared: oc_timeline::Grade) -> oc_timeline::Grade {
    // The model that looked at the frame sets this. Rust does not invent one.
    slot.grade.unwrap_or(shared)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oc_timeline::{AlphaShape, ClipId, ClipLook, Grade, Lut, MaskShape, Transform};

    fn window(media: MediaId, start: f64, end: f64, look: &str, silent: bool) -> SourceWindow {
        SourceWindow {
            media,
            start,
            end,
            duration: 80.0,
            look: look.into(),
            subject: String::new(),
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
            caption_mood: None,
            caption_look: None,
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
        let cover = timeline
            .tracks
            .iter()
            .any(|t| t.name == "GFX" && !t.clips.is_empty());
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
    fn a_revision_keeps_design_and_a_mask() {
        let picture = MediaId::new();
        let design_media = MediaId::new();
        let mut timeline = Timeline::default();
        let v1 = timeline.first_track(TrackKind::Video).unwrap().id;
        timeline
            .add_clip(
                v1,
                Clip {
                    id: ClipId::new(),
                    media_id: Some(picture),
                    kind: ClipKind::Video {
                        transform: Transform::default(),
                    },
                    start: Time::ZERO,
                    duration: Duration::from_seconds(4.0),
                    source_in: Time::from_seconds(2.0),
                    speed: 1.0,
                    group_id: None,
                    link_id: None,
                    disabled: false,
                    look: ClipLook {
                        mask: Some(AlphaShape {
                            shape: MaskShape::Ellipse,
                            x: 0.4,
                            y: 0.4,
                            w: 0.3,
                            h: 0.3,
                            feather: 0.1,
                            invert: false,
                        }),
                        stabilize: true,
                        ..ClipLook::default()
                    },
                },
            )
            .unwrap();
        let design_track = timeline.add_track(TrackKind::Video, "Design");
        timeline
            .add_clip(
                design_track,
                Clip {
                    id: ClipId::new(),
                    media_id: Some(design_media),
                    kind: ClipKind::Video {
                        transform: Transform::default(),
                    },
                    start: Time::ZERO,
                    duration: Duration::from_seconds(2.0),
                    source_in: Time::ZERO,
                    speed: 1.0,
                    group_id: None,
                    link_id: None,
                    disabled: false,
                    look: ClipLook::default(),
                },
            )
            .unwrap();
        let plan = EditPlan {
            style: "cinematic".into(),
            aspect: String::new(),
            letterbox: false,
            music_id: None,
            music_volume: None,
            captions: false,
            caption_mood: None,
            caption_look: None,
            grade: Grade::default(),
            slots: vec![slot(picture, 2.0)],
        };
        let windows = vec![window(picture, 0.0, 10.0, "wide", false)];
        build_plan(&mut timeline, &plan, &windows, &[], &[], &[]).unwrap();
        let design = timeline
            .tracks
            .iter()
            .find(|track| track.name == "Design")
            .expect("design track");
        assert!(
            design
                .clips
                .iter()
                .any(|clip| clip.media_id == Some(design_media)),
            "design survives the rebuild"
        );
        let program = timeline
            .tracks
            .iter()
            .filter(|track| track.name != "Design" && track.name != "GFX" && track.name != "Front")
            .flat_map(|track| track.clips.iter())
            .find(|clip| clip.media_id == Some(picture))
            .expect("program clip");
        let mask = program.look.mask.expect("mask copied onto the new clip");
        assert!((mask.x - 0.4).abs() < 1e-4);
        assert!(program.look.stabilize);
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
            caption_mood: None,
            caption_look: None,
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

    #[test]
    fn kinetic_words_follow_the_shot() {
        let picture = MediaId::new();
        let mut timeline = Timeline::default();
        let plan = EditPlan {
            style: String::new(),
            aspect: "vertical".into(),
            letterbox: false,
            music_id: None,
            music_volume: None,
            captions: true,
            caption_mood: Some(CaptionMood::Kinetic),
            caption_look: None,
            grade: Grade::default(),
            slots: vec![slot(picture, 2.0), slot(picture, 40.0)],
        };
        let lines = vec![
            SpokenLine {
                media: picture,
                start: 2.2,
                end: 4.0,
                text: "Go now".into(),
            },
            SpokenLine {
                media: picture,
                start: 40.2,
                end: 43.0,
                text: "Go now".into(),
            },
        ];
        let mut wide = window(picture, 0.0, 10.0, "bright-wide", false);
        wide.subject = "street".into();
        let mut close = window(picture, 38.0, 50.0, "close", false);
        close.subject = "person".into();
        build_plan(&mut timeline, &plan, &[wide, close], &[], &lines, &[]).unwrap();
        let cues = timeline
            .tracks
            .iter()
            .flat_map(|track| track.clips.iter())
            .find_map(|clip| match &clip.kind {
                ClipKind::Caption { cues, .. } => Some(cues.clone()),
                _ => None,
            })
            .expect("captions");
        assert_eq!(cues.len(), 2, "{cues:?}");
        assert_eq!(cues[0].place, oc_timeline::CaptionPlace::Middle);
        assert_eq!(cues[0].font, oc_timeline::CaptionFont::Display);
        assert_eq!(cues[0].effect, oc_timeline::CaptionEffect::Typewriter);
        assert_eq!(cues[1].place, oc_timeline::CaptionPlace::Lower);
        assert_eq!(cues[1].font, oc_timeline::CaptionFont::Display);
    }

    #[test]
    fn a_caption_look_overrides_the_named_mood() {
        let picture = MediaId::new();
        let mut timeline = Timeline::default();
        let plan = EditPlan {
            style: String::new(),
            aspect: String::new(),
            letterbox: false,
            music_id: None,
            music_volume: None,
            captions: true,
            caption_mood: Some(CaptionMood::Clean),
            caption_look: Some(oc_timeline::CaptionRecipe {
                punch: Some(oc_timeline::LineLook::parse("top serif fade")),
                on_face: Some(oc_timeline::CaptionPlace::Lower),
                ..oc_timeline::CaptionRecipe::default()
            }),
            grade: Grade::default(),
            slots: vec![slot(picture, 2.0)],
        };
        let lines = vec![SpokenLine {
            media: picture,
            start: 2.2,
            end: 4.0,
            text: "Go now".into(),
        }];
        let mut wide = window(picture, 0.0, 10.0, "bright-wide", false);
        wide.subject = "street".into();
        let notes = build_plan(&mut timeline, &plan, &[wide], &[], &lines, &[]).unwrap();
        assert!(notes.iter().any(|note| note == "captions custom"));
        let cue = timeline
            .tracks
            .iter()
            .flat_map(|track| track.clips.iter())
            .find_map(|clip| match &clip.kind {
                ClipKind::Caption { cues, .. } => cues.first().cloned(),
                _ => None,
            })
            .expect("caption");
        assert_eq!(cue.place, oc_timeline::CaptionPlace::Top);
        assert_eq!(cue.font, oc_timeline::CaptionFont::Serif);
        assert_eq!(cue.effect, oc_timeline::CaptionEffect::Fade);
    }

    #[test]
    fn caption_look_arrives_as_a_json_string() {
        let media = MediaId::new();
        let raw = serde_json::json!({
            "captions": true,
            "caption_look": r#"{"punch":"top serif fade","on_face":"lower"}"#,
            "slots": [{
                "media_id": media.to_string(),
                "source_in": 0.0,
                "duration": 2.0
            }]
        });
        let plan = plan_from_value(&raw).unwrap();
        let look = plan.caption_look.expect("look");
        let punch = look.punch.expect("punch");
        assert_eq!(punch.place, Some(oc_timeline::CaptionPlace::Top));
        assert_eq!(punch.font, Some(oc_timeline::CaptionFont::Serif));
        assert_eq!(punch.effect, Some(oc_timeline::CaptionEffect::Fade));
        assert_eq!(look.on_face, Some(oc_timeline::CaptionPlace::Lower));
        assert!(plan.caption_mood.is_none());
    }

    #[test]
    fn revise_plan_accepts_a_caption_look_string() {
        let picture = MediaId::new();
        let plan = EditPlan {
            style: String::new(),
            aspect: String::new(),
            letterbox: false,
            music_id: None,
            music_volume: None,
            captions: true,
            caption_mood: None,
            caption_look: None,
            grade: Grade::default(),
            slots: vec![slot(picture, 2.0)],
        };
        let change = serde_json::json!({
            "slot": 0,
            "caption_look": r#"{"question":"middle serif fade"}"#
        });
        let next = revise_plan(&plan, &[change]).unwrap();
        let question = next.caption_look.expect("look").question.expect("question");
        assert_eq!(question.place, Some(oc_timeline::CaptionPlace::Middle));
        assert_eq!(question.font, Some(oc_timeline::CaptionFont::Serif));
        assert_eq!(question.effect, Some(oc_timeline::CaptionEffect::Fade));
    }

    #[test]
    fn a_caption_note_that_names_nothing_is_dropped() {
        let media = MediaId::new();
        let raw = serde_json::json!({
            "captions": true,
            "aspect": "vertical",
            "caption_look": "make it funky",
            "slots": [{
                "media_id": media.to_string(),
                "source_in": 0.0,
                "duration": 2.0
            }]
        });
        let plan = plan_from_value(&raw).unwrap();
        assert!(plan.caption_look.is_none());
        assert!(plan.captions);
    }

    #[test]
    fn one_caption_key_leaves_the_other_lines_to_the_shot() {
        let picture = MediaId::new();
        let mut timeline = Timeline::default();
        let plan = EditPlan {
            style: String::new(),
            aspect: "vertical".into(),
            letterbox: false,
            music_id: None,
            music_volume: None,
            captions: true,
            caption_mood: None,
            caption_look: oc_timeline::CaptionRecipe::from_loose("hook top display typewriter"),
            grade: Grade::default(),
            slots: vec![slot(picture, 2.0), slot(picture, 40.0)],
        };
        let lines = vec![
            SpokenLine {
                media: picture,
                start: 2.2,
                end: 4.0,
                text: "Go now".into(),
            },
            SpokenLine {
                media: picture,
                start: 40.2,
                end: 44.0,
                text: "the city opens up from here".into(),
            },
        ];
        let mut wide = window(picture, 0.0, 10.0, "bright-wide", false);
        wide.subject = "street".into();
        let mut later = window(picture, 38.0, 50.0, "bright-wide", false);
        later.subject = "street".into();
        build_plan(&mut timeline, &plan, &[wide, later], &[], &lines, &[]).unwrap();
        let cues = timeline
            .tracks
            .iter()
            .flat_map(|track| track.clips.iter())
            .find_map(|clip| match &clip.kind {
                ClipKind::Caption { cues, .. } => Some(cues.clone()),
                _ => None,
            })
            .expect("captions");
        assert_eq!(cues.len(), 2, "{cues:?}");
        assert_eq!(cues[0].place, oc_timeline::CaptionPlace::Top);
        assert_eq!(cues[0].font, oc_timeline::CaptionFont::Display);
        assert_eq!(cues[0].effect, oc_timeline::CaptionEffect::Typewriter);
        assert_eq!(cues[1].place, oc_timeline::CaptionPlace::Lower);
        assert_eq!(cues[1].font, oc_timeline::CaptionFont::Serif);
        assert_eq!(cues[1].effect, oc_timeline::CaptionEffect::Fade);
        assert_ne!(cues[0].place, cues[1].place);
    }

    #[test]
    fn a_talking_slot_keeps_the_last_word_and_drops_the_next_sentence() {
        let happen = MediaId::new();
        let projects = MediaId::new();
        let food = MediaId::new();
        let closer = MediaId::new();
        let quiet = MediaId::new();
        let mut chopped = slot(happen, 0.0);
        chopped.duration = 3.58;
        let mut nibble = slot(projects, 3.92);
        nibble.duration = 5.15;
        let mut house = slot(food, 11.48);
        house.duration = 5.97;
        let mut ending = slot(closer, 4.82);
        ending.duration = 3.08;
        ending.fade_out = Some(0.55);
        let mut silent = slot(quiet, 30.0);
        silent.duration = 4.0;
        let plan = EditPlan {
            style: String::new(),
            aspect: String::new(),
            letterbox: false,
            music_id: None,
            music_volume: None,
            captions: false,
            caption_mood: None,
            caption_look: None,
            grade: Grade::default(),
            slots: vec![chopped, nibble, house, ending, silent],
        };
        let lines = vec![
            line(happen, 0.0, 3.60, "now is your time to make it happen"),
            line(projects, 4.0, 9.0, "where you will build projects"),
            line(projects, 9.0, 12.0, "and build using git and github"),
            line(food, 8.0, 11.52, "from an external sponsor"),
            line(food, 11.52, 13.68, "arduino kits for the winners"),
            line(food, 13.68, 17.54, "food and refreshments are on the house"),
            line(food, 17.54, 20.0, "the next sentence starts here"),
            line(closer, 2.0, 4.80, "hurry up and register now"),
            line(closer, 5.22, 7.46, "see you all at the campus"),
        ];
        let windows = vec![
            window(happen, 0.0, 40.0, "interior", false),
            window(projects, 0.0, 40.0, "interior", false),
            window(food, 0.0, 40.0, "interior", false),
            window(closer, 0.0, 40.0, "interior", false),
            window(quiet, 0.0, 40.0, "wide", true),
        ];
        let mut timeline = Timeline::default();
        build_plan(&mut timeline, &plan, &windows, &[], &lines, &[]).unwrap();
        let pictures = program_pictures(&timeline);
        assert_eq!(pictures.len(), 5);
        assert!(
            near(pictures[0].source_in.as_seconds(), 0.0),
            "{pictures:?}"
        );
        assert!(
            near(pictures[0].source_out().as_seconds(), 3.82),
            "last word kept {:?}",
            pictures[0].source_out().as_seconds()
        );
        assert!(near(pictures[1].source_in.as_seconds(), 3.92));
        assert!(
            near(pictures[1].source_out().as_seconds(), 9.0),
            "next sentence dropped {:?}",
            pictures[1].source_out().as_seconds()
        );
        assert!(near(pictures[2].source_in.as_seconds(), 11.52));
        assert!(
            near(pictures[2].source_out().as_seconds(), 17.54),
            "house finishes {:?}",
            pictures[2].source_out().as_seconds()
        );
        assert!(near(pictures[3].source_in.as_seconds(), 5.14));
        assert!(near(pictures[3].source_out().as_seconds(), 7.68));
        assert!(
            near(pictures[3].look.fade_out.as_seconds(), 0.22),
            "fade {:?}",
            pictures[3].look.fade_out.as_seconds()
        );
        assert!(near(pictures[4].source_in.as_seconds(), 30.0));
        assert!(near(pictures[4].source_out().as_seconds(), 34.0));
    }

    #[test]
    fn a_piece_grade_stays_until_the_model_sets_one() {
        let picture = MediaId::new();
        let mut own = slot(picture, 12.0);
        own.grade = Some(Grade {
            lut: Lut::Warm,
            saturation: 0.3,
            ..Grade::default()
        });
        let mut mono = slot(picture, 40.0);
        mono.grade = Some(Grade {
            lut: Lut::Mono,
            ..Grade::default()
        });
        let shared = Grade {
            lut: Lut::Film,
            ..Grade::default()
        };
        let plan = EditPlan {
            style: "cinematic".into(),
            aspect: String::new(),
            letterbox: false,
            music_id: None,
            music_volume: None,
            captions: false,
            caption_mood: None,
            caption_look: None,
            grade: shared,
            slots: vec![slot(picture, 1.0), own, slot(picture, 22.0), mono],
        };
        let windows = vec![
            window(picture, 0.0, 10.0, "interior", false),
            window(picture, 10.0, 20.0, "dark", false),
            window(picture, 20.0, 32.0, "bright-wide", false),
            window(picture, 38.0, 50.0, "close", false),
        ];
        let mut timeline = Timeline::default();
        build_plan(&mut timeline, &plan, &windows, &[], &[], &[]).unwrap();
        let pictures = program_pictures(&timeline);
        assert_eq!(pictures.len(), 4);
        assert_eq!(pictures[0].look.grade, shared);
        assert_eq!(pictures[2].look.grade, shared);
        assert_eq!(pictures[1].look.grade.lut, Lut::Warm);
        assert!((pictures[1].look.grade.saturation - 0.3).abs() < 1e-4);
        assert_eq!(pictures[3].look.grade.lut, Lut::Mono);
    }

    fn near(actual: f64, expected: f64) -> bool {
        (actual - expected).abs() < 0.02
    }

    fn line(media: MediaId, start: f64, end: f64, text: &str) -> SpokenLine {
        SpokenLine {
            media,
            start,
            end,
            text: text.into(),
        }
    }

    fn program_pictures(timeline: &Timeline) -> Vec<&Clip> {
        timeline
            .tracks
            .iter()
            .filter(|track| track.kind == TrackKind::Video && track.name != "GFX")
            .flat_map(|track| track.clips.iter())
            .filter(|clip| matches!(clip.kind, ClipKind::Video { .. }))
            .collect()
    }
}
