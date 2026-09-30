use std::cell::{Cell, RefCell};

use dioxus::html::PointerData;
use dioxus::prelude::*;
use oc_tools::ToolId;
use uuid::Uuid;
use wasm_bindgen::JsCast;
use web_sys::{Blob, BlobPropertyBag, HtmlVideoElement, Url};

#[derive(Clone, Copy)]
pub struct Clock {
    pub current: Signal<f64>,
    pub duration: Signal<f64>,
    pub playing: Signal<bool>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Video,
    Audio,
    Image,
}

#[derive(Clone, PartialEq)]
pub struct MediaItem {
    pub id: String,
    pub name: String,
    pub kind: MediaKind,
    pub url: String,
    pub content_type: String,
    pub duration: f64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TrackKindUi {
    Video,
    Audio,
    Caption,
}

pub use oc_tools::TimelineEditMode as EditMode;

pub use oc_tools::ToolId as EditTool;

#[derive(Clone)]
pub struct RulerMark {
    pub time: f64,
    pub major: bool,
    pub label: String,
}

/// Adaptive ticks like Kdenlive: labels stay readable as you zoom.
pub fn ruler_marks_nle(span: f64, pps: f64) -> Vec<RulerMark> {
    let span = span.max(1.0);
    let pps = pps.max(1.0);
    let candidates = [
        0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0,
    ];
    let major = candidates
        .iter()
        .copied()
        .find(|step| *step * pps >= 72.0)
        .unwrap_or(300.0);
    let minor = if major >= 1.0 { major / 5.0 } else { major / 2.0 };
    let mut marks = Vec::new();
    let mut i = 0i32;
    loop {
        let time = i as f64 * minor;
        if time > span + major {
            break;
        }
        let is_major = (time / major - (time / major).round()).abs() < 1e-6;
        marks.push(RulerMark {
            time,
            major: is_major,
            label: if is_major {
                format_tc_short(time)
            } else {
                String::new()
            },
        });
        i += 1;
        if i > 4000 {
            break;
        }
    }
    marks
}

pub fn format_tc_short(secs: f64) -> String {
    let total = secs.max(0.0);
    let h = (total / 3600.0) as u32;
    let m = ((total % 3600.0) / 60.0) as u32;
    let s = (total % 60.0) as u32;
    let f = (total.fract() * 30.0) as u32;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}:{f:02}")
    } else {
        format!("{m:02}:{s:02}:{f:02}")
    }
}

pub fn trim_in_mode(tracks: &mut [EditorTrack], clip_id: &str, new_start: f64, ripple: bool) {
    let Some((ti, ci)) = locate_clip(tracks, clip_id) else {
        return;
    };
    let clip = &tracks[ti].clips[ci];
    let mut start = new_start.max(0.0);
    let max_start = clip.end() - 0.05;
    if start > max_start {
        start = max_start;
    }
    let prev_end = tracks[ti]
        .clips
        .iter()
        .filter(|other| other.id != clip_id && other.end() <= clip.start + 1e-6)
        .map(TimelineClip::end)
        .fold(0.0, f64::max);
    start = start.max(prev_end);
    let delta = start - clip.start;
    if delta.abs() < 1e-4 {
        return;
    }
    let clip = &mut tracks[ti].clips[ci];
    clip.source_in = (clip.source_in + delta).max(0.0);
    clip.duration = (clip.duration - delta).max(0.05);
    clip.start = start;
    if ripple {
        shift_after(&mut tracks[ti], clip_id, -delta);
    }
}

pub fn trim_out_mode(tracks: &mut [EditorTrack], clip_id: &str, new_end: f64, ripple: bool) {
    let Some((ti, ci)) = locate_clip(tracks, clip_id) else {
        return;
    };
    let clip = &tracks[ti].clips[ci];
    let old_end = clip.end();
    let mut end = new_end.max(clip.start + 0.05);
    if !ripple {
        if let Some(next) = tracks[ti]
            .clips
            .iter()
            .filter(|other| other.id != clip_id && other.start >= clip.end() - 1e-6)
            .map(|other| other.start)
            .fold(None, |acc, s| Some(acc.map_or(s, |a: f64| a.min(s))))
        {
            end = end.min(next);
        }
    }
    tracks[ti].clips[ci].duration = (end - tracks[ti].clips[ci].start).max(0.05);
    if ripple {
        let delta = tracks[ti].clips[ci].end() - old_end;
        shift_after(&mut tracks[ti], clip_id, delta);
    }
}

fn shift_after(track: &mut EditorTrack, clip_id: &str, delta: f64) {
    if delta.abs() < 1e-4 {
        return;
    }
    let Some(end) = track
        .clips
        .iter()
        .find(|clip| clip.id == clip_id)
        .map(TimelineClip::end)
    else {
        return;
    };
    for clip in &mut track.clips {
        if clip.id != clip_id && clip.start >= end - delta - 1e-4 {
            clip.start = (clip.start + delta).max(0.0);
        }
    }
}

#[derive(Clone, PartialEq)]
pub struct TimelineClip {
    pub id: String,
    pub media_id: String,
    pub start: f64,
    pub duration: f64,
    /// Offset into the source file. Split keeps this so the right half starts mid-file.
    pub source_in: f64,
    pub speed: f64,
    pub group_id: String,
    pub link_id: String,
    pub disabled: bool,
    pub transition: String,
    pub graphic: String,
}

impl TimelineClip {
    pub fn end(&self) -> f64 {
        self.start + self.duration
    }
}

pub fn clip_name(clip: &TimelineClip, media_name: Option<&str>) -> String {
    if !clip.graphic.is_empty() {
        clip.graphic.clone()
    } else {
        media_name.unwrap_or("Clip").to_string()
    }
}

#[derive(Clone, PartialEq)]
pub struct EditorTrack {
    pub id: String,
    pub name: String,
    pub kind: TrackKindUi,
    pub muted: bool,
    pub hidden: bool,
    pub clips: Vec<TimelineClip>,
}

impl TrackKindUi {
    pub fn prefix(self) -> &'static str {
        match self {
            Self::Video => "V",
            Self::Audio => "A",
            Self::Caption => "Cc",
        }
    }
}

fn empty_track(id: &str, name: &str, kind: TrackKindUi) -> EditorTrack {
    EditorTrack {
        id: id.into(),
        name: name.into(),
        kind,
        muted: false,
        hidden: false,
        clips: Vec::new(),
    }
}

pub fn base_lane_height(kind: TrackKindUi) -> f64 {
    match kind {
        TrackKindUi::Video => 58.0,
        TrackKindUi::Audio => 40.0,
        TrackKindUi::Caption => 26.0,
    }
}

pub fn min_lane_height(kind: TrackKindUi) -> f64 {
    match kind {
        TrackKindUi::Video => 28.0,
        TrackKindUi::Audio => 20.0,
        TrackKindUi::Caption => 16.0,
    }
}

pub fn timeline_viewport_h() -> f64 {
    web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.query_selector(".tl-v").ok().flatten())
        .map(|el| f64::from(el.client_height()))
        .filter(|h| *h > 40.0)
        .unwrap_or(280.0)
}

pub fn timeline_viewport_w() -> f64 {
    web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.query_selector(".tl-scroll").ok().flatten())
        .map(|el| f64::from(el.client_width()))
        .filter(|w| *w > 120.0)
        .unwrap_or(800.0)
}

pub fn max_timeline_h() -> f64 {
    let window = web_sys::window()
        .and_then(|w| w.inner_height().ok())
        .and_then(|v| v.as_f64())
        .unwrap_or(800.0);
    (window - 220.0).max(240.0)
}

pub fn capture_pointer(evt: &Event<PointerData>) {
    let data = evt.data();
    let Some(native) = data.downcast::<web_sys::PointerEvent>() else {
        return;
    };
    let Some(target) = native.target() else {
        return;
    };
    let Ok(el) = target.dyn_into::<web_sys::Element>() else {
        return;
    };
    let _ = el.set_pointer_capture(native.pointer_id());
}

/// Shrink lanes so every track fits in the timeline, like Kdenlive "Fit tracks to view".
pub fn fit_scale(tracks: &[EditorTrack], viewport: f64) -> f64 {
    let want: f64 = tracks.iter().map(|track| base_lane_height(track.kind)).sum();
    let avail = (viewport - 24.0).max(48.0);
    if want <= avail || want <= 0.0 {
        1.0
    } else {
        (avail / want).clamp(0.4, 1.0)
    }
}

pub fn lane_height(kind: TrackKindUi, scale: f64) -> f64 {
    (base_lane_height(kind) * scale).max(min_lane_height(kind))
}

pub fn lane_scale_from_dom() -> f64 {
    web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.query_selector(".tl-body").ok().flatten())
        .and_then(|el| el.get_attribute("data-scale"))
        .and_then(|s| s.parse().ok())
        .filter(|s: &f64| *s > 0.0)
        .unwrap_or(1.0)
}

pub fn film_tiles(width_px: f64) -> i32 {
    ((width_px / 48.0).floor() as i32).clamp(1, 32)
}

pub const PPS_MIN: f64 = 10.0;
pub const PPS_MAX: f64 = 80.0;

pub fn clamp_pps(pps: f64) -> f64 {
    pps.clamp(PPS_MIN, PPS_MAX)
}

pub fn wave_bars(seed: &str, n: usize) -> Vec<u8> {
    let mut h = 2166136261u32;
    for byte in seed.as_bytes() {
        h ^= u32::from(*byte);
        h = h.wrapping_mul(16777619);
    }
    (0..n)
        .map(|i| {
            h = h.wrapping_mul(1664525).wrapping_add(1013904223 + i as u32);
            18 + (h % 78) as u8
        })
        .collect()
}

pub fn track_at_y(rows: &[EditorTrack], y: f64) -> Option<String> {
    track_at_y_from(rows, y, 24.0)
}

fn track_at_y_from(rows: &[EditorTrack], y: f64, origin: f64) -> Option<String> {
    let scale = lane_scale_from_dom();
    let mut cursor = origin;
    for track in rows {
        let h = lane_height(track.kind, scale);
        if y >= cursor && y < cursor + h {
            return Some(track.id.clone());
        }
        cursor += h;
    }
    None
}

fn track_id_at_point(client_x: f64, client_y: f64) -> Option<String> {
    let doc = web_sys::window()?.document()?;
    let mut node = doc.element_from_point(client_x as f32, client_y as f32);
    while let Some(el) = node {
        if let Some(id) = el.get_attribute("data-track") {
            if !id.is_empty() {
                return Some(id);
            }
        }
        node = el.parent_element();
    }
    None
}

pub fn scroll_left() -> f64 {
    web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.query_selector(".tl-scroll").ok().flatten())
        .map(|el| f64::from(el.scroll_left()))
        .unwrap_or(0.0)
}

pub fn next_track_name(tracks: &[EditorTrack], kind: TrackKindUi) -> String {
    let prefix = kind.prefix();
    let max = tracks
        .iter()
        .filter(|track| track.kind == kind)
        .filter_map(|track| track.name.strip_prefix(prefix)?.parse::<u32>().ok())
        .max()
        .unwrap_or(0);
    format!("{prefix}{}", max + 1)
}

pub fn media_track_kind(kind: MediaKind) -> TrackKindUi {
    match kind {
        MediaKind::Audio => TrackKindUi::Audio,
        MediaKind::Video | MediaKind::Image => TrackKindUi::Video,
    }
}

pub fn clip_duration(item: &MediaItem) -> f64 {
    if item.duration > 0.05 {
        item.duration
    } else {
        5.0
    }
}

#[derive(Clone, PartialEq)]
pub enum DragSource {
    Clip { clip_id: String },
    Media { media_id: String },
}

#[derive(Clone, PartialEq)]
pub struct DragSession {
    pub source: DragSource,
    pub grab: f64,
    pub at: f64,
    pub track_id: String,
    pub duration: f64,
    pub name: String,
    pub url: String,
    pub media_kind: MediaKind,
    pub origin_x: f64,
    pub origin_y: f64,
    pub client_x: f64,
    pub client_y: f64,
    pub moved: bool,
    pub over_timeline: bool,
}

pub fn timeline_hit(
    client_x: f64,
    client_y: f64,
    pps: f64,
    rows: &[EditorTrack],
) -> Option<(f64, String)> {
    let doc = web_sys::window()?.document()?;
    let scroll = doc.query_selector(".tl-scroll").ok().flatten()?;
    let srect = scroll.get_bounding_client_rect();
    let over_timeline = doc
        .query_selector(".tl-body")
        .ok()
        .flatten()
        .map(|el| {
            let r = el.get_bounding_client_rect();
            client_x >= r.left()
                && client_x <= r.right()
                && client_y >= r.top()
                && client_y <= r.bottom()
        })
        .unwrap_or(
            client_y >= srect.top()
                && client_y <= srect.bottom()
                && client_x >= srect.left()
                && client_x <= srect.right(),
        );
    if !over_timeline {
        return None;
    }
    let x = client_x - srect.left() + f64::from(scroll.scroll_left());
    let time = (x / pps.max(1.0)).max(0.0);
    if let Some(id) = track_id_at_point(client_x, client_y) {
        return Some((time, id));
    }
    if let Some(lanes) = doc.query_selector(".tl-lanes").ok().flatten() {
        let lrect = lanes.get_bounding_client_rect();
        let y = client_y - lrect.top();
        if let Some(id) = track_at_y_from(rows, y, 0.0) {
            return Some((time, id));
        }
    }
    track_at_y(rows, client_y - srect.top()).map(|id| (time, id))
}

pub fn drop_on_track(
    tracks: &mut Vec<EditorTrack>,
    item: &MediaItem,
    track_id: &str,
    start: f64,
) -> bool {
    let kind = media_track_kind(item.kind);
    let duration = clip_duration(item);
    let Some(track) = tracks
        .iter_mut()
        .find(|track| track.id == track_id && track.kind == kind)
    else {
        return false;
    };
    let start = snap_start(track, start.max(0.0), duration, None);
    push_clip(track, item.id.clone(), start, duration, 0.0);
    sort_clips(track);
    true
}

pub fn update_drag(
    session: &mut DragSession,
    client_x: f64,
    client_y: f64,
    pps: f64,
    rows: &[EditorTrack],
) {
    session.client_x = client_x;
    session.client_y = client_y;
    if (client_x - session.origin_x).hypot(client_y - session.origin_y) > 4.0 {
        session.moved = true;
    }
    let Some((time, track_id)) = timeline_hit(client_x, client_y, pps, rows) else {
        session.over_timeline = false;
        return;
    };
    session.over_timeline = true;
    session.at = (time - session.grab).max(0.0);
    if let Some(track) = rows.iter().find(|track| track.id == track_id)
        && track.kind == media_track_kind(session.media_kind)
    {
        session.track_id = track_id;
    }
}

pub fn commit_drag(
    session: &DragSession,
    tracks: &mut Vec<EditorTrack>,
    library: &[MediaItem],
    tool: ToolId,
) -> bool {
    if !session.moved || !session.over_timeline {
        return false;
    }
    let pps = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.query_selector(".tl-scroll").ok().flatten())
        .and_then(|el| el.get_attribute("data-pps"))
        .and_then(|s| s.parse().ok())
        .unwrap_or(36.0);
    let rows = display_tracks(tracks);
    let mut session = session.clone();
    let cx = session.client_x;
    let cy = session.client_y;
    update_drag(&mut session, cx, cy, pps, &rows);
    match &session.source {
        DragSource::Clip { clip_id } => {
            let origin = tracks
                .iter()
                .flat_map(|track| track.clips.iter())
                .find(|clip| clip.id == *clip_id)
                .map(|clip| clip.start)
                .unwrap_or(session.at);
            match tool {
                ToolId::Slip => slip_clip(tracks, clip_id, session.at - origin),
                ToolId::Spacer => space_from(tracks, session.at - origin, origin),
                ToolId::Roll => roll_clip(tracks, clip_id, session.at + session.grab),
                ToolId::Slide => slide_clip(tracks, clip_id, session.at),
                ToolId::RateStretch => rate_stretch_clip(tracks, clip_id, session.at + session.grab),
                _ => relocate_clip(tracks, clip_id, &session.track_id, session.at, true),
            }
            true
        }
        DragSource::Media { media_id } => {
            let Some(item) = library.iter().find(|item| item.id == *media_id) else {
                return false;
            };
            drop_on_track(tracks, item, &session.track_id, session.at)
        }
    }
}

pub fn place_clip(
    tracks: &mut Vec<EditorTrack>,
    item: &MediaItem,
    playhead: f64,
    target_id: Option<&str>,
    mode: EditMode,
) {
    let kind = match item.kind {
        MediaKind::Audio => TrackKindUi::Audio,
        MediaKind::Video | MediaKind::Image => TrackKindUi::Video,
    };
    let duration = if item.duration > 0.05 {
        item.duration
    } else {
        5.0
    };
    let start = playhead.max(0.0);
    if mode == EditMode::Normal {
        if let Some(id) = target_id
            && drop_on_track(tracks, item, id, start)
        {
            return;
        }
        if let Some(idx) = tracks.iter().position(|track| track.kind == kind) {
            let start = snap_start(&tracks[idx], start, duration, None);
            push_clip(&mut tracks[idx], item.id.clone(), start, duration, 0.0);
            sort_clips(&mut tracks[idx]);
            return;
        }
        let name = next_track_name(tracks, kind);
        let mut track = empty_track(
            &format!("{}-{}", kind.prefix().to_ascii_lowercase(), Uuid::now_v7()),
            &name,
            kind,
        );
        push_clip(&mut track, item.id.clone(), start, duration, 0.0);
        tracks.push(track);
        return;
    }
    let idx = target_id
        .and_then(|id| {
            tracks
                .iter()
                .position(|track| track.id == id && track.kind == kind)
        })
        .or_else(|| tracks.iter().position(|track| track.kind == kind));
    if let Some(idx) = idx {
        apply_edit(&mut tracks[idx], item.id.clone(), start, duration, mode);
        return;
    }
    let name = next_track_name(tracks, kind);
    let mut track = empty_track(
        &format!("{}-{}", kind.prefix().to_ascii_lowercase(), Uuid::now_v7()),
        &name,
        kind,
    );
    apply_edit(&mut track, item.id.clone(), start, duration, mode);
    tracks.push(track);
}

fn range_hits(track: &EditorTrack, start: f64, end: f64, except: Option<&str>) -> bool {
    track.clips.iter().any(|clip| {
        except != Some(clip.id.as_str()) && clip.start < end - 1e-6 && clip.end() > start + 1e-6
    })
}

pub fn relocate_clip(
    tracks: &mut [EditorTrack],
    clip_id: &str,
    dest_id: &str,
    new_start: f64,
    snap: bool,
) {
    let new_start = new_start.max(0.0);
    let Some((src, clip_i)) = locate_clip(tracks, clip_id) else {
        return;
    };
    let dest = tracks.iter().position(|track| track.id == dest_id).unwrap_or(src);
    if tracks[src].kind != tracks[dest].kind {
        let dur = tracks[src].clips[clip_i].duration;
        let start = if snap {
            snap_start(&tracks[src], new_start, dur, Some(clip_id))
        } else {
            new_start
        };
        tracks[src].clips[clip_i].start = start;
        return;
    }
    if src == dest {
        let dur = tracks[src].clips[clip_i].duration;
        let start = if snap {
            snap_start(&tracks[src], new_start, dur, Some(clip_id))
        } else {
            new_start
        };
        tracks[src].clips[clip_i].start = start;
        sort_clips(&mut tracks[src]);
        return;
    }
    let mut clip = tracks[src].clips.remove(clip_i);
    clip.start = if snap {
        snap_start(&tracks[dest], new_start, clip.duration, None)
    } else {
        new_start
    };
    tracks[dest].clips.push(clip);
    sort_clips(&mut tracks[dest]);
}

fn locate_clip(tracks: &[EditorTrack], clip_id: &str) -> Option<(usize, usize)> {
    for (ti, track) in tracks.iter().enumerate() {
        if let Some(ci) = track.clips.iter().position(|clip| clip.id == clip_id) {
            return Some((ti, ci));
        }
    }
    None
}

fn snap_start(track: &EditorTrack, start: f64, duration: f64, except: Option<&str>) -> f64 {
    let start = start.max(0.0);
    let end = start + duration;
    let Some(blocker) = track.clips.iter().find(|clip| {
        except != Some(clip.id.as_str()) && clip.start < end - 1e-6 && clip.end() > start + 1e-6
    }) else {
        return start;
    };
    let after = blocker.end();
    let before = (blocker.start - duration).max(0.0);
    let before_ok = !range_hits(track, before, before + duration, except);
    if !before_ok {
        return after;
    }
    if (after - start).abs() <= (start - before).abs() {
        after
    } else {
        before
    }
}

fn sort_clips(track: &mut EditorTrack) {
    track
        .clips
        .sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));
}

fn apply_edit(
    track: &mut EditorTrack,
    media_id: String,
    playhead: f64,
    duration: f64,
    mode: EditMode,
) {
    let start = playhead.max(0.0);
    match mode {
        EditMode::Normal => place_normal(track, media_id, start, duration),
        EditMode::Insert => place_insert(track, media_id, start, duration),
        EditMode::Overwrite => place_overwrite(track, media_id, start, duration),
    }
    track
        .clips
        .sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));
}

fn place_normal(track: &mut EditorTrack, media_id: String, playhead: f64, duration: f64) {
    let mut start = playhead;
    for clip in &track.clips {
        if start < clip.end() && start + duration > clip.start {
            start = clip.end();
        }
    }
    push_clip(track, media_id, start, duration, 0.0);
}

fn place_insert(track: &mut EditorTrack, media_id: String, playhead: f64, duration: f64) {
    split_track_at(track, playhead);
    for clip in &mut track.clips {
        if clip.start >= playhead - 1e-6 {
            clip.start += duration;
        }
    }
    push_clip(track, media_id, playhead, duration, 0.0);
}

fn place_overwrite(track: &mut EditorTrack, media_id: String, playhead: f64, duration: f64) {
    let end = playhead + duration;
    split_track_at(track, playhead);
    split_track_at(track, end);
    track
        .clips
        .retain(|clip| !(clip.start >= playhead - 1e-6 && clip.end() <= end + 1e-6));
    push_clip(track, media_id, playhead, duration, 0.0);
}

fn roll_clip(tracks: &mut [EditorTrack], clip_id: &str, at: f64) {
    let Some((ti, ci)) = locate_clip(tracks, clip_id) else {
        return;
    };
    let left_i = if ci + 1 < tracks[ti].clips.len()
        && (tracks[ti].clips[ci + 1].start - tracks[ti].clips[ci].end()).abs() < 0.05
    {
        ci
    } else if ci > 0
        && (tracks[ti].clips[ci].start - tracks[ti].clips[ci - 1].end()).abs() < 0.05
    {
        ci - 1
    } else {
        return;
    };
    let left_start = tracks[ti].clips[left_i].start;
    let right_end = tracks[ti].clips[left_i + 1].end();
    let at = at.clamp(left_start + 0.05, right_end - 0.05);
    let right_start = tracks[ti].clips[left_i + 1].start;
    tracks[ti].clips[left_i].duration = at - left_start;
    let right = &mut tracks[ti].clips[left_i + 1];
    right.source_in = (right.source_in + (at - right_start)).max(0.0);
    right.start = at;
    right.duration = right_end - at;
}

fn slide_clip(tracks: &mut [EditorTrack], clip_id: &str, new_start: f64) {
    let Some((ti, ci)) = locate_clip(tracks, clip_id) else {
        return;
    };
    if ci == 0 || ci + 1 >= tracks[ti].clips.len() {
        return;
    }
    let left_start = tracks[ti].clips[ci - 1].start;
    let mid_dur = tracks[ti].clips[ci].duration;
    let right_end = tracks[ti].clips[ci + 1].end();
    let min_start = left_start + 0.05;
    let max_start = (right_end - mid_dur - 0.05).max(min_start);
    let new_start = new_start.clamp(min_start, max_start);
    let new_right = new_start + mid_dur;
    tracks[ti].clips[ci - 1].duration = new_start - left_start;
    tracks[ti].clips[ci].start = new_start;
    let old_right = tracks[ti].clips[ci + 1].start;
    let right = &mut tracks[ti].clips[ci + 1];
    right.source_in = (right.source_in + (new_right - old_right)).max(0.0);
    right.start = new_right;
    right.duration = right_end - new_right;
}

fn rate_stretch_clip(tracks: &mut [EditorTrack], clip_id: &str, new_end: f64) {
    let Some((ti, ci)) = locate_clip(tracks, clip_id) else {
        return;
    };
    let start = tracks[ti].clips[ci].start;
    let old_dur = tracks[ti].clips[ci].duration.max(0.05);
    let new_dur = (new_end - start).max(0.05);
    let old_speed = if tracks[ti].clips[ci].speed > 0.05 {
        tracks[ti].clips[ci].speed
    } else {
        1.0
    };
    tracks[ti].clips[ci].speed = old_speed * (old_dur / new_dur);
    tracks[ti].clips[ci].duration = new_dur;
}

fn slip_clip(tracks: &mut [EditorTrack], clip_id: &str, delta: f64) {
    if delta.abs() < 1e-4 {
        return;
    }
    for track in tracks {
        if let Some(clip) = track.clips.iter_mut().find(|clip| clip.id == clip_id) {
            clip.source_in = (clip.source_in + delta).max(0.0);
            return;
        }
    }
}

fn space_from(tracks: &mut [EditorTrack], delta: f64, origin: f64) {
    if delta.abs() < 1e-4 {
        return;
    }
    for track in tracks {
        for clip in &mut track.clips {
            if clip.start + 1e-4 >= origin {
                clip.start = (clip.start + delta).max(0.0);
            }
        }
    }
}

fn push_clip(track: &mut EditorTrack, media_id: String, start: f64, duration: f64, source_in: f64) {
    track.clips.push(TimelineClip {
        id: Uuid::now_v7().to_string(),
        media_id,
        start,
        duration,
        source_in,
        speed: 1.0,
        group_id: String::new(),
        link_id: String::new(),
        disabled: false,
        transition: String::new(),
        graphic: String::new(),
    });
}

fn split_track_at(track: &mut EditorTrack, at: f64) {
    let mut next = Vec::with_capacity(track.clips.len() + 1);
    for clip in track.clips.drain(..) {
        if at > clip.start + 1e-4 && at < clip.end() - 1e-4 {
            let left_dur = at - clip.start;
            let right_dur = clip.end() - at;
            next.push(TimelineClip {
                id: clip.id,
                media_id: clip.media_id.clone(),
                start: clip.start,
                duration: left_dur,
                source_in: clip.source_in,
                speed: clip.speed,
                group_id: clip.group_id.clone(),
                link_id: clip.link_id.clone(),
                disabled: clip.disabled,
                transition: clip.transition.clone(),
                graphic: clip.graphic.clone(),
            });
            next.push(TimelineClip {
                id: Uuid::now_v7().to_string(),
                media_id: clip.media_id,
                start: at,
                duration: right_dur,
                source_in: clip.source_in + left_dur,
                speed: clip.speed,
                group_id: clip.group_id,
                link_id: clip.link_id,
                disabled: clip.disabled,
                transition: String::new(),
                graphic: clip.graphic,
            });
        } else {
            next.push(clip);
        }
    }
    track.clips = next;
}

pub fn timeline_end(tracks: &[EditorTrack]) -> f64 {
    tracks
        .iter()
        .flat_map(|track| track.clips.iter())
        .map(TimelineClip::end)
        .fold(0.0, f64::max)
}

/// Pull a later picture clip back across a hole so playback cannot skip it.
/// Drawings, labels, and the corner window keep the gaps between spoken lines.
pub fn close_editor_gaps(tracks: &mut [EditorTrack]) {
    for track in tracks {
        if track.hidden || track.kind != TrackKindUi::Video || is_overlay_track(&track.name) {
            continue;
        }
        let mut order: Vec<usize> = (0..track.clips.len())
            .filter(|&i| !track.clips[i].disabled && !track.clips[i].media_id.is_empty())
            .collect();
        order.sort_by(|&a, &b| {
            track.clips[a]
                .start
                .partial_cmp(&track.clips[b].start)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut cursor = order.first().map(|&i| track.clips[i].start).unwrap_or(0.0);
        for i in order {
            if track.clips[i].start > cursor + 0.08 {
                track.clips[i].start = cursor;
            }
            cursor = track.clips[i].end();
        }
    }
}

/// End of the picture the monitor should play. Captions and the source file length do not extend it.
pub fn program_end(tracks: &[EditorTrack]) -> f64 {
    tracks
        .iter()
        .filter(|track| !track.hidden && track.kind == TrackKindUi::Video)
        .flat_map(|track| track.clips.iter())
        .filter(|clip| !clip.disabled)
        .map(TimelineClip::end)
        .fold(0.0, f64::max)
}

pub fn display_tracks(tracks: &[EditorTrack]) -> Vec<EditorTrack> {
    let caps: Vec<_> = tracks
        .iter()
        .filter(|track| track.kind == TrackKindUi::Caption)
        .cloned()
        .collect();
    let rest: Vec<_> = tracks
        .iter()
        .filter(|track| track.kind != TrackKindUi::Caption)
        .cloned()
        .collect();
    caps.into_iter().chain(rest).collect()
}

pub fn set_media_duration(library: &mut [MediaItem], tracks: &mut [EditorTrack], url: &str, duration: f64) -> bool {
    if !(duration.is_finite() && duration > 0.05) {
        return false;
    }
    let Some(item) = library.iter_mut().find(|item| item.url == url) else {
        return false;
    };
    let old = item.duration;
    let mut changed = (old - duration).abs() > 0.05;
    item.duration = duration;
    let id = item.id.clone();
    for track in tracks.iter_mut() {
        let mut bounds: Vec<(f64, f64)> = track
            .clips
            .iter()
            .map(|c| (c.start, c.duration))
            .collect();
        bounds.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        let only = track.clips.iter().filter(|c| c.media_id == id).count() == 1;
        for clip in track.clips.iter_mut() {
            if clip.media_id != id {
                continue;
            }
            // A 5s drop before the file length is known. A real excerpt stays as cut.
            let looks_placeholder = only && (clip.duration - 5.0).abs() < 0.05;
            if !looks_placeholder || clip.source_in > 0.05 {
                continue;
            }
            let next_start = bounds
                .iter()
                .filter(|(start, _)| *start > clip.start + 1e-3)
                .map(|(start, _)| *start)
                .fold(None, |acc, s| Some(acc.map_or(s, |a: f64| a.min(s))));
            let room = next_start
                .map(|s| (s - clip.start).max(0.05))
                .unwrap_or(duration);
            let next = duration.min(room);
            if (clip.duration - next).abs() > 0.05 {
                clip.duration = next;
                changed = true;
            }
        }
    }
    changed
}

impl MediaKind {
    pub fn from_name(name: &str) -> Self {
        let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        match ext.as_str() {
            "mp3" | "wav" | "aac" | "m4a" | "ogg" | "flac" => Self::Audio,
            "png" | "jpg" | "jpeg" | "gif" | "webp" => Self::Image,
            _ => Self::Video,
        }
    }

    pub fn mime(name: &str) -> &'static str {
        let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        match ext.as_str() {
            "mp4" => "video/mp4",
            "webm" => "video/webm",
            "mov" => "video/quicktime",
            "mp3" => "audio/mpeg",
            "wav" => "audio/wav",
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            _ => "application/octet-stream",
        }
    }
}

pub fn blob_url(bytes: &[u8], mime: &str) -> Option<String> {
    let array = js_sys::Uint8Array::new_with_length(bytes.len() as u32);
    array.copy_from(bytes);
    let parts = js_sys::Array::of1(&array);
    let opts = BlobPropertyBag::new();
    opts.set_type(mime);
    let blob = Blob::new_with_u8_array_sequence_and_options(&parts, &opts).ok()?;
    Url::create_object_url_with_blob(&blob).ok()
}

pub fn item_from_bytes_id(name: String, bytes: &[u8], id: String) -> Option<MediaItem> {
    let kind = MediaKind::from_name(&name);
    let content_type = MediaKind::mime(&name).to_string();
    let url = blob_url(bytes, &content_type)?;
    Some(MediaItem {
        id,
        name,
        kind,
        url,
        content_type,
        duration: 0.0,
    })
}

thread_local! {
    static PLAYHEAD: Cell<f64> = const { Cell::new(0.0) };
    static LAST_TICK_MS: Cell<f64> = const { Cell::new(0.0) };
    static LAST_PLAY_MS: Cell<f64> = const { Cell::new(0.0) };
    static LAST_SEEK_MS: Cell<f64> = const { Cell::new(0.0) };
    static FRONT_IS_B: Cell<bool> = const { Cell::new(false) };
}

pub fn playhead_now() -> f64 {
    PLAYHEAD.with(Cell::get)
}

pub fn set_playhead(time: f64) {
    PLAYHEAD.with(|cell| cell.set(time.max(0.0)));
}

pub fn reset_tick_clock() {
    LAST_TICK_MS.with(|cell| cell.set(0.0));
}

pub fn advance_playhead(span: f64) -> f64 {
    let ms = js_sys::Date::now();
    let last = LAST_TICK_MS.with(Cell::get);
    let dt = if last <= 0.0 {
        0.016
    } else {
        ((ms - last) / 1000.0).clamp(0.0, 0.08)
    };
    LAST_TICK_MS.with(|cell| cell.set(ms));
    let next = (playhead_now() + dt).clamp(0.0, span.max(0.0));
    set_playhead(next);
    next
}

#[derive(Clone)]
pub struct ProgramShot {
    pub media_id: String,
    pub url: String,
    pub kind: MediaKind,
    pub start: f64,
    pub source_in: f64,
    pub duration: f64,
}

/// Design, the corner window, and text labels are drawn on top of the program.
/// They are not the shot the monitor plays.
fn is_overlay_track(name: &str) -> bool {
    matches!(name, "Design" | "Front" | "GFX")
}

pub fn clip_under(tracks: &[EditorTrack], library: &[MediaItem], time: f64) -> Option<ProgramShot> {
    let mut best: Option<ProgramShot> = None;
    for track in tracks {
        if track.hidden || track.kind != TrackKindUi::Video || is_overlay_track(&track.name) {
            continue;
        }
        for clip in &track.clips {
            if clip.disabled || time + 1e-4 < clip.start || time >= clip.end() {
                continue;
            }
            // A label has no picture. Skipping it here keeps the speaker playing.
            let Some(shot) = shot_from(library, clip) else {
                continue;
            };
            // Later start wins (V2 over a V1 tail). Same start → later track.
            if best.as_ref().is_none_or(|b| shot.start + 1e-6 >= b.start) {
                best = Some(shot);
            }
        }
    }
    best
}

pub fn video_duration_from_src(src: &str) -> Option<f64> {
    let doc = web_sys::window()?.document()?;
    let list = doc.query_selector_all("video").ok()?;
    for i in 0..list.length() {
        let Ok(video) = list.item(i)?.dyn_into::<HtmlVideoElement>() else {
            continue;
        };
        let found = video.current_src() == src || video.src() == src;
        if !found {
            continue;
        }
        let duration = video.duration();
        if duration.is_finite() && duration > 0.05 {
            return Some(duration);
        }
    }
    None
}

fn query_video(selector: &str) -> Option<HtmlVideoElement> {
    web_sys::window()?
        .document()?
        .query_selector(selector)
        .ok()
        .flatten()?
        .dyn_into::<HtmlVideoElement>()
        .ok()
}

fn front_is_b() -> bool {
    FRONT_IS_B.with(Cell::get)
}

fn swap_program() {
    FRONT_IS_B.with(|c| c.set(!c.get()));
}

pub fn preview_video() -> Option<HtmlVideoElement> {
    if front_is_b() {
        query_video(".preview-video-b")
    } else {
        query_video(".preview-video")
    }
}

fn standby_video() -> Option<HtmlVideoElement> {
    if front_is_b() {
        query_video(".preview-video")
    } else {
        query_video(".preview-video-b")
    }
}

pub fn following_shot(
    tracks: &[EditorTrack],
    library: &[MediaItem],
    time: f64,
) -> Option<ProgramShot> {
    let cur = clip_under(tracks, library, time)?;
    let cur_end = cur.start + cur.duration;
    let mut join: Option<(f64, String)> = None;
    let mut later: Option<(f64, String)> = None;
    for track in tracks {
        if track.hidden || track.kind != TrackKindUi::Video {
            continue;
        }
        if is_overlay_track(&track.name) {
            continue;
        }
        for clip in &track.clips {
            if clip.disabled || clip.media_id.is_empty() || clip.start <= cur.start + 0.04 {
                continue;
            }
            if shot_from(library, clip).is_none() {
                continue;
            }
            if is_join_ui(cur.start, cur_end, clip.start) {
                if join.as_ref().is_none_or(|(s, _)| clip.start < *s) {
                    join = Some((clip.start, clip.id.clone()));
                }
            } else if clip.start >= cur_end - 0.05 && clip.start - cur_end <= 0.35 {
                if later.as_ref().is_none_or(|(s, _)| clip.start < *s) {
                    later = Some((clip.start, clip.id.clone()));
                }
            }
        }
    }
    let id = join.or(later).map(|(_, id)| id)?;
    let clip = tracks
        .iter()
        .flat_map(|t| t.clips.iter())
        .find(|c| c.id == id)?;
    shot_from(library, clip)
}

fn is_join_ui(a_start: f64, a_end: f64, b_start: f64) -> bool {
    b_start > a_start + 0.05 && b_start < a_end + 0.2 && b_start > a_end - 1.2
}

fn shot_from(library: &[MediaItem], clip: &TimelineClip) -> Option<ProgramShot> {
    let item = library.iter().find(|item| item.id == clip.media_id)?;
    Some(ProgramShot {
        media_id: item.id.clone(),
        url: item.url.clone(),
        kind: item.kind,
        start: clip.start,
        source_in: clip.source_in,
        duration: clip.duration,
    })
}

fn contiguous_source(a: &ProgramShot, b: &ProgramShot) -> bool {
    a.media_id == b.media_id
        && (a.source_in + a.duration - b.source_in).abs() < 0.08
}

fn preroll(video: &HtmlVideoElement, shot: &ProgramShot) {
    let loaded = video.get_attribute("data-media").unwrap_or_default();
    if loaded != shot.media_id {
        let _ = video.set_attribute("data-media", &shot.media_id);
        video.set_src(&shot.url);
        video.set_muted(true);
        return;
    }
    if video.ready_state() >= 2 {
        let drift = (video.current_time() - shot.source_in).abs();
        if drift > 0.08 {
            video.set_current_time(shot.source_in.max(0.0));
        }
    }
    if !video.paused() {
        let _ = video.pause();
    }
}

fn preroll_ready(video: &HtmlVideoElement, shot: &ProgramShot) -> bool {
    video.get_attribute("data-media").unwrap_or_default() == shot.media_id
        && video.ready_state() >= 2
        && (video.current_time() - shot.source_in).abs() < 0.2
}

fn set_class_off(selector: &str, off: bool) {
    let Some(el) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.query_selector(selector).ok().flatten())
    else {
        return;
    };
    let class = el.get_attribute("class").unwrap_or_default();
    let mut parts: Vec<&str> = class.split_whitespace().filter(|p| *p != "off").collect();
    if off {
        parts.push("off");
    }
    let _ = el.set_attribute("class", &parts.join(" "));
}

fn seek_video(video: &HtmlVideoElement, time: f64, force: bool) {
    let ms = js_sys::Date::now();
    let last = LAST_SEEK_MS.with(Cell::get);
    if !force && ms - last < 80.0 {
        return;
    }
    LAST_SEEK_MS.with(|cell| cell.set(ms));
    video.set_current_time(time.max(0.0));
}

pub fn css_filter(grade: oc_core::Grade, fx: oc_core::Fx) -> String {
    let brightness = (1.0 + grade.exposure + grade.gain * 0.35 + grade.lift * 0.15).clamp(0.15, 2.8);
    let contrast = (1.0 + grade.contrast + grade.gamma * 0.45).clamp(0.2, 2.8);
    let saturation = (1.0 + grade.saturation).clamp(0.0, 2.6);
    let hue = grade.temperature * 18.0;
    let blur = fx.blur * 8.0;
    let mut filter = format!(
        "brightness({brightness:.3}) contrast({contrast:.3}) saturate({saturation:.3}) hue-rotate({hue:.1}deg) blur({blur:.2}px)"
    );
    if grade.cube.is_none() {
        let named = match grade.lut {
            oc_core::Lut::None => "",
            oc_core::Lut::Film => " sepia(0.35) contrast(1.05) saturate(0.9)",
            oc_core::Lut::Cool => " hue-rotate(14deg) saturate(1.08)",
            oc_core::Lut::Warm => " sepia(0.28) saturate(1.12)",
            oc_core::Lut::TealOrange => " hue-rotate(-10deg) saturate(1.28) contrast(1.06)",
            oc_core::Lut::Mono => " grayscale(1)",
        };
        filter.push_str(named);
    }
    filter
}

fn mix_preview_css(
    kind: oc_core::TransitionKind,
    mix: f64,
    opacity: f64,
    filter: &str,
) -> (f64, String, String, String) {
    if mix <= 0.0 || kind == oc_core::TransitionKind::Cut {
        return (opacity, "none".into(), "none".into(), filter.into());
    }
    if let Some((dx, dy)) = kind.slide_delta() {
        return (
            opacity,
            format!("translate({:.1}%, {:.1}%)", dx * mix * 100.0, dy * mix * 100.0),
            "none".into(),
            filter.into(),
        );
    }
    if let Some(clip) = kind.wipe_inset(mix) {
        return (opacity, "none".into(), clip, filter.into());
    }
    match kind {
        oc_core::TransitionKind::FadeBlack => {
            let dip = 1.0 - (2.0 * mix - 1.0).abs();
            let f = format!("{filter} brightness({:.3})", (1.0 - dip).max(0.05));
            ((opacity * (1.0 - mix)).clamp(0.0, 1.0), "none".into(), "none".into(), f)
        }
        oc_core::TransitionKind::FadeWhite => {
            let f = format!("{filter} brightness({:.3})", 1.0 + mix);
            ((opacity * (1.0 - mix)).clamp(0.0, 1.0), "none".into(), "none".into(), f)
        }
        oc_core::TransitionKind::CircleOpen => {
            let r = mix * 80.0;
            (
                opacity,
                "none".into(),
                format!("circle({r:.1}% at 50% 50%)"),
                filter.into(),
            )
        }
        oc_core::TransitionKind::CircleClose => {
            let r = (1.0 - mix) * 80.0;
            (
                opacity,
                "none".into(),
                format!("circle({r:.1}% at 50% 50%)"),
                filter.into(),
            )
        }
        oc_core::TransitionKind::Radial => {
            let r = mix * 100.0;
            (
                opacity,
                "none".into(),
                format!("circle({r:.1}% at 50% 50%)"),
                filter.into(),
            )
        }
        oc_core::TransitionKind::Pixelize => {
            let f = format!("{filter} contrast({:.2}) saturate({:.2})", 1.0 + mix, 1.0 - mix * 0.4);
            ((opacity * (1.0 - mix * 0.5)).clamp(0.0, 1.0), "none".into(), "none".into(), f)
        }
        _ => (
            (opacity * (1.0 - mix)).clamp(0.0, 1.0),
            "none".into(),
            "none".into(),
            filter.into(),
        ),
    }
}

fn incoming_preview_css(kind: oc_core::TransitionKind, mix: f64) -> (f64, String) {
    if let Some((dx, dy)) = kind.slide_delta() {
        return (
            1.0,
            format!(
                "translate({:.1}%, {:.1}%)",
                -dx * (1.0 - mix) * 100.0,
                -dy * (1.0 - mix) * 100.0
            ),
        );
    }
    match kind {
        oc_core::TransitionKind::FadeBlack | oc_core::TransitionKind::FadeWhite => {
            (mix.clamp(0.0, 1.0), "none".into())
        }
        oc_core::TransitionKind::Cut => (0.0, "none".into()),
        _ => (mix.clamp(0.15, 1.0), "none".into()),
    }
}

fn caption_line(text: &str, into: f64, span: f64) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() <= 7 {
        return words.join(" ");
    }
    let groups = words.len().div_ceil(6).max(1);
    let idx = ((into / span.max(0.01)) * groups as f64).floor() as usize;
    words
        .chunks(6)
        .nth(idx.min(groups - 1))
        .map(|group| group.join(" "))
        .unwrap_or_default()
}

pub fn apply_monitor_look(
    engine: &oc_core::Timeline,
    library: &[MediaItem],
    now: f64,
    playing: bool,
) {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let t = oc_core::Time::from_seconds(now);
    let mut filter = "none".to_string();
    let mut opacity = 1.0_f64;
    let mut mix = 0.0_f64;
    let mut kind = oc_core::TransitionKind::Cut;
    let mut vignette = 0.0_f32;
    let mut grain = 0.0_f32;
    let mut zoom = 1.0_f32;
    let mut pan_x = 0.0_f32;
    let mut pan_y = 0.0_f32;
    let mut volume = 1.0_f64;
    let mut graphics: Vec<(String, String, Option<f32>, Option<f32>)> = Vec::new();
    let mut next_url = String::new();
    let mut next_src = 0.0_f64;
    let mut mask: Option<oc_core::AlphaShape> = None;
    let mut cube_id: Option<u32> = None;
    let mut look_clip = String::new();

    for track in &engine.tracks {
        if track.hidden || track.muted {
            continue;
        }
        for clip in &track.clips {
            if clip.disabled || !clip.contains(t) {
                continue;
            }
            let local = (t - clip.start).as_seconds();
            let fade = clip.look.fade_gain(local, clip.duration.as_seconds());
            match &clip.kind {
                oc_core::ClipKind::Video { transform } => {
                    zoom = transform.scale;
                    pan_x = transform.x;
                    pan_y = transform.y;
                    filter = css_filter(clip.look.grade, clip.look.fx);
                    mask = clip.look.mask;
                    cube_id = clip.look.grade.cube;
                    look_clip = clip.id.to_string();
                    opacity = fade;
                    vignette = clip.look.fx.vignette;
                    grain = clip.look.fx.grain;
                    kind = clip.look.transition;
                    let next = engine.tracks.iter().flat_map(|t| t.clips.iter()).find(|other| {
                        other.id != clip.id
                            && matches!(other.kind, oc_core::ClipKind::Video { .. })
                            && !other.disabled
                            && is_join_ui(
                                clip.start.as_seconds(),
                                clip.end().as_seconds(),
                                other.start.as_seconds(),
                            )
                    });
                    let dur = next
                        .map(|n| {
                            clip.look
                                .mix_window(clip.duration.as_seconds(), n.duration.as_seconds())
                        })
                        .unwrap_or(0.0);
                    if dur > 1e-4 {
                        let start = clip.end().as_seconds() - dur;
                        if now >= start && now < clip.end().as_seconds() {
                            if let Some(other) = next {
                                mix = ((now - start) / dur).clamp(0.0, 1.0);
                                if let Some(id) = other.media_id {
                                    if let Some(item) =
                                        library.iter().find(|m| m.id == id.to_string())
                                    {
                                        next_url = item.url.clone();
                                        next_src = other.source_in.as_seconds()
                                            + (now - other.start.as_seconds()).max(0.0);
                                    }
                                }
                            }
                        }
                    }
                }
                oc_core::ClipKind::Audio { volume: v, ducked } => {
                    let duck = if *ducked { 0.3 } else { 1.0 };
                    volume = fade * f64::from(*v) * duck;
                }
                oc_core::ClipKind::Graphic { graphic } => {
                    let cls = match graphic.kind {
                        oc_core::GraphicKind::Title => "title",
                        oc_core::GraphicKind::LowerThird => "lower",
                        oc_core::GraphicKind::Card => "card-gfx",
                        oc_core::GraphicKind::Shape => "shape",
                        oc_core::GraphicKind::Sticker => "sticker",
                    };
                    graphics.push((cls.into(), graphic.text.clone(), graphic.x, graphic.y));
                }
                oc_core::ClipKind::Caption { style: _, cues } => {
                    let local_t = oc_core::Time::from_ticks((t - clip.start).as_ticks());
                    if let Some(cue) = cues.iter().find(|c| local_t >= c.start && local_t < c.end) {
                        let span = (cue.end - cue.start).as_seconds().max(0.3);
                        let into = (local_t - cue.start).as_seconds().clamp(0.0, span);
                        graphics.push((
                            "caption".into(),
                            caption_line(&cue.text, into, span),
                            None,
                            None,
                        ));
                    }
                }
            }
        }
    }

    if let Some(clip) = selected_video(engine, t) {
        filter = css_filter(clip.look.grade, clip.look.fx);
        mask = clip.look.mask;
        cube_id = clip.look.grade.cube;
        look_clip = clip.id.to_string();
        if let oc_core::ClipKind::Video { transform } = &clip.kind {
            zoom = transform.scale;
            pan_x = transform.x;
            pan_y = transform.y;
        }
        vignette = clip.look.fx.vignette;
        grain = clip.look.fx.grain;
    }
    remember_look(engine, &filter, cube_id, mask, &look_clip);

    let person = frame_card(engine, now, "Front");
    if let Some(video) = preview_video() {
        let (a, mix_tf, clip_path, extra_filter) = mix_preview_css(kind, mix, opacity, &filter);
        let transform = if person.is_some() {
            "none".to_string()
        } else if (zoom - 1.0).abs() > 0.01 || pan_x.abs() > 0.5 || pan_y.abs() > 0.5 {
            let punch = format!("scale({zoom:.3}) translate({pan_x:.1}px,{pan_y:.1}px)");
            if mix_tf == "none" {
                punch
            } else {
                format!("{punch} {mix_tf}")
            }
        } else {
            mix_tf
        };
        let pip = person
            .map(|card| {
                format!(
                    "left:{:.2}%;top:{:.2}%;width:{:.2}%;height:{:.2}%;right:auto;bottom:auto;object-fit:cover;z-index:4;",
                    card.x * 100.0,
                    card.y * 100.0,
                    card.w * 100.0,
                    card.h * 100.0
                )
            })
            .unwrap_or_default();
        let _ = video.set_attribute(
            "style",
            &format!(
                "filter:{extra_filter};opacity:{a:.3};transform:{transform};clip-path:{clip_path};{pip}"
            ),
        );
        video.set_volume(volume.clamp(0.0, 1.0));
    }

    if let Some(b) = standby_video() {
        let show = mix > 0.0 && kind != oc_core::TransitionKind::Cut && !next_url.is_empty();
        let standby_sel = if front_is_b() {
            ".preview-video"
        } else {
            ".preview-video-b"
        };
        set_class_off(standby_sel, !show);
        if show {
            if b.get_attribute("data-url").unwrap_or_default() != next_url {
                let _ = b.set_attribute("data-url", &next_url);
                if b.get_attribute("data-media").unwrap_or_default().is_empty() {
                    b.set_src(&next_url);
                }
            }
            if b.ready_state() >= 2 && (b.current_time() - next_src).abs() > 0.12 {
                b.set_current_time(next_src.max(0.0));
            }
            let (b_op, b_tf) = incoming_preview_css(kind, mix);
            let _ = b.set_attribute("style", &format!("opacity:{b_op:.3};transform:{b_tf}"));
        }
    }

    if let Some(el) = doc.query_selector(".preview-vignette").ok().flatten() {
        let _ = el.set_attribute(
            "style",
            &format!(
                "box-shadow: inset 0 0 {}px rgba(0,0,0,{:.2})",
                80.0 + vignette * 140.0,
                vignette * 0.85
            ),
        );
        set_class_off(".preview-vignette", vignette < 0.02);
    }
    if let Some(el) = doc.query_selector(".preview-grain").ok().flatten() {
        let _ = el.set_attribute("style", &format!("opacity:{:.2}", grain));
        set_class_off(".preview-grain", grain < 0.02);
    }
    if let Some(layer) = doc.query_selector(".preview-gfx").ok().flatten() {
        let gfx_class = if engine.letterbox {
            "preview-gfx letterboxed"
        } else {
            "preview-gfx"
        };
        let _ = layer.set_attribute("class", gfx_class);
        layer.set_inner_html("");
        for (cls, text, x, y) in graphics {
            if let Some(node) = doc.create_element("div").ok() {
                let _ = node.set_attribute("class", &format!("gfx {cls}"));
                if let (Some(x), Some(y)) = (x, y) {
                    let (x, max_w) = label_clear_of_card(x, person);
                    let _ = node.set_attribute(
                        "style",
                        &format!(
                            "position:absolute;left:{:.2}%;top:{:.2}%;transform:translate(-50%,-50%);margin:0;width:max-content;max-width:{:.0}%;font-size:clamp(15px,3.2cqw,28px);line-height:1.15;text-align:center;",
                            x * 100.0,
                            y * 100.0,
                            max_w
                        ),
                    );
                }
                node.set_text_content(Some(&text));
                let _ = layer.append_child(&node);
            }
        }
    }
    paint_design(&doc, engine, library, now, playing);
    paint_monitor_matte(&doc, engine.letterbox);
    let _ = paint_grade_canvas();
}

/// Keep a label in the open side of the frame when the person window would cover it.
fn label_clear_of_card(x: f32, card: Option<oc_core::FrameCard>) -> (f32, f32) {
    let Some(card) = card else {
        return (x, 80.0);
    };
    let right = card.x + card.w;
    let inside = x > card.x + 0.02 && x < right - 0.02;
    if inside {
        let left_free = card.x;
        let right_free = 1.0 - right;
        if left_free >= right_free {
            let cx = (card.x * 0.5).clamp(0.12, (card.x - 0.06).max(0.12));
            return (cx, ((card.x - 0.04) * 100.0).clamp(24.0, 70.0));
        }
        let cx = (right + right_free * 0.5).clamp(right + 0.06, 0.88);
        return (cx, ((right_free - 0.04) * 100.0).clamp(24.0, 70.0));
    }
    if x <= card.x {
        return (x, ((card.x - 0.04) * 100.0).clamp(24.0, 80.0));
    }
    (x, ((1.0 - right - 0.04) * 100.0).clamp(24.0, 80.0))
}

fn frame_card(engine: &oc_core::Timeline, now: f64, track_name: &str) -> Option<oc_core::FrameCard> {
    let t = oc_core::Time::from_seconds(now);
    engine.tracks.iter().find_map(|track| {
        if track.name != track_name {
            return None;
        }
        track.clips.iter().find_map(|clip| {
            (!clip.disabled && clip.contains(t))
                .then_some(clip.look.card)
                .flatten()
        })
    })
}

fn hide_design() {
    set_class_off(".preview-design", true);
    set_class_off(".preview-design-clip", true);
    if let Some(video) = query_video(".preview-design-clip") {
        let _ = video.pause();
    }
}

fn paint_design(
    doc: &web_sys::Document,
    engine: &oc_core::Timeline,
    library: &[MediaItem],
    now: f64,
    playing: bool,
) {
    let t = oc_core::Time::from_seconds(now);
    let design = engine.tracks.iter().find_map(|track| {
        if track.name != "Design" {
            return None;
        }
        track.clips.iter().find(|clip| {
            !clip.disabled
                && clip.contains(t)
                && matches!(clip.kind, oc_core::ClipKind::Video { .. })
        })
    });
    let Some(clip) = design else {
        hide_design();
        return;
    };
    let Some(item) = clip.media_id.and_then(|id| {
        library
            .iter()
            .find(|item| item.id == id.to_string())
            .cloned()
    }) else {
        hide_design();
        return;
    };
    let style = if let Some(card) = clip.look.card {
        format!(
            "left:{:.2}%;top:{:.2}%;width:{:.2}%;height:{:.2}%;",
            card.x * 100.0,
            card.y * 100.0,
            card.w * 100.0,
            card.h * 100.0
        )
    } else {
        "left:0;top:0;width:100%;height:100%;".to_string()
    };
    if item.kind == MediaKind::Video {
        set_class_off(".preview-design", true);
        let Some(video) = query_video(".preview-design-clip") else {
            return;
        };
        let loaded = video.get_attribute("data-media").unwrap_or_default();
        if loaded != item.id {
            let _ = video.set_attribute("data-media", &item.id);
            video.set_src(&item.url);
            video.set_muted(true);
        }
        let _ = video.set_attribute("style", &style);
        set_class_off(".preview-design-clip", false);
        let local = (now - clip.start.as_seconds())
            .clamp(0.0, clip.duration.as_seconds().max(0.04));
        let ready = video.ready_state() >= 2;
        let drift = (video.current_time() - local).abs();
        if !playing {
            let _ = video.pause();
            if ready && drift > 0.04 {
                video.set_current_time(local);
            }
        } else if video.paused() {
            if ready && drift > 0.08 {
                video.set_current_time(local);
            }
            if ready {
                let _ = video.play();
            }
        } else if ready && drift > 0.45 {
            video.set_current_time(local);
        }
        return;
    }
    set_class_off(".preview-design-clip", true);
    if let Some(video) = query_video(".preview-design-clip") {
        let _ = video.pause();
    }
    let Some(img) = doc.query_selector(".preview-design").ok().flatten() else {
        return;
    };
    let _ = img.set_attribute("src", &item.url);
    let _ = img.set_attribute("style", &style);
    set_class_off(".preview-design", false);
}

pub fn sync_monitor(library: &[MediaItem], tracks: &[EditorTrack], now: f64, playing: bool) {
    let shot = clip_under(tracks, library, now);
    let next = following_shot(tracks, library, now);
    let video = preview_video();
    match shot {
        None => {
            if let Some(video) = video {
                let _ = video.pause();
            }
            set_class_off(".preview-video", true);
            set_class_off(".preview-video-b", true);
            set_class_off(".preview-image", true);
            set_class_off(".monitor-blank", false);
        }
        Some(shot) if shot.kind == MediaKind::Image => {
            if let Some(video) = video {
                let _ = video.pause();
            }
            if let Some(img) = web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.query_selector(".preview-image").ok().flatten())
            {
                let _ = img.set_attribute("src", &shot.url);
            }
            set_class_off(".preview-video", true);
            set_class_off(".preview-video-b", true);
            set_class_off(".preview-image", false);
            set_class_off(".monitor-blank", true);
        }
        Some(shot) => {
            let Some(video) = video else { return };
            if let Some(standby) = standby_video() {
                if let Some(next) = &next {
                    if next.kind == MediaKind::Video {
                        preroll(&standby, next);
                    }
                }
            }
            let loaded = video.get_attribute("data-media").unwrap_or_default();
            let media_changed = loaded != shot.media_id;
            if media_changed {
                let _ = video.set_attribute("data-media", &shot.media_id);
                video.set_src(&shot.url);
                video.set_muted(false);
                LAST_PLAY_MS.with(|cell| cell.set(0.0));
                LAST_SEEK_MS.with(|cell| cell.set(0.0));
            }
            let take = shot.duration.max(0.05);
            let src_time = (shot.source_in + (now - shot.start))
                .clamp(shot.source_in, shot.source_in + take);
            let src_end = shot.source_in + take;
            let ready = video.ready_state() >= 2;
            let paused = video.paused();
            let drift = (video.current_time() - src_time).abs();
            let since_play = js_sys::Date::now() - LAST_PLAY_MS.with(Cell::get);
            let near_end = now >= shot.start + take - 0.05
                || (ready && video.current_time() >= src_end - 0.04);
            let keep_rolling = next
                .as_ref()
                .is_some_and(|n| contiguous_source(&shot, n));

            if !playing {
                let _ = video.pause();
                if ready && drift > 0.04 {
                    seek_video(&video, src_time, false);
                }
            } else if near_end && ready {
                if keep_rolling {
                    if let Some(n) = &next {
                        set_playhead(n.start.max(now) + 1e-3);
                        LAST_TICK_MS.with(|cell| cell.set(js_sys::Date::now()));
                    }
                } else if next.as_ref().is_some_and(|n| {
                    n.kind == MediaKind::Video
                        && standby_video().is_some_and(|s| preroll_ready(&s, n))
                }) {
                    if let Some(n) = &next {
                        if let Some(standby) = standby_video() {
                            standby.set_muted(false);
                            let _ = standby.play();
                        }
                        let _ = video.pause();
                        video.set_muted(true);
                        swap_program();
                        set_playhead(n.start.max(now) + 1e-3);
                        LAST_PLAY_MS.with(|cell| cell.set(js_sys::Date::now()));
                        LAST_TICK_MS.with(|cell| cell.set(js_sys::Date::now()));
                    }
                } else if next.is_some() {
                    // Hold the outgoing frame until the next shot is seeked.
                    // Jumping the playhead first shows a blank or a frozen frame.
                    set_playhead((shot.start + take - 0.04).max(shot.start));
                    LAST_TICK_MS.with(|cell| cell.set(js_sys::Date::now()));
                } else {
                    let _ = video.pause();
                    set_playhead(shot.start + take);
                    LAST_TICK_MS.with(|cell| cell.set(js_sys::Date::now()));
                }
            } else if paused {
                if ready && (media_changed || drift > 0.08) {
                    seek_video(&video, src_time, media_changed);
                }
                if ready && since_play > 40.0 {
                    LAST_PLAY_MS.with(|cell| cell.set(js_sys::Date::now()));
                    video.set_muted(false);
                    let _ = video.play();
                }
            } else if ready && drift > 0.45 && since_play > 250.0 {
                seek_video(&video, src_time, false);
            }

            if playing && !paused && ready && !near_end {
                let derived = shot.start + (video.current_time() - shot.source_in);
                if derived.is_finite() && derived + 0.02 >= now {
                    set_playhead(derived.clamp(shot.start.max(now), shot.start + take - 1e-3));
                    LAST_TICK_MS.with(|cell| cell.set(js_sys::Date::now()));
                }
            }

            let a_front = !front_is_b();
            set_class_off(".preview-video", !a_front);
            set_class_off(".preview-video-b", a_front);
            set_class_off(".preview-image", true);
            set_class_off(".monitor-blank", true);
        }
    }
}

pub fn paint_clock() {
    paint_playhead(playhead_now());
}

pub fn paint_playhead(now: f64) {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let scroll = doc.query_selector(".tl-scroll").ok().flatten();
    let pps = scroll
        .as_ref()
        .and_then(|el| el.get_attribute("data-pps"))
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(24.0);
    let span = scroll
        .as_ref()
        .and_then(|el| el.get_attribute("data-span"))
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|s| *s > 0.0)
        .unwrap_or(1.0);
    if let Some(head) = doc.query_selector(".playhead").ok().flatten() {
        let _ = head.set_attribute("style", &format!("left: {}px", now * pps));
    }
    if let Some(label) = doc.query_selector(".tl-clock").ok().flatten() {
        label.set_text_content(Some(&format!(
            "{} / {}",
            format_clock(now),
            format_clock(span)
        )));
    }
}

pub fn seek_to(mut clock: Clock, time: f64) {
    let t = time.max(0.0);
    set_playhead(t);
    clock.current.set(t);
    reset_tick_clock();
    paint_playhead(t);
}

pub fn seek_by(clock: Clock, delta: f64) {
    let now = *clock.current.read();
    seek_to(clock, now + delta);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(id: &str, media: &str, start: f64, duration: f64) -> TimelineClip {
        TimelineClip {
            id: id.into(),
            media_id: media.into(),
            start,
            duration,
            source_in: 0.0,
            speed: 1.0,
            group_id: String::new(),
            link_id: String::new(),
            disabled: false,
            transition: String::new(),
            graphic: String::new(),
        }
    }

    fn video_track(clips: Vec<TimelineClip>) -> EditorTrack {
        video_track_named("v1", "V1", clips)
    }

    fn video_track_named(id: &str, name: &str, clips: Vec<TimelineClip>) -> EditorTrack {
        EditorTrack {
            id: id.into(),
            name: name.into(),
            kind: TrackKindUi::Video,
            muted: false,
            hidden: false,
            clips,
        }
    }

    fn item(id: &str, url: &str, duration: f64) -> MediaItem {
        MediaItem {
            id: id.into(),
            name: format!("{id}.mp4"),
            kind: MediaKind::Video,
            url: url.into(),
            content_type: "video/mp4".into(),
            duration,
        }
    }

    #[test]
    fn metadata_does_not_stretch_clip_over_next() {
        let mut library = vec![item("a", "blob:a", 5.0)];
        let mut tracks = vec![video_track(vec![
            clip("c1", "a", 0.0, 5.0),
            clip("c2", "b", 5.0, 5.0),
        ])];
        set_media_duration(&mut library, &mut tracks, "blob:a", 40.0);
        assert!((library[0].duration - 40.0).abs() < 1e-6);
        assert!(
            (tracks[0].clips[0].duration - 5.0).abs() < 1e-6,
            "must not cover the next clip"
        );
    }

    #[test]
    fn metadata_does_not_stretch_an_excerpt_to_the_file() {
        let mut library = vec![item("a", "blob:a", 359.0)];
        let mut tracks = vec![video_track(vec![
            clip("c1", "a", 0.0, 2.8),
            clip("c2", "a", 2.8, 6.1),
        ])];
        tracks[0].clips[1].source_in = 6.9;
        set_media_duration(&mut library, &mut tracks, "blob:a", 359.0);
        assert!((tracks[0].clips[0].duration - 2.8).abs() < 1e-6);
        assert!((tracks[0].clips[1].duration - 6.1).abs() < 1e-6);
    }

    #[test]
    fn clip_under_prefers_later_start_when_overlapping() {
        let library = vec![item("a", "blob:a", 40.0), item("b", "blob:b", 8.0)];
        let tracks = vec![video_track(vec![
            clip("c1", "a", 0.0, 40.0),
            clip("c2", "b", 5.0, 5.0),
        ])];
        let shot = clip_under(&tracks, &library, 6.0).unwrap();
        assert_eq!(shot.media_id, "b");
    }

    #[test]
    fn following_shot_is_the_next_take() {
        let library = vec![item("a", "blob:a", 40.0), item("b", "blob:b", 8.0)];
        let tracks = vec![video_track(vec![
            clip("c1", "a", 0.0, 4.0),
            clip("c2", "b", 4.0, 4.0),
        ])];
        let next = following_shot(&tracks, &library, 2.0).unwrap();
        assert_eq!(next.media_id, "b");
        assert!((next.start - 4.0).abs() < 1e-6);
        assert!(following_shot(&tracks, &library, 5.0).is_none());
    }

    #[test]
    fn following_shot_does_not_skip_a_hole() {
        let library = vec![item("a", "blob:a", 40.0)];
        let tracks = vec![video_track(vec![
            clip("c1", "a", 0.0, 10.0),
            clip("c2", "a", 30.0, 8.0),
        ])];
        assert!(following_shot(&tracks, &library, 8.0).is_none());
    }

    #[test]
    fn contiguous_excerpts_keep_rolling() {
        let a = ProgramShot {
            media_id: "x".into(),
            url: "blob:x".into(),
            kind: MediaKind::Video,
            start: 0.0,
            source_in: 10.0,
            duration: 4.0,
        };
        let b = ProgramShot {
            media_id: "x".into(),
            url: "blob:x".into(),
            kind: MediaKind::Video,
            start: 4.0,
            source_in: 14.0,
            duration: 3.0,
        };
        let c = ProgramShot {
            media_id: "x".into(),
            url: "blob:x".into(),
            kind: MediaKind::Video,
            start: 7.0,
            source_in: 30.0,
            duration: 2.0,
        };
        assert!(contiguous_source(&a, &b));
        assert!(!contiguous_source(&b, &c));
    }

    #[test]
    fn clip_under_keeps_the_speaker_when_a_label_has_no_picture() {
        let library = vec![item("a", "blob:a", 40.0), item("d", "blob:d", 6.0)];
        let mut label = clip("g1", "", 5.0, 6.0);
        label.graphic = "Bullish flag".into();
        let tracks = vec![
            video_track(vec![clip("c1", "a", 0.0, 40.0)]),
            video_track_named("design", "Design", vec![clip("d1", "d", 5.0, 6.0)]),
            video_track_named("front", "Front", vec![clip("f1", "a", 5.0, 6.0)]),
            video_track_named("gfx", "GFX", vec![label]),
        ];
        let shot = clip_under(&tracks, &library, 6.0).unwrap();
        assert_eq!(shot.media_id, "a");
        assert!((shot.start - 0.0).abs() < 1e-6);
        assert!(following_shot(&tracks, &library, 6.0).is_none());
    }

    #[test]
    fn close_editor_gaps_leaves_a_drawing_on_its_spoken_line() {
        let mut tracks = vec![video_track_named(
            "design",
            "Design",
            vec![clip("a", "d", 5.0, 6.0), clip("b", "d", 26.8, 6.0)],
        )];
        close_editor_gaps(&mut tracks);
        assert!((tracks[0].clips[0].start - 5.0).abs() < 1e-6);
        assert!((tracks[0].clips[1].start - 26.8).abs() < 1e-6);
    }

    #[test]
    fn label_moves_out_of_the_person_window() {
        let card = oc_core::FrameCard {
            x: 0.56,
            y: 0.18,
            w: 0.38,
            h: 0.64,
        };
        let (x, width) = label_clear_of_card(0.50, Some(card));
        assert!(x < card.x, "the words have to sit in the open side, x={x}");
        assert!(width <= card.x * 100.0);
        let (parked, _) = label_clear_of_card(0.27, Some(card));
        assert!((parked - 0.27).abs() < 1e-4);
    }

    #[test]
    fn a_label_clip_uses_its_words_as_the_name() {
        let mut label = clip("g", "", 0.0, 4.0);
        label.graphic = "Bullish flag".into();
        assert_eq!(clip_name(&label, None), "Bullish flag");
        let named = clip("p", "m", 0.0, 4.0);
        assert_eq!(clip_name(&named, Some("design-flag.jpg")), "design-flag.jpg");
        let bare = clip("q", "", 0.0, 4.0);
        assert_eq!(clip_name(&bare, None), "Clip");
    }

    #[test]
    fn overlap_prefers_later_track_clip() {
        let library = vec![item("a", "blob:a", 40.0), item("b", "blob:b", 20.0)];
        let tracks = vec![
            video_track(vec![clip("c1", "a", 0.0, 8.0)]),
            video_track_named("v2", "V2", vec![clip("c2", "b", 7.0, 12.0)]),
        ];
        let shot = clip_under(&tracks, &library, 7.4).unwrap();
        assert_eq!(shot.media_id, "b", "V2 must win the overlap, not V1");
        let next = following_shot(&tracks, &library, 6.0).unwrap();
        assert_eq!(next.media_id, "b");
    }
}

pub fn format_clock(secs: f64) -> String {
    let total = secs.max(0.0);
    let m = (total / 60.0) as u32;
    let s = (total % 60.0) as u32;
    format!("{m:02}:{s:02}")
}

const GRADE_W: u32 = 160;
const GRADE_H: u32 = 90;

#[derive(Clone)]
struct LiveLook {
    filter: String,
    cube_id: Option<u32>,
    cube: Option<oc_core::CubeLut>,
    mask: Option<oc_core::AlphaShape>,
    letterbox: bool,
    background: String,
    clip_id: String,
}

impl Default for LiveLook {
    fn default() -> Self {
        Self {
            filter: "none".into(),
            cube_id: None,
            cube: None,
            mask: None,
            letterbox: false,
            background: "#000000".into(),
            clip_id: String::new(),
        }
    }
}

struct MaskGesture {
    edge: String,
    x0: f64,
    y0: f64,
    origin: oc_core::AlphaShape,
    width: f64,
    height: f64,
}

thread_local! {
    static SELECTED: RefCell<Option<String>> = const { RefCell::new(None) };
    static LIVE: RefCell<LiveLook> = RefCell::new(LiveLook {
        filter: String::new(),
        cube_id: None,
        cube: None,
        mask: None,
        letterbox: false,
        background: String::new(),
        clip_id: String::new(),
    });
    static GESTURE: RefCell<Option<MaskGesture>> = const { RefCell::new(None) };
    static METER: RefCell<Option<web_sys::AnalyserNode>> = const { RefCell::new(None) };
    static METER_CTX: RefCell<Option<web_sys::AudioContext>> = const { RefCell::new(None) };
}

pub struct MonitorChrome {
    pub letterbox: bool,
    pub mask: bool,
    pub cube: bool,
}

/// What the monitor overlays should show for this playhead.
/// `selected` wins when it is a video clip under the playhead.
pub fn monitor_chrome(
    engine: &oc_core::Timeline,
    now: f64,
    selected: Option<&str>,
) -> MonitorChrome {
    let t = oc_core::Time::from_seconds(now);
    let mut mask = None;
    let mut cube = None;
    for track in &engine.tracks {
        if track.hidden || track.muted {
            continue;
        }
        for clip in &track.clips {
            if clip.disabled || !clip.contains(t) {
                continue;
            }
            if matches!(clip.kind, oc_core::ClipKind::Video { .. }) {
                mask = clip.look.mask;
                cube = clip.look.grade.cube;
            }
        }
    }
    if let Some(raw) = selected.and_then(|raw| Uuid::parse_str(raw).ok()) {
        if let Some((_, clip)) = engine.find_clip(oc_core::ClipId::from_uuid(raw)) {
            if !clip.disabled
                && clip.contains(t)
                && matches!(clip.kind, oc_core::ClipKind::Video { .. })
            {
                mask = clip.look.mask;
                cube = clip.look.grade.cube;
            }
        }
    }
    MonitorChrome {
        letterbox: engine.letterbox,
        mask: mask.is_some(),
        cube: cube.is_some(),
    }
}

pub fn set_selected_clip(id: Option<String>) {
    SELECTED.with(|slot| *slot.borrow_mut() = id.filter(|raw| !raw.is_empty()));
}

fn selected_video(engine: &oc_core::Timeline, at: oc_core::Time) -> Option<&oc_core::Clip> {
    let raw = SELECTED.with(|slot| slot.borrow().clone())?;
    let id = Uuid::parse_str(&raw).ok()?;
    let (_, clip) = engine.find_clip(oc_core::ClipId::from_uuid(id))?;
    if clip.disabled || !clip.contains(at) || !matches!(clip.kind, oc_core::ClipKind::Video { .. }) {
        None
    } else {
        Some(clip)
    }
}

fn remember_look(
    engine: &oc_core::Timeline,
    filter: &str,
    cube_id: Option<u32>,
    mask: Option<oc_core::AlphaShape>,
    clip_id: &str,
) {
    LIVE.with(|slot| {
        let mut look = slot.borrow_mut();
        look.filter = filter.to_string();
        look.mask = mask;
        look.letterbox = engine.letterbox;
        look.background = if engine.background.is_empty() {
            "#000000".into()
        } else {
            engine.background.clone()
        };
        look.clip_id = clip_id.to_string();
        if look.cube_id != cube_id {
            look.cube = cube_id.and_then(|id| engine.cubes.iter().find(|cube| cube.id == id).cloned());
            look.cube_id = cube_id;
        }
    });
}

fn paint_monitor_matte(doc: &web_sys::Document, letterbox: bool) {
    let (mask, background, dragging) = LIVE.with(|slot| {
        let look = slot.borrow();
        (look.mask, look.background.clone(), GESTURE.with(|g| g.borrow().is_some()))
    });
    if let Some(host) = doc.query_selector(".preview-mask-host").ok().flatten() {
        if let Some(mask) = mask {
            host.set_inner_html(&mask_svg(&mask, &background));
            set_class_off(".preview-mask-host", false);
        } else {
            host.set_inner_html("");
            set_class_off(".preview-mask-host", true);
        }
    }
    if let Some(bars) = doc.query_selector(".preview-letterbox").ok().flatten() {
        let _ = bars.set_attribute(
            "style",
            &format!(
                "background: linear-gradient(to bottom, {background} 12%, transparent 12%, transparent 88%, {background} 88%)"
            ),
        );
    }
    set_class_off(".preview-letterbox", !letterbox);
    if !dragging {
        if let Some(mask) = mask {
            place_mask_box(&mask);
            set_class_off("#mask-handles", false);
        } else {
            set_class_off("#mask-handles", true);
        }
    }
}

fn mask_svg(mask: &oc_core::AlphaShape, background: &str) -> String {
    let (outside, inside) = if mask.invert {
        ("black", "white")
    } else {
        ("white", "black")
    };
    let shape = match mask.shape {
        oc_core::MaskShape::Ellipse => format!(
            r#"<ellipse cx="{:.4}" cy="{:.4}" rx="{:.4}" ry="{:.4}" fill="{inside}"/>"#,
            mask.x,
            mask.y,
            mask.w * 0.5,
            mask.h * 0.5
        ),
        oc_core::MaskShape::Diamond => {
            let hw = mask.w * 0.5;
            let hh = mask.h * 0.5;
            format!(
                r#"<polygon points="{x:.4},{top:.4} {right:.4},{y:.4} {x:.4},{bottom:.4} {left:.4},{y:.4}" fill="{inside}"/>"#,
                x = mask.x,
                y = mask.y,
                top = mask.y - hh,
                right = mask.x + hw,
                bottom = mask.y + hh,
                left = mask.x - hw
            )
        }
        oc_core::MaskShape::Triangle => {
            let hw = mask.w * 0.5;
            let hh = mask.h * 0.5;
            format!(
                r#"<polygon points="{x:.4},{top:.4} {right:.4},{bottom:.4} {left:.4},{bottom:.4}" fill="{inside}"/>"#,
                x = mask.x,
                top = mask.y - hh,
                right = mask.x + hw,
                left = mask.x - hw,
                bottom = mask.y + hh
            )
        }
        oc_core::MaskShape::Rectangle => format!(
            r#"<rect x="{:.4}" y="{:.4}" width="{:.4}" height="{:.4}" fill="{inside}"/>"#,
            mask.x - mask.w * 0.5,
            mask.y - mask.h * 0.5,
            mask.w,
            mask.h
        ),
    };
    let blur = if mask.feather > 0.01 {
        format!(
            r#"<filter id="oc-feather"><feGaussianBlur stdDeviation="{:.4}"/></filter>"#,
            mask.feather * 0.04
        )
    } else {
        String::new()
    };
    let filter = if mask.feather > 0.01 {
        r#" filter="url(#oc-feather)""#
    } else {
        ""
    };
    format!(
        r#"<svg viewBox="0 0 1 1" preserveAspectRatio="none"><defs>{blur}<mask id="oc-matte" maskContentUnits="objectBoundingBox"><g{filter}><rect x="0" y="0" width="1" height="1" fill="{outside}"/>{shape}</g></mask></defs><rect x="0" y="0" width="1" height="1" fill="{background}" mask="url(#oc-matte)"/></svg>"#
    )
}

fn place_mask_box(mask: &oc_core::AlphaShape) {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let Some(box_el) = doc.get_element_by_id("mask-box") else {
        return;
    };
    let left = (mask.x - mask.w * 0.5) * 100.0;
    let top = (mask.y - mask.h * 0.5) * 100.0;
    let _ = box_el.set_attribute(
        "style",
        &format!(
            "left:{left:.2}%;top:{top:.2}%;width:{:.2}%;height:{:.2}%",
            mask.w * 100.0,
            mask.h * 100.0
        ),
    );
}

pub fn mask_down(evt: &Event<PointerData>) {
    let Some(edge) = pointer_edge(evt) else {
        return;
    };
    let Some(origin) = LIVE.with(|slot| slot.borrow().mask) else {
        return;
    };
    let origin_pt = evt.client_coordinates();
    let x0 = origin_pt.x;
    let y0 = origin_pt.y;
    let (width, height) = monitor_size();
    GESTURE.with(|slot| {
        *slot.borrow_mut() = Some(MaskGesture {
            edge,
            x0,
            y0,
            origin,
            width,
            height,
        })
    });
}

pub fn mask_move(evt: &Event<PointerData>) {
    let Some(next) = dragged_mask(evt) else {
        return;
    };
    place_mask_box(&next);
    let background = LIVE.with(|slot| slot.borrow().background.clone());
    if let Some(host) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.query_selector(".preview-mask-host").ok().flatten())
    {
        host.set_inner_html(&mask_svg(&next, &background));
    }
}

pub fn mask_up(evt: &Event<PointerData>) -> Option<(String, oc_core::AlphaShape)> {
    let shape = dragged_mask(evt);
    let clip_id = LIVE.with(|slot| slot.borrow().clip_id.clone());
    GESTURE.with(|slot| *slot.borrow_mut() = None);
    let shape = shape?;
    if clip_id.is_empty() {
        None
    } else {
        Some((clip_id, shape))
    }
}

fn dragged_mask(evt: &Event<PointerData>) -> Option<oc_core::AlphaShape> {
    GESTURE.with(|slot| {
        let gesture = slot.borrow();
        let gesture = gesture.as_ref()?;
        let point = evt.client_coordinates();
        let dx = ((point.x - gesture.x0) / gesture.width.max(1.0)) as f32;
        let dy = ((point.y - gesture.y0) / gesture.height.max(1.0)) as f32;
        let mut shape = gesture.origin;
        match gesture.edge.as_str() {
            "e" => {
                shape.w += dx;
                shape.x += dx * 0.5;
            }
            "w" => {
                shape.w -= dx;
                shape.x += dx * 0.5;
            }
            "s" => {
                shape.h += dy;
                shape.y += dy * 0.5;
            }
            "n" => {
                shape.h -= dy;
                shape.y += dy * 0.5;
            }
            "ne" => {
                shape.w += dx;
                shape.x += dx * 0.5;
                shape.h -= dy;
                shape.y += dy * 0.5;
            }
            "nw" => {
                shape.w -= dx;
                shape.x += dx * 0.5;
                shape.h -= dy;
                shape.y += dy * 0.5;
            }
            "se" => {
                shape.w += dx;
                shape.x += dx * 0.5;
                shape.h += dy;
                shape.y += dy * 0.5;
            }
            "sw" => {
                shape.w -= dx;
                shape.x += dx * 0.5;
                shape.h += dy;
                shape.y += dy * 0.5;
            }
            _ => {
                shape.x += dx;
                shape.y += dy;
            }
        }
        shape.w = shape.w.clamp(0.05, 1.0);
        shape.h = shape.h.clamp(0.05, 1.0);
        shape.x = shape.x.clamp(shape.w * 0.5, 1.0 - shape.w * 0.5);
        shape.y = shape.y.clamp(shape.h * 0.5, 1.0 - shape.h * 0.5);
        Some(shape)
    })
}

fn pointer_edge(evt: &Event<PointerData>) -> Option<String> {
    let data = evt.data();
    let native = data.downcast::<web_sys::PointerEvent>()?;
    let target = native.target()?.dyn_into::<web_sys::Element>().ok()?;
    let el = if target.has_attribute("data-edge") {
        target
    } else {
        target.closest("[data-edge]").ok().flatten()?
    };
    el.get_attribute("data-edge")
}

fn monitor_size() -> (f64, f64) {
    web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.query_selector(".monitor").ok().flatten())
        .map(|el| {
            let rect = el.get_bounding_client_rect();
            (rect.width().max(1.0), rect.height().max(1.0))
        })
        .unwrap_or((16.0, 9.0))
}

pub fn graded_frame() -> Option<Vec<u8>> {
    paint_grade_canvas()
}

fn paint_grade_canvas() -> Option<Vec<u8>> {
    let video = preview_video()?;
    if video.ready_state() < 2 {
        set_class_off("#grade-canvas", true);
        return None;
    }
    let doc = web_sys::window()?.document()?;
    let canvas = doc
        .get_element_by_id("grade-canvas")?
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .ok()?;
    canvas.set_width(GRADE_W);
    canvas.set_height(GRADE_H);
    let ctx = canvas
        .get_context("2d")
        .ok()
        .flatten()?
        .dyn_into::<web_sys::CanvasRenderingContext2d>()
        .ok()?;
    let (filter, show_cube) = LIVE.with(|slot| {
        let look = slot.borrow();
        (look.filter.clone(), look.cube_id.is_some())
    });
    let _ = ctx.set_filter(&filter);
    ctx.draw_image_with_html_video_element_and_dw_and_dh(
        &video,
        0.0,
        0.0,
        GRADE_W as f64,
        GRADE_H as f64,
    )
    .ok()?;
    let _ = ctx.set_filter("none");
    let image = ctx
        .get_image_data(0.0, 0.0, GRADE_W as f64, GRADE_H as f64)
        .ok()?;
    let mut pixels = image.data().0;
    apply_live_pixels(&mut pixels, GRADE_W, GRADE_H);
    set_class_off("#grade-canvas", !show_cube);
    if show_cube {
        let array = js_sys::Uint8ClampedArray::from(pixels.as_slice());
        if let Ok(painted) = web_sys::ImageData::new_with_js_u8_clamped_array_and_sh(
            &array,
            GRADE_W,
            GRADE_H,
        ) {
            let _ = ctx.put_image_data(&painted, 0.0, 0.0);
        }
    }
    Some(pixels)
}

fn apply_live_pixels(pixels: &mut [u8], width: u32, height: u32) {
    LIVE.with(|slot| {
        let look = slot.borrow();
        if let Some(cube) = look.cube.as_ref() {
            for chunk in pixels.chunks_mut(4) {
                if chunk.len() < 3 {
                    break;
                }
                let sample = cube.sample(
                    chunk[0] as f32 / 255.0,
                    chunk[1] as f32 / 255.0,
                    chunk[2] as f32 / 255.0,
                );
                chunk[0] = (sample[0].clamp(0.0, 1.0) * 255.0) as u8;
                chunk[1] = (sample[1].clamp(0.0, 1.0) * 255.0) as u8;
                chunk[2] = (sample[2].clamp(0.0, 1.0) * 255.0) as u8;
            }
        }
        let bg = hex_rgb(&look.background);
        if let Some(mask) = look.mask {
            for y in 0..height {
                for x in 0..width {
                    let u = (x as f32 + 0.5) / width as f32;
                    let v = (y as f32 + 0.5) / height as f32;
                    let inside = shape_contains(mask, u, v);
                    let cover = if mask.invert { inside } else { !inside };
                    if cover {
                        let i = ((y * width + x) * 4) as usize;
                        pixels[i] = bg.0;
                        pixels[i + 1] = bg.1;
                        pixels[i + 2] = bg.2;
                    }
                }
            }
        }
        if look.letterbox {
            let bar = ((height as f32) * 0.12).round() as u32;
            for y in 0..height {
                if y >= bar && y + bar < height {
                    continue;
                }
                for x in 0..width {
                    let i = ((y * width + x) * 4) as usize;
                    pixels[i] = bg.0;
                    pixels[i + 1] = bg.1;
                    pixels[i + 2] = bg.2;
                }
            }
        }
    });
}

fn shape_contains(mask: oc_core::AlphaShape, u: f32, v: f32) -> bool {
    let hw = (mask.w * 0.5).max(0.001);
    let hh = (mask.h * 0.5).max(0.001);
    let nx = (u - mask.x) / hw;
    let ny = (v - mask.y) / hh;
    match mask.shape {
        oc_core::MaskShape::Rectangle => nx.abs() <= 1.0 && ny.abs() <= 1.0,
        oc_core::MaskShape::Ellipse => nx * nx + ny * ny <= 1.0,
        oc_core::MaskShape::Diamond => nx.abs() + ny.abs() <= 1.0,
        oc_core::MaskShape::Triangle => {
            let t = (ny + 1.0) * 0.5;
            t >= 0.0 && t <= 1.0 && nx.abs() <= t
        }
    }
}

fn hex_rgb(color: &str) -> (u8, u8, u8) {
    let hex = oc_core::canonical_color(color);
    let bytes = hex.trim_start_matches('#');
    let parse = |range: std::ops::Range<usize>| {
        u8::from_str_radix(bytes.get(range).unwrap_or("00"), 16).unwrap_or(0)
    };
    (parse(0..2), parse(2..4), parse(4..6))
}

pub fn resume_meter() {
    ensure_meter();
    METER_CTX.with(|slot| {
        if let Some(ctx) = slot.borrow().as_ref() {
            let _ = ctx.resume();
        }
    });
}

pub fn ensure_meter() {
    if METER.with(|slot| slot.borrow().is_some()) {
        return;
    }
    let Some(video) = preview_video() else {
        return;
    };
    let Ok(ctx) = web_sys::AudioContext::new() else {
        return;
    };
    let _ = ctx.resume();
    METER_CTX.with(|slot| *slot.borrow_mut() = Some(ctx.clone()));
    let media: &web_sys::HtmlMediaElement = video.unchecked_ref();
    let Ok(source) = ctx.create_media_element_source(media) else {
        return;
    };
    let Ok(analyser) = ctx.create_analyser() else {
        return;
    };
    analyser.set_fft_size(256);
    let dest = ctx.destination();
    let node: &web_sys::AudioNode = dest.unchecked_ref();
    // The element can be captured once. If the graph does not reach the
    // speakers, connect the source straight through so playback stays audible.
    if source.connect_with_audio_node(&analyser).is_err()
        || analyser.connect_with_audio_node(node).is_err()
    {
        let _ = source.connect_with_audio_node(node);
        return;
    }
    METER.with(|slot| *slot.borrow_mut() = Some(analyser));
}

pub fn meter_peak() -> f32 {
    METER.with(|slot| {
        let held = slot.borrow();
        let Some(analyser) = held.as_ref() else {
            return 0.0;
        };
        let bins = analyser.frequency_bin_count() as usize;
        let mut data = vec![0u8; bins.max(1)];
        analyser.get_byte_frequency_data(&mut data);
        data.into_iter().max().unwrap_or(0) as f32 / 255.0
    })
}
