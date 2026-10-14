mod api;
mod auth;
mod bind;
mod hold;
mod media;
mod pages;
mod studio;
mod toast;
mod tools;

use dioxus::html::FileData;
use dioxus::prelude::*;
#[cfg(target_arch = "wasm32")]
use media::item_from_blob;
use media::{
    Clock, DragSession, DragSource, EditMode, EditTool, EditorTrack, MediaItem, MediaKind,
    TrackKindUi, advance_playhead, apply_monitor_look, capture_pointer, clamp_pps, clip_duration,
    clip_name, commit_drag, display_tracks, film_tiles, fit_scale, format_clock, lane_height,
    max_timeline_h, next_track_name, paint_clock, paint_playhead, place_clip, playhead_now,
    preview_video, program_end, reset_tick_clock, ruler_marks_nle, scroll_left, seek_by,
    set_media_duration, set_playhead, sync_monitor, timeline_end, timeline_viewport_h,
    timeline_viewport_w, update_drag, video_duration_from_src,
};
use oc_core::TimelineEditMode;
use oc_core::{
    ExportPreset, Fx, Grade, Graphic, Op, Timeline as EngineTimeline, TrackKind, TransitionKind,
};
use oc_tools::{ToolId, actions as cut_actions, modes as edit_tools, track_actions};
use pages::{Export, Login, NewProject, Projects};
use toast::{ToastProvider, show_toast};
use wasm_bindgen::JsCast;

const CSS: &str = include_str!("../assets/style.css");

#[derive(Clone, Routable, PartialEq)]
pub enum Route {
    #[route("/")]
    Login {},
    #[route("/projects")]
    Projects {},
    #[route("/projects/new")]
    NewProject {},
    #[route("/:id/workspace")]
    Workspace { id: String },
    #[route("/:id/export")]
    Export { id: String },
}

#[derive(Clone, Copy, PartialEq)]
enum AssetTab {
    Media,
    Text,
    Captions,
    Audio,
    Elements,
    Transitions,
    Visuals,
    Settings,
}

impl AssetTab {
    const ALL: [Self; 8] = [
        Self::Media,
        Self::Text,
        Self::Captions,
        Self::Audio,
        Self::Elements,
        Self::Transitions,
        Self::Visuals,
        Self::Settings,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Media => "Media",
            Self::Text => "Text",
            Self::Captions => "Captions",
            Self::Audio => "Audio",
            Self::Elements => "Elements",
            Self::Transitions => "Transitions",
            Self::Visuals => "Visuals",
            Self::Settings => "Settings",
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Aspect {
    Landscape,
    Vertical,
    Square,
    Classic,
}

impl Aspect {
    fn class(self) -> &'static str {
        match self {
            Self::Landscape => "monitor",
            Self::Vertical => "monitor ar-9-16",
            Self::Square => "monitor ar-1-1",
            Self::Classic => "monitor ar-4-3",
        }
    }

    fn size(self) -> &'static str {
        match self {
            Self::Landscape => "1920 × 1080",
            Self::Vertical => "1080 × 1920",
            Self::Square => "1080 × 1080",
            Self::Classic => "1440 × 1080",
        }
    }

    fn pixels(self) -> (u32, u32) {
        match self {
            Self::Landscape => (1920, 1080),
            Self::Vertical => (1080, 1920),
            Self::Square => (1080, 1080),
            Self::Classic => (1440, 1080),
        }
    }

    fn ratio(self) -> &'static str {
        match self {
            Self::Landscape => "16:9",
            Self::Vertical => "9:16",
            Self::Square => "1:1",
            Self::Classic => "4:3",
        }
    }

    fn from_pixels(width: u32, height: u32) -> Self {
        if width == height {
            Self::Square
        } else if height > width {
            Self::Vertical
        } else if (width as f64 / height as f64 - 4.0 / 3.0).abs() < 0.08 {
            Self::Classic
        } else {
            Self::Landscape
        }
    }
}

#[derive(Clone, PartialEq)]
struct HeldImport {
    id: String,
    name: String,
    content_type: String,
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    static HELD_FILES: std::cell::RefCell<std::collections::HashMap<String, web_sys::File>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

fn remember_import(id: &str, file: &web_sys::File) {
    #[cfg(target_arch = "wasm32")]
    HELD_FILES.with(|slot| {
        slot.borrow_mut().insert(id.to_string(), file.clone());
    });
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (id, file);
    }
}

fn held_file(id: &str) -> Option<web_sys::File> {
    #[cfg(target_arch = "wasm32")]
    {
        HELD_FILES.with(|slot| slot.borrow().get(id).cloned())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = id;
        None
    }
}

fn forget_import(id: &str) {
    #[cfg(target_arch = "wasm32")]
    HELD_FILES.with(|slot| {
        slot.borrow_mut().remove(id);
    });
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = id;
    }
}

#[derive(Clone, Copy)]
struct CtxHeld(Signal<Vec<HeldImport>>);

#[derive(Clone, Copy)]
struct CtxUploading(Signal<Vec<String>>);

fn begin_upload(mut uploading: Signal<Vec<String>>, id: &str) {
    IN_FLIGHT.with(|slot| {
        slot.borrow_mut().insert(id.to_string());
    });
    let mut list = uploading.write();
    if !list.iter().any(|row| row == id) {
        list.push(id.to_string());
    }
}

fn end_upload(mut uploading: Signal<Vec<String>>, id: &str) {
    IN_FLIGHT.with(|slot| {
        slot.borrow_mut().remove(id);
    });
    uploading.write().retain(|row| row != id);
}

fn note_upload(
    uploading: Signal<Vec<String>>,
    id: &str,
    name: &str,
    result: Result<(), String>,
) {
    end_upload(uploading, id);
    match result {
        Ok(()) => show_toast().success(format!("{name} saved in this browser")),
        Err(err) => show_toast().error(format!("{name} was not saved in this browser: {err}")),
    }
}

#[derive(Clone, Copy)]
struct CtxProject(Signal<String>);
#[derive(Clone, Copy)]
struct CtxTargetTrack(Signal<String>);
#[derive(Clone, Copy)]
struct CtxActive(Signal<Option<String>>);
#[derive(Clone, Copy)]
struct CtxSelected(Signal<Option<String>>);
#[derive(Clone, Copy)]
struct CtxZoom(Signal<f64>);
#[derive(Clone, Copy)]
struct CtxTimelineH(Signal<f64>);

#[derive(Clone, Copy, PartialEq)]
pub(crate) struct WorkspaceSave {
    pub project_id: Signal<String>,
    pub tracks: Signal<Vec<EditorTrack>>,
    pub engine: Signal<EngineTimeline>,
    pub aspect: Signal<Aspect>,
    pub persist_q: Signal<Vec<Vec<Op>>>,
    pub persist_busy: Signal<bool>,
    pub undo: Signal<oc_core::UndoStack>,
}

fn persist(save: WorkspaceSave) {
    let pid = save.project_id.peek().clone();
    if pid.is_empty() {
        return;
    }
    let (width, height) = save.aspect.peek().pixels();
    let timeline =
        bind::timeline_from_tracks(&save.tracks.peek(), &save.engine.peek(), width, height);
    let mut engine = save.engine;
    spawn(async move {
        if let Ok(next) = api::save_timeline(&pid, timeline).await {
            engine.set(next);
        }
    });
}

thread_local! {
    static FRAME_CHOSEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static FRAME_ADOPTED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn frame_chosen() -> bool {
    FRAME_CHOSEN.with(std::cell::Cell::get)
}

fn frame_adopted() -> bool {
    FRAME_ADOPTED.with(std::cell::Cell::get)
}

fn choose_frame() {
    FRAME_CHOSEN.with(|flag| flag.set(true));
}

fn is_camera_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    !name.starts_with("motion-") && !name.starts_with("broll-")
}

fn camera_item(tracks: &[EditorTrack], library: &[MediaItem], now: f64) -> Option<MediaItem> {
    if let Some(shot) = crate::media::clip_under(tracks, library, now) {
        if shot.kind == MediaKind::Video {
            if let Some(item) = library.iter().find(|item| item.id == shot.media_id) {
                if item.kind == MediaKind::Video && is_camera_name(&item.name) {
                    return Some(item.clone());
                }
            }
        }
    }
    library
        .iter()
        .find(|item| item.kind == MediaKind::Video && is_camera_name(&item.name))
        .cloned()
}

fn measured_display(item: &MediaItem) -> Option<(u32, u32)> {
    let doc = web_sys::window()?.document()?;
    let list = doc.query_selector_all("video").ok()?;
    for i in 0..list.length() {
        let video = list.item(i)?.dyn_into::<web_sys::HtmlVideoElement>().ok()?;
        if video.current_src() != item.url && video.src() != item.url {
            continue;
        }
        let width = video.video_width();
        let height = video.video_height();
        if width > 1 && height > 1 {
            return Some((width, height));
        }
    }
    None
}

fn adopt_frame(mut save: WorkspaceSave, width: u32, height: u32, force: bool) {
    if width <= 1 || height <= 1 {
        return;
    }
    if !force && (frame_chosen() || frame_adopted()) {
        return;
    }
    let (width, height) = oc_tools::frame_for_picture(width, height, 0);
    let next = Aspect::from_pixels(width, height);
    if *save.aspect.peek() != next {
        save.aspect.set(next);
        persist(save);
    }
    if force {
        choose_frame();
    } else {
        FRAME_ADOPTED.with(|flag| flag.set(true));
    }
}

fn store_frame(save: WorkspaceSave, media_id: &str, width: u32, height: u32, force: bool) {
    let pid = save.project_id.peek().clone();
    let media_id = media_id.to_string();
    if !pid.is_empty() && width > 1 && height > 1 {
        spawn(async move {
            let _ = api::patch_media_frame(&pid, &media_id, width, height).await;
        });
    }
    adopt_frame(save, width, height, force);
}

enum FrameMiss {
    NoPicture,
    Unmeasured,
    Unreadable,
}

async fn probe_camera_frame(
    save: WorkspaceSave,
    library: Signal<Vec<MediaItem>>,
    force: bool,
) -> Result<(), FrameMiss> {
    if !force && (frame_chosen() || frame_adopted()) {
        return Ok(());
    }
    let Some(item) = camera_item(&save.tracks.peek(), &library.peek(), playhead_now()) else {
        return Err(FrameMiss::NoPicture);
    };
    if let Some((width, height)) = measured_display(&item) {
        store_frame(save, &item.id, width, height, force);
        return Ok(());
    }
    #[cfg(target_arch = "wasm32")]
    {
        let pid = save.project_id.peek().clone();
        let id = item.id.clone();
        let (width, height) = read_held_size(&pid, &id).await?;
        if save.project_id.peek().as_str() != pid {
            return Ok(());
        }
        store_frame(save, &id, width, height, force);
        return Ok(());
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = item;
        Err(FrameMiss::Unmeasured)
    }
}

#[cfg(target_arch = "wasm32")]
async fn read_held_size(project_id: &str, media_id: &str) -> Result<(u32, u32), FrameMiss> {
    if let Some(file) = held_file(media_id) {
        return media::picture_size(&file)
            .await
            .ok_or(FrameMiss::Unreadable);
    }
    let blob = hold::load_blob(project_id, media_id)
        .await
        .map_err(|_| FrameMiss::Unmeasured)?;
    media::picture_size(&blob)
        .await
        .ok_or(FrameMiss::Unreadable)
}

fn note_loaded_picture(
    save: WorkspaceSave,
    library: &[MediaItem],
    video: &web_sys::HtmlVideoElement,
) {
    let src = video.current_src();
    let raw = video.src();
    let Some(item) = library
        .iter()
        .find(|item| item.url == src || item.url == raw)
    else {
        return;
    };
    if item.kind != MediaKind::Video || !is_camera_name(&item.name) {
        return;
    }
    let width = video.video_width();
    let height = video.video_height();
    if width > 1 && height > 1 {
        store_frame(save, &item.id, width, height, false);
        return;
    }
    #[cfg(target_arch = "wasm32")]
    {
        let pid = save.project_id.peek().clone();
        let id = item.id.clone();
        spawn(async move {
            if let Ok((width, height)) = read_held_size(&pid, &id).await {
                store_frame(save, &id, width, height, false);
            }
        });
    }
}

fn caption_under(timeline: &EngineTimeline, at: f64) -> Option<(oc_core::ClipId, usize, String)> {
    for track in &timeline.tracks {
        if track.hidden || track.kind != TrackKind::Caption {
            continue;
        }
        for clip in &track.clips {
            if clip.disabled {
                continue;
            }
            let start = clip.start.as_seconds();
            if at + 1e-4 < start || at >= start + clip.duration.as_seconds() {
                continue;
            }
            let local = at - start;
            let oc_core::ClipKind::Caption { cues, .. } = &clip.kind else {
                continue;
            };
            for (index, cue) in cues.iter().enumerate() {
                if local + 1e-4 >= cue.start.as_seconds() && local < cue.end.as_seconds() {
                    return Some((clip.id, index, cue.text.clone()));
                }
            }
        }
    }
    None
}

#[derive(Clone, Copy, PartialEq)]
enum ChatRole {
    User,
    Bot,
    Tool,
    Status,
    Thought,
}

#[derive(Clone, PartialEq)]
struct ChatMsg {
    role: ChatRole,
    text: String,
    tool_id: String,
    tool_name: String,
    tool_status: String,
    tool_args: String,
    tool_result: String,
    /// Expanded body. Pending tools start open; finished ones collapse.
    open: bool,
}

impl ChatMsg {
    fn user(text: impl Into<String>) -> Self {
        Self {
            role: ChatRole::User,
            text: text.into(),
            tool_id: String::new(),
            tool_name: String::new(),
            tool_status: String::new(),
            tool_args: String::new(),
            tool_result: String::new(),
            open: false,
        }
    }

    fn bot(text: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Bot,
            text: text.into(),
            tool_id: String::new(),
            tool_name: String::new(),
            tool_status: String::new(),
            tool_args: String::new(),
            tool_result: String::new(),
            open: false,
        }
    }

    fn status(text: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Status,
            text: text.into(),
            tool_id: String::new(),
            tool_name: String::new(),
            tool_status: String::new(),
            tool_args: String::new(),
            tool_result: String::new(),
            open: false,
        }
    }
}

fn main() {
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    rsx! {
        style { "{CSS}" }
        ToastProvider {
            Router::<Route> {}
        }
    }
}

/// Drops the tab-leave warning when the last workspace unmounts.
/// A newer workspace keeps its own handler if this one drops second.
struct LeaveGuard {
    ticket: u64,
    closure: Option<wasm_bindgen::closure::Closure<dyn FnMut(web_sys::BeforeUnloadEvent)>>,
}

impl Drop for LeaveGuard {
    fn drop(&mut self) {
        let current = LEAVE_GEN.with(|slot| slot.get());
        if current == self.ticket {
            if let Some(win) = web_sys::window() {
                win.set_onbeforeunload(None);
            }
        }
        self.closure.take();
    }
}

std::thread_local! {
    static LEAVE_GEN: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

fn current_load(ticket: u64, tickets: Signal<u64>) -> bool {
    *tickets.peek() == ticket
}

/// Signals that move together when the open workspace changes.
#[derive(Clone, Copy)]
struct WorkspaceView {
    load_ticket: Signal<u64>,
    name: Signal<String>,
    open_tabs: Signal<Vec<OpenTab>>,
    tracks: Signal<Vec<EditorTrack>>,
    clock: Clock,
    aspect: Signal<Aspect>,
    target_track: Signal<String>,
    engine: Signal<EngineTimeline>,
    undo: Signal<oc_core::UndoStack>,
    library: Signal<Vec<MediaItem>>,
    active: Signal<Option<String>>,
    messages: Signal<Vec<ChatMsg>>,
    draft: Signal<String>,
    selected: Signal<Option<String>>,
    held: Signal<Vec<HeldImport>>,
    uploading: Signal<Vec<String>>,
    drag: Signal<Option<DragSession>>,
    save: WorkspaceSave,
}

/// One workspace kept for the rest of this browser tab.
/// The blob URLs stay valid, so the filmstrips and the preview do not reload.
#[derive(Clone)]
struct WorkspaceSnap {
    name: String,
    tracks: Vec<EditorTrack>,
    engine: EngineTimeline,
    library: Vec<MediaItem>,
    active: Option<String>,
    aspect: Aspect,
    playhead: f64,
    duration: f64,
    target_track: String,
    undo: oc_core::UndoStack,
    held: Vec<HeldImport>,
    messages: Vec<ChatMsg>,
    draft: String,
    selected: Option<String>,
}

struct StoredSnap {
    ticket: u64,
    /// Written when the user leaves. A fill that finishes afterwards may add
    /// blob URLs, and it may not replace the timeline.
    pinned: bool,
    snap: WorkspaceSnap,
}

std::thread_local! {
    static WORKSPACES: std::cell::RefCell<std::collections::HashMap<String, StoredSnap>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
    static SHOWN: std::cell::RefCell<std::collections::HashSet<String>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
    static WARMING: std::cell::RefCell<std::collections::HashSet<String>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
    /// Blob rows imported in this tab. A later server list must not drop them.
    static PINNED: std::cell::RefCell<std::collections::HashMap<String, Vec<MediaItem>>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
    /// "{project}/{media}" removed in this tab. A reload clears this and the row returns.
    static BURIED: std::cell::RefCell<std::collections::HashSet<String>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
    /// "{project}/{media}" already registered this tab. Attach must not post it again.
    static REGISTERED: std::cell::RefCell<std::collections::HashSet<String>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
    /// Media ids whose browser save is still running. The overlay reads this, not the whole bin.
    static IN_FLIGHT: std::cell::RefCell<std::collections::HashSet<String>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
}

fn is_ready(id: &str) -> bool {
    WORKSPACES.with(|slot| slot.borrow().contains_key(id))
}

fn is_shown(id: &str) -> bool {
    SHOWN.with(|slot| slot.borrow().contains(id))
}

fn mark_shown(id: &str) {
    if id.is_empty() {
        return;
    }
    SHOWN.with(|slot| {
        slot.borrow_mut().insert(id.to_string());
    });
}

fn is_loading(id: &str) -> bool {
    WARMING.with(|slot| slot.borrow().contains(id))
}

fn mark_loading(id: &str) {
    WARMING.with(|slot| {
        slot.borrow_mut().insert(id.to_string());
    });
}

fn unmark_loading(id: &str) {
    WARMING.with(|slot| {
        slot.borrow_mut().remove(id);
    });
}

/// Keeps a second load of the same workspace from starting.
struct LoadMark(String);

impl LoadMark {
    fn arm(id: &str) -> Self {
        mark_loading(id);
        Self(id.to_string())
    }
}

impl Drop for LoadMark {
    fn drop(&mut self) {
        unmark_loading(&self.0);
    }
}

fn stored_ticket(id: &str) -> Option<u64> {
    WORKSPACES.with(|slot| slot.borrow().get(id).map(|row| row.ticket))
}

fn recall(id: &str) -> Option<WorkspaceSnap> {
    WORKSPACES.with(|slot| slot.borrow().get(id).map(|row| row.snap.clone()))
}

fn revoke_blob_url(url: &str) {
    if !url.starts_with("blob:") {
        return;
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = web_sys::Url::revoke_object_url(url);
    }
}

fn urls_not_in(from: &[MediaItem], keep: &[MediaItem]) -> Vec<String> {
    from.iter()
        .filter(|item| item.url.starts_with("blob:") && !keep.iter().any(|row| row.url == item.url))
        .map(|item| item.url.clone())
        .collect()
}

fn first_playable(items: &[MediaItem]) -> Option<String> {
    items.iter().find_map(|item| {
        if item.url.is_empty() {
            None
        } else {
            Some(item.url.clone())
        }
    })
}

/// Copy blob URLs onto matching rows. New rows are added whole.
/// Blob URLs that lost to one already on screen are returned so they can be revoked.
fn merge_media(kept: &mut Vec<MediaItem>, incoming: &[MediaItem]) -> Vec<String> {
    let mut unused = Vec::new();
    for item in incoming {
        if let Some(existing) = kept.iter_mut().find(|row| row.id == item.id) {
            if !item.url.starts_with("blob:") {
                continue;
            }
            if existing.url.starts_with("blob:") {
                if existing.url != item.url {
                    unused.push(item.url.clone());
                }
            } else {
                existing.url = item.url.clone();
                if existing.content_type.is_empty() {
                    existing.content_type = item.content_type.clone();
                }
                if existing.duration <= 0.05 && item.duration > 0.05 {
                    existing.duration = item.duration;
                }
            }
        } else {
            kept.push(item.clone());
        }
    }
    unused
}

fn revoke_all(urls: Vec<String>) {
    for url in urls {
        revoke_blob_url(&url);
    }
}

fn project_media_key(project_id: &str, media_id: &str) -> String {
    format!("{project_id}/{media_id}")
}

fn is_buried(project_id: &str, media_id: &str) -> bool {
    let key = project_media_key(project_id, media_id);
    BURIED.with(|slot| slot.borrow().contains(&key))
}

fn bury_media(project_id: &str, media_id: &str) {
    if project_id.is_empty() || media_id.is_empty() {
        return;
    }
    let key = project_media_key(project_id, media_id);
    BURIED.with(|slot| {
        slot.borrow_mut().insert(key);
    });
    PINNED.with(|slot| {
        if let Some(rows) = slot.borrow_mut().get_mut(project_id) {
            rows.retain(|item| item.id != media_id);
        }
    });
    IN_FLIGHT.with(|slot| {
        slot.borrow_mut().remove(media_id);
    });
}

fn pin_media(project_id: &str, item: &MediaItem) {
    if project_id.is_empty() || item.id.is_empty() || !item.url.starts_with("blob:") {
        return;
    }
    if is_buried(project_id, &item.id) {
        return;
    }
    PINNED.with(|slot| {
        let mut map = slot.borrow_mut();
        let rows = map.entry(project_id.to_string()).or_default();
        if let Some(existing) = rows.iter_mut().find(|row| row.id == item.id) {
            existing.url = item.url.clone();
            if existing.content_type.is_empty() {
                existing.content_type = item.content_type.clone();
            }
            if existing.duration <= 0.05 && item.duration > 0.05 {
                existing.duration = item.duration;
            }
        } else {
            rows.push(item.clone());
        }
    });
}

fn pin_has_url(project_id: &str, url: &str) -> bool {
    if !url.starts_with("blob:") {
        return false;
    }
    PINNED.with(|slot| {
        slot.borrow()
            .get(project_id)
            .is_some_and(|rows| rows.iter().any(|item| item.url == url))
    })
}

fn pin_has_id(project_id: &str, media_id: &str) -> bool {
    PINNED.with(|slot| {
        slot.borrow()
            .get(project_id)
            .is_some_and(|rows| rows.iter().any(|item| item.id == media_id))
    })
}

fn insert_pins(project_id: &str, library: &mut Vec<MediaItem>) {
    let pins = PINNED.with(|slot| slot.borrow().get(project_id).cloned().unwrap_or_default());
    for pin in pins {
        if is_buried(project_id, &pin.id) {
            continue;
        }
        if let Some(existing) = library.iter_mut().find(|row| row.id == pin.id) {
            if existing.url.is_empty() && pin.url.starts_with("blob:") {
                existing.url = pin.url.clone();
                if existing.content_type.is_empty() {
                    existing.content_type = pin.content_type.clone();
                }
                if existing.duration <= 0.05 && pin.duration > 0.05 {
                    existing.duration = pin.duration;
                }
            }
        } else {
            library.push(pin);
        }
    }
}

fn remember_shown_blobs(project_id: &str, library: &[MediaItem]) {
    for item in library {
        if item.url.starts_with("blob:") {
            pin_media(project_id, item);
        }
    }
}

fn is_registered(project_id: &str, media_id: &str) -> bool {
    let key = project_media_key(project_id, media_id);
    REGISTERED.with(|slot| slot.borrow().contains(&key))
}

fn note_registered(project_id: &str, media_id: &str) {
    if project_id.is_empty() || media_id.is_empty() {
        return;
    }
    let key = project_media_key(project_id, media_id);
    REGISTERED.with(|slot| {
        slot.borrow_mut().insert(key);
    });
}

fn clear_project_marks(project_id: &str) {
    PINNED.with(|slot| {
        slot.borrow_mut().remove(project_id);
    });
    let prefix = format!("{project_id}/");
    BURIED.with(|slot| {
        slot.borrow_mut().retain(|key| !key.starts_with(&prefix));
    });
    REGISTERED.with(|slot| {
        slot.borrow_mut().retain(|key| !key.starts_with(&prefix));
    });
}

fn uploads_showing(library: &[MediaItem]) -> Vec<String> {
    IN_FLIGHT.with(|slot| {
        let inflight = slot.borrow();
        library
            .iter()
            .filter(|item| inflight.contains(&item.id))
            .map(|item| item.id.clone())
            .collect()
    })
}

fn held_for_project(project_id: &str, held: &[HeldImport]) -> Vec<HeldImport> {
    held.iter()
        .filter(|row| !is_registered(project_id, &row.id))
        .cloned()
        .collect()
}

/// Keep clips from both lists, except ones removed in this tab.
/// A blob already on a row wins. An empty row takes a blob. Pins fill anything still missing.
fn union_library(project_id: &str, mut base: Vec<MediaItem>, extra: &[MediaItem]) -> Vec<MediaItem> {
    base.retain(|item| !is_buried(project_id, &item.id));
    let extra: Vec<MediaItem> = extra
        .iter()
        .filter(|item| !is_buried(project_id, &item.id))
        .cloned()
        .collect();
    let _ = merge_media(&mut base, &extra);
    insert_pins(project_id, &mut base);
    base
}

fn revoke_spare(project_id: &str, urls: Vec<String>) {
    for url in urls {
        if pin_has_url(project_id, &url) {
            continue;
        }
        revoke_blob_url(&url);
    }
}

fn seal_active(active: &mut Option<String>, library: &[MediaItem]) {
    if active.as_deref().unwrap_or("").is_empty() {
        *active = first_playable(library);
    }
}

fn remember_fill(id: &str, ticket: u64, mut snap: WorkspaceSnap) -> bool {
    WORKSPACES.with(|slot| {
        let action = {
            let map = slot.borrow();
            match map.get(id) {
                Some(existing) if existing.ticket > ticket => 0u8,
                Some(existing) if existing.ticket == ticket && existing.pinned => 1u8,
                Some(_) => 2u8,
                None => 3u8,
            }
        };
        let mut map = slot.borrow_mut();
        match action {
            // An older load must not replace the bin, and must not revoke URLs still on screen.
            0 => false,
            1 => {
                if let Some(existing) = map.get_mut(id) {
                    let stash = std::mem::take(&mut existing.snap.library);
                    existing.snap.library = union_library(id, stash, &snap.library);
                    remember_shown_blobs(id, &existing.snap.library);
                    seal_active(&mut existing.snap.active, &existing.snap.library);
                }
                true
            }
            _ => {
                let previous = map
                    .get(id)
                    .map(|existing| existing.snap.library.clone())
                    .unwrap_or_default();
                snap.library = union_library(id, snap.library, &previous);
                remember_shown_blobs(id, &snap.library);
                seal_active(&mut snap.active, &snap.library);
                map.insert(
                    id.to_string(),
                    StoredSnap {
                        ticket,
                        pinned: false,
                        snap,
                    },
                );
                true
            }
        }
    })
}

fn remember_stash(id: &str, ticket: u64, mut snap: WorkspaceSnap) {
    WORKSPACES.with(|slot| {
        let newer = slot
            .borrow()
            .get(id)
            .is_some_and(|existing| existing.ticket > ticket);
        if newer {
            return;
        }
        let mut map = slot.borrow_mut();
        let previous = map
            .get(id)
            .map(|existing| existing.snap.library.clone())
            .unwrap_or_default();
        snap.library = union_library(id, snap.library, &previous);
        remember_shown_blobs(id, &snap.library);
        seal_active(&mut snap.active, &snap.library);
        map.insert(
            id.to_string(),
            StoredSnap {
                ticket,
                pinned: true,
                snap,
            },
        );
    });
}

fn merge_stored_blobs(id: &str, items: &[MediaItem]) -> Vec<String> {
    WORKSPACES.with(|slot| {
        let mut map = slot.borrow_mut();
        // No stored workspace yet. The caller still shows these blob URLs.
        // Revoking them here blanked the bin for clips that had just been imported.
        let Some(existing) = map.get_mut(id) else {
            return Vec::new();
        };
        let incoming: Vec<MediaItem> = items
            .iter()
            .filter(|item| !is_buried(id, &item.id))
            .cloned()
            .collect();
        let unused = merge_media(&mut existing.snap.library, &incoming);
        existing
            .snap
            .library
            .retain(|item| !is_buried(id, &item.id));
        insert_pins(id, &mut existing.snap.library);
        remember_shown_blobs(id, &existing.snap.library);
        seal_active(&mut existing.snap.active, &existing.snap.library);
        unused
            .into_iter()
            .filter(|url| {
                !pin_has_url(id, url) && !existing.snap.library.iter().any(|item| item.url == *url)
            })
            .collect()
    })
}

fn forget_workspace(id: &str) {
    let stored = WORKSPACES.with(|slot| slot.borrow_mut().remove(id));
    SHOWN.with(|slot| {
        slot.borrow_mut().remove(id);
    });
    WARMING.with(|slot| {
        slot.borrow_mut().remove(id);
    });
    let pins = PINNED.with(|slot| slot.borrow().get(id).cloned().unwrap_or_default());
    clear_project_marks(id);
    let Some(stored) = stored else {
        for item in &pins {
            revoke_blob_url(&item.url);
        }
        return;
    };
    for item in &stored.snap.library {
        IN_FLIGHT.with(|slot| {
            slot.borrow_mut().remove(&item.id);
        });
        revoke_blob_url(&item.url);
    }
    for item in &pins {
        revoke_blob_url(&item.url);
    }
    for row in &stored.snap.held {
        forget_import(&row.id);
    }
}

fn snapshot(view: &WorkspaceView) -> WorkspaceSnap {
    WorkspaceSnap {
        name: view.name.peek().clone(),
        tracks: view.tracks.peek().clone(),
        engine: view.engine.peek().clone(),
        library: view.library.peek().clone(),
        active: view.active.peek().clone(),
        aspect: *view.aspect.peek(),
        playhead: playhead_now(),
        duration: *view.clock.duration.peek(),
        target_track: view.target_track.peek().clone(),
        undo: view.undo.peek().clone(),
        held: view.held.peek().clone(),
        messages: view.messages.peek().clone(),
        draft: view.draft.peek().clone(),
        selected: view.selected.peek().clone(),
    }
}

/// The tab click writes the next name before this runs. The tab row still has the old one.
fn leaving_snap(id: &str, view: &WorkspaceView) -> WorkspaceSnap {
    let mut snap = snapshot(view);
    if view.save.project_id.peek().as_str() != id
        && let Some(tab) = view.open_tabs.peek().iter().find(|tab| tab.id == id)
    {
        snap.name = tab.name.clone();
    }
    snap
}

fn stash_workspace(id: &str, ticket: u64, view: &WorkspaceView) {
    if id.is_empty() || !is_shown(id) {
        return;
    }
    remember_stash(id, ticket, leaving_snap(id, view));
}

/// Which thread and composer a snapshot may write.
///
/// `keep_live` is a refresh of the project already on screen. A send that
/// landed while that refresh was loading has to stay. A project switch
/// shows the saved chat instead.
fn cached_thread<'a>(
    keep_live: bool,
    live_messages: &'a [ChatMsg],
    live_draft: &'a str,
    snap_messages: &'a [ChatMsg],
    snap_draft: &'a str,
) -> (&'a [ChatMsg], &'a str) {
    if !keep_live {
        return (snap_messages, snap_draft);
    }
    if !live_messages.is_empty() {
        return (live_messages, live_draft);
    }
    if !live_draft.trim().is_empty() {
        let messages = if snap_messages.is_empty() {
            live_messages
        } else {
            snap_messages
        };
        return (messages, live_draft);
    }
    (snap_messages, snap_draft)
}

fn show_cached(snap: &WorkspaceSnap, id: &str, view: &mut WorkspaceView, keep_live: bool) {
    let mut clock = view.clock;
    view.name.set(snap.name.clone());
    view.tracks.set(snap.tracks.clone());
    clock.playing.set(false);
    clock.current.set(snap.playhead);
    clock.duration.set(snap.duration);
    reset_tick_clock();
    set_playhead(snap.playhead);
    paint_playhead(snap.playhead);
    view.aspect.set(snap.aspect);
    view.target_track.set(snap.target_track.clone());
    view.engine.set(snap.engine.clone());
    view.undo.set(snap.undo.clone());
    let base = snap.library.clone();
    // A project switch must not mix in the previous project's rows. The signal
    // still holds them until this set. Same-project refreshes keep clips that
    // landed while the snapshot was being loaded.
    let library = if keep_live {
        union_library(id, base, &view.library.peek().clone())
    } else {
        union_library(id, base, &[])
    };
    remember_shown_blobs(id, &library);
    if view.library.peek().clone() != library {
        view.library.set(library.clone());
    }
    let active = match &snap.active {
        Some(url) if !url.is_empty() => Some(url.clone()),
        _ => first_playable(&library),
    };
    view.active.set(active);
    // A same-project refresh often finishes after Send. That snap still has
    // the empty thread and the sentence that was in the box. Writing it back
    // leaves Pause up, the suggestions on screen, and the sent line nowhere.
    let live_messages = view.messages.peek().clone();
    let live_draft = view.draft.peek().clone();
    let (next_messages, next_draft) = cached_thread(
        keep_live,
        &live_messages,
        &live_draft,
        &snap.messages,
        &snap.draft,
    );
    if view.messages.peek().as_slice() != next_messages {
        view.messages.set(next_messages.to_vec());
    }
    if view.draft.peek().as_str() != next_draft {
        view.draft.set(next_draft.to_string());
    }
    view.selected.set(snap.selected.clone());
    let held = held_for_project(id, &snap.held);
    if view.held.peek().clone() != held {
        view.held.set(held);
    }
    let showing = uploads_showing(&library);
    if view.uploading.peek().clone() != showing {
        view.uploading.set(showing);
    }
    view.drag.set(None);
    mark_shown(id);
    note_open_tab(view, id, Some(&snap.name));
    sync_monitor(&snap.engine, &library, &snap.tracks, snap.playhead, false);
    schedule_missing_blobs(id, view);
    warm_other_tabs(id, view.open_tabs);
}

fn schedule_missing_blobs(id: &str, view: &WorkspaceView) {
    if !view.library.peek().iter().any(|item| item.url.is_empty()) {
        return;
    }
    let pid = id.to_string();
    let library = view.library;
    let active = view.active;
    spawn(async move {
        let current = library.peek().clone();
        let ids: Vec<String> = current.iter().map(|item| item.id.clone()).collect();
        let Some(done) = attach_local_media(&pid, current, &ids, |items| {
            if page_project_id().is_some_and(|page| page != pid) {
                return;
            }
            merge_into_library(&pid, library, active, items);
        })
        .await
        else {
            return;
        };
        revoke_all(merge_stored_blobs(&pid, &done));
        if page_project_id().is_none_or(|page| page == pid) {
            merge_into_library(&pid, library, active, &done);
        }
    });
}

fn warm_other_tabs(current: &str, tabs: Signal<Vec<OpenTab>>) {
    let ids: Vec<String> = tabs
        .peek()
        .iter()
        .map(|tab| tab.id.clone())
        .filter(|id| id != current)
        .collect();
    for id in ids {
        warm_workspace(id);
    }
}

fn warm_workspace(pid: String) {
    if pid.is_empty() || is_ready(&pid) || is_loading(&pid) {
        return;
    }
    mark_loading(&pid);
    spawn(async move {
        let loaded = load_snap(&pid).await;
        if let Some(snap) = loaded {
            let _ = remember_fill(&pid, 0, snap);
        }
        unmark_loading(&pid);
    });
}

async fn load_snap(pid: &str) -> Option<WorkspaceSnap> {
    let project = api::get_project(pid).await.ok()?;
    let tracks = bind::tracks_from_timeline(&project.timeline);
    let duration = project.timeline.duration().as_seconds();
    let aspect = Aspect::from_pixels(project.timeline.width, project.timeline.height);
    let target_track = project
        .timeline
        .first_track(TrackKind::Video)
        .or_else(|| project.timeline.tracks.first())
        .map(|track| track.id.to_string())
        .unwrap_or_default();
    let undo = tools::load_undo(pid);
    let mut items = Vec::new();
    let mut remote_ids = Vec::new();
    if let Ok(remote) = api::list_media(pid).await {
        remote_ids = remote.iter().map(|item| item.id.clone()).collect();
        items = merge_items(items, remote);
    }
    let items = attach_local_media(pid, items, &remote_ids, |_| {}).await?;
    Some(WorkspaceSnap {
        name: project.name,
        tracks,
        engine: project.timeline,
        library: items.clone(),
        active: first_playable(&items),
        aspect,
        playhead: 0.0,
        duration,
        target_track,
        undo,
        held: Vec::new(),
        messages: Vec::new(),
        draft: String::new(),
        selected: None,
    })
}

fn clear_editor(view: &mut WorkspaceView) {
    let mut clock = view.clock;
    clock.playing.set(false);
    clock.current.set(0.0);
    clock.duration.set(0.0);
    reset_tick_clock();
    set_playhead(0.0);
    view.tracks
        .set(bind::tracks_from_timeline(&EngineTimeline::default()));
    view.engine.set(EngineTimeline::default());
    view.aspect.set(Aspect::Landscape);
    view.target_track.set(String::new());
    view.undo.set(oc_core::UndoStack::new());
    // project_id is already the workspace being opened. Pins belong to that
    // id, so the previous project's clips are not carried across.
    let pid = view.save.project_id.peek().clone();
    let library = union_library(&pid, Vec::new(), &[]);
    remember_shown_blobs(&pid, &library);
    view.library.set(library.clone());
    view.active.set(first_playable(&library));
    view.messages.set(Vec::new());
    view.draft.set(String::new());
    view.selected.set(None);
    let kept: Vec<HeldImport> = view
        .held
        .peek()
        .iter()
        .filter(|row| !is_registered(&pid, &row.id) && pin_has_id(&pid, &row.id))
        .cloned()
        .collect();
    view.held.set(kept);
    view.uploading.set(uploads_showing(&library));
    view.drag.set(None);
}

fn note_open_tab(view: &mut WorkspaceView, id: &str, title: Option<&str>) {
    let mut list = view.open_tabs.peek().clone();
    ensure_tab(&mut list, id);
    if let Some(title) = title.filter(|title| !title.trim().is_empty()) {
        rename_tab(&mut list, id, title);
    }
    store_open_tabs(&list);
    view.open_tabs.set(list);
}

/// Point the open editor at `id` immediately.
/// A workspace already opened in this tab is shown from memory, with the same blob URLs.
fn adopt_route(
    id: String,
    mut loaded_for: Signal<String>,
    mut project_id: Signal<String>,
    mut view: WorkspaceView,
) {
    let previous = loaded_for.peek().clone();
    if previous != id {
        let still_open = view.open_tabs.peek().iter().any(|tab| tab.id == previous);
        if still_open {
            stash_workspace(&previous, *view.load_ticket.peek(), &view);
        } else if !previous.is_empty() {
            forget_workspace(&previous);
        }
    }
    let ticket = view.load_ticket.peek().wrapping_add(1);
    view.load_ticket.set(ticket);
    loaded_for.set(id.clone());
    project_id.set(id.clone());
    if let Some(snap) = recall(&id) {
        show_cached(&snap, &id, &mut view, previous == id);
        return;
    }
    let label = view
        .open_tabs
        .peek()
        .iter()
        .find(|tab| tab.id == id)
        .map(|tab| tab.name.clone())
        .unwrap_or_else(|| "Untitled".to_string());
    view.name.set(label);
    clear_editor(&mut view);
    note_open_tab(&mut view, &id, None);
    spawn(async move {
        open_cached_or_fill(id, ticket, view).await;
    });
}

async fn open_cached_or_fill(pid: String, ticket: u64, mut view: WorkspaceView) {
    if is_loading(&pid) {
        while is_loading(&pid) {
            if !current_load(ticket, view.load_ticket) {
                return;
            }
            gloo_timers::future::TimeoutFuture::new(40).await;
        }
        if !current_load(ticket, view.load_ticket) {
            return;
        }
        if let Some(snap) = recall(&pid) {
            show_cached(&snap, &pid, &mut view, true);
            return;
        }
    }
    fill_workspace(pid, ticket, view).await;
}

fn publish_library(view: &mut WorkspaceView, pid: &str, _ticket: u64, items: &[MediaItem]) {
    if view.save.project_id.peek().as_str() != pid {
        return;
    }
    // The server list was often fetched before register finished. Replacing the
    // bin with that list dropped every clip imported while "Saving..." was up.
    // A reload read the rows back. Merge instead, and keep blob URLs already on screen.
    merge_into_library(pid, view.library, view.active, items);
}

/// Copy `incoming` onto the bin. Rows already on screen stay, including a clip
/// that is not in `incoming` yet. Clips pinned this session are put back.
fn merge_into_library(
    project_id: &str,
    mut library: Signal<Vec<MediaItem>>,
    mut active: Signal<Option<String>>,
    incoming: &[MediaItem],
) {
    let current = library.peek().clone();
    let live = union_library(project_id, current.clone(), incoming);
    remember_shown_blobs(project_id, &live);
    let spare = urls_not_in(&current, &live);
    if active.peek().is_none() || active.peek().as_deref().is_some_and(|url| url.is_empty()) {
        active.set(first_playable(&live));
    }
    if live != current {
        library.set(live);
    }
    revoke_spare(project_id, spare);
}

fn publish_project(view: &mut WorkspaceView, pid: &str, ticket: u64, project: &oc_core::Project) {
    if !current_load(ticket, view.load_ticket) {
        return;
    }
    let mut clock = view.clock;
    view.name.set(project.name.clone());
    note_open_tab(view, pid, Some(&project.name));
    view.tracks
        .set(bind::tracks_from_timeline(&project.timeline));
    clock.duration.set(project.timeline.duration().as_seconds());
    view.aspect.set(Aspect::from_pixels(
        project.timeline.width,
        project.timeline.height,
    ));
    let target = project
        .timeline
        .first_track(TrackKind::Video)
        .or_else(|| project.timeline.tracks.first())
        .map(|track| track.id.to_string())
        .unwrap_or_default();
    view.target_track.set(target);
    view.engine.set(project.timeline.clone());
    view.undo.set(tools::load_undo(pid));
    mark_shown(pid);
}

async fn fill_workspace(pid: String, ticket: u64, mut view: WorkspaceView) {
    if stored_ticket(&pid).is_some_and(|stored| stored > ticket) {
        if current_load(ticket, view.load_ticket)
            && let Some(snap) = recall(&pid)
        {
            show_cached(&snap, &pid, &mut view, true);
        }
        return;
    }
    let mark = LoadMark::arm(&pid);
    let project = match api::get_project(&pid).await {
        Ok(project) => project,
        Err(_) => {
            if current_load(ticket, view.load_ticket)
                && let Some(track) = view.engine.peek().first_track(TrackKind::Video)
            {
                view.target_track.set(track.id.to_string());
            }
            return;
        }
    };
    if stored_ticket(&pid).is_some_and(|stored| stored > ticket) {
        if current_load(ticket, view.load_ticket)
            && let Some(snap) = recall(&pid)
        {
            show_cached(&snap, &pid, &mut view, true);
        }
        return;
    }
    publish_project(&mut view, &pid, ticket, &project);

    let mut items = Vec::new();
    let mut remote_ids = Vec::new();
    if let Ok(remote) = api::list_media(&pid).await {
        remote_ids = remote.iter().map(|item| item.id.clone()).collect();
        items = merge_items(items, remote);
        publish_library(&mut view, &pid, ticket, &items);
    }
    let pid_for_blobs = pid.clone();
    let mut view_for_blobs = view;
    let Some(items) = attach_local_media(&pid, items, &remote_ids, |rows| {
        publish_library(&mut view_for_blobs, &pid_for_blobs, ticket, rows);
    })
    .await
    else {
        return;
    };
    if current_load(ticket, view.load_ticket) {
        merge_into_library(&pid, view.library, view.active, &items);
        let _ = remember_fill(&pid, ticket, snapshot(&view));
        drop(mark);
        warm_other_tabs(&pid, view.open_tabs);
        #[cfg(target_arch = "wasm32")]
        {
            let _ = probe_camera_frame(view.save, view.library, false).await;
        }
    } else {
        let mut snap = WorkspaceSnap {
            name: project.name,
            tracks: bind::tracks_from_timeline(&project.timeline),
            engine: project.timeline.clone(),
            library: items,
            active: None,
            aspect: Aspect::from_pixels(project.timeline.width, project.timeline.height),
            playhead: 0.0,
            duration: project.timeline.duration().as_seconds(),
            target_track: project
                .timeline
                .first_track(TrackKind::Video)
                .or_else(|| project.timeline.tracks.first())
                .map(|track| track.id.to_string())
                .unwrap_or_default(),
            undo: tools::load_undo(&pid),
            held: Vec::new(),
            messages: Vec::new(),
            draft: String::new(),
            selected: None,
        };
        snap.active = first_playable(&snap.library);
        let _ = remember_fill(&pid, ticket, snap);
        drop(mark);
    }
}

#[component]
fn Workspace(id: String) -> Element {
    let ai_open = use_signal(|| true);
    use_effect(move || {
        install_studio_shortcut(ai_open);
    });
    rsx! {
        WorkspaceScreen { id, ai_open }
    }
}

#[component]
fn WorkspaceScreen(id: String, ai_open: Signal<bool>) -> Element {
    let nav = navigator();
    let name = use_signal(|| "Untitled".to_string());
    let tab = use_signal(|| AssetTab::Media);
    let aspect = use_signal(|| Aspect::Landscape);
    let playing = use_signal(|| false);
    let current = use_signal(|| 0.0);
    let duration = use_signal(|| 0.0);
    let draft = use_signal(String::new);
    let messages = use_signal(Vec::<ChatMsg>::new);
    let mut ai_width = use_signal(|| 320.0);
    let mut ai_drag = use_signal(|| None::<(f64, f64)>);
    let library = use_signal(Vec::<MediaItem>::new);
    let mut tracks = use_signal(|| bind::tracks_from_timeline(&EngineTimeline::default()));
    let active = use_signal(|| None::<String>);
    let project_id = use_signal(|| id.clone());
    let loaded_for = use_signal(|| id.clone());
    let load_ticket = use_signal(|| 1u64);
    let route_id = id.clone();
    let open_tabs = use_signal(move || {
        let mut list = load_open_tabs();
        ensure_tab(&mut list, &route_id);
        store_open_tabs(&list);
        list
    });
    let engine = use_signal(EngineTimeline::default);
    let edit_mode = use_signal(|| EditMode::Normal);
    let edit_tool = use_signal(|| EditTool::Select);
    let mut target_track = use_signal(String::new);
    let mut drag = use_signal(|| None::<DragSession>);
    let pps = use_signal(|| 36.0_f64);
    let mut suppress_seek = use_signal(|| false);
    let mut tl_h = use_signal(|| 340.0_f64);
    let mut tl_drag = use_signal(|| None::<(f64, f64)>);
    let persist_q = use_signal(Vec::<Vec<Op>>::new);
    let persist_busy = use_signal(|| false);
    let undo = use_signal(oc_core::UndoStack::new);
    let selected_clip = use_signal(|| None::<String>);
    let held = use_signal(Vec::<HeldImport>::new);
    let uploading = use_signal(Vec::<String>::new);
    let clock = Clock {
        current,
        duration,
        playing,
    };
    let save = WorkspaceSave {
        project_id,
        tracks,
        engine,
        aspect,
        persist_q,
        persist_busy,
        undo,
    };

    use_context_provider(|| tab);
    use_context_provider(|| aspect);
    use_context_provider(|| library);
    use_context_provider(|| tracks);
    use_context_provider(|| CtxActive(active));
    use_context_provider(|| CtxProject(project_id));
    use_context_provider(|| CtxHeld(held));
    use_context_provider(|| CtxUploading(uploading));
    use_context_provider(|| clock);
    use_context_provider(|| edit_mode);
    use_context_provider(|| edit_tool);
    use_context_provider(|| CtxTargetTrack(target_track));
    use_context_provider(|| drag);
    use_context_provider(|| CtxZoom(pps));
    use_context_provider(|| suppress_seek);
    use_context_provider(|| CtxTimelineH(tl_h));
    use_context_provider(|| tl_drag);
    use_context_provider(|| save);
    use_context_provider(|| CtxSelected(selected_clip));

    use_effect(move || {
        if !auth::is_signed_in() {
            nav.replace(Route::Login {});
        }
    });
    let leave_slot = use_hook(|| std::rc::Rc::new(std::cell::RefCell::new(None::<LeaveGuard>)));
    use_effect(move || {
        let leave_slot = leave_slot.clone();
        let held = held;
        *leave_slot.borrow_mut() = None;
        let Some(win) = web_sys::window() else {
            return;
        };
        let closure = wasm_bindgen::closure::Closure::wrap(Box::new(
            move |event: web_sys::BeforeUnloadEvent| {
                flush_chat_on_leave();
                if held.peek().is_empty() {
                    return;
                }
                event.prevent_default();
                event.set_return_value(
                    "Imported files are only in this tab. Leave and a refresh will not have them.",
                );
            },
        )
            as Box<dyn FnMut(web_sys::BeforeUnloadEvent)>);
        let handler: &js_sys::Function = closure.as_ref().unchecked_ref();
        win.set_onbeforeunload(Some(handler));
        let ticket = LEAVE_GEN.with(|slot| {
            let next = slot.get().wrapping_add(1);
            slot.set(next);
            next
        });
        *leave_slot.borrow_mut() = Some(LeaveGuard {
            ticket,
            closure: Some(closure),
        });
    });

    let view = WorkspaceView {
        load_ticket,
        name,
        open_tabs,
        tracks,
        clock,
        aspect,
        target_track,
        engine,
        undo,
        library,
        active,
        messages,
        draft,
        selected: selected_clip,
        held,
        uploading,
        drag,
        save,
    };
    let primed = use_hook(|| std::cell::Cell::new(false));
    use_drop(move || {
        let id = project_id.peek().clone();
        if id.is_empty() {
            return;
        }
        let listed = open_tabs.peek().iter().any(|tab| tab.id == id);
        if !listed {
            forget_workspace(&id);
            return;
        }
        stash_workspace(&id, *load_ticket.peek(), &view);
    });

    // A tab click writes project_id before the router rerenders this page.
    // Reading it here subscribes the screen so that write paints on the next
    // turn. The open project is the URL, which replaceState has already set.
    let _ = project_id.read();
    let live_id = page_project_id().unwrap_or_else(|| id.clone());
    let switched = loaded_for.peek().as_str() != live_id.as_str();
    if switched {
        adopt_route(live_id.clone(), loaded_for, project_id, view);
    }
    if !primed.replace(true)
        && !switched
        && let Some(snap) = recall(&live_id)
    {
        let mut view = view;
        show_cached(&snap, &live_id, &mut view, true);
    }

    use_future(move || {
        let view = view;
        async move {
            if *view.load_ticket.peek() != 1 {
                return;
            }
            let pid = project_id.peek().clone();
            if is_ready(&pid) {
                warm_other_tabs(&pid, view.open_tabs);
                return;
            }
            open_cached_or_fill(pid, 1, view).await;
        }
    });

    use_future(move || async move {
        loop {
            gloo_timers::future::TimeoutFuture::new(33).await;
            if !*clock.playing.peek() {
                reset_tick_clock();
                continue;
            }
            let end = crate::media::program_end(&tracks.peek());
            if end <= 0.05 {
                let mut playing = clock.playing;
                playing.set(false);
                continue;
            }
            // Wall clock owns the playhead so a stalled picture does not freeze the captions.
            let mut before = advance_playhead(end).min(end);
            if crate::media::clip_under(&tracks.peek(), &library.peek(), before).is_none() {
                if let Some(next) =
                    crate::media::next_picture_after(&tracks.peek(), &library.peek(), before, 0.45)
                {
                    before = next.start.min(end);
                    set_playhead(before);
                }
            }
            if before >= end - 0.02 {
                set_playhead(end);
                paint_playhead(end);
                sync_monitor(&engine.peek(), &library.peek(), &tracks.peek(), end, false);
                let mut playing = clock.playing;
                playing.set(false);
                let mut current = clock.current;
                current.set(end);
                continue;
            }
            sync_monitor(
                &engine.peek(),
                &library.peek(),
                &tracks.peek(),
                before,
                true,
            );
            apply_monitor_look(
                &engine.peek(),
                &library.peek(),
                playhead_now().max(before).min(end),
                true,
            );
            let under = crate::media::clip_under(
                &tracks.peek(),
                &library.peek(),
                playhead_now().max(before).min(end),
            );
            let now = match under {
                Some(shot) if shot.kind == MediaKind::Video => playhead_now().min(end),
                Some(_) => advance_playhead(end),
                None => {
                    let mut playing = clock.playing;
                    playing.set(false);
                    let parked = playhead_now().min(end);
                    set_playhead(parked);
                    sync_monitor(
                        &engine.peek(),
                        &library.peek(),
                        &tracks.peek(),
                        parked,
                        false,
                    );
                    paint_playhead(parked);
                    let mut current = clock.current;
                    current.set(parked);
                    continue;
                }
            };
            paint_playhead(now);
            if now >= end - 0.02 {
                set_playhead(end);
                paint_playhead(end);
                let mut playing = clock.playing;
                playing.set(false);
                let mut current = clock.current;
                current.set(end);
            }
        }
    });

    rsx! {
        document::Title { "OpenCut — Workspace" }
        div {
            class: {
                let mut cls = String::from("editor");
                if drag.read().is_some() {
                    cls.push_str(" dragging");
                }
                if tl_drag.read().is_some() {
                    cls.push_str(" tl-resizing");
                }
                cls
            },
            onmousemove: move |evt| {
                if let Some((sx, sw)) = *ai_drag.read() {
                    let dx = sx - evt.client_coordinates().x;
                    ai_width.set((sw + dx).clamp(280.0, 880.0));
                }
                if let Some((sy, sh)) = *tl_drag.read() {
                    let dy = sy - evt.client_coordinates().y;
                    tl_h.set((sh + dy).clamp(200.0, max_timeline_h()));
                }
                let Some(mut session) = drag.peek().clone() else { return };
                let rows = display_tracks(&tracks.read());
                update_drag(
                    &mut session,
                    evt.client_coordinates().x,
                    evt.client_coordinates().y,
                    *pps.peek(),
                    &rows,
                );
                drag.set(Some(session));
            },
            onmouseup: move |_| {
                ai_drag.set(None);
                tl_drag.set(None);
                let Some(session) = drag.peek().clone() else { return };
                if session.moved {
                    suppress_seek.set(true);
                }
                if commit_drag(
                    &session,
                    &mut tracks.write(),
                    &library.read(),
                    *edit_tool.peek(),
                ) {
                    target_track.set(session.track_id.clone());
                    let end = timeline_end(&tracks.read());
                    if end > *clock.duration.peek() {
                        let mut duration = clock.duration;
                        duration.set(end);
                    }
                    persist(save);
                }
                drag.set(None);
            },
            Header { name, aspect, tab, ai_open, tabs: open_tabs }
            div { class: "stage",
                div { class: "main-col",
                    div { class: "body",
                        div { class: "top",
                            Assets { tab }
                            Preview { aspect, playing }
                        }
                        Timeline {}
                    }
                }
                AiSidebar { key: "{id}", draft, messages, ai_open, ai_width, ai_drag }
            }
            {drag_chip(drag)}
        }
    }
}

fn drag_chip(drag: Signal<Option<DragSession>>) -> Element {
    let session = drag.read().clone();
    let Some(session) = session else {
        return rsx! {};
    };
    if !session.moved {
        return rsx! {};
    }
    let left = session.client_x + 14.0;
    let top = session.client_y + 14.0;
    let name = session.name.clone();
    let url = session.url.clone();
    let kind = session.media_kind;
    let dur = format_clock(session.duration);
    let hint = if session.over_timeline {
        "Release to drop"
    } else {
        "Drop on a track"
    };
    rsx! {
        div {
            class: "drag-chip",
            style: "left: {left}px; top: {top}px",
            div { class: "drag-chip-media",
                match (url.is_empty(), kind) {
                    (true, _) => rsx! {},
                    (false, MediaKind::Image) => rsx! { img { src: "{url}", alt: "" } },
                    (false, MediaKind::Audio) => rsx! { IconWave {} },
                    (false, MediaKind::Video) => rsx! {
                        video { src: "{url}", muted: true, preload: "metadata" }
                    },
                }
            }
            span { class: "drag-chip-name", "{name}" }
            span { class: "drag-chip-meta", "{dur} · {hint}" }
        }
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
struct OpenTab {
    id: String,
    name: String,
}

const OPEN_TABS_KEY: &str = "opencut_open_tabs";

fn load_open_tabs() -> Vec<OpenTab> {
    let Some(win) = web_sys::window() else {
        return Vec::new();
    };
    let Ok(Some(storage)) = win.local_storage() else {
        return Vec::new();
    };
    storage
        .get_item(OPEN_TABS_KEY)
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn store_open_tabs(tabs: &[OpenTab]) {
    let Some(win) = web_sys::window() else {
        return;
    };
    let Ok(Some(storage)) = win.local_storage() else {
        return;
    };
    if let Ok(raw) = serde_json::to_string(tabs) {
        let _ = storage.set_item(OPEN_TABS_KEY, &raw);
    }
}

/// Keep this project in the open row. An existing name stays until a rename.
fn ensure_tab(tabs: &mut Vec<OpenTab>, id: &str) {
    if id.is_empty() || tabs.iter().any(|tab| tab.id == id) {
        return;
    }
    tabs.push(OpenTab {
        id: id.to_string(),
        name: "Untitled".to_string(),
    });
}

fn rename_tab(tabs: &mut Vec<OpenTab>, id: &str, name: &str) {
    let name = name.trim();
    if id.is_empty() || name.is_empty() {
        return;
    }
    if let Some(tab) = tabs.iter_mut().find(|tab| tab.id == id) {
        tab.name = name.to_string();
    } else {
        tabs.push(OpenTab {
            id: id.to_string(),
            name: name.to_string(),
        });
    }
}

fn close_open_tab(tabs: &[OpenTab], id: &str) -> Vec<OpenTab> {
    tabs.iter().filter(|tab| tab.id != id).cloned().collect()
}

/// The tab that becomes active after `id` closes. The neighbor to the right, else the left.
fn neighbor_after_close(tabs: &[OpenTab], id: &str) -> Option<String> {
    let Some(index) = tabs.iter().position(|tab| tab.id == id) else {
        return None;
    };
    let rest = close_open_tab(tabs, id);
    if rest.is_empty() {
        return None;
    }
    let next = index.min(rest.len() - 1);
    Some(rest[next].id.clone())
}

fn submit_named_project(
    new_name: Signal<String>,
    mut naming: Signal<bool>,
    mut creating: Signal<bool>,
    mut tabs: Signal<Vec<OpenTab>>,
    mut project_id: Signal<String>,
    mut name: Signal<String>,
    held: Signal<Vec<HeldImport>>,
) {
    let title = new_name.peek().trim().to_string();
    if *creating.peek() {
        return;
    }
    if title.is_empty() {
        show_toast().error("Give the workspace a name");
        return;
    }
    if !confirm_leave(held) {
        return;
    }
    creating.set(true);
    naming.set(false);
    let mut list = tabs.peek().clone();
    rename_tab(&mut list, &project_id.peek(), &name.peek());
    store_open_tabs(&list);
    spawn(async move {
        match api::create_project(&title).await {
            Ok(project) => {
                let mut list = load_open_tabs();
                rename_tab(&mut list, &project.id, &project.name);
                store_open_tabs(&list);
                tabs.set(list);
                project_id.set(project.id.clone());
                name.set(project.name.clone());
                creating.set(false);
                navigator().replace(Route::Workspace { id: project.id });
            }
            Err(err) => {
                show_toast().error(err);
                creating.set(false);
                naming.set(true);
            }
        }
    });
}

#[cfg(test)]
fn fresh_project_name(tabs: &[OpenTab]) -> String {
    let used = |name: &str| tabs.iter().any(|tab| tab.name == name);
    if !used("Untitled") {
        return "Untitled".to_string();
    }
    for n in 2..100 {
        let candidate = format!("Untitled {n}");
        if !used(&candidate) {
            return candidate;
        }
    }
    "Untitled".to_string()
}

pub(crate) fn forget_open_tab(id: &str) {
    store_open_tabs(&close_open_tab(&load_open_tabs(), id));
    forget_workspace(id);
}

#[cfg(test)]
mod open_tab_tests {
    use super::{
        Aspect, EditorTrack, EngineTimeline, MediaItem, MediaKind, OpenTab, TrackKindUi,
        WorkspaceSnap, bury_media, close_open_tab, ensure_tab, forget_workspace, fresh_project_name,
        merge_media, neighbor_after_close, pin_media, recall, remember_fill, remember_stash,
        rename_tab,
    };

    fn sample_snap(name: &str, media_id: &str, url: &str) -> WorkspaceSnap {
        WorkspaceSnap {
            name: name.into(),
            tracks: vec![EditorTrack {
                id: "t".into(),
                name: "V1".into(),
                kind: TrackKindUi::Video,
                muted: false,
                hidden: false,
                clips: Vec::new(),
            }],
            engine: EngineTimeline::default(),
            library: vec![MediaItem {
                id: media_id.into(),
                name: "clip.mp4".into(),
                kind: MediaKind::Video,
                url: url.into(),
                content_type: "video/mp4".into(),
                duration: 3.0,
            }],
            active: None,
            aspect: Aspect::Landscape,
            playhead: 1.5,
            duration: 3.0,
            target_track: "t".into(),
            undo: oc_core::UndoStack::new(),
            held: Vec::new(),
            messages: Vec::new(),
            draft: String::new(),
            selected: None,
        }
    }

    #[test]
    fn a_second_project_stays_beside_the_first() {
        let mut tabs = Vec::new();
        rename_tab(&mut tabs, "a", "Makers");
        rename_tab(&mut tabs, "b", "Untitled 2");
        assert_eq!(tabs.len(), 2);
        rename_tab(&mut tabs, "a", "Makers documentary");
        assert_eq!(tabs[0].name, "Makers documentary");
        assert_eq!(tabs[1].id, "b");
    }

    #[test]
    fn closing_the_open_tab_selects_the_neighbor() {
        let tabs = vec![
            OpenTab {
                id: "a".into(),
                name: "A".into(),
            },
            OpenTab {
                id: "b".into(),
                name: "B".into(),
            },
            OpenTab {
                id: "c".into(),
                name: "C".into(),
            },
        ];
        assert_eq!(neighbor_after_close(&tabs, "b").as_deref(), Some("c"));
        assert_eq!(neighbor_after_close(&tabs, "c").as_deref(), Some("b"));
        assert_eq!(close_open_tab(&tabs, "a").len(), 2);
        assert!(neighbor_after_close(&tabs[..1], "a").is_none());
    }

    #[test]
    fn the_next_project_name_is_numbered() {
        assert_eq!(fresh_project_name(&[]), "Untitled");
        let tabs = vec![OpenTab {
            id: "a".into(),
            name: "Untitled".into(),
        }];
        assert_eq!(fresh_project_name(&tabs), "Untitled 2");
    }

    #[test]
    fn ensure_tab_does_not_rename_or_duplicate() {
        let mut tabs = vec![OpenTab {
            id: "a".into(),
            name: "Reel".into(),
        }];
        ensure_tab(&mut tabs, "a");
        ensure_tab(&mut tabs, "b");
        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs[0].name, "Reel");
        assert_eq!(tabs[1].name, "Untitled");
    }

    #[test]
    fn a_later_fill_replaces_a_warm_snapshot() {
        let id = "snap-warm-vs-fill";
        forget_workspace(id);
        remember_fill(id, 0, sample_snap("warm", "m", "https://old"));
        assert_eq!(recall(id).unwrap().name, "warm");
        remember_fill(id, 1, sample_snap("live", "m", "blob:live"));
        let stored = recall(id).unwrap();
        assert_eq!(stored.name, "live");
        assert_eq!(stored.library[0].url, "blob:live");
        forget_workspace(id);
    }

    #[test]
    fn a_stash_keeps_edits_when_the_fill_finishes_late() {
        let id = "snap-stash-wins";
        forget_workspace(id);
        let mut edited = sample_snap("edited", "clip", "");
        edited.tracks[0].name = "V9".into();
        remember_stash(id, 2, edited);

        let mut late = sample_snap("server", "clip", "blob:clip");
        late.library.push(MediaItem {
            id: "extra".into(),
            name: "b.mp4".into(),
            kind: MediaKind::Video,
            url: "blob:extra".into(),
            content_type: "video/mp4".into(),
            duration: 2.0,
        });
        remember_fill(id, 2, late);

        let stored = recall(id).unwrap();
        assert_eq!(stored.name, "edited");
        assert_eq!(stored.tracks[0].name, "V9");
        assert_eq!(stored.library[0].url, "blob:clip");
        assert!(stored.library.iter().any(|item| item.url == "blob:extra"));
        forget_workspace(id);
    }

    #[test]
    fn a_stash_keeps_the_blob_already_on_screen() {
        let id = "snap-keep-blob";
        forget_workspace(id);
        remember_stash(id, 2, sample_snap("edited", "clip", "blob:kept"));
        remember_fill(id, 2, sample_snap("server", "clip", "blob:other"));
        assert_eq!(recall(id).unwrap().library[0].url, "blob:kept");
        forget_workspace(id);
    }

    #[test]
    fn an_older_ticket_does_not_replace_a_newer_snapshot() {
        let id = "snap-ticket-order";
        forget_workspace(id);
        remember_fill(id, 4, sample_snap("new", "m", "blob:new"));
        remember_fill(id, 1, sample_snap("old", "m", "blob:old"));
        remember_stash(id, 2, sample_snap("stale", "m", "blob:stale"));
        remember_fill(id, 0, sample_snap("warm", "m", "blob:warm"));
        let stored = recall(id).unwrap();
        assert_eq!(stored.name, "new");
        assert_eq!(stored.library[0].url, "blob:new");
        forget_workspace(id);
        assert!(recall(id).is_none());
    }

    fn clip(id: &str, url: &str) -> MediaItem {
        MediaItem {
            id: id.into(),
            name: format!("{id}.mp4"),
            kind: MediaKind::Video,
            url: url.into(),
            content_type: "video/mp4".into(),
            duration: 8.0,
        }
    }

    #[test]
    fn a_refresh_keeps_a_clip_that_is_only_in_this_tab() {
        let mut live = vec![clip("new", "blob:new")];
        let server = vec![clip("old", "")];
        let unused = merge_media(&mut live, &server);
        assert!(unused.is_empty());
        assert_eq!(live.len(), 2);
        assert_eq!(live[0].url, "blob:new");
        assert!(live.iter().any(|item| item.id == "old"));

        let unused = merge_media(&mut live, &[clip("new", "")]);
        assert!(unused.is_empty());
        assert_eq!(
            live.iter().find(|item| item.id == "new").unwrap().url,
            "blob:new"
        );

        let unused = merge_media(&mut live, &[]);
        assert!(unused.is_empty());
        assert_eq!(live.len(), 2);
    }

    #[test]
    fn a_later_fill_keeps_the_clip_imported_before_it() {
        let id = "snap-keep-earlier";
        forget_workspace(id);
        remember_fill(id, 1, sample_snap("one", "first", "blob:first"));
        remember_fill(id, 2, sample_snap("two", "second", "blob:second"));
        let stored = recall(id).unwrap();
        assert!(
            stored
                .library
                .iter()
                .any(|item| item.id == "first" && item.url == "blob:first")
        );
        assert!(
            stored
                .library
                .iter()
                .any(|item| item.id == "second" && item.url == "blob:second")
        );
        forget_workspace(id);
    }

    #[test]
    fn a_pinned_clip_survives_a_fill_that_never_listed_it() {
        let id = "snap-pin-third";
        forget_workspace(id);
        remember_fill(id, 1, sample_snap("one", "first", "blob:first"));
        pin_media(id, &clip("third", "blob:third"));
        remember_fill(id, 2, sample_snap("two", "second", "blob:second"));
        let stored = recall(id).unwrap();
        assert!(
            stored
                .library
                .iter()
                .any(|item| item.id == "first" && item.url == "blob:first")
        );
        assert!(
            stored
                .library
                .iter()
                .any(|item| item.id == "third" && item.url == "blob:third")
        );
        forget_workspace(id);
        remember_fill(id, 3, sample_snap("after", "second", "blob:second"));
        assert!(
            !recall(id)
                .unwrap()
                .library
                .iter()
                .any(|item| item.id == "third")
        );
        forget_workspace(id);
    }

    #[test]
    fn a_later_fill_keeps_the_blob_when_the_new_row_has_no_url() {
        let id = "snap-empty-url";
        forget_workspace(id);
        remember_fill(id, 1, sample_snap("one", "clip", "blob:clip"));
        remember_fill(id, 2, sample_snap("two", "clip", ""));
        assert_eq!(recall(id).unwrap().library[0].url, "blob:clip");
        forget_workspace(id);
    }

    #[test]
    fn a_removed_clip_stays_gone_for_this_session() {
        let id = "snap-bury";
        forget_workspace(id);
        remember_fill(id, 1, sample_snap("one", "clip", "blob:clip"));
        bury_media(id, "clip");
        remember_fill(id, 2, sample_snap("two", "clip", "blob:again"));
        assert!(!recall(id).unwrap().library.iter().any(|item| item.id == "clip"));
        forget_workspace(id);
        remember_fill(id, 3, sample_snap("reload", "clip", "blob:clip"));
        assert!(recall(id).unwrap().library.iter().any(|item| item.id == "clip"));
        forget_workspace(id);
    }
}

#[component]
fn Header(
    name: Signal<String>,
    aspect: Signal<Aspect>,
    tab: Signal<AssetTab>,
    ai_open: Signal<bool>,
    tabs: Signal<Vec<OpenTab>>,
) -> Element {
    let nav = navigator();
    let mut project_id = use_context::<CtxProject>().0;
    let save = use_context::<WorkspaceSave>();
    let held = use_context::<CtxHeld>().0;
    let uploading = use_context::<CtxUploading>().0;
    let library = use_context::<Signal<Vec<MediaItem>>>();
    let mut frame_menu = use_signal(|| false);
    let creating = use_signal(|| false);
    let mut naming = use_signal(|| false);
    let mut new_name = use_signal(String::new);
    let unsaved = !held.read().is_empty();
    let current = *aspect.read();
    let settings_on = *tab.read() == AssetTab::Settings;
    let menu_open = *frame_menu.read();
    let chat_open = *ai_open.read();
    let open_tabs = tabs.read().clone();
    let signal_id = project_id.read().clone();
    let current_id = match use_route::<Route>() {
        Route::Workspace { id } => id,
        _ => signal_id,
    };
    rsx! {
        header { class: "header",
            div { class: "header-left",
                button {
                    class: "logo",
                    title: "Projects",
                    onclick: move |_| {
                        if !confirm_leave(held) {
                            return;
                        }
                        navigator().replace(Route::Projects {});
                    },
                    IconScissors {}
                }
            }
            div { class: "proj-tabs",
                    for project in open_tabs {
                        {
                            let key_id = project.id.clone();
                            let switch_id = project.id.clone();
                            let close_id = project.id;
                            let label = project.name;
                            let viewing_switch = current_id.clone();
                            let viewing_close = current_id.clone();
                            let active = key_id == current_id;
                            rsx! {
                                div {
                                    key: "{key_id}",
                                    class: if active { "proj-tab on" } else { "proj-tab" },
                                    if active {
                                        input {
                                            class: "proj-tab-name",
                                            value: "{name}",
                                            oninput: move |e| name.set(e.value()),
                                            onblur: move |_| {
                                                let title = name.peek().trim().to_string();
                                                if title.is_empty() {
                                                    return;
                                                }
                                                let pid = project_id.peek().clone();
                                                let mut list = tabs.peek().clone();
                                                rename_tab(&mut list, &pid, &title);
                                                store_open_tabs(&list);
                                                tabs.set(list);
                                                spawn(async move {
                                                    let _ = api::rename_project(&pid, &title).await;
                                                });
                                            },
                                        }
                                    } else {
                                        button {
                                            class: "proj-tab-name",
                                            onclick: move |_| {
                                                if switch_id == project_id.peek().as_str()
                                                    && switch_id == viewing_switch
                                                {
                                                    return;
                                                }
                                                if !confirm_leave(held) {
                                                    return;
                                                }
                                                let mut list = tabs.peek().clone();
                                                rename_tab(
                                                    &mut list,
                                                    &project_id.peek(),
                                                    &name.peek(),
                                                );
                                                store_open_tabs(&list);
                                                tabs.set(list);
                                                project_id.set(switch_id.clone());
                                                name.set(label.clone());
                                                nav.replace(Route::Workspace {
                                                    id: switch_id.clone(),
                                                });
                                            },
                                            "{label}"
                                        }
                                    }
                                    button {
                                        class: "proj-tab-x",
                                        title: "Close",
                                        onclick: move |evt| {
                                            evt.stop_propagation();
                                            let closing_current = close_id == viewing_close
                                                || close_id == project_id.peek().as_str();
                                            if closing_current && !confirm_leave(held) {
                                                return;
                                            }
                                            let current = tabs.peek().clone();
                                            let next = neighbor_after_close(&current, &close_id);
                                            let rest = close_open_tab(&current, &close_id);
                                            store_open_tabs(&rest);
                                            tabs.set(rest.clone());
                                            if !closing_current {
                                                forget_workspace(&close_id);
                                                return;
                                            }
                                            if let Some(next_id) = next {
                                                if let Some(tab) =
                                                    rest.iter().find(|tab| tab.id == next_id)
                                                {
                                                    name.set(tab.name.clone());
                                                }
                                                project_id.set(next_id.clone());
                                                nav.replace(Route::Workspace { id: next_id });
                                            } else {
                                                nav.replace(Route::Projects {});
                                            }
                                        },
                                        "×"
                                    }
                                }
                            }
                        }
                    }
                    if *naming.read() {
                        form {
                            class: "proj-name-form",
                            onsubmit: move |evt| {
                                evt.prevent_default();
                                submit_named_project(
                                    new_name, naming, creating, tabs, project_id, name, held,
                                );
                            },
                            input {
                                class: "proj-name-input",
                                placeholder: "Workspace name",
                                value: "{new_name}",
                                autofocus: true,
                                oninput: move |evt| new_name.set(evt.value()),
                                onkeydown: move |evt| {
                                    if evt.key() == Key::Escape {
                                        naming.set(false);
                                    }
                                },
                            }
                            button {
                                class: "proj-name-add",
                                disabled: *creating.read(),
                                "Add"
                            }
                        }
                    } else {
                        button {
                            class: "proj-add",
                            title: "New project",
                            disabled: *creating.read(),
                            onclick: move |_| {
                                if *creating.peek() {
                                    return;
                                }
                                new_name.set(String::new());
                                naming.set(true);
                            },
                            "+"
                        }
                    }
                }
            div { class: "header-right",
                if unsaved {
                    button {
                        class: "btn",
                        title: "Keep imported files in this browser",
                        onclick: move |_| save_held_imports(held, uploading, save),
                        "Save here"
                    }
                }
                div { class: "nav-aspect",
                    button {
                        class: "nav-chip",
                        title: "Frame",
                        onclick: move |_| {
                            let open = *frame_menu.peek();
                            frame_menu.set(!open);
                        },
                        "{current.ratio()}"
                        span { class: "nav-caret", "▾" }
                    }
                    if menu_open {
                        div {
                            class: "nav-backdrop",
                            onclick: move |_| frame_menu.set(false),
                        }
                        div { class: "nav-menu",
                            button {
                                class: if current == Aspect::Landscape { "on" } else { "" },
                                onclick: move |_| {
                                    choose_frame();
                                    aspect.set(Aspect::Landscape);
                                    persist(save);
                                    frame_menu.set(false);
                                },
                                "16:9"
                                span { "1920 × 1080" }
                            }
                            button {
                                class: if current == Aspect::Vertical { "on" } else { "" },
                                onclick: move |_| {
                                    choose_frame();
                                    aspect.set(Aspect::Vertical);
                                    persist(save);
                                    frame_menu.set(false);
                                },
                                "9:16"
                                span { "1080 × 1920" }
                            }
                            button {
                                class: if current == Aspect::Square { "on" } else { "" },
                                onclick: move |_| {
                                    choose_frame();
                                    aspect.set(Aspect::Square);
                                    persist(save);
                                    frame_menu.set(false);
                                },
                                "1:1"
                                span { "1080 × 1080" }
                            }
                            button {
                                class: if current == Aspect::Classic { "on" } else { "" },
                                onclick: move |_| {
                                    choose_frame();
                                    aspect.set(Aspect::Classic);
                                    persist(save);
                                    frame_menu.set(false);
                                },
                                "4:3"
                                span { "1440 × 1080" }
                            }
                            button {
                                title: "Match the picture under the playhead",
                                onclick: move |_| {
                                    frame_menu.set(false);
                                    choose_frame();
                                    let library = library;
                                    spawn(async move {
                                        if let Err(miss) = probe_camera_frame(save, library, true).await {
                                            let message = match miss {
                                                FrameMiss::NoPicture => "No picture under the playhead",
                                                FrameMiss::Unmeasured => "This browser has not measured that picture",
                                                FrameMiss::Unreadable => "Could not read the picture size",
                                            };
                                            show_toast().error(message);
                                        }
                                    });
                                },
                                "Match"
                                span { "Playhead" }
                            }
                        }
                    }
                }
                button {
                    class: if settings_on { "btn btn-icon nav-icon on" } else { "btn btn-icon nav-icon" },
                    title: "Settings",
                    onclick: move |_| {
                        if *tab.peek() == AssetTab::Settings {
                            tab.set(AssetTab::Media);
                        } else {
                            tab.set(AssetTab::Settings);
                        }
                    },
                    IconGear {}
                }
                button {
                    class: "btn btn-primary nav-export",
                    onclick: move |_| {
                        if !confirm_leave(held) {
                            return;
                        }
                        nav.push(Route::Export {
                            id: project_id.peek().clone(),
                        });
                    },
                    "Export"
                }
                button {
                    class: if chat_open { "btn btn-icon nav-icon on" } else { "btn btn-icon nav-icon" },
                    title: if chat_open { "Hide chat" } else { "Show chat" },
                    onclick: move |_| {
                        let next = !*ai_open.peek();
                        ai_open.set(next);
                    },
                    IconPanel {}
                }
            }
        }
    }
}

#[component]
fn Assets(tab: Signal<AssetTab>) -> Element {
    let current = *tab.read();
    rsx! {
        aside { class: "panel assets",
            nav { class: "rail",
                for item in AssetTab::ALL {
                    button {
                        class: if item == current { "rail-btn on" } else { "rail-btn" },
                        title: item.label(),
                        onclick: move |_| tab.set(item),
                        {tab_icon(item)}
                    }
                }
            }
            if current == AssetTab::Media {
                MediaPanel {}
            } else {
                div { class: "assets-body",
                    div { class: "assets-title", "{current.label()}" }
                    AssetView { tab: current }
                }
            }
        }
    }
}

fn tab_icon(tab: AssetTab) -> Element {
    match tab {
        AssetTab::Media => rsx! { IconFilm {} },
        AssetTab::Text => rsx! { IconType {} },
        AssetTab::Captions => rsx! { IconCaptions {} },
        AssetTab::Audio => rsx! { IconWave {} },
        AssetTab::Elements => rsx! { IconShapes {} },
        AssetTab::Transitions => rsx! { IconTransition {} },
        AssetTab::Visuals => rsx! { IconSliders {} },
        AssetTab::Settings => rsx! { IconGear {} },
    }
}

fn mix_preview_class(kind: TransitionKind) -> &'static str {
    match kind {
        TransitionKind::Cut => "cut",
        TransitionKind::Dissolve => "dissolve",
        TransitionKind::FadeBlack => "fadeblack",
        TransitionKind::FadeWhite => "fadewhite",
        TransitionKind::Slide | TransitionKind::SlideRight => "slide-r",
        TransitionKind::SlideLeft => "slide-l",
        TransitionKind::SlideUp => "slide-u",
        TransitionKind::SlideDown => "slide-d",
        TransitionKind::Wipe | TransitionKind::WipeLeft | TransitionKind::SmoothLeft => "wipe-l",
        TransitionKind::WipeRight | TransitionKind::SmoothRight => "wipe-r",
        TransitionKind::WipeUp | TransitionKind::SmoothUp => "wipe-u",
        TransitionKind::WipeDown | TransitionKind::SmoothDown => "wipe-d",
        TransitionKind::WipeTl => "wipe-tl",
        TransitionKind::WipeTr => "wipe-tr",
        TransitionKind::WipeBl => "wipe-bl",
        TransitionKind::WipeBr => "wipe-br",
        TransitionKind::CoverLeft => "cover-l",
        TransitionKind::CoverRight => "cover-r",
        TransitionKind::CoverUp => "cover-u",
        TransitionKind::CoverDown => "cover-d",
        TransitionKind::RevealLeft => "reveal-l",
        TransitionKind::RevealRight => "reveal-r",
        TransitionKind::RevealUp => "reveal-u",
        TransitionKind::RevealDown => "reveal-d",
        TransitionKind::CircleOpen => "circle-open",
        TransitionKind::CircleClose => "circle-close",
        TransitionKind::Radial => "radial",
        TransitionKind::Pixelize => "pixel",
        TransitionKind::HorzOpen => "horz-open",
        TransitionKind::VertOpen => "vert-open",
    }
}

#[component]
fn MixPreview(class: &'static str) -> Element {
    rsx! {
        span { class: "mix-preview mix-{class}",
            i { class: "a" }
            i { class: "b" }
        }
    }
}

fn ask_text(title: &str, fallback: &str) -> String {
    web_sys::window()
        .and_then(|w| w.prompt_with_message(title).ok().flatten())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

fn live_note(save: WorkspaceSave, library: &[MediaItem], result: Result<Vec<String>, String>) {
    match result {
        Ok(notes) => {
            apply_monitor_look(&save.engine.peek(), library, playhead_now(), false);
            let text = notes
                .iter()
                .filter(|n| !n.is_empty())
                .cloned()
                .collect::<Vec<_>>()
                .join(" · ");
            if text.contains("no following") {
                show_toast().warn(text);
            } else if !text.is_empty() {
                show_toast().success(text);
            }
        }
        Err(err) => show_toast().error(err),
    }
}

#[component]
fn AssetView(tab: AssetTab) -> Element {
    let save = use_context::<WorkspaceSave>();
    let clock = use_context::<Clock>();
    let target_track = use_context::<CtxTargetTrack>().0;
    let selected_clip = use_context::<CtxSelected>().0;
    let library = use_context::<Signal<Vec<MediaItem>>>();
    let mut tab_sig = use_context::<Signal<AssetTab>>();

    let at = move || playhead_now().max(*clock.current.peek());
    let sel = move || selected_clip.peek().clone();
    let track = move || target_track.peek().clone();

    match tab {
        AssetTab::Media => rsx! { MediaPanel {} },
        AssetTab::Text => rsx! {
            div { class: "card-list",
                button {
                    class: "card",
                    onclick: move |_| {
                        let text = ask_text("Title", "Title");
                        live_note(save, &library.peek(), tools::add_graphic_at(save, at(), Graphic::title(text), 4.0));
                    },
                    b { "Title" }
                    span { "Large heading on the picture" }
                }
                button {
                    class: "card",
                    onclick: move |_| {
                        let text = ask_text("Lower third", "Name");
                        live_note(save, &library.peek(), tools::add_graphic_at(save, at(), Graphic::lower_third(text), 4.0));
                    },
                    b { "Lower third" }
                    span { "Name and title card" }
                }
                button {
                    class: "card",
                    onclick: move |_| {
                        let text = ask_text("Card", "Card");
                        live_note(save, &library.peek(), tools::add_graphic_at(save, at(), Graphic::card(text), 3.0));
                    },
                    b { "Card" }
                    span { "Full-frame graphic" }
                }
            }
        },
        AssetTab::Captions => {
            let line = caption_under(&save.engine.read(), *clock.current.read());
            let idle = line.is_none();
            rsx! {
                div { class: "drop",
                    IconCaptions {}
                    strong { "Captions" }
                    small { "The line under the playhead. Generate reads speech into a caption track." }
                    p { class: "caption-now",
                        if let Some((_, _, text)) = &line {
                            "{text}"
                        } else {
                            "No caption under the playhead"
                        }
                    }
                    button {
                        class: "btn caption-edit",
                        disabled: idle,
                        onclick: move |_| {
                            let Some((clip_id, index, text)) = caption_under(&save.engine.peek(), playhead_now()) else {
                                show_toast().error("No caption under the playhead");
                                return;
                            };
                            let Some(next) = web_sys::window().and_then(|window| {
                                window.prompt_with_message_and_default("Caption", &text).ok().flatten()
                            }) else {
                                return;
                            };
                            let next = next.trim().to_string();
                            if next.is_empty() {
                                show_toast().error("A caption line cannot be empty");
                                return;
                            }
                            live_note(save, &library.peek(), tools::set_caption_text(save, clip_id, index, next));
                        },
                        "Edit caption"
                    }
                    button {
                        class: "btn btn-primary",
                        onclick: move |_| {
                            let pid = save.project_id.peek().clone();
                            let clock = clock;
                            spawn(async move {
                                if let Err(err) = api::ensure_speech(&pid).await {
                                    show_toast().error(format!("Speech was not stored — {err}"));
                                    return;
                                }
                                match api::generate_captions(&pid).await {
                                    Ok((timeline, note)) => {
                                        let mut save = save;
                                        show_timeline(&mut save, &clock, timeline);
                                        show_toast().success(note);
                                    }
                                    Err(err) => show_toast().error(err),
                                }
                            });
                        },
                        "Generate captions"
                    }
                }
            }
        }
        AssetTab::Audio => rsx! {
            div { class: "card-list",
                studio::Mixer {}
                button {
                    class: "card",
                    onclick: move |_| {
                        live_note(save, &library.peek(), tools::set_volume_at(save, sel().as_deref(), &track(), at(), 0.7));
                    },
                    b { "Volume" }
                    span { "Drop selected audio to 70%" }
                }
                button {
                    class: "card",
                    onclick: move |_| {
                        live_note(save, &library.peek(), tools::set_fade_at(save, sel().as_deref(), &track(), at(), 0.8));
                    },
                    b { "Fade" }
                    span { "0.8s fade in and out" }
                }
                button {
                    class: "card",
                    onclick: move |_| live_note(save, &library.peek(), tools::duck_at(save)),
                    b { "Duck" }
                    span { "Lower music under speech" }
                }
                button {
                    class: "card",
                    onclick: move |_| tab_sig.set(AssetTab::Media),
                    b { "Music" }
                    span { "Import a soundtrack in Media" }
                }
            }
        },
        AssetTab::Elements => rsx! {
            div {
                studio::Generators { at: at() }
            }
            div { class: "card-list",
                button {
                    class: "card",
                    onclick: move |_| {
                        live_note(save, &library.peek(), tools::add_graphic_at(save, at(), Graphic::shape(), 3.0));
                    },
                    b { "Shapes" }
                    span { "Rects, circles, lines" }
                }
                button {
                    class: "card",
                    onclick: move |_| {
                        live_note(save, &library.peek(), tools::add_graphic_at(save, at(), Graphic::sticker("★"), 3.0));
                    },
                    b { "Stickers" }
                    span { "Emojis and badges" }
                }
            }
        },
        AssetTab::Transitions => rsx! {
            div { class: "mix-panel",
                div { class: "mix-group", "Clip fade" }
                div { class: "mix-grid",
                    button {
                        class: "mix-tile",
                        title: "0.8s from black at the clip start",
                        onclick: move |_| {
                            live_note(save, &library.peek(), tools::set_fade_ends(save, sel().as_deref(), &track(), at(), 0.8, 0.0));
                        },
                        MixPreview { class: "fadein" }
                        b { "Fade in" }
                    }
                    button {
                        class: "mix-tile",
                        title: "0.8s to black at the clip end",
                        onclick: move |_| {
                            live_note(save, &library.peek(), tools::set_fade_ends(save, sel().as_deref(), &track(), at(), 0.0, 0.8));
                        },
                        MixPreview { class: "fadeout" }
                        b { "Fade out" }
                    }
                }
                for group in ["Cut", "Dissolve", "Wipe", "Slide", "Shape"] {
                    div { class: "mix-group", "{group}" }
                    div { class: "mix-grid",
                        for kind in TransitionKind::ALL.iter().copied().filter(|k| k.group() == group) {
                            button {
                                class: "mix-tile",
                                title: kind.hint(),
                                onclick: move |_| {
                                    live_note(save, &library.peek(), tools::set_transition_at(save, sel().as_deref(), &track(), at(), kind));
                                },
                                MixPreview { class: mix_preview_class(kind) }
                                b { "{kind.label()}" }
                            }
                        }
                    }
                }
            }
        },
        AssetTab::Visuals => rsx! {
            div {
                studio::CurvesPanel { selected: sel().unwrap_or_default(), track: track(), at: at() }
                studio::MaskPanel { selected: sel().unwrap_or_default(), track: track(), at: at() }
                studio::TimeRemap { selected: sel().unwrap_or_default(), track: track(), at: at() }
            }
            div { class: "card-list",
                button {
                    class: "card",
                    onclick: move |_| {
                        live_note(save, &library.peek(), tools::set_grade_at(save, sel().as_deref(), &track(), at(), Grade::punchy()));
                    },
                    b { "Color" }
                    span { "Punchy grade on the clip" }
                }
                button {
                    class: "card",
                    onclick: move |_| {
                        live_note(save, &library.peek(), tools::set_fx_at(save, sel().as_deref(), &track(), at(), Fx::film()));
                    },
                    b { "Effects" }
                    span { "Grain and vignette" }
                }
                button {
                    class: "card",
                    title: "Audio starts 12 frames before the picture",
                    onclick: move |_| {
                        let step = frame_step(save);
                        live_note(save, &library.peek(), tools::jl_at(save, sel().as_deref(), &track(), at(), step, 0.0));
                    },
                    b { "J-cut" }
                    span { "Sound leads the picture by 12 frames" }
                }
                button {
                    class: "card",
                    title: "Audio continues 12 frames after the picture",
                    onclick: move |_| {
                        let step = frame_step(save);
                        live_note(save, &library.peek(), tools::jl_at(save, sel().as_deref(), &track(), at(), 0.0, step));
                    },
                    b { "L-cut" }
                    span { "Sound holds 12 frames after the picture" }
                }
                label { class: "card cube-load",
                    b { "Load .cube" }
                    span { "3D LUT on the selected clip" }
                    input {
                        r#type: "file",
                        accept: ".cube,text/plain",
                        hidden: true,
                        onchange: move |evt| {
                            let save = save;
                            let library = library;
                            let selected = sel();
                            let track_id = track();
                            let at_s = at();
                            spawn(async move {
                                for file in evt.files() {
                                    let Ok(bytes) = file.read_bytes().await else {
                                        show_toast().error("Could not read the .cube file");
                                        continue;
                                    };
                                    let text = String::from_utf8_lossy(&bytes).into_owned();
                                    live_note(
                                        save,
                                        &library.peek(),
                                        tools::import_cube_text(
                                            save,
                                            selected.as_deref(),
                                            &track_id,
                                            at_s,
                                            text,
                                        ),
                                    );
                                    break;
                                }
                            });
                        },
                    }
                }
            }
        },
        AssetTab::Settings => {
            let (rate, bg, bg_name) = {
                let engine = save.engine.read();
                let bg = oc_core::canonical_color(&engine.background);
                (
                    format!("{} fps", engine.frame_rate.label()),
                    bg.clone(),
                    background_label(&bg),
                )
            };
            rsx! {
                studio::UndoHistory {}
                div { class: "card-list",
                    button {
                        class: "card",
                        title: "Cycle 23.976, 24, 25, 29.97, 30, 50, 59.94, 60",
                        onclick: move |_| {
                            let next = save.engine.peek().frame_rate.cycle();
                            live_note(save, &library.peek(), tools::set_frame_rate(save, next));
                        },
                        b { "Frame rate" }
                        span { "{rate}" }
                    }
                    div { class: "card settings-color",
                        button {
                            title: "Cycle black, white, gray, and charcoal",
                            onclick: move |_| {
                                let next = next_background(&save.engine.peek().background);
                                live_note(save, &library.peek(), tools::set_background(save, next));
                            },
                            b { "Background" }
                            span { "{bg_name}" }
                        }
                        input {
                            r#type: "color",
                            value: "{bg}",
                            title: "Pick a monitor color",
                            onchange: move |evt| {
                                let color = evt.value();
                                if !color.is_empty() {
                                    live_note(save, &library.peek(), tools::set_background(save, &color));
                                }
                            },
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn MediaTile(
    item: MediaItem,
    mut library: Signal<Vec<MediaItem>>,
    mut tracks: Signal<Vec<EditorTrack>>,
    mut active: Signal<Option<String>>,
    clock: Clock,
    target_track: Signal<String>,
    edit_mode: Signal<EditMode>,
    mut drag: Signal<Option<DragSession>>,
    mut held: Signal<Vec<HeldImport>>,
    mut uploading: Signal<Vec<String>>,
    save: WorkspaceSave,
) -> Element {
    let url = item.url.clone();
    let pick_url = url.clone();
    let add_url = url.clone();
    let id = item.id.clone();
    let selected = active.read().as_deref() == Some(url.as_str());
    let uploading_now = uploading.read().iter().any(|row| row == &id);
    let tile_class = if uploading_now {
        "media-tile uploading"
    } else if selected {
        "media-tile on"
    } else {
        "media-tile"
    };
    let kind = item.kind;
    let name = item.name.clone();
    let drag_item = item.clone();
    let kind_tag = match kind {
        MediaKind::Video => " video",
        MediaKind::Audio => " audio",
        MediaKind::Image => " image",
    };
    let tile_class = format!("{tile_class}{kind_tag}");
    rsx! {
        div {
            class: "{tile_class}",
            title: if uploading_now {
                "Saving in this browser..."
            } else if url.is_empty() {
                "This file is only in the browser that imported it"
            } else {
                "Drag onto a track, or press + at the playhead"
            },
            button {
                class: "media-add",
                title: "Add to timeline",
                onclick: move |evt| {
                    evt.stop_propagation();
                    if let Some(item) = library
                        .read()
                        .iter()
                        .find(|item| item.url == add_url)
                        .cloned()
                    {
                        place_clip(
                            &mut tracks.write(),
                            &item,
                            *clock.current.peek(),
                            Some(target_track.peek().as_str()),
                            *edit_mode.peek(),
                            save.engine.peek().mark_range(),
                        );
                        let end = timeline_end(&tracks.read());
                        if end > *clock.duration.peek() {
                            let mut duration = clock.duration;
                            duration.set(end);
                        }
                        persist(save);
                    }
                },
                "+"
            }
            button {
                class: "media-pick",
                onmousedown: move |evt| {
                    evt.stop_propagation();
                    drag.set(Some(DragSession {
                        source: DragSource::Media {
                            media_id: drag_item.id.clone(),
                        },
                        grab: 0.0,
                        at: *clock.current.peek(),
                        track_id: target_track.peek().clone(),
                        duration: clip_duration(&drag_item),
                        name: drag_item.name.clone(),
                        url: drag_item.url.clone(),
                        media_kind: drag_item.kind,
                        origin_x: evt.client_coordinates().x,
                        origin_y: evt.client_coordinates().y,
                        client_x: evt.client_coordinates().x,
                        client_y: evt.client_coordinates().y,
                        moved: false,
                        over_timeline: false,
                    }));
                },
                onclick: move |_| active.set(Some(pick_url.clone())),
                div { class: "media-icon",
                    match (url.is_empty(), kind) {
                        (true, _) => rsx! { span { class: "media-missing", "Not in this browser" } },
                        (false, MediaKind::Image) => rsx! { img { src: "{url}", alt: "" } },
                        (false, MediaKind::Video) => {
                            let probe_url = url.clone();
                            rsx! {
                                video {
                                    src: "{url}",
                                    muted: true,
                                    preload: "metadata",
                                    onloadedmetadata: move |_| {
                                        let Some(duration) = video_duration_from_src(&probe_url) else {
                                            return;
                                        };
                                        let changed = set_media_duration(
                                            &mut library.write(),
                                            &mut tracks.write(),
                                            &probe_url,
                                            duration,
                                        );
                                        if changed {
                                            let mid = library
                                                .read()
                                                .iter()
                                                .find(|item| item.url == probe_url)
                                                .map(|item| item.id.clone());
                                            let pid = save.project_id.peek().clone();
                                            if let Some(mid) = mid {
                                                spawn(async move {
                                                    let _ = crate::api::patch_media_duration(
                                                        &pid, &mid, duration,
                                                    )
                                                    .await;
                                                });
                                            }
                                        }
                                    },
                                }
                            }
                        }
                        (false, MediaKind::Audio) => rsx! { IconWave {} },
                    }
                }
                span { class: "media-name", "{name}" }
            }
            if uploading_now {
                div { class: "media-upload", "Saving..." }
            } else if url.is_empty() {
                div { class: "media-upload", "Not in this browser" }
            }
            button {
                class: "media-x",
                title: "Remove",
                onclick: move |_| {
                    let remove_id = id.clone();
                    let remove_url = url.clone();
                    let pid = save.project_id.peek().clone();
                    bury_media(&pid, &remove_id);
                    library.write().retain(|item| item.id != remove_id);
                    held.write().retain(|row| row.id != remove_id);
                    end_upload(uploading, &remove_id);
                    for track in tracks.write().iter_mut() {
                        track.clips.retain(|clip| clip.media_id != remove_id);
                    }
                    if active.read().as_deref() == Some(remove_url.as_str()) {
                        let next = library.read().first().map(|item| item.url.clone());
                        active.set(next);
                    }
                    if !pin_has_url(&pid, &remove_url)
                        && !library.peek().iter().any(|item| item.url == remove_url)
                    {
                        revoke_blob_url(&remove_url);
                    }
                    persist(save);
                },
                "×"
            }
        }
    }
}

#[component]
fn MediaPanel() -> Element {
    let library = use_context::<Signal<Vec<MediaItem>>>();
    let tracks = use_context::<Signal<Vec<EditorTrack>>>();
    let active = use_context::<CtxActive>().0;
    let clock = use_context::<Clock>();
    let target_track = use_context::<CtxTargetTrack>().0;
    let edit_mode = use_context::<Signal<EditMode>>();
    let drag = use_context::<Signal<Option<DragSession>>>();
    let held = use_context::<CtxHeld>().0;
    let uploading = use_context::<CtxUploading>().0;
    let save = use_context::<WorkspaceSave>();
    // Own the rows before building the tiles. A video metadata event can write
    // the bin, and that write panics if this read is still held.
    let rows = library.read().clone();

    rsx! {
        div { class: "assets-body",
            div { class: "assets-head",
                div { class: "assets-title", "Media" }
                label { class: "btn btn-primary import-btn",
                    "Import"
                    input {
                        r#type: "file",
                        accept: "video/*,audio/*,image/*",
                        multiple: true,
                        hidden: true,
                        onchange: move |evt| {
                            spawn(async move {
                                for file in evt.files() {
                                    import_browser_file(file, save, library, active, held, uploading)
                                        .await;
                                }
                            });
                        },
                    }
                }
            }
            div { class: "media-grid",
                for item in rows {
                    MediaTile {
                        key: "{item.id}",
                        item: item.clone(),
                        library,
                        tracks,
                        active,
                        clock,
                        target_track,
                        edit_mode,
                        drag,
                        held,
                        uploading,
                        save,
                    }
                }
            }
        }
    }
}

#[component]
fn Preview(aspect: Signal<Aspect>, playing: Signal<bool>) -> Element {
    let current = *aspect.read();
    let size = current.size();
    let mut library = use_context::<Signal<Vec<MediaItem>>>();
    let mut tracks = use_context::<Signal<Vec<EditorTrack>>>();
    let clock = use_context::<Clock>();
    let save = use_context::<WorkspaceSave>();
    let selected_clip = use_context::<CtxSelected>().0;
    let _ = playing;
    let now = playhead_now().max(*clock.current.read());
    let chrome = media::monitor_chrome(&save.engine.read(), now, selected_clip.read().as_deref());
    let monitor_bg = oc_core::canonical_color(&save.engine.read().background);
    let grade_class = if chrome.cube {
        "preview-grade"
    } else {
        "preview-grade off"
    };
    let mask_class = if chrome.mask {
        "preview-mask-host"
    } else {
        "preview-mask-host off"
    };
    let letter_class = if chrome.letterbox {
        "preview-letterbox"
    } else {
        "preview-letterbox off"
    };
    let handle_class = if chrome.mask {
        "mask-handles"
    } else {
        "mask-handles off"
    };
    let gfx_class = if chrome.letterbox {
        "preview-gfx letterboxed"
    } else {
        "preview-gfx"
    };

    use_effect(move || {
        media::set_selected_clip(selected_clip.read().clone());
        let playing = *clock.playing.read();
        if playing {
            return;
        }
        let now = *clock.current.read();
        sync_monitor(
            &save.engine.read(),
            &library.read(),
            &tracks.read(),
            now,
            false,
        );
        apply_monitor_look(&save.engine.read(), &library.read(), now, false);
    });

    rsx! {
        section { class: "panel preview",
            div { class: "monitor-wrap",
                div { class: current.class(), style: "background: {monitor_bg}",
                    video {
                        class: "preview-video off",
                        preload: "auto",
                        playsinline: true,
                        onloadedmetadata: move |_| {
                            let mut changed = false;
                            if let Some(video) = preview_video() {
                                let src = video.current_src();
                                changed = set_media_duration(
                                    &mut library.write(),
                                    &mut tracks.write(),
                                    &src,
                                    video.duration(),
                                );
                                let end = program_end(&tracks.read());
                                if end > 0.05 {
                                    let mut duration = clock.duration;
                                    duration.set(end);
                                }
                                let now = playhead_now();
                                if let Some(shot) = crate::media::clip_under(
                                    &tracks.read(),
                                    &library.read(),
                                    now,
                                ) {
                                    let rate = if shot.speed.is_finite() && shot.speed > 0.0 {
                                        shot.speed.clamp(0.25, 4.0)
                                    } else {
                                        1.0
                                    };
                                    video.set_current_time(
                                        (shot.source_in + (now - shot.start).max(0.0) * rate)
                                            .max(0.0),
                                    );
                                }
                                note_loaded_picture(save, &library.peek(), &video);
                            }
                            if changed {
                                persist(save);
                            }
                            paint_clock();
                        },
                    }
                    video {
                        class: "preview-video-b off",
                        preload: "auto",
                        playsinline: true,
                        muted: true,
                    }
                    audio { class: "preview-bed", preload: "auto" }
                    audio { class: "preview-bed", preload: "auto" }
                    img { class: "preview-image off", alt: "" }
                    img { class: "preview-design off", alt: "" }
                    video {
                        class: "preview-design-clip off",
                        preload: "auto",
                        playsinline: true,
                        muted: true,
                    }
                    canvas {
                        id: "program-canvas",
                        class: "program-canvas off",
                        width: "420",
                        height: "236",
                    }
                    canvas {
                        id: "grade-canvas",
                        class: "{grade_class}",
                        width: "160",
                        height: "90",
                    }
                    div { class: "{mask_class}" }
                    div { class: "{letter_class}" }
                    div { class: "preview-vignette off" }
                    div { class: "preview-grain off" }
                    div { class: "{gfx_class}" }
                    div {
                        id: "mask-handles",
                        class: "{handle_class}",
                        onpointerdown: move |evt| {
                            capture_pointer(&evt);
                            media::mask_down(&evt);
                        },
                        onpointermove: move |evt| media::mask_move(&evt),
                        onpointerup: move |evt| commit_mask(save, &library.peek(), &evt),
                        onpointercancel: move |evt| commit_mask(save, &library.peek(), &evt),
                        div {
                            id: "mask-box",
                            class: "mask-box",
                            "data-edge": "move",
                            span { class: "mask-handle nw", "data-edge": "nw" }
                            span { class: "mask-handle ne", "data-edge": "ne" }
                            span { class: "mask-handle sw", "data-edge": "sw" }
                            span { class: "mask-handle se", "data-edge": "se" }
                            span { class: "mask-handle n", "data-edge": "n" }
                            span { class: "mask-handle s", "data-edge": "s" }
                            span { class: "mask-handle e", "data-edge": "e" }
                            span { class: "mask-handle w", "data-edge": "w" }
                        }
                    }
                    div { class: "monitor-blank",
                        span { class: "monitor-meta", "{size}" }
                    }
                }
            }
            studio::MulticamBank {}
        }
    }
}

#[component]
fn Timeline() -> Element {
    let library = use_context::<Signal<Vec<MediaItem>>>();
    let mut tracks = use_context::<Signal<Vec<EditorTrack>>>();
    let clock = use_context::<Clock>();
    let mut target_track = use_context::<CtxTargetTrack>().0;
    let mut edit_mode = use_context::<Signal<EditMode>>();
    let mut edit_tool = use_context::<Signal<EditTool>>();
    let mut selected_clip = use_context::<CtxSelected>().0;
    let mut drag = use_context::<Signal<Option<DragSession>>>();
    let mut pps = use_context::<CtxZoom>().0;
    let mut suppress_seek = use_context::<Signal<bool>>();
    let mut tl_h = use_context::<CtxTimelineH>().0;
    let mut tl_drag = use_context::<Signal<Option<(f64, f64)>>>();
    let save = use_context::<WorkspaceSave>();
    let uploading = use_context::<CtxUploading>().0;
    let mut view_h = use_signal(|| 280.0_f64);
    let mut view_w = use_signal(|| 800.0_f64);
    let mut trim = use_signal(|| None::<(String, bool)>);
    let now = *clock.current.read();
    let span = program_end(&tracks.read()).max(8.0);
    let canvas_w = (span * *pps.read() + 160.0).max(*view_w.read());
    let playhead = format!("left: {}px", now * *pps.read());
    let now_label = format_clock(now);
    let dur_label = format_clock(span);
    let rows = display_tracks(&tracks.read());
    let scale = fit_scale(&rows, *view_h.read());

    use_effect(move || {
        view_h.set(timeline_viewport_h());
        view_w.set(timeline_viewport_w());
    });
    use_future(move || async move {
        loop {
            gloo_timers::future::TimeoutFuture::new(250).await;
            let h = timeline_viewport_h();
            if (h - *view_h.peek()).abs() > 1.0 {
                view_h.set(h);
            }
            let w = timeline_viewport_w();
            if (w - *view_w.peek()).abs() > 1.0 {
                view_w.set(w);
            }
        }
    });
    let tool = *edit_tool.read();
    let target_id = target_track.read().clone();
    let marks = ruler_marks_nle(span, *pps.read());

    rsx! {
        section {
            class: "panel timeline",
            tabindex: "0",
            style: {
                let h = (*tl_h.read()).round();
                format!("flex: 0 0 {h}px; height: {h}px; max-height: {h}px")
            },
            onkeydown: move |evt| {
                if evt.modifiers().ctrl() && evt.key() == Key::Character("z".into()) {
                    if evt.modifiers().shift() {
                        let _ = tools::redo_edit(save);
                    } else {
                        let _ = tools::undo_edit(save);
                    }
                    return;
                }
                if *edit_tool.peek() == ToolId::Multicam {
                    if let Key::Character(c) = evt.key() {
                        if let Some(n) = c.chars().next().and_then(|ch| ch.to_digit(10)) {
                            if (1..10).contains(&n) {
                                let id = {
                                    let tl = save.engine.peek();
                                    tl.tracks
                                        .iter()
                                        .filter(|t| t.kind == TrackKind::Video)
                                        .nth((n - 1) as usize)
                                        .map(|track| track.id.to_string())
                                };
                                if let Some(id) = id {
                                    let _ = tools::multicam_at(save, &id, playhead_now().max(*clock.current.peek()));
                                }
                                return;
                            }
                        }
                    }
                }
                match evt.key() {
                    Key::Character(c) if c == "1" => edit_mode.set(EditMode::Normal),
                    Key::Character(c) if c == "2" => edit_mode.set(EditMode::Insert),
                    Key::Character(c) if c == "3" => edit_mode.set(EditMode::Overwrite),
                    Key::Character(c) if c.eq_ignore_ascii_case("s") => {
                        if evt.modifiers().shift() {
                            let _ = tools::split_all_at(save, playhead_now().max(*clock.current.peek()));
                        } else {
                            edit_tool.set(ToolId::Select);
                        }
                    }
                    Key::Character(c) if c.eq_ignore_ascii_case("x") => {
                        edit_tool.set(ToolId::Razor);
                    }
                    Key::Character(c) if c.eq_ignore_ascii_case("m") => {
                        edit_tool.set(ToolId::Spacer);
                    }
                    Key::Character(c) if c.eq_ignore_ascii_case("y") => {
                        edit_tool.set(ToolId::Slip);
                    }
                    Key::Character(c) if c.eq_ignore_ascii_case("b") => {
                        edit_tool.set(ToolId::Ripple);
                    }
                    Key::Character(c) if c.eq_ignore_ascii_case("n") => {
                        edit_tool.set(ToolId::Roll);
                    }
                    Key::Character(c) if c.eq_ignore_ascii_case("u") => {
                        edit_tool.set(ToolId::Slide);
                    }
                    Key::Character(c) if c.eq_ignore_ascii_case("r") => {
                        edit_tool.set(ToolId::RateStretch);
                    }
                    Key::Character(c) if c.eq_ignore_ascii_case("i") => {
                        let _ = tools::mark_in_at(save, playhead_now().max(*clock.current.peek()));
                    }
                    Key::Character(c) if c.eq_ignore_ascii_case("o") => {
                        let _ = tools::mark_out_at(save, playhead_now().max(*clock.current.peek()));
                    }
                    Key::Character(c) if c.eq_ignore_ascii_case("d") => {
                        let at = playhead_now().max(*clock.current.peek());
                        let _ = tools::detach_audio_at(save, target_track.peek().as_str(), at);
                    }
                    Key::Character(c) if c.eq_ignore_ascii_case("g") => {
                        let at = playhead_now().max(*clock.current.peek());
                        if evt.modifiers().shift() {
                            let _ = tools::ungroup_at(save, target_track.peek().as_str(), at);
                        } else {
                            let _ = tools::group_at(save, target_track.peek().as_str(), at);
                        }
                    }
                    _ => {}
                }
            },
            onpointermove: move |evt| {
                drag_trim(
                    tracks,
                    library,
                    *pps.peek(),
                    *edit_tool.peek() == ToolId::Ripple,
                    trim,
                    evt.client_coordinates().x,
                    evt.client_coordinates().y,
                );
            },
            onpointerup: move |_| {
                commit_trim(save, tracks, trim);
            },
            onpointercancel: move |_| {
                commit_trim(save, tracks, trim);
            },
            div {
                class: "tl-split",
                title: "Drag up or down to resize the timeline",
                onpointerdown: move |evt| {
                    evt.stop_propagation();
                    evt.prevent_default();
                    capture_pointer(&evt);
                    tl_drag.set(Some((evt.client_coordinates().y, *tl_h.peek())));
                },
                onpointermove: move |evt| {
                    let Some((origin_y, origin_h)) = *tl_drag.peek() else {
                        return;
                    };
                    let next = (origin_h + (origin_y - evt.client_coordinates().y))
                        .clamp(200.0, max_timeline_h());
                    tl_h.set(next);
                },
                onpointerup: move |_| tl_drag.set(None),
                onpointercancel: move |_| tl_drag.set(None),
            }
            div { class: "tl-toolbar",
                button {
                    class: "play",
                    title: if *clock.playing.read() { "Pause" } else { "Play" },
                    onclick: move |_| {
                        let mut playing = clock.playing;
                        let next = !*playing.read();
                        if next {
                            crate::media::set_playhead(*clock.current.peek());
                            crate::media::resume_meter();
                            reset_tick_clock();
                        } else {
                            let mut current = clock.current;
                            current.set(playhead_now());
                            reset_tick_clock();
                        }
                        playing.set(next);
                    },
                    if *clock.playing.read() {
                        IconPause {}
                    } else {
                        IconPlay {}
                    }
                }
                button {
                    class: "btn-ghost btn-icon",
                    title: "Skip back",
                    onclick: move |_| { seek_by(clock, -5.0); paint_clock(); },
                    IconSkipBack {}
                }
                button {
                    class: "btn-ghost btn-icon",
                    title: "Skip forward",
                    onclick: move |_| { seek_by(clock, 5.0); paint_clock(); },
                    IconSkipFwd {}
                }
                span { class: "timecode tl-clock", "{now_label} / {dur_label}" }
                div { class: "sep" }
                select {
                    class: "mode-select",
                    title: "How drops land: Normal keeps gaps, Insert pushes, Overwrite covers",
                    value: "{edit_mode.read().key()}",
                    onchange: move |evt| {
                        edit_mode.set(EditMode::from_key(&evt.value()));
                    },
                    for mode in TimelineEditMode::ALL {
                        option {
                            value: "{mode.key()}",
                            selected: *edit_mode.read() == mode,
                            "{mode.label()}"
                        }
                    }
                }
                div { class: "sep" }
                div { class: "tool-group",
                    for spec in edit_tools() {
                        {
                            let id = spec.id;
                            let on = tool == id;
                            rsx! {
                                button {
                                    class: if on { "btn-icon on" } else { "btn-icon" },
                                    title: "{spec.tip}",
                                    onclick: move |_| edit_tool.set(id),
                                    {mode_glyph(id)}
                                }
                            }
                        }
                    }
                }
                div { class: "sep" }
                div { class: "tool-group",
                    for spec in cut_actions() {
                        {
                            let id = spec.id;
                            rsx! {
                                button {
                                    class: "btn-icon",
                                    title: "{spec.tip}",
                                    onclick: move |_| fire_tool(id, save, clock, target_track, &mut tracks, edit_mode),
                                    {tool_glyph(id)}
                                }
                            }
                        }
                    }
                }
                div { class: "sep" }
                div { class: "tool-group",
                    for spec in track_actions() {
                        {
                            let id = spec.id;
                            rsx! {
                                button {
                                    class: "btn-icon",
                                    title: "{spec.tip}",
                                    onclick: move |_| fire_tool(id, save, clock, target_track, &mut tracks, edit_mode),
                                    {tool_glyph(id)}
                                }
                            }
                        }
                    }
                }
            }
            div { class: "tl-body", "data-scale": "{scale}",
            div { class: "tl-v",
                div { class: "tl-heads",
                    div { class: "ruler-gutter" }
                    for track in rows.iter() {
                        {
                            let track_id = track.id.clone();
                            let select_id = track_id.clone();
                            let selected = target_id == track.id;
                            let can_remove = tracks.read().iter().filter(|t| t.kind == track.kind).count() > 1;
                            let kind_class = match track.kind {
                                TrackKindUi::Video => "head-v",
                                TrackKindUi::Audio => "head-a",
                                TrackKindUi::Caption => "head-c",
                            };
                            let head_class = if selected {
                                format!("track-head {kind_class} on")
                            } else {
                                format!("track-head {kind_class}")
                            };
                            let muted = track.muted;
                            let hidden = track.hidden;
                            let mute_id = track_id.clone();
                            let hide_id = track_id.clone();
                            let add_id = track_id.clone();
                            let add_kind = track.kind;
                            rsx! {
                                div {
                                    class: "{head_class}",
                                    "data-track": "{track.id}",
                                    style: "height: {lane_height(track.kind, scale)}px",
                                    title: "Target this track",
                                    onclick: move |_| target_track.set(select_id.clone()),
                                    span { class: "th-name", "{track.name}" }
                                    div { class: "th-tools",
                                        button {
                                            class: "th-btn",
                                            title: match add_kind {
                                                TrackKindUi::Video => "Add video track",
                                                TrackKindUi::Audio => "Add audio track",
                                                TrackKindUi::Caption => "Add caption track",
                                            },
                                            onclick: move |evt| {
                                                evt.stop_propagation();
                                                add_track_after(
                                                    &mut tracks,
                                                    &mut target_track,
                                                    add_kind,
                                                    &add_id,
                                                    save,
                                                );
                                                crate::toast::show_toast().success(match add_kind {
                                                    TrackKindUi::Video => "Added a video track",
                                                    TrackKindUi::Audio => "Added an audio track",
                                                    TrackKindUi::Caption => "Added a caption track",
                                                });
                                            },
                                            "+"
                                        }
                                        button {
                                            class: if hidden { "th-btn on" } else { "th-btn" },
                                            title: "Hide",
                                            onclick: move |evt| {
                                                evt.stop_propagation();
                                                if let Some(track) = tracks.write().iter_mut().find(|t| t.id == hide_id) {
                                                    track.hidden = !track.hidden;
                                                }
                                                persist(save);
                                            },
                                            "H"
                                        }
                                        button {
                                            class: if muted { "th-btn on" } else { "th-btn" },
                                            title: "Mute",
                                            onclick: move |evt| {
                                                evt.stop_propagation();
                                                if let Some(track) = tracks.write().iter_mut().find(|t| t.id == mute_id) {
                                                    track.muted = !track.muted;
                                                }
                                                persist(save);
                                            },
                                            "M"
                                        }
                                        if can_remove {
                                            button {
                                                class: "track-x",
                                                title: "Remove track",
                                                onclick: move |evt| {
                                                    evt.stop_propagation();
                                                    tracks.write().retain(|t| t.id != track_id);
                                                    persist(save);
                                                },
                                                "×"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                div {
                    class: "tl-scroll",
                    "data-pps": "{*pps.read()}",
                    "data-span": "{span}",
                    onclick: move |evt| {
                        if *suppress_seek.peek() {
                            suppress_seek.set(false);
                            return;
                        }
                        let x = evt.element_coordinates().x + scroll_left();
                        if x > 0.0 {
                            seek_to_time(clock, x / *pps.read());
                        }
                    },
                    div {
                        class: "ruler",
                        style: "width: {canvas_w}px",
                        title: "Scroll to zoom time",
                        onwheel: move |evt| {
                            let dy = evt.data().delta().strip_units().y;
                            if dy.abs() < 0.1 {
                                return;
                            }
                            evt.prevent_default();
                            evt.stop_propagation();
                            let factor = if dy > 0.0 { 1.0 / 1.15 } else { 1.15 };
                            let next = clamp_pps(*pps.peek() * factor);
                            pps.set(next);
                        },
                        for mark in marks.iter() {
                            {
                                let left = mark.time * *pps.read();
                                let cls = if mark.major { "r-tick major" } else { "r-tick" };
                                rsx! {
                                    span { class: "{cls}", style: "left: {left}px",
                                        if mark.major {
                                            "{mark.label}"
                                        }
                                    }
                                }
                            }
                        }
                        for marker in save.engine.read().markers.iter() {
                            {
                                let left = marker.time.as_seconds() * *pps.read();
                                let name = marker.name.clone();
                                rsx! {
                                    span {
                                        class: "r-tick marker",
                                        style: "left: {left}px",
                                        title: "{name}",
                                    }
                                }
                            }
                        }
                    }
                    div { class: "tl-lanes", style: "width: {canvas_w}px",
                        div { class: "playhead", style: "{playhead}" }
                        for track in rows.iter() {
                            {
                                let kind_class = match track.kind {
                                    TrackKindUi::Video => "lane-v",
                                    TrackKindUi::Audio => "lane-a",
                                    TrackKindUi::Caption => "lane-c",
                                };
                                let lane_id = track.id.clone();
                                let pick_lane = lane_id.clone();
                                let h = lane_height(track.kind, scale);
                                let dimmed = track.hidden || track.muted;
                                let ghost = drag.read().as_ref().and_then(|session| {
                                    if session.moved
                                        && session.over_timeline
                                        && session.track_id == lane_id
                                    {
                                        Some((session.at, session.duration, session.name.clone()))
                                    } else {
                                        None
                                    }
                                });
                                let lifted_id = drag.read().as_ref().and_then(|session| match &session.source {
                                    DragSource::Clip { clip_id } if session.moved => Some(clip_id.clone()),
                                    _ => None,
                                });
                                rsx! {
                                    div {
                                        class: {
                                            let mut cls = format!("lane {kind_class}");
                                            if dimmed {
                                                cls.push_str(" dim");
                                            }
                                            if ghost.is_some() {
                                                cls.push_str(" drop-on");
                                            }
                                            cls
                                        },
                                        "data-track": "{lane_id}",
                                        style: "height: {h}px",
                                        onclick: move |evt| {
                                            evt.stop_propagation();
                                            if *suppress_seek.peek() {
                                                suppress_seek.set(false);
                                                return;
                                            }
                                            target_track.set(pick_lane.clone());
                                            let x = evt.element_coordinates().x;
                                            if x <= 0.0 {
                                                return;
                                            }
                                            let at = x / *pps.read();
                                            if *edit_tool.peek() == EditTool::Razor {
                                                let _ = tools::split_at(save, pick_lane.as_str(), at);
                                                return;
                                            }
                                            if *edit_tool.peek() == EditTool::Multicam {
                                                let _ = tools::multicam_at(
                                                    save,
                                                    pick_lane.as_str(),
                                                    playhead_now().max(*clock.current.peek()),
                                                );
                                                return;
                                            }
                                            seek_to_time(clock, at);
                                        },
                                        for clip in track.clips.iter() {
                                            {
                                                let left = clip.start * *pps.read();
                                                let width = (clip.duration * *pps.read()).max(24.0);
                                                let item = library.read().iter().find(|m| m.id == clip.media_id).cloned();
                                                let name = clip_name(clip, item.as_ref().map(|m| m.name.as_str()));
                                                let url = item.as_ref().map(|m| m.url.clone()).unwrap_or_default();
                                                let media_kind = item.as_ref().map(|m| m.kind).unwrap_or(MediaKind::Video);
                                                let is_video = media_kind == MediaKind::Video;
                                                let is_image = media_kind == MediaKind::Image;
                                                let is_audio = media_kind == MediaKind::Audio
                                                    || track.kind == TrackKindUi::Audio;
                                                let selected = selected_clip.read().as_deref() == Some(clip.id.as_str());
                                                let clip_uploading = uploading.read().iter().any(|row| row == &clip.media_id);
                                                let lifted = lifted_id.as_deref() == Some(clip.id.as_str());
                                                let mix = !clip.transition.is_empty()
                                                    && clip.transition != "cut";
                                                let clip_class = {
                                                    let mut c = format!("nle-clip {kind_class}");
                                                    if lifted {
                                                        c.push_str(" lifted");
                                                    }
                                                    if clip.disabled {
                                                        c.push_str(" dim");
                                                    }
                                                    if selected {
                                                        c.push_str(" on");
                                                    }
                                                    if mix {
                                                        c.push_str(" mix");
                                                    }
                                                    if !clip.graphic.is_empty() {
                                                        c.push_str(" gfx-clip");
                                                    }
                                                    if clip_uploading {
                                                        c.push_str(" uploading");
                                                    }
                                                    c
                                                };
                                                let tiles = film_tiles(width);
                                                let clip_id = clip.id.clone();
                                                let trim_in_id = clip_id.clone();
                                                let trim_out_id = clip_id.clone();
                                                let from_track = lane_id.clone();
                                                let clip_start = clip.start;
                                                let clip_dur = clip.duration;
                                                let grab_name = name.clone();
                                                let grab_url = url.clone();
                                                rsx! {
                                                    div {
                                                        class: "{clip_class}",
                                                        style: "left: {left}px; width: {width}px",
                                                        title: if mix {
                                                            format!("{name} · {}", clip.transition)
                                                        } else {
                                                            name.clone()
                                                        },
                                                        onmousedown: move |evt| {
                                                            let edge_x = evt.element_coordinates().x;
                                                            let edge_w = (clip_dur * *pps.peek()).max(24.0);
                                                            if edge_x <= 12.0 || edge_x >= edge_w - 12.0 {
                                                                evt.stop_propagation();
                                                                evt.prevent_default();
                                                                studio::hold_film(true);
                                                                suppress_seek.set(true);
                                                                selected_clip.set(Some(clip_id.clone()));
                                                                trim.set(Some((
                                                                    clip_id.clone(),
                                                                    edge_x >= edge_w - 12.0,
                                                                )));
                                                                return;
                                                            }
                                                            evt.stop_propagation();
                                                            selected_clip.set(Some(clip_id.clone()));
                                                            target_track.set(from_track.clone());
                                                            let at = clip_start + evt.element_coordinates().x / *pps.peek();
                                                            if *edit_tool.peek() == EditTool::Razor {
                                                                let _ = tools::split_at(save, from_track.as_str(), at);
                                                                return;
                                                            }
                                                            drag.set(Some(DragSession {
                                                                source: DragSource::Clip {
                                                                    clip_id: clip_id.clone(),
                                                                },
                                                                grab: evt.element_coordinates().x / *pps.peek(),
                                                                at: clip_start,
                                                                track_id: from_track.clone(),
                                                                duration: clip_dur,
                                                                name: grab_name.clone(),
                                                                url: grab_url.clone(),
                                                                media_kind,
                                                                origin_x: evt.client_coordinates().x,
                                                                origin_y: evt.client_coordinates().y,
                                                                client_x: evt.client_coordinates().x,
                                                                client_y: evt.client_coordinates().y,
                                                                moved: false,
                                                                over_timeline: false,
                                                            }));
                                                        },
                                                        if is_audio {
                                                            studio::Waveform { url: url.clone(), bars: (tiles as usize * 5).max(12), seed: name.clone() }
                                                        } else if is_image && !url.is_empty() {
                                                            div { class: "nle-strip",
                                                                img { src: "{url}", alt: "" }
                                                            }
                                                        } else if is_video {
                                                            studio::Filmstrip {
                                                                key: "{clip_id}",
                                                                url: url.clone(),
                                                                media_id: clip.media_id.clone(),
                                                                tiles,
                                                                source_in: clip.source_in,
                                                                source_out: clip.source_in
                                                                    + clip.duration
                                                                        * if clip.speed.is_finite() && clip.speed > 0.0 {
                                                                            clip.speed
                                                                        } else {
                                                                            1.0
                                                                        },
                                                            }
                                                        } else {
                                                            div { class: "nle-strip" }
                                                        }
                                                        span {
                                                            class: "nle-name",
                                                            if clip_uploading { "Saving..." } else { "{name}" }
                                                        }
                                                        div {
                                                            class: "nle-trim in",
                                                            title: "Trim the start",
                                                            onpointerdown: move |evt| {
                                                                evt.stop_propagation();
                                                                capture_pointer(&evt);
                                                                studio::hold_film(true);
                                                                suppress_seek.set(true);
                                                                selected_clip.set(Some(trim_in_id.clone()));
                                                                trim.set(Some((trim_in_id.clone(), false)));
                                                            },
                                                            onmousedown: move |evt| {
                                                                evt.stop_propagation();
                                                                evt.prevent_default();
                                                            },
                                                            onpointermove: move |evt| {
                                                                drag_trim(
                                                                    tracks,
                                                                    library,
                                                                    *pps.peek(),
                                                                    *edit_tool.peek() == ToolId::Ripple,
                                                                    trim,
                                                                    evt.client_coordinates().x,
                                                                    evt.client_coordinates().y,
                                                                );
                                                            },
                                                            onpointerup: move |_| {
                                                                commit_trim(save, tracks, trim);
                                                            },
                                                            onpointercancel: move |_| {
                                                                commit_trim(save, tracks, trim);
                                                            },
                                                        }
                                                        div {
                                                            class: "nle-trim out",
                                                            title: "Trim the end",
                                                            onpointerdown: move |evt| {
                                                                evt.stop_propagation();
                                                                capture_pointer(&evt);
                                                                studio::hold_film(true);
                                                                suppress_seek.set(true);
                                                                selected_clip.set(Some(trim_out_id.clone()));
                                                                trim.set(Some((trim_out_id.clone(), true)));
                                                            },
                                                            onmousedown: move |evt| {
                                                                evt.stop_propagation();
                                                                evt.prevent_default();
                                                            },
                                                            onpointermove: move |evt| {
                                                                drag_trim(
                                                                    tracks,
                                                                    library,
                                                                    *pps.peek(),
                                                                    *edit_tool.peek() == ToolId::Ripple,
                                                                    trim,
                                                                    evt.client_coordinates().x,
                                                                    evt.client_coordinates().y,
                                                                );
                                                            },
                                                            onpointerup: move |_| {
                                                                commit_trim(save, tracks, trim);
                                                            },
                                                            onpointercancel: move |_| {
                                                                commit_trim(save, tracks, trim);
                                                            },
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        if let Some((at, dur, ref ghost_name)) = ghost {
                                            {
                                                let left = at * *pps.read();
                                                let width = (dur * *pps.read()).max(24.0);
                                                let label = format!(
                                                    "{}  {}",
                                                    ghost_name,
                                                    format_clock(dur)
                                                );
                                                rsx! {
                                                    div {
                                                        class: "drop-caret",
                                                        style: "left: {left}px",
                                                    }
                                                    div {
                                                        class: "nle-clip {kind_class} ghost",
                                                        style: "left: {left}px; width: {width}px",
                                                        span { class: "nle-name", "{label}" }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            }
        }
    }
}

fn confirm_leave(held: Signal<Vec<HeldImport>>) -> bool {
    if held.peek().is_empty() {
        return true;
    }
    let n = held.peek().len();
    let msg = format!(
        "{n} imported file(s) are only in this tab. Leave and a refresh will not have them."
    );
    web_sys::window()
        .and_then(|w| w.confirm_with_message(&msg).ok())
        .unwrap_or(false)
}

/// Put `item` back on the bin when a stale refresh removed it or cleared its blob URL.
#[cfg(target_arch = "wasm32")]
fn ensure_library_item(
    project_id: &str,
    library: Signal<Vec<MediaItem>>,
    active: Signal<Option<String>>,
    item: &MediaItem,
) {
    pin_media(project_id, item);
    if page_project_id().is_some_and(|page| page != project_id) {
        return;
    }
    merge_into_library(project_id, library, active, std::slice::from_ref(item));
}

#[cfg(target_arch = "wasm32")]
fn still_on_project(project_id: &str) -> bool {
    page_project_id().is_none_or(|page| page == project_id)
}

/// The save failed after the user left this project. Remember it on that project's snap.
#[cfg(target_arch = "wasm32")]
fn remember_failed_hold(project_id: &str, row: HeldImport) {
    if is_registered(project_id, &row.id) {
        return;
    }
    WORKSPACES.with(|slot| {
        let mut map = slot.borrow_mut();
        let Some(existing) = map.get_mut(project_id) else {
            return;
        };
        if existing.snap.held.iter().any(|item| item.id == row.id) {
            return;
        }
        existing.snap.held.push(row);
    });
}

fn save_held_imports(
    mut held: Signal<Vec<HeldImport>>,
    uploading: Signal<Vec<String>>,
    save: WorkspaceSave,
) {
    let batch = held.peek().clone();
    if batch.is_empty() {
        return;
    }
    let pid = page_project_id().unwrap_or_else(|| save.project_id.peek().clone());
    spawn(async move {
        for file in batch {
            if is_registered(&pid, &file.id) {
                held.write().retain(|row| row.id != file.id);
                forget_import(&file.id);
                continue;
            }
            begin_upload(uploading, &file.id);
            let result = save_one_import(&pid, &file).await;
            if result.is_ok() {
                held.write().retain(|row| row.id != file.id);
                forget_import(&file.id);
                note_registered(&pid, &file.id);
            }
            note_upload(uploading, &file.id, &file.name, result);
        }
    });
}

async fn import_browser_file(
    file: FileData,
    save: WorkspaceSave,
    mut library: Signal<Vec<MediaItem>>,
    mut active: Signal<Option<String>>,
    mut held: Signal<Vec<HeldImport>>,
    uploading: Signal<Vec<String>>,
) {
    let name = file.name();
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (file, save, library, active, held, uploading, name);
    }
    #[cfg(target_arch = "wasm32")]
    {
        let Some(web_file) = file.inner().downcast_ref::<web_sys::File>().cloned() else {
            show_toast().error(format!("Could not read {name}"));
            return;
        };
        if web_file.size() <= 0.0 {
            show_toast().error(format!("{name} is empty"));
            return;
        }
        let ctype = MediaKind::mime(&name).to_string();
        let id = uuid::Uuid::now_v7().to_string();
        let Some(item) = item_from_blob(name.clone(), &web_file, id.clone()) else {
            show_toast().error(format!("Could not open {name}"));
            return;
        };
        if matches!(item.kind, MediaKind::Video | MediaKind::Image) && active.peek().is_none() {
            active.set(Some(item.url.clone()));
        }
        let visible = item.clone();
        let pid = save.project_id.peek().clone();
        library.write().push(visible.clone());
        // Pin before the await. A refresh that finishes during the save must put this tile back.
        pin_media(&pid, &visible);
        remember_import(&id, &web_file);
        begin_upload(uploading, &id);
        let kept = api::keep_imported_blob(&pid, &id, &name, &ctype, &web_file).await;
        let on_project = still_on_project(&pid);
        if on_project {
            ensure_library_item(&pid, library, active, &visible);
        }
        if let Err(err) = kept {
            let failed = HeldImport {
                id: id.clone(),
                name: name.clone(),
                content_type: ctype.clone(),
            };
            if on_project {
                if !held.peek().iter().any(|row| row.id == id) {
                    held.write().push(failed);
                }
            } else {
                remember_failed_hold(&pid, failed);
            }
            note_upload(uploading, &id, &name, Err(err));
            return;
        }
        if on_project {
            held.write().retain(|row| row.id != id);
        }
        forget_import(&id);
        note_registered(&pid, &id);
        // Speech can take minutes. The file is already in this browser, so the overlay stops here.
        end_upload(uploading, &id);
        show_toast().success(format!("{name} saved in this browser"));
        queue_import_followup(SpeechJob {
            project_id: pid,
            media_id: id,
            name,
            content_type: ctype,
            file: web_file,
            save,
            library,
            active,
            item: visible,
        });
    }
}

#[cfg(target_arch = "wasm32")]
struct SpeechJob {
    project_id: String,
    media_id: String,
    name: String,
    content_type: String,
    file: web_sys::File,
    save: WorkspaceSave,
    library: Signal<Vec<MediaItem>>,
    active: Signal<Option<String>>,
    item: MediaItem,
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    static SPEECH_JOBS: std::cell::RefCell<Vec<SpeechJob>> = std::cell::RefCell::new(Vec::new());
    static SPEECH_BUSY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Picture size and speech run one file at a time. Several at once exhaust the tab.
#[cfg(target_arch = "wasm32")]
fn queue_import_followup(job: SpeechJob) {
    SPEECH_JOBS.with(|slot| slot.borrow_mut().push(job));
    pump_speech();
}

#[cfg(target_arch = "wasm32")]
fn pump_speech() {
    if SPEECH_BUSY.with(std::cell::Cell::get) {
        return;
    }
    let job = SPEECH_JOBS.with(|slot| {
        let mut jobs = slot.borrow_mut();
        if jobs.is_empty() {
            None
        } else {
            Some(jobs.remove(0))
        }
    });
    let Some(job) = job else {
        return;
    };
    SPEECH_BUSY.with(|slot| slot.set(true));
    spawn(async move {
        struct Release;
        impl Drop for Release {
            fn drop(&mut self) {
                SPEECH_BUSY.with(|slot| slot.set(false));
                pump_speech();
            }
        }
        let _release = Release;
        run_speech_job(job).await;
    });
}

#[cfg(target_arch = "wasm32")]
async fn run_speech_job(job: SpeechJob) {
    if job.content_type.starts_with("video/") && is_camera_name(&job.name) {
        if let Some((width, height)) = media::picture_size(&job.file).await {
            store_frame(job.save, &job.media_id, width, height, false);
        }
    }
    let speech = api::transcribe_blob(
        &job.project_id,
        &job.media_id,
        &job.content_type,
        &job.file,
    )
    .await;
    ensure_library_item(&job.project_id, job.library, job.active, &job.item);
    if let Err(err) = speech {
        show_toast().error(format!(
            "{} is in this browser. Speech was not read — {err}",
            job.name
        ));
    }
}

async fn save_one_import(pid: &str, file: &HeldImport) -> Result<(), String> {
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(web_file) = held_file(&file.id) {
            return api::store_imported_blob(
                pid,
                &file.id,
                &file.name,
                &file.content_type,
                &web_file,
            )
            .await;
        }
        let blob = hold::load_blob(pid, &file.id).await?;
        return api::store_imported_blob(pid, &file.id, &file.name, &file.content_type, &blob)
            .await;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (pid, file);
        Err("import runs in the browser".into())
    }
}

fn page_project_id() -> Option<String> {
    let path = web_sys::window()?.location().pathname().ok()?;
    let id = path.trim_matches('/').strip_suffix("/workspace")?;
    let id = id.trim_matches('/');
    uuid::Uuid::parse_str(id).ok()?;
    Some(id.to_string())
}

fn seek_to_time(clock: Clock, time: f64) {
    crate::media::seek_to(clock, time);
}

fn add_track(
    tracks: &mut Signal<Vec<EditorTrack>>,
    target_track: &mut Signal<String>,
    kind: TrackKindUi,
    save: WorkspaceSave,
) {
    add_track_after(tracks, target_track, kind, "", save);
}

fn add_track_after(
    tracks: &mut Signal<Vec<EditorTrack>>,
    target_track: &mut Signal<String>,
    kind: TrackKindUi,
    after_id: &str,
    save: WorkspaceSave,
) {
    let name = next_track_name(&tracks.read(), kind);
    let id = uuid::Uuid::now_v7().to_string();
    let track = EditorTrack {
        id: id.clone(),
        name,
        kind,
        muted: false,
        hidden: false,
        clips: Vec::new(),
    };
    let mut list = tracks.write();
    let fallback = match kind {
        TrackKindUi::Video | TrackKindUi::Caption => 0,
        TrackKindUi::Audio => list.len(),
    };
    let at = list
        .iter()
        .position(|t| t.id == after_id)
        .map(|i| i + 1)
        .unwrap_or(fallback);
    let at = at.min(list.len());
    list.insert(at, track);
    drop(list);
    target_track.set(id);
    persist(save);
}

#[component]
fn AiSidebar(
    draft: Signal<String>,
    mut messages: Signal<Vec<ChatMsg>>,
    ai_open: Signal<bool>,
    ai_width: Signal<f64>,
    mut ai_drag: Signal<Option<(f64, f64)>>,
) -> Element {
    let save = use_context::<WorkspaceSave>();
    let clock = use_context::<Clock>();
    let target_track = use_context::<CtxTargetTrack>().0;
    let library = use_context::<Signal<Vec<MediaItem>>>();
    let active = use_context::<CtxActive>().0;
    let held = use_context::<CtxHeld>().0;
    let uploading = use_context::<CtxUploading>().0;
    let mut picker_open = use_signal(|| false);
    let mut providers = use_signal(Vec::<api::AiProvider>::new);
    let mut provider_id = use_signal(|| "xai".to_string());
    let mut model_id = use_signal(|| "grok-4.6".to_string());
    let mut model_name = use_signal(|| "Grok 4.6".to_string());
    let busy = use_signal(|| false);
    let mut chats = use_signal(Vec::<api::ChatSummary>::new);
    let mut chat_id = use_signal(|| None::<String>);
    let mut chat_menu = use_signal(|| false);
    let open_gen = use_signal(|| 0u32);

    // A schema dump already stored as a reply collapses on the next render,
    // including a chat that was open before this build loaded.
    use_effect(move || {
        let current = messages.read().clone();
        let mut next = current.clone();
        fold_messages(&mut next);
        if next != current {
            messages.set(next);
        }
    });
    // The refresh above can still clear the signal after Send. busy stays
    // true, so Pause is showing and the suggestions come back. Put the sent
    // line back on this project and take it out of the box.
    use_effect(move || {
        let _ = messages.read().len();
        let running = *busy.read();
        let _ = draft.read().len();
        if !running {
            return;
        }
        let pid = save.project_id.peek().clone();
        let missing = live_turn_missing(&messages.peek(), &pid);
        if missing {
            splice_live_turn(&mut messages.write(), &pid);
        }
        let typed = draft.peek().clone();
        if composer_echoes_turn(&typed, &pid) {
            draft.set(String::new());
        }
    });

    let mut worked_tick = use_signal(|| 0u32);
    use_effect(move || {
        let _ = *worked_tick.read();
        if !*busy.read() {
            return;
        }
        spawn(async move {
            gloo_timers::future::TimeoutFuture::new(1000).await;
            if *busy.peek() {
                let n = *worked_tick.peek();
                worked_tick.set(n.wrapping_add(1));
            }
        });
    });

    use_future(move || async move {
        if let Ok(list) = api::list_ai_providers().await {
            if let Some(first) = list.iter().find(|p| p.connected).or_else(|| list.first()) {
                if let Some(model) = first.models.first() {
                    provider_id.set(first.id.clone());
                    model_id.set(model.id.clone());
                    model_name.set(model.name.clone());
                }
            }
            providers.set(list);
        }
    });
    let project_for_chats = save.project_id;
    let mut listed_for = use_signal(String::new);
    use_effect(move || {
        let n = messages.read().len();
        if n == 0 {
            return;
        }
        // set_scroll_top runs JS. Doing that inside the effect re-enters the
        // browser while Dioxus is still diffing, which panics the element arena.
        gloo_timers::callback::Timeout::new(0, scroll_thread).forget();
    });
    use_effect(move || {
        let pid = project_for_chats.read().clone();
        if pid.is_empty() || listed_for.peek().as_str() == pid.as_str() {
            return;
        }
        let switching = !listed_for.peek().is_empty();
        listed_for.set(pid.clone());
        if switching {
            chats.set(Vec::new());
            chat_id.set(None);
        }
        if !auth::is_signed_in() {
            return;
        }
        spawn(async move {
            let Ok(remote) = api::list_chats(&pid).await else {
                return;
            };
            if project_for_chats.peek().as_str() != pid || listed_for.peek().as_str() != pid.as_str()
            {
                return;
            }
            let local = chats.peek().clone();
            chats.set(merge_remote_chats(remote, &local));
            if chat_id.peek().is_some() || !messages.peek().is_empty() {
                return;
            }
            let Some(first) = chats.peek().first().cloned() else {
                return;
            };
            let before_len = messages.peek().len();
            let before_turn = api::chat_generation();
            chat_id.set(Some(first.id.clone()));
            if let Ok(detail) = api::get_chat(&pid, &first.id).await {
                if project_for_chats.peek().as_str() != pid
                    || chat_id.peek().as_deref() != Some(first.id.as_str())
                {
                    return;
                }
                // The list request started on an empty thread. A line typed
                // since then has to stay; the stored copy is still empty.
                if stored_load_is_stale(
                    *busy.peek(),
                    before_len,
                    messages.peek().len(),
                    before_turn,
                    api::chat_generation(),
                ) {
                    return;
                }
                apply_stored(&mut messages, detail.messages);
            }
        });
    });
    let suggestions = [
        ("Split at playhead", "split"),
        ("Merge clips", "merge"),
        ("Delete clip", "delete"),
        (
            "Remove filler words",
            "Remove all filler words like um, uh, like from the transcript",
        ),
        ("Add subtitles", "Generate subtitles for the current video"),
        (
            "Remove silences",
            "Detect and remove long silences from the timeline",
        ),
        (
            "Improve audio",
            "Apply noise reduction and normalize audio levels",
        ),
        (
            "Generate thumbnail",
            "Create a thumbnail image for this video",
        ),
        (
            "Auto color grade",
            "Apply automatic color grading to the video clips",
        ),
    ];

    let open = *ai_open.read();
    let panel_w = if open { *ai_width.read() } else { 0.0 };
    let current_chat = chat_id.read().clone();
    let saved_chats = chats.read().clone();
    let chat_title = saved_chats
        .iter()
        .find(|row| Some(&row.id) == current_chat.as_ref())
        .map(|row| row.title.clone())
        .unwrap_or_else(|| "New chat".into());
    let menu_open = *chat_menu.read();
    let running = *busy.read();
    let _worked_tick = *worked_tick.read();
    let project_now = save.project_id.peek().clone();
    let mut folded = messages.read().clone();
    fold_messages(&mut folded);
    if running {
        splice_live_turn(&mut folded, &project_now);
    }
    let chat_empty = folded.is_empty();
    let typed = draft.read().clone();
    let composer = if composer_echoes_turn(&typed, &project_now) {
        String::new()
    } else {
        typed
    };
    rsx! {
        aside {
            class: if open { "ai" } else { "ai collapsed" },
            style: "width: {panel_w}px; min-width: {panel_w}px; max-width: {panel_w}px; flex: 0 0 {panel_w}px;",
            if open {
                div {
                    class: "ai-resize",
                    onmousedown: move |evt| {
                        evt.prevent_default();
                        ai_drag.set(Some((evt.client_coordinates().x, *ai_width.read())));
                    },
                }
            }
            if open {
                div { class: "ai-head",
                    div { class: "ai-head-actions ai-copy",
                        button {
                            class: "chat-switch",
                            title: "Saved chats",
                            onclick: move |_| {
                                let next = !*chat_menu.peek();
                                chat_menu.set(next);
                            },
                            span { "{chat_title}" }
                            span { class: "chat-caret", if menu_open { "▴" } else { "▾" } }
                        }
                        button {
                            class: "chat-new",
                            title: "New chat",
                            disabled: running,
                            onclick: move |_| start_new_chat(
                                chat_id,
                                chats,
                                messages,
                                draft,
                                chat_menu,
                                busy,
                                save.project_id.peek().clone(),
                            ),
                            "+"
                        }
                    }
                }
            }
            if open && menu_open {
                div { class: "chat-menu",
                    if saved_chats.is_empty() {
                        p { class: "chat-menu-empty", "No saved chats yet" }
                    }
                    for chat in saved_chats.iter() {
                        {
                            let id = chat.id.clone();
                            let selected = current_chat.as_deref() == Some(id.as_str());
                            let pid = save.project_id.peek().clone();
                            let label = chat_menu_label(chat, &saved_chats);
                            rsx! {
                                button {
                                    class: if selected { "on" } else { "" },
                                    disabled: running,
                                    onclick: move |_| open_saved_chat(
                                        id.clone(),
                                        chat_id,
                                        chats,
                                        messages,
                                        draft,
                                        chat_menu,
                                        open_gen,
                                        busy,
                                        pid.clone(),
                                    ),
                                    "{label}"
                                }
                            }
                        }
                    }
                }
            }
            if open {
            div { class: "ai-body", id: "ai-thread",
                if chat_empty {
                    div { class: "ai-empty",
                        IconSpark {}
                        h3 { "AI Studio" }
                        p { "Brainstorm ideas, plan your video, or run AI commands." }
                        div { class: "suggest",
                            for (label, prompt) in suggestions {
                                button {
                                    r#type: "button",
                                    onclick: move |_| {
                                        draft.set(prompt.to_string());
                                        focus_prompt();
                                    },
                                    "{label}"
                                }
                            }
                        }
                    }
                } else {
                    div { class: "msgs",
                        for row in chat_view(&folded, running, if running { live_worked_ms() } else { 0 }) {
                            match row {
                                ChatView::Msg(i) => {
                                    let msg = folded[i].clone();
                                    rsx! { { render_chat_msg(messages, msg) } }
                                }
                                ChatView::Worked { nth, open, ms } => {
                                    let label = worked_label(ms);
                                    rsx! { { render_worked_row(messages, nth, open, label) } }
                                }
                                ChatView::Tools { start, end } => {
                                    let group = folded[start..end].to_vec();
                                    rsx! { { render_tool_group(messages, group) } }
                                }
                            }
                        }
                    }
                }
            }
            div { class: "ai-foot",
                if *picker_open.read() {
                    ProviderPicker {
                        providers,
                        provider_id,
                        model_id,
                        model_name,
                        picker_open,
                    }
                }
                div { class: "ai-box",
                    textarea {
                        id: "ai-prompt",
                        rows: "2",
                        placeholder: "What should we do next?",
                        value: "{composer}",
                        oninput: move |e| {
                            let next = e.value();
                            if composer_echoes_turn(&next, &save.project_id.peek()) {
                                clear_prompt_element();
                                return;
                            }
                            draft.set(next);
                        },
                        onkeydown: move |e| {
                            if e.key() == Key::Enter && !e.modifiers().shift() {
                                e.prevent_default();
                                queue_send_prompt(
                                    draft,
                                    messages,
                                    save,
                                    clock,
                                    target_track,
                                    provider_id,
                                    model_id,
                                    busy,
                                    chat_id,
                                    chats,
                                    library,
                                    active,
                                    held,
                                    uploading,
                                    chat_menu,
                                );
                            }
                        },
                    }
                    div { class: "ai-box-bar",
                        button {
                            class: "model-chip",
                            title: "Choose provider and model",
                            onclick: move |_| {
                                let next = !*picker_open.peek();
                                picker_open.set(next);
                            },
                            span { class: "model-chip-name", "{model_name}" }
                            span { class: "model-chip-caret", if *picker_open.read() { "▴" } else { "▾" } }
                        }
                        if *busy.read() {
                            button {
                                r#type: "button",
                                class: "send pause",
                                title: "Stop the chat",
                                onclick: move |_| stop_chat(messages, busy, draft),
                                IconPause {}
                            }
                        } else {
                            button {
                                r#type: "button",
                                class: if composer.trim().is_empty() { "send is-empty" } else { "send" },
                                aria_disabled: composer.trim().is_empty(),
                                onclick: move |_| queue_send_prompt(
                                    draft,
                                    messages,
                                    save,
                                    clock,
                                    target_track,
                                    provider_id,
                                    model_id,
                                    busy,
                                    chat_id,
                                    chats,
                                    library,
                                    active,
                                    held,
                                    uploading,
                                    chat_menu,
                                ),
                                IconSend {}
                            }
                        }
                    }
                }
            }
            }
        }
    }
}

#[component]
fn ProviderPicker(
    providers: Signal<Vec<api::AiProvider>>,
    mut provider_id: Signal<String>,
    mut model_id: Signal<String>,
    mut model_name: Signal<String>,
    mut picker_open: Signal<bool>,
) -> Element {
    let active = provider_id.read().clone();
    rsx! {
        div { class: "model-modal",
            div { class: "model-modal-side",
                for p in providers.read().iter() {
                    {
                        let id = p.id.clone();
                        let name = p.name.clone();
                        let connected = p.connected;
                        let first = p.models.first().cloned();
                        rsx! {
                            button {
                                class: if active == id { "provider-tab on" } else { "provider-tab" },
                                title: if connected { name.clone() } else { p.login_hint.clone() },
                                onclick: move |_| {
                                    provider_id.set(id.clone());
                                    if let Some(m) = &first {
                                        model_id.set(m.id.clone());
                                        model_name.set(m.name.clone());
                                    }
                                },
                                span { class: "provider-dot",
                                    if connected { "●" } else { "○" }
                                }
                                span { "{name}" }
                            }
                        }
                    }
                }
            }
            div { class: "model-modal-list",
                for p in providers.read().iter() {
                    if p.id == active {
                        if !p.connected {
                            div { class: "auth-banner",
                                "{p.login_hint}"
                            }
                        }
                        for m in p.models.iter() {
                            {
                                let mid = m.id.clone();
                                let mname = m.name.clone();
                                let pid = p.id.clone();
                                let selected = *model_id.read() == mid;
                                rsx! {
                                    button {
                                        class: if selected { "model-option on" } else { "model-option" },
                                        onclick: move |_| {
                                            provider_id.set(pid.clone());
                                            model_id.set(mid.clone());
                                            model_name.set(mname.clone());
                                            picker_open.set(false);
                                        },
                                        span { "{m.name}" }
                                        span { class: "model-id", "{m.id}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn apply_chat_event(mut messages: Signal<Vec<ChatMsg>>, ev: api::ChatStreamEvent) {
    match ev.kind.as_str() {
        "status" => {
            if ev.text.trim().is_empty() || is_acp_log(&ev.text) {
                return;
            }
            let mut list = messages.write();
            if let Some(last) = list.iter_mut().rev().find(|m| m.role == ChatRole::Status) {
                last.text = ev.text;
            } else {
                list.push(ChatMsg::status(ev.text));
            }
        }
        "text" => {
            if is_acp_log(&ev.text) {
                return;
            }
            let mut list = messages.write();
            append_model_chunk(&mut list, &ev.text, false);
        }
        "tool" => upsert_tool(messages, ev),
        "error" => {
            if ev.text.trim().is_empty() {
                return;
            }
            clear_status(messages);
            messages
                .write()
                .push(ChatMsg::bot(format!("Error: {}", ev.text)));
        }
        "thought" => {
            if is_acp_log(&ev.text) {
                return;
            }
            let mut list = messages.write();
            append_model_chunk(&mut list, &ev.text, true);
        }
        "note" => {
            if !ev.text.is_empty() && !is_acp_log(&ev.text) {
                messages.write().push(ChatMsg::status(ev.text));
            }
        }
        "done" => clear_status(messages),
        _ => {}
    }
}

fn upsert_tool(mut messages: Signal<Vec<ChatMsg>>, ev: api::ChatStreamEvent) {
    let args = compact_json(&ev.args);
    let result = ev.result.clone().unwrap_or_default();
    let id = if ev.id.is_empty() {
        ev.name.clone()
    } else {
        ev.id.clone()
    };
    let mut list = messages.write();
    if let Some(existing) = list.iter_mut().rev().find(|m| {
        m.role == ChatRole::Tool
            && (m.tool_id == id || (id.is_empty() && m.tool_name == short_tool_name(&ev.name)))
    }) {
        existing.tool_status = ev.status;
        if !ev.name.is_empty() {
            existing.tool_name = short_tool_name(&ev.name);
        }
        if !args.is_empty() {
            existing.tool_args = args;
        }
        if !result.is_empty() {
            existing.tool_result = result;
        }
    } else {
        let open = joining_first_tool_group(&list);
        list.push(ChatMsg {
            role: ChatRole::Tool,
            text: ev.name.clone(),
            tool_id: id,
            tool_name: short_tool_name(&ev.name),
            tool_status: if ev.status.is_empty() {
                "pending".into()
            } else {
                ev.status
            },
            tool_args: args,
            tool_result: result,
            open,
        });
    }
    for msg in list.iter_mut() {
        if msg.role == ChatRole::Thought {
            msg.open = false;
        }
    }
}

enum ChatRow {
    Msg(usize),
    Tools { start: usize, end: usize },
}

fn chat_rows(msgs: &[ChatMsg]) -> Vec<ChatRow> {
    let mut rows = Vec::new();
    let mut i = 0;
    while i < msgs.len() {
        if msgs[i].role == ChatRole::Tool {
            let start = i;
            while i < msgs.len() && msgs[i].role == ChatRole::Tool {
                i += 1;
            }
            rows.push(ChatRow::Tools { start, end: i });
        } else {
            rows.push(ChatRow::Msg(i));
            i += 1;
        }
    }
    rows
}

enum ChatView {
    Msg(usize),
    Worked { nth: usize, open: bool, ms: u64 },
    Tools { start: usize, end: usize },
}

/// One row per message, plus a "Worked for" disclosure after each timed turn.
/// Collapsing that row hides the turn's command groups. The prose stays.
fn chat_view(msgs: &[ChatMsg], running: bool, live_ms: u64) -> Vec<ChatView> {
    let last_user = msgs.iter().rposition(|m| m.role == ChatRole::User);
    let mut out = Vec::new();
    let mut nth = 0usize;
    for row in chat_rows(msgs) {
        match row {
            ChatRow::Msg(i) => {
                out.push(ChatView::Msg(i));
                if msgs[i].role == ChatRole::User {
                    let live = running && last_user == Some(i);
                    let stored = parse_worked_ms(&msgs[i].tool_result);
                    if live || stored.is_some() {
                        let ms = if live { live_ms } else { stored.unwrap_or(0) };
                        out.push(ChatView::Worked {
                            nth,
                            open: msgs[i].open,
                            ms,
                        });
                    }
                    nth += 1;
                }
            }
            ChatRow::Tools { start, end } => {
                let hidden = user_before(msgs, start).is_some_and(|idx| {
                    let user = &msgs[idx];
                    let stamped = parse_worked_ms(&user.tool_result).is_some();
                    let live = running && last_user == Some(idx);
                    (stamped || live) && !user.open
                });
                if !hidden {
                    out.push(ChatView::Tools { start, end });
                }
            }
        }
    }
    out
}

fn user_before(msgs: &[ChatMsg], at: usize) -> Option<usize> {
    msgs.get(..at)
        .and_then(|prior| prior.iter().rposition(|msg| msg.role == ChatRole::User))
}

fn render_worked_row(
    messages: Signal<Vec<ChatMsg>>,
    nth: usize,
    open: bool,
    label: String,
) -> Element {
    rsx! {
        button {
            class: "worked",
            title: "Show the commands from this turn",
            onclick: move |_| toggle_worked(messages, nth),
            span { "{label}" }
            span { class: "worked-caret", if open { "▾" } else { "▸" } }
        }
    }
}

fn toggle_worked(mut messages: Signal<Vec<ChatMsg>>, nth: usize) {
    let mut seen = 0usize;
    for msg in messages.write().iter_mut() {
        if msg.role != ChatRole::User {
            continue;
        }
        if seen == nth {
            msg.open = !msg.open;
            return;
        }
        seen += 1;
    }
}

fn render_chat_msg(messages: Signal<Vec<ChatMsg>>, msg: ChatMsg) -> Element {
    match msg.role {
        ChatRole::User => rsx! { div { class: "bubble user", "{msg.text}" } },
        ChatRole::Bot => {
            let (trace, visible) = detach_trace(&msg.text);
            let hide =
                !trace.is_empty() || looks_like_tool_trace(&msg.text) || leak_marker(&msg.text);
            let visible = if looks_like_tool_trace(&visible) || leak_marker(&visible) {
                String::new()
            } else {
                visible
            };
            if !hide {
                rsx! { div { class: "bubble bot", "{msg.text}" } }
            } else {
                let visible = visible.clone();
                rsx! {
                    div { class: "bubble thought",
                        button { class: "tool-head",
                            span { class: "tool-caret", "▸" }
                            span { "Thinking" }
                        }
                    }
                    if !visible.is_empty() {
                        div { class: "bubble bot", "{visible}" }
                    }
                }
            }
        }
        ChatRole::Status => rsx! { div { class: "bubble status", "{msg.text}" } },
        ChatRole::Thought => {
            let open = msg.open;
            let id = msg.tool_id.clone();
            let text = msg.text.clone();
            rsx! {
                div { class: "bubble thought",
                    button {
                        class: "tool-head",
                        onclick: move |_| toggle_msg(messages, &id),
                        span { class: "tool-caret", if open { "▾" } else { "▸" } }
                        span { "Thinking" }
                    }
                    if open {
                        div { class: "thought-body", "{text}" }
                    }
                }
            }
        }
        ChatRole::Tool => rsx! { div {} },
    }
}

fn render_tool_group(messages: Signal<Vec<ChatMsg>>, group: Vec<ChatMsg>) -> Element {
    let n = group.len();
    let active = group.iter().any(|m| m.tool_status == "pending");
    let open = group.iter().any(|m| m.open);
    let id = group.first().map(|m| m.tool_id.clone()).unwrap_or_default();
    let word = if n == 1 { "command" } else { "commands" };
    let label = if active {
        format!("Running {n} {word}")
    } else {
        format!("Ran {n} {word}")
    };
    let lines: Vec<(String, bool)> = group
        .iter()
        .map(|msg| {
            (
                command_line(&msg.tool_name, &msg.tool_args),
                msg.tool_status == "error",
            )
        })
        .collect();
    let show_lines = active || open;
    rsx! {
        div { class: "bubble tool",
            button {
                class: "tool-head",
                onclick: move |_| toggle_tool_group(messages, &id),
                span { class: "tool-mark", ">_" }
                span { "{label}" }
                span { class: "tool-caret end", if show_lines { "▾" } else { "▸" } }
            }
            if show_lines {
                for (line, failed) in lines {
                    div { class: "tool-line",
                        span { class: "tool-mark", ">" }
                        span { class: "tool-name", "{line}" }
                        if failed {
                            span { class: "tool-status error", "failed" }
                        }
                    }
                }
            }
        }
    }
}

/// One tool, written the way a person would say it. The raw name stays stored.
fn command_line(name: &str, args: &str) -> String {
    let detail = arg_detail(args);
    let verb = match name {
        "submit_edit" => "Cut the timeline",
        "revise_edit" => "Revised the cut",
        "watch" => "Watched a clip",
        "see" => "Looked at a frame",
        "list_gaps" => "Listed the gaps",
        "list_bin" | "list_media" => "Listed the bin",
        "list_timeline" => "Read the timeline",
        "list_cues" => "Read the transcript",
        "add_motion" => "Placed a graphic",
        "edit_skill" => "Opened an edit page",
        "generate_broll" => "Generated a picture",
        "export" => "Exported the video",
        "add_captions" | "generate_captions" => "Wrote the captions",
        "set_grade" => "Set the grade",
        "trim" | "trim_end" => "Trimmed a clip",
        "place_clip" | "insert" | "overwrite" => "Placed a clip",
        "cover" => "Covered a jump",
        "find_shots" => "Found the shots",
        "get_media" => "Read a file",
        "transcribe_media" => "Transcribed a file",
        other => return humanize_tool(other),
    };
    match detail {
        Some(bit) => format!("{verb}: \"{bit}\""),
        None => verb.to_string(),
    }
}

fn arg_detail(args: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(args).ok()?;
    for key in ["text", "prompt", "design", "page", "name", "title"] {
        let Some(bit) = value.get(key).and_then(|item| item.as_str()) else {
            continue;
        };
        let bit = bit.trim();
        if bit.is_empty() || bit.len() > 80 || bit.starts_with('{') {
            continue;
        }
        return Some(bit.to_string());
    }
    None
}

fn humanize_tool(name: &str) -> String {
    let words = name.replace('_', " ");
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => "Ran a command".into(),
    }
}

fn toggle_tool_group(mut messages: Signal<Vec<ChatMsg>>, id: &str) {
    let mut list = messages.write();
    let Some(idx) = list.iter().position(|m| m.tool_id == id) else {
        return;
    };
    let mut start = idx;
    while start > 0 && list[start - 1].role == ChatRole::Tool {
        start -= 1;
    }
    let mut end = idx;
    while end + 1 < list.len() && list[end + 1].role == ChatRole::Tool {
        end += 1;
    }
    let next = !list[idx].open;
    for msg in &mut list[start..=end] {
        msg.open = next;
    }
}

fn toggle_msg(mut messages: Signal<Vec<ChatMsg>>, id: &str) {
    if let Some(msg) = messages.write().iter_mut().find(|m| m.tool_id == id) {
        msg.open = !msg.open;
    }
}

fn short_tool_name(raw: &str) -> String {
    let s = raw.rsplit([':', '/', '@']).next().unwrap_or(raw);
    s.rsplit("__").next().unwrap_or(s).trim().to_string()
}

/// Grok CLI tracing that was forwarded into the chat. Not a reply.
fn is_acp_log(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("sampling.request")
        || lower.contains("cli-chat-proxy")
        || lower.contains("auth_prefix")
        || lower.contains("encrypted_content")
        || lower.contains("sse_chunk")
        || lower.contains("api_backend")
        || text.contains("[0m")
        || text.contains("[32m")
        || text.contains("[2m")
        || text.contains('\u{1b}')
}

fn looks_like_tool_trace(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return false;
    }
    if t.contains("function_call")
        || t.contains("tool_call")
        || t.contains("\"type\":\"function\"")
        || t.contains("\"type\": \"function\"")
        || t.contains("sessionUpdate")
        || t.contains("session/prompt")
        || t.contains("$schema")
        || t.contains("json-schema.org")
        || t.contains("\"parameters\"")
        || t.contains("\"input_schema\"")
        || t.contains("\"properties\"")
        || t.contains("scheduler_")
        || t.contains("Usage notes:")
        || t.contains("fire_immediately")
        || t.contains("main-agent")
        || t.contains("\\\"parameters\\\"")
        || t.contains("\\\"properties\\\"")
        || t.contains("\\\"$schema\\\"")
        || escaped_schema_dump(t)
    {
        return true;
    }
    let punct = t
        .chars()
        .filter(|c| matches!(c, '{' | '}' | '"' | '[' | ']' | ':'))
        .count();
    t.len() > 80 && punct * 4 > t.len()
}

/// The Grok tool list arrives as one long line with literal `\n` and quotes.
fn escaped_schema_dump(text: &str) -> bool {
    let breaks = text.matches("\\n").count();
    breaks >= 3
        && text.len() > 80
        && (text.contains('"') || text.contains('{') || text.contains("\\\""))
}

/// More of a schema that is already sitting in Thinking.
fn chunk_continues_trace(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return false;
    }
    if looks_like_tool_trace(t) {
        return true;
    }
    matches!(
        t.chars().next(),
        Some('{' | '}' | '"' | '[' | ']' | ',' | ':' | '\\')
    )
}

/// A schema dump belongs under Thinking. A spoken paragraph after it stays visible.
fn detach_trace(text: &str) -> (String, String) {
    let trimmed = text.trim();
    if !looks_like_tool_trace(trimmed) {
        return (String::new(), trimmed.to_string());
    }
    if let Some((trace, tail)) = split_spoken_tail(trimmed) {
        return (trace, tail);
    }
    (trimmed.to_string(), String::new())
}

fn split_spoken_tail(text: &str) -> Option<(String, String)> {
    for sep in ["\n\n", "\\n\\n"] {
        if let Some(idx) = text.rfind(sep) {
            let tail = text[idx + sep.len()..].trim();
            if spoken_reply_text(tail) {
                return Some((text[..idx].trim().to_string(), tail.to_string()));
            }
        }
    }
    if let Some(idx) = text.rfind('}') {
        let tail = text[idx + 1..].trim().trim_start_matches("\\n").trim();
        if spoken_reply_text(tail) {
            return Some((text[..=idx].trim().to_string(), tail.to_string()));
        }
    }
    let chars: Vec<char> = text.chars().collect();
    let mut i = chars.len();
    while i > 0 && !matches!(chars[i - 1], '{' | '}' | '"' | '[' | ']' | '\\' | '$' | ':') {
        i -= 1;
    }
    if i > 0 && i < chars.len() {
        let tail: String = chars[i..].iter().collect();
        let tail = tail.trim();
        if spoken_reply_text(tail) {
            let head: String = chars[..i].iter().collect();
            return Some((head.trim().to_string(), tail.to_string()));
        }
    }
    None
}

fn prose_paragraph(text: &str) -> bool {
    let words = text.split_whitespace().count();
    let punct = text
        .chars()
        .filter(|c| matches!(c, '{' | '}' | '"' | '[' | ']'))
        .count();
    words >= 6 && punct < 4 && !looks_like_tool_trace(text)
}

fn spoken_reply_text(text: &str) -> bool {
    prose_paragraph(text) && !leak_marker(text)
}

fn leak_marker(text: &str) -> bool {
    text.contains("$schema")
        || text.contains("json-schema")
        || text.contains("scheduler_")
        || text.contains("Usage notes")
        || text.contains("fire_immediately")
        || text.contains("main-agent")
        || text.contains("function_call")
        || text.contains("tool_call")
        || text.contains("stdout")
        || text.contains("recurring interval")
        || text.contains("fields replace")
        || text.contains("Interval format")
        || text.contains("Create-only")
        || text.contains("long-running script")
        || text.contains("Use this tool when")
        || text.contains("omitted ones")
        || text.contains("auto-expire")
        || text.contains("timeout_ms")
}

/// Short schema tokens stay inside Thinking. A full spoken sentence starts the reply.
fn chunk_stays_in_trace(thought: &str, chunk: &str) -> bool {
    looks_like_tool_trace(thought) && !spoken_reply_text(chunk)
}

fn bot_joins_trace(bot: &str, chunk: &str) -> bool {
    if spoken_reply_text(bot) {
        return false;
    }
    looks_like_tool_trace(bot)
        || looks_like_tool_trace(chunk)
        || looks_like_tool_trace(&format!("{bot}{chunk}"))
}

/// Append one streamed chunk, then hide any reply bubble that is a tool schema.
fn append_model_chunk(list: &mut Vec<ChatMsg>, chunk: &str, as_thought: bool) {
    let chunk = if as_thought {
        chunk.trim().to_string()
    } else {
        strip_tool_lines(chunk)
    };
    if chunk.is_empty() {
        return;
    }
    for msg in list.iter_mut() {
        if matches!(msg.role, ChatRole::Tool | ChatRole::Thought) {
            msg.open = false;
        }
    }
    let append = match list.last() {
        Some(last) if last.role == ChatRole::Thought => {
            as_thought || chunk_stays_in_trace(&last.text, &chunk)
        }
        Some(last) if last.role == ChatRole::Bot => {
            !as_thought || bot_joins_trace(&last.text, &chunk)
        }
        _ => false,
    };
    if append {
        list.last_mut().unwrap().text.push_str(&chunk);
    } else if as_thought {
        let n = list.len();
        list.push(closed_thought(chunk, n));
    } else {
        list.push(ChatMsg::bot(chunk));
    }
    fold_messages(list);
}

/// Every schema bubble becomes a collapsed Thinking row, not only the last one.
fn fold_messages(list: &mut Vec<ChatMsg>) {
    list.retain(|msg| {
        msg.role == ChatRole::User || msg.role == ChatRole::Tool || !is_acp_log(&msg.text)
    });
    let mut i = 0;
    while i < list.len() {
        if list[i].role != ChatRole::Bot {
            i += 1;
            continue;
        }
        let (trace, visible) = detach_trace(&list[i].text);
        if trace.is_empty() {
            i += 1;
            continue;
        }
        list[i] = closed_thought(trace, i);
        if !visible.trim().is_empty() {
            list.insert(i + 1, ChatMsg::bot(visible));
        }
        i += 1;
    }
    let mut i = 0;
    while i + 1 < list.len() {
        if list[i].role == ChatRole::Thought && list[i + 1].role == ChatRole::Thought {
            let text = std::mem::take(&mut list[i + 1].text);
            list[i].text.push_str(&text);
            list[i].open = false;
            list.remove(i + 1);
        } else {
            i += 1;
        }
    }
    absorb_trace_fragments(list);
    peel_thought_replies(list);
}

/// A tool row can split one schema dump into several bubbles. Pull those
/// pieces into the one Thinking row for this turn. A spoken reply stays put.
fn absorb_trace_fragments(list: &mut Vec<ChatMsg>) {
    let mut start = 0;
    while start < list.len() {
        if list[start].role == ChatRole::User {
            start += 1;
            continue;
        }
        let mut end = start;
        while end < list.len() && list[end].role != ChatRole::User {
            end += 1;
        }
        let span = &list[start..end];
        let has_schema = span.iter().any(|msg| {
            matches!(msg.role, ChatRole::Bot | ChatRole::Thought)
                && looks_like_tool_trace(&msg.text)
        });
        let pieces: Vec<usize> = (start..end)
            .filter(|&i| has_schema && should_fold_into_trace(&list[i]))
            .collect();
        if pieces.len() > 1 {
            let id = pieces
                .iter()
                .find_map(|&i| {
                    (list[i].role == ChatRole::Thought && !list[i].tool_id.is_empty())
                        .then(|| list[i].tool_id.clone())
                })
                .unwrap_or_else(|| format!("thought-{start}"));
            let mut trace = String::new();
            for &i in &pieces {
                trace.push_str(&list[i].text);
            }
            let mut out = Vec::with_capacity(end - start);
            let mut placed = false;
            for i in start..end {
                if pieces.contains(&i) {
                    if !placed {
                        let mut thought = closed_thought(trace.clone(), start);
                        thought.tool_id = id.clone();
                        out.push(thought);
                        placed = true;
                    }
                } else {
                    out.push(list[i].clone());
                }
            }
            if list[start..end] != out[..] {
                list.splice(start..end, out.iter().cloned());
                end = start + out.len();
            }
        }
        start = end;
    }
}

fn should_fold_into_trace(msg: &ChatMsg) -> bool {
    match msg.role {
        ChatRole::Thought => looks_like_tool_trace(&msg.text),
        ChatRole::Bot => {
            let t = msg.text.trim();
            let lower = t.starts_with(|c: char| c.is_lowercase());
            if spoken_reply_text(t) && !lower {
                return false;
            }
            looks_like_tool_trace(t)
                || chunk_continues_trace(t)
                || leak_marker(t)
                || t.matches("\\n").count() >= 2
                || lower
        }
        _ => false,
    }
}

fn peel_thought_replies(list: &mut Vec<ChatMsg>) {
    let mut i = 0;
    while i < list.len() {
        if list[i].role != ChatRole::Thought {
            i += 1;
            continue;
        }
        let (trace, visible) = detach_trace(&list[i].text);
        if trace.is_empty() || visible.trim().is_empty() {
            i += 1;
            continue;
        }
        list[i].text = trace;
        list[i].open = false;
        let already = list
            .iter()
            .any(|msg| msg.role == ChatRole::Bot && msg.text.contains(visible.trim()));
        if !already {
            list.insert(i + 1, ChatMsg::bot(visible));
        }
        i += 1;
    }
}

fn closed_thought(text: String, n: usize) -> ChatMsg {
    ChatMsg {
        role: ChatRole::Thought,
        text,
        tool_id: format!("thought-{n}"),
        tool_name: String::new(),
        tool_status: String::new(),
        tool_args: String::new(),
        tool_result: String::new(),
        open: false,
    }
}

fn strip_tool_lines(text: &str) -> String {
    let cleaned: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().starts_with("TOOL "))
        .collect();
    cleaned.join("\n")
}

fn compact_json(value: &serde_json::Value) -> String {
    if value.is_null() || value == &serde_json::json!({}) {
        return String::new();
    }
    serde_json::to_string(value).unwrap_or_default()
}

fn focus_prompt() {
    let Some(el) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|doc| doc.get_element_by_id("ai-prompt"))
    else {
        return;
    };
    if let Ok(el) = el.dyn_into::<web_sys::HtmlElement>() {
        let _ = el.focus();
    }
}

fn scroll_thread() {
    let Some(el) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|doc| doc.get_element_by_id("ai-thread"))
    else {
        return;
    };
    let Ok(el) = el.dyn_into::<web_sys::HtmlElement>() else {
        return;
    };
    el.set_scroll_top(el.scroll_height());
}

fn install_studio_shortcut(mut ai_open: Signal<bool>) {
    use std::cell::Cell;
    thread_local! {
        static INSTALLED: Cell<bool> = const { Cell::new(false) };
    }
    if INSTALLED.with(|flag| flag.replace(true)) {
        return;
    }
    let Some(win) = web_sys::window() else {
        INSTALLED.with(|flag| flag.set(false));
        return;
    };
    let closure =
        wasm_bindgen::closure::Closure::wrap(Box::new(move |event: web_sys::KeyboardEvent| {
            let key = event.key();
            if !(event.ctrl_key() || event.meta_key())
                || event.alt_key()
                || !key.eq_ignore_ascii_case("k")
            {
                return;
            }
            event.prevent_default();
            ai_open.set(true);
            let focus = wasm_bindgen::closure::Closure::once(|| {
                let Some(el) = web_sys::window()
                    .and_then(|window| window.document())
                    .and_then(|doc| doc.get_element_by_id("ai-prompt"))
                else {
                    return;
                };
                if let Ok(el) = el.dyn_into::<web_sys::HtmlElement>() {
                    let _ = el.focus();
                }
            });
            if let Some(window) = web_sys::window() {
                let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
                    focus.as_ref().unchecked_ref(),
                    40,
                );
            }
            focus.forget();
        }) as Box<dyn FnMut(web_sys::KeyboardEvent)>);
    if win
        .add_event_listener_with_callback("keydown", closure.as_ref().unchecked_ref())
        .is_err()
    {
        INSTALLED.with(|flag| flag.set(false));
        return;
    }
    closure.forget();
}

fn show_timeline(save: &mut WorkspaceSave, clock: &Clock, timeline: EngineTimeline) {
    let previous = save.engine.peek().clone();
    if previous != timeline {
        let mut undo = save.undo.peek().clone();
        undo.checkpoint_named(previous, "Director");
        save.undo.set(undo.clone());
        tools::store_undo(&save.project_id.peek(), &undo);
    }
    let end = timeline.duration().as_seconds();
    save.engine.set(timeline.clone());
    save.tracks.set(bind::tracks_from_timeline(&timeline));
    if end > 0.0 {
        let mut duration = clock.duration;
        duration.set(end);
    }
    let next = Aspect::from_pixels(timeline.width, timeline.height);
    if *save.aspect.peek() != next {
        save.aspect.set(next);
        FRAME_ADOPTED.with(|flag| flag.set(true));
    }
}

fn merge_items(mut merged: Vec<MediaItem>, remote: Vec<MediaItem>) -> Vec<MediaItem> {
    for item in remote {
        if let Some(existing) = merged.iter_mut().find(|row| row.id == item.id) {
            existing.url = crate::api::keep_play_url(&existing.url, &item.url);
            if existing.duration <= 0.05 && item.duration > 0.05 {
                existing.duration = item.duration;
            }
        } else {
            merged.push(item);
        }
    }
    merged
}

fn merge_library(
    project_id: &str,
    mut library: Signal<Vec<MediaItem>>,
    mut active: Signal<Option<String>>,
    remote: Vec<MediaItem>,
) {
    let current = library.peek().clone();
    let remote: Vec<MediaItem> = remote
        .into_iter()
        .filter(|item| !is_buried(project_id, &item.id))
        .collect();
    let mut live = current.clone();
    live.retain(|item| !is_buried(project_id, &item.id));
    live = merge_items(live, remote);
    insert_pins(project_id, &mut live);
    remember_shown_blobs(project_id, &live);
    let spare = urls_not_in(&current, &live);
    if active.peek().is_none() || active.peek().as_deref().is_some_and(|url| url.is_empty()) {
        active.set(first_playable(&live));
    }
    if live != current {
        library.set(live);
    }
    revoke_spare(project_id, spare);
}

async fn attach_local_media(
    project_id: &str,
    mut items: Vec<MediaItem>,
    remote_ids: &[String],
    mut publish: impl FnMut(&[MediaItem]),
) -> Option<Vec<MediaItem>> {
    let stored = match hold::list_for_project(project_id).await {
        Ok(rows) => rows,
        Err(_) => return None,
    };
    for file in stored {
        if is_buried(project_id, &file.id) {
            continue;
        }
        let already = items
            .iter()
            .any(|item| item.id == file.id && item.url.starts_with("blob:"));
        if already {
            if let Some(existing) = items.iter().find(|item| item.id == file.id).cloned() {
                pin_media(project_id, &existing);
            }
        } else if let Some(made) = playable_held(project_id, &file).await {
            pin_media(project_id, &made);
            if let Some(existing) = items.iter_mut().find(|item| item.id == file.id) {
                existing.url = made.url;
                if existing.content_type.is_empty() {
                    existing.content_type = made.content_type;
                }
                if existing.duration <= 0.05 && made.duration > 0.05 {
                    existing.duration = made.duration;
                }
            } else {
                items.push(made);
            }
            publish(&items);
        }
        register_if_needed(project_id, &file, remote_ids).await;
    }
    Some(items)
}

async fn register_if_needed(project_id: &str, file: &hold::HeldMeta, remote_ids: &[String]) {
    if remote_ids.iter().any(|id| id == &file.id) || is_registered(project_id, &file.id) {
        note_registered(project_id, &file.id);
        return;
    }
    if api::register_media(
        project_id,
        &file.id,
        &file.name,
        &file.content_type,
        0.0,
    )
    .await
    .is_ok()
    {
        note_registered(project_id, &file.id);
    }
}

async fn playable_held(project_id: &str, file: &hold::HeldMeta) -> Option<MediaItem> {
    #[cfg(target_arch = "wasm32")]
    {
        let blob = hold::load_blob(project_id, &file.id).await.ok()?;
        return item_from_blob(file.name.clone(), &blob, file.id.clone());
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (project_id, file);
        None
    }
}

async fn restore_browser_media(
    project_id: &str,
    library: Signal<Vec<MediaItem>>,
    mut active: Signal<Option<String>>,
    remote_ids: &[String],
    live: impl Fn() -> bool,
) {
    if !live() {
        return;
    }
    let stored = match hold::list_for_project(project_id).await {
        Ok(rows) => rows,
        Err(_) => return,
    };
    if !live() {
        return;
    }
    let mut items = library.peek().clone();
    let mut changed = false;
    for file in stored {
        if !live() {
            return;
        }
        if is_buried(project_id, &file.id) {
            continue;
        }
        let already = items
            .iter()
            .any(|item| item.id == file.id && item.url.starts_with("blob:"));
        if already {
            if let Some(existing) = items.iter().find(|item| item.id == file.id).cloned() {
                pin_media(project_id, &existing);
            }
        } else if let Some(made) = playable_held(project_id, &file).await {
            pin_media(project_id, &made);
            if !live() {
                return;
            }
            if let Some(existing) = items.iter_mut().find(|item| item.id == file.id) {
                existing.url = made.url;
                if existing.content_type.is_empty() {
                    existing.content_type = made.content_type;
                }
                changed = true;
            } else {
                items.push(made);
                changed = true;
            }
        }
        if !live() {
            return;
        }
        register_if_needed(project_id, &file, remote_ids).await;
    }
    if !live() {
        return;
    }
    if changed {
        // `items` was copied before this loop. Setting it outright dropped any
        // clip imported while the browser copies were opening.
        merge_into_library(project_id, library, active, &items);
    }
    if active.peek().is_none() || active.peek().as_deref().is_some_and(|url| url.is_empty()) {
        active.set(library.peek().iter().find_map(|item| {
            if item.url.is_empty() {
                None
            } else {
                Some(item.url.clone())
            }
        }));
    }
}

fn frame_step(save: WorkspaceSave) -> f64 {
    12.0 / save.engine.peek().frame_rate.as_f64().max(1.0)
}

fn background_label(color: &str) -> String {
    match color {
        "#000000" => "Black".into(),
        "#ffffff" => "White".into(),
        "#808080" => "Gray".into(),
        "#1a1a1a" => "Charcoal".into(),
        other => other.to_string(),
    }
}

fn next_background(color: &str) -> &'static str {
    match oc_core::canonical_color(color).as_str() {
        "#000000" => "#ffffff",
        "#ffffff" => "#808080",
        "#808080" => "#1a1a1a",
        _ => "#000000",
    }
}

fn commit_mask(save: WorkspaceSave, library: &[MediaItem], evt: &Event<dioxus::html::PointerData>) {
    let Some((clip_id, shape)) = media::mask_up(evt) else {
        return;
    };
    live_note(
        save,
        library,
        tools::set_mask_at(save, Some(&clip_id), "", playhead_now(), Some(shape)),
    );
}

fn stop_chat(mut messages: Signal<Vec<ChatMsg>>, busy: Signal<bool>, draft: Signal<String>) {
    api::request_chat_stop();
    finish_chat_stop(&mut messages.write());
    stamp_live_turn(&mut messages.write());
    release_chat_busy(busy, draft);
}

/// The line that started the turn on screen. A snapshot refresh must not
/// drop it while Pause is still showing.
struct LiveTurn {
    project_id: String,
    text: String,
}

thread_local! {
    static LIVE_TURN: std::cell::RefCell<Option<LiveTurn>> = const { std::cell::RefCell::new(None) };
}

fn begin_live_turn(project_id: &str, text: String) {
    LIVE_TURN.with(|slot| {
        *slot.borrow_mut() = Some(LiveTurn {
            project_id: project_id.to_string(),
            text,
        });
    });
}

fn end_live_turn() {
    LIVE_TURN.with(|slot| *slot.borrow_mut() = None);
}

fn live_turn() -> Option<(String, String)> {
    LIVE_TURN.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|turn| (turn.project_id.clone(), turn.text.clone()))
    })
}

fn live_turn_missing(list: &[ChatMsg], project_id: &str) -> bool {
    let Some((pid, text)) = live_turn() else {
        return false;
    };
    if pid != project_id || text.is_empty() {
        return false;
    }
    !list
        .iter()
        .any(|msg| msg.role == ChatRole::User && msg.text == text)
}

fn composer_echoes_turn(next: &str, project_id: &str) -> bool {
    live_turn().is_some_and(|(pid, text)| {
        pid == project_id && !text.is_empty() && next.trim() == text.trim()
    })
}

/// The sent line belongs in the thread. A wiped list gets it back, ahead of
/// any status rows the request added after the wipe.
fn splice_live_turn(list: &mut Vec<ChatMsg>, project_id: &str) {
    let Some((pid, text)) = live_turn() else {
        return;
    };
    if pid != project_id || text.is_empty() {
        return;
    }
    if list
        .iter()
        .any(|msg| msg.role == ChatRole::User && msg.text == text)
    {
        return;
    }
    let at = list
        .iter()
        .rposition(|msg| {
            matches!(
                msg.role,
                ChatRole::User | ChatRole::Bot | ChatRole::Tool | ChatRole::Thought
            )
        })
        .map(|i| i + 1)
        .unwrap_or(0);
    let mut row = ChatMsg::user(text);
    row.open = true;
    list.insert(at, row);
}

fn release_chat_busy(mut busy: Signal<bool>, mut draft: Signal<String>) {
    if let Some((_, text)) = live_turn() {
        if !text.is_empty() && draft.peek().trim() == text.trim() {
            draft.set(String::new());
            clear_prompt_element();
        }
    }
    end_live_turn();
    busy.set(false);
}

/// Pause ended the turn. Pending tools leave the yellow state, and the
/// composer is idle so the button is send again.
fn finish_chat_stop(messages: &mut Vec<ChatMsg>) {
    for msg in messages.iter_mut() {
        if msg.role == ChatRole::Tool && msg.tool_status == "pending" {
            msg.tool_status = "stopped".into();
            msg.open = false;
        }
    }
    messages.retain(|msg| msg.role != ChatRole::Status);
    messages.push(ChatMsg::status("Stopped".to_string()));
}

fn clear_status(mut messages: Signal<Vec<ChatMsg>>) {
    messages.write().retain(|m| m.role != ChatRole::Status);
}

fn finish_bot_text(mut messages: Signal<Vec<ChatMsg>>, text: String) {
    clear_status(messages);
    let mut list = messages.write();
    settle_reply(&mut list, &text);
}

fn settle_reply(list: &mut Vec<ChatMsg>, text: &str) {
    fold_messages(list);
    let (trace, visible) = detach_trace(text);
    if !trace.is_empty() {
        if let Some(thought) = list
            .iter_mut()
            .rev()
            .find(|msg| msg.role == ChatRole::Thought && traces_overlap(&msg.text, &trace))
        {
            if trace.len() > thought.text.len() {
                thought.text = trace;
            }
            thought.open = false;
        } else {
            let n = list.len();
            list.push(closed_thought(trace, n));
        }
    }
    let visible = visible.trim();
    if visible.is_empty() || looks_like_tool_trace(visible) || leak_marker(visible) {
        return;
    }
    if list
        .iter()
        .any(|msg| msg.role == ChatRole::Bot && msg.text.contains(visible))
    {
        return;
    }
    list.push(ChatMsg::bot(visible.to_string()));
}

fn traces_overlap(existing: &str, incoming: &str) -> bool {
    let a = existing.trim();
    let b = incoming.trim();
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if a.contains(b) || b.contains(a) {
        return true;
    }
    let n = a.chars().count().min(b.chars().count()).min(80);
    if n < 40 {
        return false;
    }
    a.chars().take(n).eq(b.chars().take(n))
}

fn short_api_error(err: &str) -> String {
    if err.contains("404") {
        return "the API needs a restart before chats can be saved".into();
    }
    err.split(" for url")
        .next()
        .unwrap_or(err)
        .trim()
        .to_string()
}

fn persistable(list: &[ChatMsg]) -> Vec<api::StoredMsg> {
    let mut folded = list.to_vec();
    fold_messages(&mut folded);
    folded.into_iter().filter_map(stored_from_msg).collect()
}

fn stored_from_msg(msg: ChatMsg) -> Option<api::StoredMsg> {
    let role = match msg.role {
        ChatRole::User => "user",
        ChatRole::Bot => "assistant",
        ChatRole::Tool => "tool",
        ChatRole::Thought => "thought",
        ChatRole::Status => return None,
    };
    if is_acp_log(&msg.text) || is_acp_log(&msg.tool_result) {
        return None;
    }
    if msg.text.trim().is_empty() && msg.role != ChatRole::Tool {
        return None;
    }
    Some(api::StoredMsg {
        role: role.into(),
        text: msg.text,
        tool_id: msg.tool_id,
        tool_name: msg.tool_name,
        tool_status: msg.tool_status,
        tool_args: msg.tool_args,
        tool_result: msg.tool_result,
    })
}

fn msg_from_stored(msg: api::StoredMsg, n: usize) -> Option<ChatMsg> {
    match msg.role.as_str() {
        "user" => {
            let mut row = ChatMsg::user(msg.text);
            row.tool_result = msg.tool_result;
            Some(row)
        }
        "assistant" | "bot" => Some(ChatMsg::bot(msg.text)),
        "thought" => {
            let mut thought = closed_thought(msg.text, n);
            if !msg.tool_id.is_empty() {
                thought.tool_id = msg.tool_id;
            }
            Some(thought)
        }
        "tool" => Some(ChatMsg {
            role: ChatRole::Tool,
            text: msg.text,
            tool_id: msg.tool_id,
            tool_name: msg.tool_name,
            tool_status: if msg.tool_status.is_empty() {
                "done".into()
            } else {
                msg.tool_status
            },
            tool_args: msg.tool_args,
            tool_result: msg.tool_result,
            open: false,
        }),
        _ => None,
    }
}

fn apply_stored(messages: &mut Signal<Vec<ChatMsg>>, rows: Vec<api::StoredMsg>) {
    let mut next = Vec::new();
    for (n, row) in rows.into_iter().enumerate() {
        if let Some(msg) = msg_from_stored(row, n) {
            next.push(msg);
        }
    }
    reveal_worked_turns(&mut next);
    messages.set(next);
}

/// A list that left the server before a create finished must not drop that chat.
fn merge_remote_chats(
    remote: Vec<api::ChatSummary>,
    local: &[api::ChatSummary],
) -> Vec<api::ChatSummary> {
    let mut out = Vec::new();
    for row in local {
        if !remote.iter().any(|item| item.id == row.id) {
            out.push(row.clone());
        }
    }
    out.extend(remote);
    out
}

/// A chat load that started on an empty thread must not replace a line the
/// person already sent, or a turn that has started.
fn stored_load_is_stale(
    busy: bool,
    before_len: usize,
    now_len: usize,
    before_turn: u32,
    now_turn: u32,
) -> bool {
    busy || now_len != before_len || now_turn != before_turn
}

/// A create that returns after the person has typed or sent must not clear that.
fn new_chat_should_reset(busy: bool, messages: usize, draft: &str) -> bool {
    !busy && messages == 0 && draft.trim().is_empty()
}

fn chat_menu_label(chat: &api::ChatSummary, all: &[api::ChatSummary]) -> String {
    let twins = all.iter().filter(|row| row.title == chat.title).count();
    if twins < 2 {
        return chat.title.clone();
    }
    let nth = all
        .iter()
        .filter(|row| row.title == chat.title)
        .position(|row| row.id == chat.id)
        .unwrap_or(0);
    format!("{} {}", chat.title, nth + 1)
}

fn remember_summary(chats: &mut Signal<Vec<api::ChatSummary>>, chat: api::ChatSummary) {
    let mut list = chats.peek().clone();
    list.retain(|row| row.id != chat.id);
    list.insert(0, chat);
    chats.set(list);
}

fn chat_event_is_saved(kind: &str) -> bool {
    matches!(kind, "text" | "thought" | "tool" | "done" | "error")
}

#[derive(Clone)]
struct ChatFlush {
    project_id: String,
    chat_id: String,
    body: String,
}

struct ChatSaveJob {
    project_id: String,
    snap: Vec<api::StoredMsg>,
    bound: std::rc::Rc<std::cell::RefCell<Option<String>>>,
    chat_id: Signal<Option<String>>,
    chats: Signal<Vec<api::ChatSummary>>,
}

struct ChatSaveQueue {
    latest: Option<ChatSaveJob>,
    running: bool,
}

thread_local! {
    static CHAT_SAVES: std::cell::RefCell<ChatSaveQueue> =
        std::cell::RefCell::new(ChatSaveQueue {
            latest: None,
            running: false,
        });
    static CHAT_FLUSH: std::cell::RefCell<Option<ChatFlush>> = std::cell::RefCell::new(None);
}

fn note_flush(project_id: &str, chat_id: Option<&str>, snap: &[api::StoredMsg]) {
    let Some(chat_id) = chat_id.filter(|id| !id.is_empty()) else {
        return;
    };
    let body = serde_json::json!({ "messages": snap }).to_string();
    CHAT_FLUSH.with(|slot| {
        *slot.borrow_mut() = Some(ChatFlush {
            project_id: project_id.to_string(),
            chat_id: chat_id.to_string(),
            body,
        });
    });
}

fn flush_chat_on_leave() {
    let Some(pending) = CHAT_FLUSH.with(|slot| slot.borrow().clone()) else {
        return;
    };
    api::keepalive_chat_put(&pending.project_id, &pending.chat_id, &pending.body);
}

/// Coalesce streamed chunks into one save. A later snapshot replaces an older
/// one that has not been written yet, so a slow PUT cannot put the user line
/// back on top of the model's reply.
fn queue_chat_save(
    project_id: String,
    bound: std::rc::Rc<std::cell::RefCell<Option<String>>>,
    chat_id: Signal<Option<String>>,
    chats: Signal<Vec<api::ChatSummary>>,
    messages: Signal<Vec<ChatMsg>>,
) {
    let snap = persistable(&messages.read());
    if snap.is_empty() {
        return;
    }
    note_flush(&project_id, bound.borrow().as_deref(), &snap);
    let start = CHAT_SAVES.with(|queue| {
        let mut queue = queue.borrow_mut();
        queue.latest = Some(ChatSaveJob {
            project_id,
            snap,
            bound,
            chat_id,
            chats,
        });
        if queue.running {
            false
        } else {
            queue.running = true;
            true
        }
    });
    if start {
        spawn(async move {
            drive_chat_saves().await;
        });
    }
}

async fn drive_chat_saves() {
    loop {
        gloo_timers::future::TimeoutFuture::new(400).await;
        let job = CHAT_SAVES.with(|queue| queue.borrow_mut().latest.take());
        let Some(job) = job else {
            CHAT_SAVES.with(|queue| queue.borrow_mut().running = false);
            return;
        };
        let mut id = job.bound.borrow().clone();
        if let Ok(saved) =
            store_snap(&job.project_id, &mut id, job.chat_id, job.chats, job.snap).await
        {
            *job.bound.borrow_mut() = Some(saved);
        }
    }
}

async fn await_chat_saves() {
    loop {
        let busy = CHAT_SAVES.with(|queue| {
            let queue = queue.borrow();
            queue.running || queue.latest.is_some()
        });
        if !busy {
            return;
        }
        gloo_timers::future::TimeoutFuture::new(40).await;
    }
}

async fn save_chat_now(
    project_id: &str,
    bound: &std::rc::Rc<std::cell::RefCell<Option<String>>>,
    chat_id: Signal<Option<String>>,
    chats: Signal<Vec<api::ChatSummary>>,
    messages: Signal<Vec<ChatMsg>>,
) -> Result<(), String> {
    await_chat_saves().await;
    let snap = persistable(&messages.read());
    let mut id = bound.borrow().clone();
    let saved = store_snap(project_id, &mut id, chat_id, chats, snap).await?;
    *bound.borrow_mut() = Some(saved);
    Ok(())
}

async fn store_snap(
    project_id: &str,
    bound: &mut Option<String>,
    mut chat_id: Signal<Option<String>>,
    mut chats: Signal<Vec<api::ChatSummary>>,
    snap: Vec<api::StoredMsg>,
) -> Result<String, String> {
    if !auth::is_signed_in() {
        return Err("Sign in to keep chats.".into());
    }
    let id = if let Some(id) = bound.clone() {
        id
    } else {
        let chat = api::create_chat(project_id).await?;
        let id = chat.id.clone();
        *bound = Some(id.clone());
        if chat_id.peek().is_none() {
            chat_id.set(Some(id.clone()));
        }
        remember_summary(&mut chats, chat);
        id
    };
    let chat = api::save_chat(project_id, &id, &snap).await?;
    note_flush(project_id, Some(&id), &snap);
    remember_summary(&mut chats, chat);
    Ok(id)
}

fn start_new_chat(
    mut chat_id: Signal<Option<String>>,
    mut chats: Signal<Vec<api::ChatSummary>>,
    mut messages: Signal<Vec<ChatMsg>>,
    mut draft: Signal<String>,
    mut chat_menu: Signal<bool>,
    busy: Signal<bool>,
    project_id: String,
) {
    if *busy.peek() {
        return;
    }
    chat_menu.set(false);
    if !auth::is_signed_in() {
        messages
            .write()
            .push(ChatMsg::status("Sign in to keep chats."));
        return;
    }
    spawn(async move {
        match api::create_chat(&project_id).await {
            Ok(chat) => {
                remember_summary(&mut chats, chat.clone());
                if new_chat_should_reset(
                    *busy.peek(),
                    messages.peek().len(),
                    draft.peek().as_str(),
                ) {
                    chat_id.set(Some(chat.id.clone()));
                    messages.set(Vec::new());
                    draft.set(String::new());
                }
                chat_menu.set(true);
                gloo_timers::future::TimeoutFuture::new(40).await;
                focus_prompt();
            }
            Err(err) => {
                messages.write().push(ChatMsg::status(format!(
                    "Could not start a chat — {}",
                    short_api_error(&err)
                )));
            }
        }
    });
}

fn open_saved_chat(
    id: String,
    mut chat_id: Signal<Option<String>>,
    mut chats: Signal<Vec<api::ChatSummary>>,
    mut messages: Signal<Vec<ChatMsg>>,
    mut draft: Signal<String>,
    mut chat_menu: Signal<bool>,
    mut open_gen: Signal<u32>,
    busy: Signal<bool>,
    project_id: String,
) {
    if *busy.peek() {
        return;
    }
    if !auth::is_signed_in() {
        return;
    }
    let ticket = open_gen.peek().wrapping_add(1);
    open_gen.set(ticket);
    chat_id.set(Some(id.clone()));
    chat_menu.set(false);
    draft.set(String::new());
    let before_len = messages.peek().len();
    let before_turn = api::chat_generation();
    spawn(async move {
        match api::get_chat(&project_id, &id).await {
            Ok(detail) => {
                if *open_gen.peek() != ticket {
                    return;
                }
                if stored_load_is_stale(
                    *busy.peek(),
                    before_len,
                    messages.peek().len(),
                    before_turn,
                    api::chat_generation(),
                ) {
                    return;
                }
                chat_id.set(Some(detail.id.clone()));
                remember_summary(
                    &mut chats,
                    api::ChatSummary {
                        id: detail.id,
                        title: detail.title,
                    },
                );
                apply_stored(&mut messages, detail.messages);
            }
            Err(err) => {
                if *open_gen.peek() != ticket {
                    return;
                }
                if stored_load_is_stale(
                    *busy.peek(),
                    before_len,
                    messages.peek().len(),
                    before_turn,
                    api::chat_generation(),
                ) {
                    return;
                }
                messages.set(vec![ChatMsg::status(format!(
                    "Could not open that chat — {}",
                    short_api_error(&err)
                ))]);
            }
        }
    });
}

async fn store_finished_video(
    pid: &str,
    preset_name: &str,
    handoff: bool,
    mut messages: Signal<Vec<ChatMsg>>,
) {
    if !handoff {
        messages.write().push(ChatMsg::status(
            "Render queued. The finished video is stored on Cloudflare.".to_string(),
        ));
        return;
    }
    messages.write().push(ChatMsg::status(
        "Sending this browser's clips. The finished video is stored on Cloudflare.".to_string(),
    ));
    if let Err(err) = api::send_browser_sources(pid).await {
        messages.write().push(ChatMsg::status(err));
        return;
    }
    let Some(preset) = ExportPreset::parse(preset_name) else {
        messages.write().push(ChatMsg::status(
            "The finished video was not queued.".to_string(),
        ));
        return;
    };
    if let Err(err) = api::apply_ops(pid, vec![Op::Export { preset }]).await {
        messages.write().push(ChatMsg::status(format!(
            "The finished video was not queued — {err}"
        )));
        return;
    }
    messages.write().push(ChatMsg::status(
        "Render queued. Open Export to play it. The finished video is stored on Cloudflare."
            .to_string(),
    ));
}

thread_local! {
    static TURN_START_MS: std::cell::Cell<f64> = const { std::cell::Cell::new(0.0) };
}

fn now_ms() -> f64 {
    // wasm32-unknown-unknown has no SystemTime. Date.now is the same clock
    // the preview loop already uses, and it does not panic on send.
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Date::now()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|dur| dur.as_secs_f64() * 1000.0)
            .unwrap_or(0.0)
    }
}

fn live_worked_ms() -> u64 {
    let start = TURN_START_MS.with(|cell| cell.get());
    if start <= 0.0 {
        return 0;
    }
    (now_ms() - start).max(0.0) as u64
}

fn worked_label(ms: u64) -> String {
    let total = ms / 1000;
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let seconds = total % 60;
    if hours > 0 {
        format!("Worked for {hours}h {minutes}m {seconds}s")
    } else if minutes > 0 {
        format!("Worked for {minutes}m {seconds}s")
    } else {
        format!("Worked for {seconds}s")
    }
}

fn parse_worked_ms(text: &str) -> Option<u64> {
    text.strip_prefix("ms:")?.parse().ok()
}

fn note_elapsed(list: &mut [ChatMsg], user_at: usize, elapsed_ms: u64) {
    let user_at = if list
        .get(user_at)
        .is_some_and(|msg| msg.role == ChatRole::User)
    {
        user_at
    } else if let Some(idx) = list.iter().rposition(|msg| msg.role == ChatRole::User) {
        idx
    } else {
        return;
    };
    let first = parse_worked_ms(&list[user_at].tool_result).is_none();
    list[user_at].tool_result = format!("ms:{elapsed_ms}");
    if first && list[user_at].open {
        open_first_tool_group(list, user_at);
    }
}

fn open_first_tool_group(list: &mut [ChatMsg], user_at: usize) {
    let end = list
        .iter()
        .skip(user_at + 1)
        .position(|msg| msg.role == ChatRole::User)
        .map(|offset| user_at + 1 + offset)
        .unwrap_or(list.len());
    let mut seen_tool = false;
    let mut left_group = false;
    for msg in &mut list[user_at + 1..end] {
        if msg.role == ChatRole::Tool {
            if !left_group {
                msg.open = true;
                seen_tool = true;
            }
        } else if seen_tool {
            left_group = true;
        }
    }
}

/// A new command joins the first group of this turn until a reply breaks it.
fn joining_first_tool_group(list: &[ChatMsg]) -> bool {
    let Some(user_at) = list.iter().rposition(|msg| msg.role == ChatRole::User) else {
        return true;
    };
    let mut seen_tool = false;
    for msg in &list[user_at + 1..] {
        if msg.role == ChatRole::Tool {
            seen_tool = true;
        } else if seen_tool {
            return false;
        }
    }
    true
}

fn reveal_worked_turns(list: &mut [ChatMsg]) {
    let marks: Vec<usize> = list
        .iter()
        .enumerate()
        .filter(|(_, msg)| {
            msg.role == ChatRole::User && parse_worked_ms(&msg.tool_result).is_some()
        })
        .map(|(idx, _)| idx)
        .collect();
    for idx in marks {
        list[idx].open = true;
        open_first_tool_group(list, idx);
    }
}

fn stamp_elapsed(mut messages: Signal<Vec<ChatMsg>>, user_at: usize, started: f64) {
    if started <= 0.0 {
        return;
    }
    let elapsed = (now_ms() - started).max(0.0) as u64;
    note_elapsed(&mut messages.write(), user_at, elapsed);
}

fn stamp_live_turn(list: &mut [ChatMsg]) {
    let start = TURN_START_MS.with(|cell| cell.get());
    if start <= 0.0 {
        return;
    }
    let Some(user_at) = list.iter().rposition(|msg| msg.role == ChatRole::User) else {
        return;
    };
    let elapsed = (now_ms() - start).max(0.0) as u64;
    note_elapsed(list, user_at, elapsed);
}

struct TurnStamp {
    messages: Signal<Vec<ChatMsg>>,
    user_at: usize,
    started: f64,
    done: bool,
}

impl TurnStamp {
    fn stamp(&mut self) {
        if self.done {
            return;
        }
        self.done = true;
        stamp_elapsed(self.messages, self.user_at, self.started);
    }
}

impl Drop for TurnStamp {
    fn drop(&mut self) {
        self.stamp();
    }
}

thread_local! {
    // Enter and the send button can both fire before the deferred send runs.
    static SEND_QUEUED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Clears the one-shot send flag when the deferred turn finishes or is dropped.
struct ReleaseSendQueued;

impl Drop for ReleaseSendQueued {
    fn drop(&mut self) {
        SEND_QUEUED.with(|flag| flag.set(false));
    }
}

/// Schedule a send after the key or click handler returns, on this sidebar's scope.
///
/// `Runtime::handle_event` keeps `runtime.elements` borrowed for the whole
/// callback. Writing the thread from that callback rerenders inside the borrow
/// and panics in `VirtualDom::next_element` (`RefCell already borrowed`).
///
/// A bare `setTimeout` callback is also outside the Dioxus runtime. `spawn`
/// then panics on an empty scope stack, the draft is already cleared, and the
/// line never paints. Spawn the wait from the handler so the resumed task still
/// has this component's scope.
fn queue_send_prompt(
    draft: Signal<String>,
    messages: Signal<Vec<ChatMsg>>,
    save: WorkspaceSave,
    clock: Clock,
    target_track: Signal<String>,
    provider_id: Signal<String>,
    model_id: Signal<String>,
    busy: Signal<bool>,
    chat_id: Signal<Option<String>>,
    chats: Signal<Vec<api::ChatSummary>>,
    library: Signal<Vec<MediaItem>>,
    active: Signal<Option<String>>,
    held: Signal<Vec<HeldImport>>,
    uploading: Signal<Vec<String>>,
    chat_menu: Signal<bool>,
) {
    let text = prompt_to_send(draft);
    if text.is_empty() || *busy.peek() {
        return;
    }
    if SEND_QUEUED.with(|flag| flag.replace(true)) {
        return;
    }
    spawn(async move {
        let _release = ReleaseSendQueued;
        // One timer tick lets the click finish and drop the element arena
        // before the thread is written. The task is polled with the scope
        // captured above, so the chat spawn does not panic.
        gloo_timers::future::TimeoutFuture::new(0).await;
        send_prompt(
            draft,
            messages,
            save,
            clock,
            target_track,
            provider_id,
            model_id,
            busy,
            chat_id,
            chats,
            library,
            active,
            held,
            uploading,
            chat_menu,
        );
    });
}

/// Text in the composer signal, or the live textarea when the signal is behind.
fn prompt_to_send(draft: Signal<String>) -> String {
    let typed = draft.peek().trim().to_string();
    if !typed.is_empty() {
        return typed;
    }
    prompt_element()
        .map(|el| el.value().trim().to_string())
        .unwrap_or_default()
}

fn clear_prompt_element() {
    if let Some(el) = prompt_element() {
        el.set_value("");
    }
}

fn prompt_element() -> Option<web_sys::HtmlTextAreaElement> {
    web_sys::window()
        .and_then(|window| window.document())
        .and_then(|doc| doc.get_element_by_id("ai-prompt"))
        .and_then(|el| el.dyn_into::<web_sys::HtmlTextAreaElement>().ok())
}

fn send_prompt(
    mut draft: Signal<String>,
    mut messages: Signal<Vec<ChatMsg>>,
    mut save: WorkspaceSave,
    clock: Clock,
    target_track: Signal<String>,
    provider_id: Signal<String>,
    model_id: Signal<String>,
    mut busy: Signal<bool>,
    chat_id: Signal<Option<String>>,
    chats: Signal<Vec<api::ChatSummary>>,
    library: Signal<Vec<MediaItem>>,
    active: Signal<Option<String>>,
    mut held: Signal<Vec<HeldImport>>,
    uploading: Signal<Vec<String>>,
    mut chat_menu: Signal<bool>,
) {
    let text = prompt_to_send(draft);
    if text.is_empty() || *busy.peek() {
        return;
    }
    // The saved-chat list sits over the first line. Close it so the send shows.
    chat_menu.set(false);
    messages.write().push(ChatMsg::user(text.clone()));
    let user_at = messages.peek().len() - 1;
    draft.set(String::new());
    clear_prompt_element();
    let at = playhead_now().max(*clock.current.peek());
    if let Ok(notes) = tools::run_intent(save, &text, target_track.peek().as_str(), at) {
        if !notes.is_empty() {
            messages.write().push(ChatMsg::bot(notes.join(" · ")));
            let snap = persistable(&messages.read());
            let pid = save.project_id.peek().clone();
            let mut bound = chat_id.peek().clone();
            spawn(async move {
                let _ = store_snap(&pid, &mut bound, chat_id, chats, snap).await;
            });
            end_live_turn();
            return;
        }
    }
    let turn = api::begin_chat();
    begin_live_turn(&save.project_id.peek(), text.clone());
    {
        let mut list = messages.write();
        if let Some(msg) = list.get_mut(user_at) {
            msg.open = true;
        }
    }
    let started = now_ms();
    TURN_START_MS.with(|cell| cell.set(started));
    busy.set(true);
    messages.write().push(ChatMsg::status(format!(
        "Sending to {} · {}…",
        provider_id.peek(),
        model_id.peek()
    )));
    let pid = save.project_id.peek().clone();
    let provider = provider_id.peek().clone();
    let model = model_id.peek().clone();
    let history: Vec<(bool, String)> = messages
        .peek()
        .iter()
        .filter(|m| matches!(m.role, ChatRole::User | ChatRole::Bot))
        .map(|m| (m.role == ChatRole::User, m.text.clone()))
        .collect();
    let batch = held.peek().clone();
    let bound = std::rc::Rc::new(std::cell::RefCell::new(chat_id.peek().clone()));
    queue_chat_save(pid.clone(), bound.clone(), chat_id, chats, messages);
    spawn(async move {
        let mut turn_clock = TurnStamp {
            messages,
            user_at,
            started,
            done: false,
        };
        if !api::chat_current(turn) {
            turn_clock.stamp();
            let _ = save_chat_now(&pid, &bound, chat_id, chats, messages).await;
            if api::chat_generation() == turn {
                release_chat_busy(busy, draft);
            }
            return;
        }
        for file in batch {
            if !api::chat_current(turn) {
                break;
            }
            if is_registered(&pid, &file.id) {
                held.write().retain(|row| row.id != file.id);
                forget_import(&file.id);
                continue;
            }
            begin_upload(uploading, &file.id);
            let result = save_one_import(&pid, &file).await;
            if result.is_ok() {
                held.write().retain(|row| row.id != file.id);
                forget_import(&file.id);
                note_registered(&pid, &file.id);
            } else if api::chat_current(turn) {
                let err = result.as_ref().err().map(String::as_str).unwrap_or("");
                messages.write().push(ChatMsg::status(format!(
                    "{} was not saved in this browser — {err}",
                    file.name
                )));
            }
            note_upload(uploading, &file.id, &file.name, result);
        }
        if !api::chat_current(turn) {
            turn_clock.stamp();
            let _ = save_chat_now(&pid, &bound, chat_id, chats, messages).await;
            if api::chat_generation() == turn {
                release_chat_busy(busy, draft);
            }
            return;
        }
        if api::chat_current(turn) {
            messages.write().push(ChatMsg::status(
                "Reading speech from this browser. The words stay with the clip.".to_string(),
            ));
        }
        if let Err(err) = api::ensure_speech(&pid).await {
            if api::chat_current(turn) {
                messages
                    .write()
                    .push(ChatMsg::status(format!("Speech was not stored — {err}")));
            }
        }
        if !api::chat_current(turn) {
            turn_clock.stamp();
            let _ = save_chat_now(&pid, &bound, chat_id, chats, messages).await;
            if api::chat_generation() == turn {
                release_chat_busy(busy, draft);
            }
            return;
        }
        let save_bound = bound.clone();
        let save_pid = pid.clone();
        let reply = api::chat_stream(&pid, &provider, &model, &history, turn, |ev| {
            if !api::chat_current(turn) {
                return;
            }
            if let Some(tl) = ev.timeline.clone() {
                show_timeline(&mut save, &clock, tl);
            }
            let kind = ev.kind.clone();
            apply_chat_event(messages, ev);
            stamp_elapsed(messages, user_at, started);
            if chat_event_is_saved(&kind) {
                queue_chat_save(
                    save_pid.clone(),
                    save_bound.clone(),
                    chat_id,
                    chats,
                    messages,
                );
            }
        })
        .await;
        if api::chat_generation() != turn {
            return;
        }
        if api::chat_stopped() {
            if api::chat_generation() == turn {
                turn_clock.stamp();
                let _ = save_chat_now(&pid, &bound, chat_id, chats, messages).await;
            }
            if api::chat_generation() == turn {
                release_chat_busy(busy, draft);
            }
            return;
        }
        match reply {
            Ok(resp) => {
                let timeline = match api::get_project(&pid).await {
                    Ok(project) => project.timeline,
                    Err(_) => resp.timeline.clone(),
                };
                show_timeline(&mut save, &clock, timeline);
                let remote_ids = if let Ok(remote) = api::list_media(&pid).await {
                    let ids = remote
                        .iter()
                        .map(|item| item.id.clone())
                        .collect::<Vec<_>>();
                    merge_library(&pid, library, active, remote);
                    ids
                } else {
                    Vec::new()
                };
                let shown = pid.clone();
                restore_browser_media(&pid, library, active, &remote_ids, || {
                    page_project_id().is_none_or(|id| id == shown)
                })
                .await;
                finish_bot_text(
                    messages,
                    if resp.text.trim().is_empty() {
                        resp.notes.join(" · ")
                    } else {
                        resp.text
                    },
                );
                if api::chat_current(turn) && !resp.export_preset.is_empty() {
                    store_finished_video(&pid, &resp.export_preset, resp.export_handoff, messages)
                        .await;
                }
            }
            Err(err) => {
                clear_status(messages);
                let keep = messages.read().iter().any(|msg| {
                    matches!(msg.role, ChatRole::Bot | ChatRole::Tool | ChatRole::Thought)
                        && (!msg.text.trim().is_empty() || !msg.tool_name.is_empty())
                });
                if !(keep && err.contains("without a reply")) {
                    messages.write().push(ChatMsg::bot(err));
                }
            }
        }
        turn_clock.stamp();
        if let Err(err) = save_chat_now(&pid, &bound, chat_id, chats, messages).await {
            messages.write().push(ChatMsg::status(format!(
                "This chat was not saved — {}",
                short_api_error(&err)
            )));
        }
        if api::chat_generation() == turn {
            release_chat_busy(busy, draft);
        }
    });
}

fn drag_trim(
    mut tracks: Signal<Vec<EditorTrack>>,
    library: Signal<Vec<MediaItem>>,
    pps: f64,
    ripple: bool,
    trim: Signal<Option<(String, bool)>>,
    client_x: f64,
    client_y: f64,
) {
    let Some((clip_id, is_out)) = trim.peek().clone() else {
        return;
    };
    let rows = display_tracks(&tracks.read());
    let Some((time, _)) = crate::media::timeline_hit(client_x, client_y, pps, &rows) else {
        return;
    };
    let media_len = crate::media::media_duration_for(&library.read(), &tracks.read(), &clip_id);
    if is_out {
        crate::media::trim_out_mode(&mut tracks.write(), &clip_id, time, ripple, media_len);
    } else {
        crate::media::trim_in_mode(&mut tracks.write(), &clip_id, time, ripple, media_len);
    }
}

fn commit_trim(
    save: WorkspaceSave,
    tracks: Signal<Vec<EditorTrack>>,
    mut trim: Signal<Option<(String, bool)>>,
) {
    studio::hold_film(false);
    let Some((clip_id, _)) = trim.peek().clone() else {
        return;
    };
    trim.set(None);
    let Some(clip) = tracks
        .peek()
        .iter()
        .flat_map(|track| track.clips.iter())
        .find(|clip| clip.id == clip_id)
        .cloned()
    else {
        return;
    };
    let _ = tools::trim_clip(save, &clip.id, clip.start, clip.duration);
}

fn fire_tool(
    id: ToolId,
    save: WorkspaceSave,
    clock: Clock,
    mut target_track: Signal<String>,
    tracks: &mut Signal<Vec<EditorTrack>>,
    mut edit_mode: Signal<EditMode>,
) {
    let at = playhead_now().max(*clock.current.peek());
    let track = target_track.peek().clone();
    let result = match id {
        ToolId::Split => tools::split_at(save, &track, at),
        ToolId::SplitAll => tools::split_all_at(save, at),
        ToolId::Merge => tools::merge_at(save, &track, at),
        ToolId::Extract => tools::delete_at(save, &track, at),
        ToolId::Lift => tools::lift_at(save, &track, at),
        ToolId::TrimStart => tools::trim_start_at(save, &track, at),
        ToolId::TrimEnd => tools::trim_end_at(save, &track, at),
        ToolId::MarkIn => tools::mark_in_at(save, at),
        ToolId::MarkOut => tools::mark_out_at(save, at),
        ToolId::InsertAt => {
            edit_mode.set(EditMode::Insert);
            Ok(vec!["drops now insert".into()])
        }
        ToolId::OverwriteAt => {
            edit_mode.set(EditMode::Overwrite);
            Ok(vec!["drops now overwrite".into()])
        }
        ToolId::InsertSpace => tools::insert_space_at(save, &track, at),
        ToolId::DeleteSpace => tools::delete_space_at(save, &track, at),
        ToolId::DetachAudio => tools::detach_audio_at(save, &track, at),
        ToolId::Group => tools::group_at(save, &track, at),
        ToolId::Ungroup => tools::ungroup_at(save, &track, at),
        ToolId::Link => tools::link_at(save, &track, at),
        ToolId::Unlink => tools::unlink_at(save, &track, at),
        ToolId::AddMarker => tools::add_marker_at(save, at, "Marker"),
        ToolId::AddVideo => {
            add_track(tracks, &mut target_track, TrackKindUi::Video, save);
            Ok(Vec::new())
        }
        ToolId::AddAudio => {
            add_track(tracks, &mut target_track, TrackKindUi::Audio, save);
            Ok(Vec::new())
        }
        ToolId::AddCaption => {
            add_track(tracks, &mut target_track, TrackKindUi::Caption, save);
            Ok(Vec::new())
        }
        _ => Ok(Vec::new()),
    };
    if let Err(err) = result {
        tracing_tool_status(err);
    }
}

fn mode_glyph(id: ToolId) -> Element {
    match id {
        ToolId::Select => rsx! {
            svg { class: "icon-sm", view_box: "0 0 16 16", fill: "none", stroke: "currentColor", stroke_width: "1.6",
                path { d: "M3.5 2.5 7 13.5 8.6 9.2 13 8Z" }
            }
        },
        ToolId::Razor => rsx! {
            svg { class: "icon-sm", view_box: "0 0 16 16", fill: "none", stroke: "currentColor", stroke_width: "1.6",
                circle { cx: "4.5", cy: "4.5", r: "2" }
                circle { cx: "4.5", cy: "11.5", r: "2" }
                path { d: "M6 5.5 13 2.5M6 10.5 13 13.5" }
            }
        },
        ToolId::Spacer => rsx! {
            svg { class: "icon-sm", view_box: "0 0 16 16", fill: "none", stroke: "currentColor", stroke_width: "1.6",
                path { d: "M2 8h12M5 5 2 8l3 3M11 5l3 3-3 3" }
            }
        },
        ToolId::Slip => rsx! {
            svg { class: "icon-sm", view_box: "0 0 16 16", fill: "none", stroke: "currentColor", stroke_width: "1.6",
                rect { x: "4", y: "4", width: "8", height: "8", rx: "1" }
                path { d: "M2 8h2M12 8h2" }
            }
        },
        ToolId::Ripple => rsx! {
            svg { class: "icon-sm", view_box: "0 0 16 16", fill: "none", stroke: "currentColor", stroke_width: "1.6",
                path { d: "M3 12V4h4l6 8" }
            }
        },
        ToolId::Roll => rsx! {
            svg { class: "icon-sm", view_box: "0 0 16 16", fill: "none", stroke: "currentColor", stroke_width: "1.6",
                path { d: "M2 4h5v8H2M9 4h5v8H9" }
                path { d: "M8 3v10" }
            }
        },
        ToolId::Slide => rsx! {
            svg { class: "icon-sm", view_box: "0 0 16 16", fill: "none", stroke: "currentColor", stroke_width: "1.6",
                rect { x: "5", y: "4", width: "6", height: "8", rx: "1" }
                path { d: "M2 8h3M11 8h3" }
            }
        },
        ToolId::RateStretch => rsx! {
            svg { class: "icon-sm", view_box: "0 0 16 16", fill: "none", stroke: "currentColor", stroke_width: "1.6",
                path { d: "M2 8h12M12 5l3 3-3 3" }
            }
        },
        ToolId::Multicam => rsx! {
            svg { class: "icon-sm", view_box: "0 0 16 16", fill: "none", stroke: "currentColor", stroke_width: "1.6",
                rect { x: "2", y: "3", width: "5", height: "4" }
                rect { x: "9", y: "3", width: "5", height: "4" }
                rect { x: "2", y: "9", width: "5", height: "4" }
                rect { x: "9", y: "9", width: "5", height: "4" }
            }
        },
        _ => tool_glyph(id),
    }
}

fn tool_glyph(id: ToolId) -> Element {
    match id {
        ToolId::Split => rsx! {
            svg { class: "icon-sm", view_box: "0 0 16 16", fill: "none", stroke: "currentColor", stroke_width: "1.6",
                rect { x: "1.5", y: "4", width: "5", height: "8", rx: "1" }
                rect { x: "9.5", y: "4", width: "5", height: "8", rx: "1" }
                path { d: "M8 2.2v11.6" }
            }
        },
        ToolId::Merge => rsx! {
            svg { class: "icon-sm", view_box: "0 0 16 16", fill: "none", stroke: "currentColor", stroke_width: "1.6",
                rect { x: "2", y: "4", width: "12", height: "8", rx: "1" }
                path { d: "M8 4v8" }
                path { d: "M6.2 8h3.6" }
            }
        },
        ToolId::Extract => rsx! {
            svg { class: "icon-sm", view_box: "0 0 16 16", fill: "none", stroke: "currentColor", stroke_width: "1.7",
                rect { x: "3", y: "3.5", width: "10", height: "9", rx: "1.2" }
                path { d: "M6 8h4" }
            }
        },
        ToolId::Lift => rsx! {
            svg { class: "icon-sm", view_box: "0 0 16 16", fill: "none", stroke: "currentColor", stroke_width: "1.7",
                path { d: "M3 4.5h10" }
                path { d: "M5 4.5v7.2a1 1 0 0 0 1 1h4a1 1 0 0 0 1-1V4.5" }
            }
        },
        ToolId::TrimStart => rsx! {
            svg { class: "icon-sm", view_box: "0 0 16 16", fill: "none", stroke: "currentColor", stroke_width: "1.7",
                path { d: "M4 2.5v11" }
                path { d: "M4 8h9" }
            }
        },
        ToolId::TrimEnd => rsx! {
            svg { class: "icon-sm", view_box: "0 0 16 16", fill: "none", stroke: "currentColor", stroke_width: "1.7",
                path { d: "M12 2.5v11" }
                path { d: "M3 8h9" }
            }
        },
        ToolId::SplitAll => rsx! {
            svg { class: "icon-sm", view_box: "0 0 16 16", fill: "none", stroke: "currentColor", stroke_width: "1.6",
                path { d: "M8 1.5v13M3 4h4M9 4h4M3 8h4M9 8h4M3 12h4M9 12h4" }
            }
        },
        ToolId::MarkIn => rsx! {
            svg { class: "icon-sm", view_box: "0 0 16 16", fill: "none", stroke: "currentColor", stroke_width: "1.6",
                path { d: "M4 2.5v11M4 8h9" }
            }
        },
        ToolId::MarkOut => rsx! {
            svg { class: "icon-sm", view_box: "0 0 16 16", fill: "none", stroke: "currentColor", stroke_width: "1.6",
                path { d: "M12 2.5v11M3 8h9" }
            }
        },
        ToolId::InsertAt => rsx! {
            svg { class: "icon-sm", view_box: "0 0 16 16", fill: "none", stroke: "currentColor", stroke_width: "1.6",
                path { d: "M8 3v10M3 8h10" }
            }
        },
        ToolId::OverwriteAt => rsx! {
            svg { class: "icon-sm", view_box: "0 0 16 16", fill: "currentColor",
                rect { x: "3", y: "5", width: "10", height: "6", rx: "1" }
            }
        },
        ToolId::InsertSpace => rsx! {
            svg { class: "icon-sm", view_box: "0 0 16 16", fill: "none", stroke: "currentColor", stroke_width: "1.6",
                path { d: "M2 8h5M9 8h5M7 5v6M9 5v6" }
            }
        },
        ToolId::DeleteSpace => rsx! {
            svg { class: "icon-sm", view_box: "0 0 16 16", fill: "none", stroke: "currentColor", stroke_width: "1.6",
                path { d: "M2 8h12M6 5 3 8l3 3M10 5l3 3-3 3" }
            }
        },
        ToolId::DetachAudio => rsx! {
            svg { class: "icon-sm", view_box: "0 0 16 16", fill: "none", stroke: "currentColor", stroke_width: "1.6",
                path { d: "M4 3h8v5H4zM4 11h8M6 13h4" }
            }
        },
        ToolId::Group => rsx! {
            svg { class: "icon-sm", view_box: "0 0 16 16", fill: "none", stroke: "currentColor", stroke_width: "1.6",
                rect { x: "2.5", y: "4", width: "5", height: "8" }
                rect { x: "8.5", y: "4", width: "5", height: "8" }
            }
        },
        ToolId::Ungroup => rsx! {
            svg { class: "icon-sm", view_box: "0 0 16 16", fill: "none", stroke: "currentColor", stroke_width: "1.6",
                rect { x: "2", y: "4", width: "5", height: "8" }
                rect { x: "9", y: "4", width: "5", height: "8" }
            }
        },
        ToolId::Link => rsx! {
            svg { class: "icon-sm", view_box: "0 0 16 16", fill: "none", stroke: "currentColor", stroke_width: "1.6",
                path { d: "M6 8h4M5 5H3.5a2 2 0 0 0 0 6H5M11 5h1.5a2 2 0 0 1 0 6H11" }
            }
        },
        ToolId::Unlink => rsx! {
            svg { class: "icon-sm", view_box: "0 0 16 16", fill: "none", stroke: "currentColor", stroke_width: "1.6",
                path { d: "M6 8h1M9 8h1M5 5H3.5a2 2 0 0 0 0 6H5M11 5h1.5a2 2 0 0 1 0 6H11" }
            }
        },
        ToolId::AddMarker => rsx! {
            svg { class: "icon-sm", view_box: "0 0 16 16", fill: "none", stroke: "currentColor", stroke_width: "1.6",
                path { d: "M8 2v8M5 12h6M8 10l3 4H5l3-4z" }
            }
        },
        ToolId::AddVideo => rsx! {
            svg { class: "icon-sm", view_box: "0 0 16 16", fill: "none", stroke: "currentColor", stroke_width: "1.6",
                rect { x: "2", y: "4", width: "12", height: "8", rx: "1" }
                path { d: "M8 6v4M6 8h4" }
            }
        },
        ToolId::AddAudio => rsx! {
            svg { class: "icon-sm", view_box: "0 0 16 16", fill: "none", stroke: "currentColor", stroke_width: "1.6",
                path { d: "M6 10V5l6-1.2V10" }
                circle { cx: "5", cy: "11", r: "1.6" }
                circle { cx: "11", cy: "10", r: "1.6" }
            }
        },
        ToolId::AddCaption => rsx! {
            svg { class: "icon-sm", view_box: "0 0 16 16", fill: "none", stroke: "currentColor", stroke_width: "1.6",
                rect { x: "2", y: "4", width: "12", height: "8", rx: "1" }
                path { d: "M5 8h2M9 8h2M5 10h6" }
            }
        },
        _ => rsx! {
            svg { class: "icon-sm", view_box: "0 0 16 16", fill: "currentColor",
                circle { cx: "8", cy: "8", r: "2" }
            }
        },
    }
}

fn tracing_tool_status(err: String) {
    if let Some(label) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.query_selector(".tl-clock").ok().flatten())
    {
        let prev = label.text_content().unwrap_or_default();
        label.set_text_content(Some(&err));
        let _ = prev;
    }
}

#[allow(non_snake_case)]
pub(crate) fn IconScissors() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "1.8",
            path { d: "M6 9a3 3 0 1 0 0-6 3 3 0 0 0 0 6Zm0 12a3 3 0 1 0 0-6 3 3 0 0 0 0 6Z" }
            path { d: "M20 4 8.12 15.88M14.47 14.48 20 20M8.12 8.12 12 12" }
        }
    }
}

#[allow(non_snake_case)]
fn IconFilm() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "1.8",
            rect { x: "3", y: "5", width: "18", height: "14", rx: "2" }
            path { d: "M7 5v14M17 5v14M3 9h4M3 15h4M17 9h4M17 15h4" }
        }
    }
}

#[allow(non_snake_case)]
fn IconType() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "1.8",
            path { d: "M4 7V5h16v2M12 5v14M8 19h8" }
        }
    }
}

#[allow(non_snake_case)]
fn IconCaptions() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "1.8",
            rect { x: "3", y: "6", width: "18", height: "12", rx: "2" }
            path { d: "M7 13h3M14 13h3M7 16h10" }
        }
    }
}

#[allow(non_snake_case)]
fn IconWave() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "1.8",
            path { d: "M4 12h2v0M8 8v8M12 5v14M16 9v6M20 11v2" }
        }
    }
}

#[allow(non_snake_case)]
fn IconShapes() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "1.8",
            rect { x: "3", y: "3", width: "8", height: "8", rx: "1" }
            circle { cx: "17", cy: "8", r: "4" }
            path { d: "M8 21 4 14h8l-4 7Zm8-1h5v-5" }
        }
    }
}

#[allow(non_snake_case)]
fn IconTransition() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "1.8",
            rect { x: "3", y: "6", width: "9", height: "12", rx: "1" }
            rect { x: "12", y: "6", width: "9", height: "12", rx: "1" }
            path { d: "M12 6v12" }
        }
    }
}

#[allow(non_snake_case)]
fn IconSliders() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "1.8",
            path { d: "M4 6h16M4 12h16M4 18h16M8 4v4M16 10v4M11 16v4" }
        }
    }
}

#[allow(non_snake_case)]
fn IconPanel() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "1.8",
            rect { x: "3", y: "4", width: "18", height: "16", rx: "2" }
            path { d: "M15 4v16" }
        }
    }
}

#[allow(non_snake_case)]
fn IconGear() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "1.8",
            circle { cx: "12", cy: "12", r: "3" }
            path { d: "M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.2a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.2a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.2a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9c.3.6.9 1 1.5 1.1H21a2 2 0 1 1 0 4h-.2a1.7 1.7 0 0 0-1.4 1Z" }
        }
    }
}

#[allow(non_snake_case)]
fn IconPlay() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "currentColor",
            path { d: "M8 5v14l11-7L8 5Z" }
        }
    }
}

#[allow(non_snake_case)]
fn IconPause() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "currentColor",
            rect { x: "6", y: "5", width: "4", height: "14" }
            rect { x: "14", y: "5", width: "4", height: "14" }
        }
    }
}

#[allow(non_snake_case)]
fn IconSkipBack() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "currentColor",
            path { d: "M6 5h2v14H6V5Zm3.5 7L18 19V5l-8.5 7Z" }
        }
    }
}

#[allow(non_snake_case)]
fn IconSkipFwd() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "currentColor",
            path { d: "M16 5h2v14h-2V5ZM6 5v14l8.5-7L6 5Z" }
        }
    }
}

#[allow(non_snake_case)]
fn IconSpark() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "1.8",
            path { d: "M12 3l1.6 5.2L19 10l-5.4 1.8L12 17l-1.6-5.2L5 10l5.4-1.8L12 3Zm6 10 0.8 2.4L21 16l-2.2.6L18 19l-.8-2.4L15 16l2.2-.6L18 13Z" }
        }
    }
}

#[allow(non_snake_case)]
fn IconSend() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "1.8",
            path { d: "M22 2 11 13M22 2l-7 20-4-9-9-4 20-7Z" }
        }
    }
}

#[cfg(test)]
mod chat_tests {
    use super::*;

    #[test]
    fn a_tool_reads_as_a_command() {
        assert_eq!(command_line("submit_edit", ""), "Cut the timeline");
        assert_eq!(
            command_line("add_motion", r#"{"design":"stat-count","text":"10"}"#),
            "Placed a graphic: \"10\""
        );
        assert_eq!(
            command_line("edit_skill", r#"{"page":"documentary"}"#),
            "Opened an edit page: \"documentary\""
        );
        assert_eq!(command_line("set_speed", ""), "Set speed");
        assert_eq!(
            command_line("watch", r#"{"media_id":"abc"}"#),
            "Watched a clip"
        );
    }

    #[test]
    fn thinking_starts_collapsed() {
        let msg = closed_thought(r#"{"type":"function_call"}"#.into(), 0);
        assert!(matches!(msg.role, ChatRole::Thought));
        assert!(!msg.open);
        assert!(looks_like_tool_trace(&msg.text));
        let schema = "fields replace old values. Usage notes:\n{\"\\$schema\":\"http://json-schema.org/draft-07/schema#\",\"name\":\"scheduler_create\",\"parameters\":{\"properties\":{}}}";
        assert!(looks_like_tool_trace(schema));
        assert!(chunk_continues_trace("\"interval\""));
        assert!(!chunk_continues_trace("through the drive, the newsroom"));
        assert!(!looks_like_tool_trace(
            "through the drive, the newsroom, and home. I'm building that into one short."
        ));
        assert!(!looks_like_tool_trace("Cut a 40s reel from the interview."));
        let mixed = format!(
            "{schema}\n\nthrough the drive, the newsroom, and home. I'm building that into one short."
        );
        let (thought, visible) = detach_trace(&mixed);
        assert!(thought.contains("scheduler_create"));
        assert!(visible.contains("newsroom"));
        let (only, rest) = detach_trace(schema);
        assert!(rest.is_empty());
        assert!(only.contains("Usage notes:"));
    }

    fn leaked_tool_schema() -> String {
        concat!(
            r#"place that runs a prompt on a recurring interval.\n"#,
            r#"Usage notes:\n- Interval format: \"5m\" (minutes), \"2h\" (hours).\n"#,
            r#""name":"scheduler_create","parameters":{"$schema":"http://json-schema.org/draft-07/schema#","properties":{"fire_immediately":{"type":"boolean"}}}"#,
            r#","name":"scheduler_delete","name":"scheduler_list","name":"monitor","description":"Every stdout line is a main-agent wake. Print only DONE/FAILED/CANCELLED."}"#,
        )
        .to_string()
    }

    #[test]
    fn schema_dump_folds_out_of_the_reply() {
        let dump = leaked_tool_schema();
        assert!(looks_like_tool_trace(&dump));
        assert!(dump.contains("\\\"5m\\\"") || dump.contains("\"5m\""));
        let mut msgs = vec![
            ChatMsg::user("Make a vlog from these clips"),
            ChatMsg::bot(dump.clone()),
            ChatMsg::user("and keep going"),
        ];
        fold_messages(&mut msgs);
        assert!(msgs.iter().any(|m| {
            m.role == ChatRole::Thought && !m.open && m.text.contains("scheduler_create")
        }));
        assert!(
            !msgs
                .iter()
                .any(|m| m.role == ChatRole::Bot && m.text.contains("$schema"))
        );
        assert_eq!(msgs[0].text, "Make a vlog from these clips");
        assert_eq!(msgs.last().unwrap().text, "and keep going");
        assert!(matches!(msgs.last().unwrap().role, ChatRole::User));
    }

    #[test]
    fn streamed_schema_tokens_collapse_and_the_reply_stays() {
        let dump = leaked_tool_schema();
        let mut msgs = vec![ChatMsg::user("Make a vlog from these clips")];
        let mut rest = dump.as_str();
        while !rest.is_empty() {
            let n = rest.len().min(17);
            let (chunk, tail) = rest.split_at(n);
            append_model_chunk(&mut msgs, chunk, false);
            rest = tail;
        }
        assert!(msgs.iter().any(|m| m.role == ChatRole::Thought && !m.open));
        assert!(
            !msgs
                .iter()
                .any(|m| m.role == ChatRole::Bot && looks_like_tool_trace(&m.text))
        );
        append_model_chunk(
            &mut msgs,
            "I'll cut a 40 second vlog from the clips you imported.",
            false,
        );
        assert!(
            msgs.iter()
                .any(|m| { m.role == ChatRole::Bot && m.text.contains("40 second") })
        );
        settle_reply(
            &mut msgs,
            &format!("{dump}\n\nI'll cut a 40 second vlog from the clips you imported."),
        );
        assert_eq!(
            msgs.iter().filter(|m| m.role == ChatRole::Thought).count(),
            1
        );
        assert!(
            msgs.iter()
                .any(|m| m.role == ChatRole::Bot && m.text.contains("40 second"))
        );
        assert!(
            !msgs
                .iter()
                .any(|m| m.role == ChatRole::Bot && m.text.contains("$schema"))
        );
    }

    #[test]
    fn reply_words_leave_the_schema_thought() {
        let mut msgs = vec![ChatMsg::user("Make a vlog")];
        append_model_chunk(&mut msgs, &leaked_tool_schema(), false);
        for word in [
            "I'll ", "cut ", "a ", "40 ", "second ", "vlog ", "from ", "the ", "clips.",
        ] {
            append_model_chunk(&mut msgs, word, false);
        }
        assert!(
            msgs.iter()
                .any(|m| m.role == ChatRole::Bot && m.text.contains("clips"))
        );
        assert!(
            !msgs
                .iter()
                .any(|m| m.role == ChatRole::Thought && m.text.contains("clips"))
        );
    }

    fn screenshot_schema() -> String {
        concat!(
            "place that runs a prompt on a recurring interval, or update an existing one in place.\\n",
            "Use this tool when a user asks you to loop, repeat, or schedule a prompt or a task.\\n",
            "Set fire_immediately: true to also fire once on creation.\\n",
            "fields replace old values, omitted ones are unchanged.\\nUsage notes:\\n- Interval format: \\\"5m\\\" (minutes), \\\"2h\\\" (hours)\\n",
            "\"name\":\"scheduler_create\",\"parameters\":{\"$schema\":\"http://json-schema.org/draft-07/schema#\",\"properties\":{\"fire_immediately\":{\"type\":\"boolean\"}}},",
            "\"name\":\"scheduler_delete\",\"name\":\"scheduler_list\",",
            "\"description\":\"Every stdout line is a main-agent wake. Print only DONE/FAILED/CANCELLED.\",",
            "\"name\":\"monitor\",\"parameters\":{\"$schema\":\"http://json-schema.org/draft-07/schema#\",\"properties\":{\"command\":{\"type\":\"string\"}}}",
        )
        .to_string()
    }

    #[test]
    fn pause_returns_the_composer_and_closes_pending_tools() {
        let mut msgs = vec![
            ChatMsg::user("edit this video"),
            ChatMsg::status("Sending to grok · grok-4.7…"),
            ChatMsg {
                role: ChatRole::Tool,
                text: "see".into(),
                tool_id: "see-1".into(),
                tool_name: "see".into(),
                tool_status: "pending".into(),
                tool_args: String::new(),
                tool_result: "media @ 0.4s".into(),
                open: true,
            },
            ChatMsg {
                role: ChatRole::Tool,
                text: "read".into(),
                tool_id: "read-1".into(),
                tool_name: "read".into(),
                tool_status: "done".into(),
                tool_args: String::new(),
                tool_result: String::new(),
                open: false,
            },
        ];
        finish_chat_stop(&mut msgs);
        assert_eq!(
            msgs.iter()
                .filter(|msg| msg.role == ChatRole::Status)
                .map(|msg| msg.text.as_str())
                .collect::<Vec<_>>(),
            vec!["Stopped"]
        );
        let see = msgs.iter().find(|msg| msg.tool_name == "see").unwrap();
        assert_eq!(see.tool_status, "stopped");
        assert!(!see.open);
        let read = msgs.iter().find(|msg| msg.tool_name == "read").unwrap();
        assert_eq!(read.tool_status, "done");
    }

    fn tool_row() -> ChatMsg {
        ChatMsg {
            role: ChatRole::Tool,
            text: "assemble".into(),
            tool_id: "tool-1".into(),
            tool_name: "assemble".into(),
            tool_status: "done".into(),
            tool_args: String::new(),
            tool_result: String::new(),
            open: false,
        }
    }

    fn schema_is_hidden(msgs: &[ChatMsg]) {
        assert!(
            msgs.iter().any(|m| m.role == ChatRole::Thought
                && !m.open
                && m.text.contains("scheduler_create")),
            "schema should be one collapsed thought"
        );
        assert!(
            !msgs.iter().any(|m| {
                m.role == ChatRole::Bot
                    && (m.text.contains("$schema")
                        || m.text.contains("scheduler_")
                        || m.text.contains("Usage notes")
                        || m.text.contains("fire_immediately")
                        || m.text.contains("fields replace")
                        || m.text.contains("recurring interval")
                        || m.text.contains("place that runs"))
            }),
            "schema leaked into a reply"
        );
    }

    #[test]
    fn screenshot_dump_is_a_collapsed_thought() {
        let dump = screenshot_schema();
        assert!(looks_like_tool_trace(&dump));
        assert!(escaped_schema_dump(&dump));
        let mut msgs = vec![
            ChatMsg::user("Make a vlog from these clips"),
            ChatMsg::bot(dump.clone()),
        ];
        fold_messages(&mut msgs);
        schema_is_hidden(&msgs);
        settle_reply(
            &mut msgs,
            &format!("{dump}\n\nI'll cut a 40 second vlog from the clips you imported."),
        );
        schema_is_hidden(&msgs);
        assert!(
            msgs.iter()
                .any(|m| m.role == ChatRole::Bot && m.text.contains("40 second"))
        );
        fold_messages(&mut msgs);
        schema_is_hidden(&msgs);
        assert_eq!(
            msgs.iter().filter(|m| m.role == ChatRole::Thought).count(),
            1
        );
    }

    #[test]
    fn tool_row_does_not_leave_the_schema_visible() {
        let dump = screenshot_schema();
        let mut msgs = vec![ChatMsg::user("Make a vlog from these clips")];
        let mut rest = dump.as_str();
        let mut n = 0;
        while !rest.is_empty() {
            let take = rest.len().min(28);
            let (chunk, tail) = rest.split_at(take);
            append_model_chunk(&mut msgs, chunk, false);
            rest = tail;
            n += 1;
            if n == 3 {
                msgs.push(tool_row());
            }
        }
        append_model_chunk(
            &mut msgs,
            "I'll cut a 40 second vlog from the clips you imported.",
            false,
        );
        schema_is_hidden(&msgs);
        assert!(msgs.iter().any(|m| m.role == ChatRole::Tool));
        assert!(
            msgs.iter()
                .any(|m| m.role == ChatRole::Bot && m.text.contains("40 second"))
        );
        assert_eq!(
            msgs.iter().filter(|m| m.role == ChatRole::Thought).count(),
            1
        );
    }

    #[test]
    fn acp_logs_are_dropped_from_the_chat() {
        let log = "acp: [2m2026-10-01T08:59:20Z[0m [32m INFO[0m sampling.request model=grok-4.7 auth_prefix=hidden sse_chunk encrypted_content api_backend=responses";
        assert!(is_acp_log(log));
        assert!(!is_acp_log("Sending to xai · grok-4.7…"));
        assert!(!is_acp_log(
            "I'll cut a 40 second vlog from the clips you imported."
        ));
        let mut msgs = vec![
            ChatMsg::user("edit like a pro editor"),
            ChatMsg::status(log.to_string()),
            ChatMsg::bot("I'll cut a 40 second vlog from the clips you imported."),
        ];
        fold_messages(&mut msgs);
        assert!(
            msgs.iter()
                .all(|m| !m.text.contains("sampling.request") && !m.text.contains("auth_prefix"))
        );
        assert!(
            msgs.iter()
                .any(|m| m.role == ChatRole::Bot && m.text.contains("40 second"))
        );
        assert!(msgs.iter().any(|m| m.role == ChatRole::User));
    }

    #[test]
    fn a_reload_keeps_the_model_reply_and_the_tools() {
        let msgs = vec![
            ChatMsg::user("make it cinematic"),
            ChatMsg {
                role: ChatRole::Tool,
                text: "submit_edit".into(),
                tool_id: "t1".into(),
                tool_name: "submit_edit".into(),
                tool_status: "done".into(),
                tool_args: String::new(),
                tool_result: "placed 8 clips".into(),
                open: false,
            },
            ChatMsg::bot("Eight shots are reseated on the words."),
        ];
        let saved = persistable(&msgs);
        assert!(saved.iter().any(|row| row.role == "user"));
        assert!(saved.iter().any(|row| {
            row.role == "tool"
                && row.tool_name == "submit_edit"
                && row.tool_result.contains("8 clips")
        }));
        assert!(
            saved
                .iter()
                .any(|row| { row.role == "assistant" && row.text.contains("reseated") })
        );
        let restored = saved
            .into_iter()
            .enumerate()
            .filter_map(|(n, row)| msg_from_stored(row, n))
            .collect::<Vec<_>>();
        assert!(restored.iter().any(|msg| msg.role == ChatRole::User));
        assert!(
            restored
                .iter()
                .any(|msg| { msg.role == ChatRole::Tool && msg.tool_name == "submit_edit" })
        );
        assert!(
            restored
                .iter()
                .any(|msg| { msg.role == ChatRole::Bot && msg.text.contains("reseated") })
        );
        assert!(chat_event_is_saved("text"));
        assert!(chat_event_is_saved("tool"));
        assert!(chat_event_is_saved("done"));
        assert!(!chat_event_is_saved("status"));
    }

    #[test]
    fn a_chat_saves_the_reply_and_drops_the_log() {
        let log = "acp: [2m INFO sampling.request auth_prefix=hidden encrypted_content";
        let msgs = vec![
            ChatMsg::user("edit like a pro editor"),
            ChatMsg::status(log.to_string()),
            ChatMsg::bot("I'll cut a 40 second vlog from the clips you imported."),
        ];
        let saved = persistable(&msgs);
        assert_eq!(saved.len(), 2);
        assert_eq!(saved[0].role, "user");
        assert_eq!(saved[1].role, "assistant");
        assert!(
            saved
                .iter()
                .all(|row| !row.text.contains("sampling.request"))
        );
        let restored: Vec<_> = saved
            .into_iter()
            .enumerate()
            .filter_map(|(n, row)| msg_from_stored(row, n))
            .collect();
        assert!(restored.iter().any(|msg| msg.role == ChatRole::User));
        assert!(
            restored
                .iter()
                .any(|msg| msg.role == ChatRole::Bot && msg.text.contains("40 second"))
        );
    }

    fn named_tool(name: &str, id: &str) -> ChatMsg {
        ChatMsg {
            role: ChatRole::Tool,
            text: name.into(),
            tool_id: id.into(),
            tool_name: name.into(),
            tool_status: "done".into(),
            tool_args: String::new(),
            tool_result: String::new(),
            open: false,
        }
    }

    #[test]
    fn worked_for_opens_the_first_command_group() {
        assert_eq!(worked_label(756_000), "Worked for 12m 36s");
        assert_eq!(worked_label(8_000), "Worked for 8s");
        assert_eq!(worked_label(90_000), "Worked for 1m 30s");
        assert_eq!(worked_label(3_661_000), "Worked for 1h 1m 1s");
        assert_eq!(parse_worked_ms("ms:12"), Some(12));
        assert_eq!(parse_worked_ms("submit_edit"), None);

        let mut msgs = vec![
            ChatMsg::user("Make a short documentary"),
            named_tool("transcribe_media", "t1"),
            named_tool("list_media", "t2"),
            ChatMsg::bot("I'll match the reference."),
            named_tool("generate_broll", "t3"),
            named_tool("generate_broll", "t4"),
        ];
        msgs[0].open = true;
        note_elapsed(&mut msgs, 0, 756_000);
        assert_eq!(parse_worked_ms(&msgs[0].tool_result), Some(756_000));
        assert!(msgs[1].open && msgs[2].open);
        assert!(!msgs[4].open && !msgs[5].open);
        assert!(joining_first_tool_group(&msgs[..3]));
        assert!(!joining_first_tool_group(&msgs[..4]));

        let view = chat_view(&msgs, false, 0);
        assert!(view.iter().any(|row| {
            matches!(
                row,
                ChatView::Worked {
                    ms: 756_000,
                    open: true,
                    ..
                }
            )
        }));
        assert_eq!(
            view.iter()
                .filter(|row| matches!(row, ChatView::Tools { .. }))
                .count(),
            2
        );

        msgs[0].open = false;
        let hidden = chat_view(&msgs, false, 0);
        assert!(
            hidden
                .iter()
                .any(|row| { matches!(row, ChatView::Worked { open: false, .. }) })
        );
        assert!(
            !hidden
                .iter()
                .any(|row| matches!(row, ChatView::Tools { .. }))
        );
        assert!(
            hidden
                .iter()
                .any(|row| { matches!(row, ChatView::Msg(i) if msgs[*i].role == ChatRole::Bot) })
        );
        note_elapsed(&mut msgs, 0, 757_000);
        assert!(!msgs[0].open);
        assert_eq!(parse_worked_ms(&msgs[0].tool_result), Some(757_000));

        let saved = persistable(&msgs);
        let mut restored = saved
            .into_iter()
            .enumerate()
            .filter_map(|(n, row)| msg_from_stored(row, n))
            .collect::<Vec<_>>();
        assert_eq!(parse_worked_ms(&restored[0].tool_result), Some(757_000));
        reveal_worked_turns(&mut restored);
        assert!(restored[0].open);
        let tools: Vec<_> = restored
            .iter()
            .filter(|msg| msg.role == ChatRole::Tool)
            .collect();
        assert_eq!(tools.len(), 4);
        assert!(tools[0].open && tools[1].open);
        assert!(!tools[2].open && !tools[3].open);

        let old = vec![ChatMsg::user("cut it"), named_tool("submit_edit", "old")];
        let old_view = chat_view(&old, false, 0);
        assert!(
            !old_view
                .iter()
                .any(|row| matches!(row, ChatView::Worked { .. }))
        );
        assert!(
            old_view
                .iter()
                .any(|row| matches!(row, ChatView::Tools { .. }))
        );

        let live = chat_view(&old, true, 4_000);
        assert!(live.iter().any(|row| {
            matches!(
                row,
                ChatView::Worked {
                    ms: 4_000,
                    open: false,
                    ..
                }
            )
        }));
        assert!(!live.iter().any(|row| matches!(row, ChatView::Tools { .. })));
    }

    #[test]
    fn the_open_chat_survives_a_stale_list() {
        let remote = vec![api::ChatSummary {
            id: "old".into(),
            title: "Yesterday".into(),
        }];
        let local = vec![
            api::ChatSummary {
                id: "newer".into(),
                title: "New chat".into(),
            },
            api::ChatSummary {
                id: "new".into(),
                title: "New chat".into(),
            },
        ];
        let merged = merge_remote_chats(remote, &local);
        assert_eq!(merged[0].id, "newer");
        assert_eq!(merged[1].id, "new");
        assert!(merged.iter().any(|row| row.id == "old"));
        assert_eq!(
            chat_menu_label(&merged[0], &merged),
            "New chat 1"
        );
        assert_eq!(chat_menu_label(&merged[2], &merged), "Yesterday");
    }

    #[test]
    fn a_send_blocks_a_late_chat_load() {
        assert!(!stored_load_is_stale(false, 0, 0, 1, 1));
        assert!(stored_load_is_stale(true, 0, 0, 1, 1));
        assert!(stored_load_is_stale(false, 0, 1, 1, 1));
        assert!(stored_load_is_stale(false, 0, 0, 1, 2));
        assert!(new_chat_should_reset(false, 0, "  "));
        assert!(!new_chat_should_reset(false, 0, "hello"));
        assert!(!new_chat_should_reset(true, 0, ""));
        assert!(!new_chat_should_reset(false, 1, ""));
    }

    #[test]
    fn a_live_send_survives_a_stale_snapshot() {
        let sent = "can you create a cinematic edit using this";
        let live = vec![ChatMsg::user(sent)];
        let (messages, draft) = cached_thread(true, &live, "", &[], sent);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].text, sent);
        assert_eq!(draft, "");
    }

    #[test]
    fn a_project_switch_shows_the_saved_chat() {
        let live = vec![ChatMsg::user("hello")];
        let saved = vec![ChatMsg::user("older")];
        let (messages, draft) = cached_thread(false, &live, "", &saved, "draft");
        assert_eq!(messages[0].text, "older");
        assert_eq!(draft, "draft");
    }

    #[test]
    fn typing_is_not_replaced_by_an_empty_snapshot() {
        let (messages, draft) = cached_thread(true, &[], "hello", &[], "");
        assert!(messages.is_empty());
        assert_eq!(draft, "hello");
    }

    #[test]
    fn an_empty_editor_still_opens_the_saved_thread() {
        let saved = vec![ChatMsg::user("saved")];
        let (messages, draft) = cached_thread(true, &[], "", &saved, "old draft");
        assert_eq!(messages[0].text, "saved");
        assert_eq!(draft, "old draft");
    }

    #[test]
    fn a_wiped_thread_shows_the_line_that_was_sent() {
        end_live_turn();
        begin_live_turn("proj", "can you create a cinematic edit using this".into());
        let mut list = Vec::new();
        splice_live_turn(&mut list, "proj");
        assert_eq!(list.len(), 1);
        assert!(list[0].open);
        assert_eq!(list[0].text, "can you create a cinematic edit using this");
        splice_live_turn(&mut list, "proj");
        assert_eq!(list.len(), 1);
        let mut statuses = vec![ChatMsg::status("Sending to xai · grok-4.7…")];
        splice_live_turn(&mut statuses, "proj");
        assert_eq!(statuses[0].text, "can you create a cinematic edit using this");
        assert!(matches!(statuses[1].role, ChatRole::Status));
        let mut other = Vec::new();
        splice_live_turn(&mut other, "nope");
        assert!(other.is_empty());
        assert!(composer_echoes_turn(
            "can you create a cinematic edit using this",
            "proj"
        ));
        assert!(composer_echoes_turn(
            "  can you create a cinematic edit using this\n",
            "proj"
        ));
        assert!(!composer_echoes_turn("can you create a cinematic edit", "proj"));
        assert!(!composer_echoes_turn(
            "can you create a cinematic edit using this",
            "other"
        ));
        end_live_turn();
        assert!(!composer_echoes_turn(
            "can you create a cinematic edit using this",
            "proj"
        ));
    }
}
