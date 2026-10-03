mod api;
mod auth;
mod bind;
mod media;
mod pages;
mod studio;
mod toast;
mod tools;

use dioxus::prelude::*;
use wasm_bindgen::JsCast;
use media::{
    Clock, DragSession, DragSource, EditMode, EditTool, EditorTrack, MediaItem, MediaKind,
    TimelineClip, TrackKindUi, advance_playhead, clip_duration, clip_name, commit_drag,
    display_tracks, film_tiles, fit_scale, format_clock, format_tc_short,
    item_from_bytes_id, lane_height, next_track_name, paint_clock, paint_playhead, place_clip,
    playhead_now, preview_video, reset_tick_clock, ruler_marks_nle, scroll_left,
    apply_monitor_look,
    seek_by, max_timeline_h, program_end, set_media_duration, set_playhead, sync_monitor, timeline_end,
    uses_wall_clock,
    timeline_viewport_h, timeline_viewport_w, update_drag,
    capture_pointer, clamp_pps, video_duration_from_src,
};
use oc_core::{
    Fx, Grade, Graphic, Op, Timeline as EngineTimeline, TrackKind, TransitionKind,
};
use oc_core::TimelineEditMode;
use oc_tools::{ToolId, actions as cut_actions, modes as edit_tools, track_actions};
use pages::{Export, Login, NewProject, Projects};
use toast::{ToastProvider, show_toast};

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

#[derive(Clone)]
struct HeldImport {
    id: String,
    name: String,
    content_type: String,
    bytes: Vec<u8>,
}

#[derive(Clone, Copy)]
struct CtxHeld(Signal<Vec<HeldImport>>);

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

#[derive(Clone, Copy)]
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
    let timeline = bind::timeline_from_tracks(&save.tracks.peek(), &save.engine.peek(), width, height);
    let mut engine = save.engine;
    spawn(async move {
        if let Ok(next) = api::save_timeline(&pid, timeline).await {
            engine.set(next);
        }
    });
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

#[component]
fn Workspace(id: String) -> Element {
    let nav = navigator();
    let name = use_signal(|| "Untitled".to_string());
    let tab = use_signal(|| AssetTab::Media);
    let aspect = use_signal(|| Aspect::Landscape);
    let playing = use_signal(|| false);
    let current = use_signal(|| 0.0);
    let duration = use_signal(|| 0.0);
    let draft = use_signal(String::new);
    let messages = use_signal(Vec::<ChatMsg>::new);
    let ai_open = use_signal(|| true);
    let mut ai_width = use_signal(|| 320.0);
    let mut ai_drag = use_signal(|| None::<(f64, f64)>);
    let library = use_signal(Vec::<MediaItem>::new);
    let mut tracks = use_signal(|| bind::tracks_from_timeline(&EngineTimeline::default()));
    let active = use_signal(|| None::<String>);
    let project_id = use_signal(|| id.clone());
    let mut engine = use_signal(EngineTimeline::default);
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
    let mut undo = use_signal(oc_core::UndoStack::new);
    let selected_clip = use_signal(|| None::<String>);
    let held = use_signal(Vec::<HeldImport>::new);
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
    use_effect(move || {
        let held = held;
        let Some(win) = web_sys::window() else {
            return;
        };
        let closure = wasm_bindgen::closure::Closure::wrap(Box::new(move |event: web_sys::BeforeUnloadEvent| {
            if held.peek().is_empty() {
                return;
            }
            event.prevent_default();
            event.set_return_value(
                "Imported files are only in this browser tab and will be removed.",
            );
        }) as Box<dyn FnMut(web_sys::BeforeUnloadEvent)>);
        let handler: &js_sys::Function = closure.as_ref().unchecked_ref();
        win.set_onbeforeunload(Some(handler));
        closure.forget();
    });
    use_effect(move || {
        install_studio_shortcut(ai_open);
    });

    use_future(move || async move {
        let pid = project_id.peek().clone();
        match api::get_project(&pid).await {
            Ok(project) => {
                let mut name = name;
                name.set(project.name);
                tracks.set(bind::tracks_from_timeline(&project.timeline));
                let mut duration = clock.duration;
                duration.set(project.timeline.duration().as_seconds());
                let mut aspect = aspect;
                aspect.set(Aspect::from_pixels(
                    project.timeline.width,
                    project.timeline.height,
                ));
                if let Some(track) = project.timeline.first_track(TrackKind::Video) {
                    target_track.set(track.id.to_string());
                } else if let Some(track) = project.timeline.tracks.first() {
                    target_track.set(track.id.to_string());
                }
                engine.set(project.timeline);
                undo.set(tools::load_undo(&pid));
            }
            Err(_) => {
                if let Some(track) = engine.peek().first_track(TrackKind::Video) {
                    target_track.set(track.id.to_string());
                }
            }
        }
        if let Ok(remote) = api::list_media(&pid).await {
            merge_library(library, active, remote);
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
            let parked = playhead_now().min(end);
            let before = if uses_wall_clock(&engine.peek(), parked) {
                advance_playhead(end).min(end)
            } else {
                parked
            };
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
            sync_monitor(&engine.peek(), &library.peek(), &tracks.peek(), before, true);
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
                    sync_monitor(&engine.peek(), &library.peek(), &tracks.peek(), parked, false);
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
                    ai_width.set((sw + dx).clamp(240.0, 720.0));
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
            div { class: "main-col",
                Header { name }
                div { class: "body",
                    div { class: "top",
                        Assets { tab }
                        Preview { aspect, playing }
                    }
                    Timeline {}
                }
            }
            AiSidebar { draft, messages, ai_open, ai_width, ai_drag }
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
                match kind {
                    MediaKind::Image => rsx! { img { src: "{url}", alt: "" } },
                    MediaKind::Audio => rsx! { IconWave {} },
                    MediaKind::Video => rsx! {
                        video { src: "{url}", muted: true, preload: "metadata" }
                    },
                }
            }
            span { class: "drag-chip-name", "{name}" }
            span { class: "drag-chip-meta", "{dur} · {hint}" }
        }
    }
}

#[component]
fn Header(name: Signal<String>) -> Element {
    let nav = navigator();
    let project_id = use_context::<CtxProject>().0;
    let save = use_context::<WorkspaceSave>();
    let held = use_context::<CtxHeld>().0;
    let library = use_context::<Signal<Vec<MediaItem>>>();
    let tracks = use_context::<Signal<Vec<EditorTrack>>>();
    let unsaved = !held.read().is_empty();
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
                input {
                    class: "project-name",
                    value: "{name}",
                    oninput: move |e| name.set(e.value()),
                    onblur: move |_| {
                        let pid = project_id.peek().clone();
                        let title = name.peek().clone();
                        spawn(async move {
                            let _ = api::rename_project(&pid, &title).await;
                        });
                    },
                }
            }
            div { class: "header-right",
                button {
                    class: if unsaved { "btn btn-primary" } else { "btn btn-ghost" },
                    disabled: !unsaved,
                    title: "Store imported files so they survive leaving this project",
                    onclick: move |_| save_held_imports(held, library, tracks, save),
                    "Save progress"
                }
                button {
                    class: "btn btn-primary",
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
        AssetTab::Captions => rsx! {
            div { class: "drop",
                IconCaptions {}
                strong { "Captions" }
                small { "Transcribe speech and drop a caption track on the timeline." }
                button {
                    class: "btn btn-primary",
                    onclick: move |_| {
                        let pid = save.project_id.peek().clone();
                        let clock = clock;
                        spawn(async move {
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
        },
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
        },
    }
}

#[component]
fn MediaPanel() -> Element {
    let mut library = use_context::<Signal<Vec<MediaItem>>>();
    let mut tracks = use_context::<Signal<Vec<EditorTrack>>>();
    let mut active = use_context::<CtxActive>().0;
    let clock = use_context::<Clock>();
    let target_track = use_context::<CtxTargetTrack>().0;
    let edit_mode = use_context::<Signal<EditMode>>();
    let mut drag = use_context::<Signal<Option<DragSession>>>();
    let mut held = use_context::<CtxHeld>().0;
    let save = use_context::<WorkspaceSave>();

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
                                    let name = file.name();
                                    let Ok(bytes) = file.read_bytes().await else { continue };
                                    let ctype = MediaKind::mime(&name).to_string();
                                    let id = uuid::Uuid::now_v7().to_string();
                                    let Some(item) = item_from_bytes_id(name.clone(), &bytes, id.clone()) else {
                                        continue;
                                    };
                                    if matches!(item.kind, MediaKind::Video | MediaKind::Image)
                                        && active.read().is_none()
                                    {
                                        active.set(Some(item.url.clone()));
                                    }
                                    library.write().push(item);
                                    held.write().push(HeldImport {
                                        id,
                                        name,
                                        content_type: ctype,
                                        bytes: bytes.to_vec(),
                                    });
                                }
                            });
                        },
                    }
                }
            }
            div { class: "media-grid",
                for item in library.read().iter() {
                    {
                        let url = item.url.clone();
                        let pick_url = url.clone();
                        let add_url = url.clone();
                        let id = item.id.clone();
                        let selected = active.read().as_deref() == Some(url.as_str());
                        let kind = item.kind;
                        let name = item.name.clone();
                        let drag_item = item.clone();
                        rsx! {
                            div {
                                class: if selected { "media-tile on" } else { "media-tile" },
                                title: "Drag onto a track, or press + at the playhead",
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
                                        match kind {
                                            MediaKind::Image => rsx! { img { src: "{url}", alt: "" } },
                                            MediaKind::Video => {
                                                let probe_url = url.clone();
                                                rsx! {
                                                    video {
                                                        src: "{url}",
                                                        muted: true,
                                                        preload: "metadata",
                                                        onloadedmetadata: move |_| {
                                                            let Some(duration) =
                                                                video_duration_from_src(&probe_url)
                                                            else {
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
                                            MediaKind::Audio => rsx! { IconWave {} },
                                        }
                                    }
                                    span { class: "media-name", "{name}" }
                                }
                                button {
                                    class: "media-x",
                                    title: "Remove",
                                    onclick: move |_| {
                                        let remove_id = id.clone();
                                        let remove_url = url.clone();
                                        library.write().retain(|item| item.id != remove_id);
                                        for track in tracks.write().iter_mut() {
                                            track.clips.retain(|clip| clip.media_id != remove_id);
                                        }
                                        if active.read().as_deref() == Some(remove_url.as_str()) {
                                            let next = library.read().first().map(|item| item.url.clone());
                                            active.set(next);
                                        }
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
    let grade_class = if chrome.cube { "preview-grade" } else { "preview-grade off" };
    let mask_class = if chrome.mask { "preview-mask-host" } else { "preview-mask-host off" };
    let letter_class = if chrome.letterbox { "preview-letterbox" } else { "preview-letterbox off" };
    let handle_class = if chrome.mask { "mask-handles" } else { "mask-handles off" };
    let gfx_class = if chrome.letterbox { "preview-gfx letterboxed" } else { "preview-gfx" };

    use_effect(move || {
        media::set_selected_clip(selected_clip.read().clone());
        let playing = *clock.playing.read();
        if playing {
            return;
        }
        let now = *clock.current.read();
        sync_monitor(&save.engine.read(), &library.read(), &tracks.read(), now, false);
        apply_monitor_look(&save.engine.read(), &library.read(), now, false);
    });

    rsx! {
        section { class: "panel preview",
            div { class: "presets",
                button {
                    class: if current == Aspect::Landscape { "preset on" } else { "preset" },
                    onclick: move |_| {
                        aspect.set(Aspect::Landscape);
                        persist(save);
                    },
                    "16:9"
                }
                button {
                    class: if current == Aspect::Vertical { "preset on" } else { "preset" },
                    onclick: move |_| {
                        aspect.set(Aspect::Vertical);
                        persist(save);
                    },
                    "9:16"
                }
                button {
                    class: if current == Aspect::Square { "preset on" } else { "preset" },
                    onclick: move |_| {
                        aspect.set(Aspect::Square);
                        persist(save);
                    },
                    "1:1"
                }
                button {
                    class: if current == Aspect::Classic { "preset on" } else { "preset" },
                    onclick: move |_| {
                        aspect.set(Aspect::Classic);
                        persist(save);
                    },
                    "4:3"
                }
            }
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
            studio::Scopes {}
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
    let target_name = tracks
        .read()
        .iter()
        .find(|track| track.id == target_id)
        .map(|track| track.name.clone())
        .unwrap_or_else(|| "V1".into());
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
            onmousemove: move |evt| {
                let Some((clip_id, is_out)) = trim.peek().clone() else { return };
                let rows_now = display_tracks(&tracks.read());
                let Some((time, _)) = crate::media::timeline_hit(
                    evt.client_coordinates().x,
                    evt.client_coordinates().y,
                    *pps.peek(),
                    &rows_now,
                ) else {
                    return;
                };
                let ripple = *edit_tool.peek() == ToolId::Ripple;
                if is_out {
                    crate::media::trim_out_mode(&mut tracks.write(), &clip_id, time, ripple);
                } else {
                    crate::media::trim_in_mode(&mut tracks.write(), &clip_id, time, ripple);
                }
            },
            onmouseup: move |_| {
                commit_trim(save, tracks, trim);
            },
            onmouseleave: move |_| {
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
                    onscroll: move |_| sync_hscroll(".tl-scroll", ".tl-hbar"),
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
                                                        } else if is_image {
                                                            div { class: "nle-strip",
                                                                img { src: "{url}", alt: "" }
                                                            }
                                                        } else if is_video {
                                                            studio::Filmstrip { url: url.clone(), tiles, ends_only: *pps.read() < 24.0 }
                                                        } else {
                                                            div { class: "nle-strip" }
                                                        }
                                                        span { class: "nle-name", "{name}" }
                                                        if selected {
                                                            div {
                                                                class: "nle-trim in",
                                                                title: "Trim in",
                                                                onmousedown: move |evt| {
                                                                    evt.stop_propagation();
                                                                    trim.set(Some((trim_in_id.clone(), false)));
                                                                },
                                                            }
                                                            div {
                                                                class: "nle-trim out",
                                                                title: "Trim out",
                                                                onmousedown: move |evt| {
                                                                    evt.stop_propagation();
                                                                    trim.set(Some((trim_out_id.clone(), true)));
                                                                },
                                                            }
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
            div {
                class: "tl-hbar",
                onscroll: move |_| sync_hscroll(".tl-hbar", ".tl-scroll"),
                div { class: "tl-hbar-inner", style: "width: {canvas_w}px" }
            }
            div { class: "tl-status",
                span { "{oc_tools::tool(tool).map(|s| s.label).unwrap_or(\"Select\")}" }
                span { "{target_name}" }
                span { "{format_tc_short(now)}" }
                span { "len {format_tc_short(span)}" }
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
        "{n} imported file(s) are only in this browser tab. Leave without saving and they will be removed."
    );
    web_sys::window()
        .and_then(|w| w.confirm_with_message(&msg).ok())
        .unwrap_or(false)
}

fn save_held_imports(
    mut held: Signal<Vec<HeldImport>>,
    mut library: Signal<Vec<MediaItem>>,
    mut tracks: Signal<Vec<EditorTrack>>,
    save: WorkspaceSave,
) {
    let batch = held.peek().clone();
    if batch.is_empty() {
        return;
    }
    let pid = page_project_id().unwrap_or_else(|| save.project_id.peek().clone());
    spawn(async move {
        let mut failed = 0usize;
        for file in batch {
            match crate::api::upload_media(&pid, &file.name, &file.content_type, file.bytes).await {
                Ok(server_id) => {
                    if let Some(item) = library.write().iter_mut().find(|item| item.id == file.id) {
                        item.id = server_id.clone();
                    }
                    let mut changed = false;
                    for track in tracks.write().iter_mut() {
                        for clip in track.clips.iter_mut() {
                            if clip.media_id == file.id {
                                clip.media_id = server_id.clone();
                                changed = true;
                            }
                        }
                    }
                    held.write().retain(|row| row.id != file.id);
                    if changed {
                        persist(save);
                    }
                }
                Err(err) => {
                    failed += 1;
                    show_toast().error(format!("Save failed for {}: {err}", file.name));
                }
            }
        }
        if failed == 0 {
            show_toast().success("Progress saved. Imported files are stored.");
        }
    });
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

fn sync_hscroll(from: &str, to: &str) {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let Some(a) = doc.query_selector(from).ok().flatten() else {
        return;
    };
    let Some(b) = doc.query_selector(to).ok().flatten() else {
        return;
    };
    if a.scroll_left() != b.scroll_left() {
        b.set_scroll_left(a.scroll_left());
    }
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
fn TrackRow(
    name: String,
    kind: &'static str,
    clips: Vec<TimelineClip>,
    span: f64,
    can_remove: bool,
    on_remove: EventHandler<()>,
) -> Element {
    let library = use_context::<Signal<Vec<MediaItem>>>();
    let span = span.max(0.001);
    rsx! {
        div { class: "track",
            div { class: "track-label",
                span { "{name}" }
                span { "{kind}" }
                if can_remove {
                    button {
                        class: "track-x",
                        title: "Remove track",
                        onclick: move |evt| {
                            evt.stop_propagation();
                            on_remove.call(());
                        },
                        "×"
                    }
                }
            }
            div { class: "lane",
                for clip in clips.iter() {
                    {
                        let left = 100.0 * clip.start / span;
                        let width = 100.0 * clip.duration / span;
                        let label = clip_name(
                            clip,
                            library
                                .read()
                                .iter()
                                .find(|item| item.id == clip.media_id)
                                .map(|item| item.name.as_str()),
                        );
                        rsx! {
                            div {
                                class: "clip-bar",
                                style: "left: {left}%; width: {width}%;",
                                title: "{label}",
                                "{label}"
                            }
                        }
                    }
                }
            }
        }
    }
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
    use_future(move || async move {
        let pid = project_for_chats.peek().clone();
        let Some(user) = auth::current_email() else {
            return;
        };
        let Ok(remote) = api::list_chats(&pid, &user).await else {
            return;
        };
        let current = chat_id.peek().clone();
        let local = chats.peek().clone();
        let merged = merge_remote_chats(remote, &local, current.as_deref());
        chats.set(merged);
        if chat_id.peek().is_some() {
            return;
        }
        let Some(first) = chats.peek().first().cloned() else {
            return;
        };
        chat_id.set(Some(first.id.clone()));
        if let Ok(detail) = api::get_chat(&pid, &first.id, &user).await {
            if chat_id.peek().as_deref() != Some(first.id.as_str()) {
                return;
            }
            apply_stored(&mut messages, detail.messages);
        }
    });
    let suggestions = [
        ("Split at playhead", "split"),
        ("Merge clips", "merge"),
        ("Delete clip", "delete"),
        ("Remove filler words", "Remove all filler words like um, uh, like from the transcript"),
        ("Add subtitles", "Generate subtitles for the current video"),
        ("Remove silences", "Detect and remove long silences from the timeline"),
        ("Improve audio", "Apply noise reduction and normalize audio levels"),
        ("Generate thumbnail", "Create a thumbnail image for this video"),
        ("Auto color grade", "Apply automatic color grading to the video clips"),
    ];

    let open = *ai_open.read();
    let panel_w = if open { *ai_width.read() } else { 40.0 };
    let current_chat = chat_id.read().clone();
    let saved_chats = chats.read().clone();
    let chat_title = saved_chats
        .iter()
        .find(|row| Some(&row.id) == current_chat.as_ref())
        .map(|row| row.title.clone())
        .unwrap_or_else(|| "New chat".into());
    let menu_open = *chat_menu.read();
    let running = *busy.read();
    let mut folded = messages.read().clone();
    fold_messages(&mut folded);
    let chat_empty = folded.is_empty();
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
            div { class: "ai-head",
                button {
                    class: "collapse",
                    title: if open { "Collapse AI" } else { "Open AI Studio" },
                    onclick: move |_| {
                        let next = !*ai_open.read();
                        ai_open.set(next);
                    },
                    if open {
                        IconChevRight {}
                    } else {
                        IconSpark {}
                    }
                }
                if open {
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
                            "New"
                        }
                    }
                    span { class: "kbd", "Ctrl+K" }
                }
            }
            if open && menu_open {
                div { class: "chat-menu",
                    if saved_chats.is_empty() {
                        p { class: "chat-menu-empty", "No saved chats yet" }
                    }
                    for chat in saved_chats {
                        {
                            let id = chat.id.clone();
                            let selected = current_chat.as_deref() == Some(id.as_str());
                            let pid = save.project_id.peek().clone();
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
                                    "{chat.title}"
                                }
                            }
                        }
                    }
                }
            }
            if open {
            div { class: "ai-body",
                if chat_empty {
                    div { class: "ai-empty",
                        IconSpark {}
                        h3 { "AI Studio" }
                        p { "Brainstorm ideas, plan your video, or run AI commands." }
                        div { class: "suggest",
                            for (label, prompt) in suggestions {
                                button {
                                    onclick: move |_| draft.set(prompt.to_string()),
                                    "{label}"
                                }
                            }
                        }
                    }
                } else {
                    div { class: "msgs",
                        for row in chat_rows(&folded) {
                            match row {
                                ChatRow::Msg(i) => {
                                    let msg = folded[i].clone();
                                    rsx! { { render_chat_msg(messages, msg) } }
                                }
                                ChatRow::Tools { start, end } => {
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
                button {
                    class: "model-chip",
                    title: "Choose provider and model",
                    onclick: move |_| {
                        let next = !*picker_open.peek();
                        picker_open.set(next);
                    },
                    span { "{model_name}" }
                    span { class: "model-chip-caret", if *picker_open.read() { "▴" } else { "▾" } }
                }
                div { class: "ai-input",
                    input {
                        id: "ai-prompt",
                        placeholder: "Make a vlog from these clips…",
                        value: "{draft}",
                        oninput: move |e| draft.set(e.value()),
                        onkeydown: move |e| {
                            if e.key() == Key::Enter {
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
                                );
                            }
                        },
                    }
                    if *busy.read() {
                        button {
                            class: "send pause",
                            title: "Stop the chat",
                            onclick: move |_| stop_chat(messages, busy),
                            "Pause"
                        }
                    } else {
                        button {
                            class: "send",
                            disabled: draft.read().trim().is_empty(),
                            onclick: move |_| send_prompt(
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
            messages.write().push(ChatMsg::bot(format!("Error: {}", ev.text)));
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
    if let Some(existing) = list
        .iter_mut()
        .rev()
        .find(|m| {
            m.role == ChatRole::Tool
                && (m.tool_id == id
                    || (id.is_empty() && m.tool_name == short_tool_name(&ev.name)))
        })
    {
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
        open: false,
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

fn render_chat_msg(messages: Signal<Vec<ChatMsg>>, msg: ChatMsg) -> Element {
    match msg.role {
        ChatRole::User => rsx! { div { class: "bubble user", "{msg.text}" } },
        ChatRole::Bot => {
            let (trace, visible) = detach_trace(&msg.text);
            let hide = !trace.is_empty()
                || looks_like_tool_trace(&msg.text)
                || leak_marker(&msg.text);
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
    let label = if active {
        format!("Running {n} tools")
    } else {
        format!("Used {n} tools")
    };
    let show_lines = active || open;
    rsx! {
        div { class: "bubble tool",
            button {
                class: "tool-head",
                onclick: move |_| toggle_tool_group(messages, &id),
                span { class: "tool-caret", if show_lines { "▾" } else { "▸" } }
                span { class: "tool-name", "{label}" }
            }
            if show_lines {
                for msg in group {
                    div { class: "tool-line",
                        span { class: "tool-name", "{msg.tool_name}" }
                        span { class: "tool-status {msg.tool_status}", "{msg.tool_status}" }
                    }
                    if open && !msg.tool_result.is_empty() {
                        pre { class: "tool-result", "{msg.tool_result}" }
                    }
                }
            }
        }
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
    if let Some(msg) = messages
        .write()
        .iter_mut()
        .find(|m| m.tool_id == id)
    {
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
        Some(last) if last.role == ChatRole::Bot => !as_thought || bot_joins_trace(&last.text, &chunk),
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
    list.retain(|msg| msg.role == ChatRole::User || msg.role == ChatRole::Tool || !is_acp_log(&msg.text));
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
    let closure = wasm_bindgen::closure::Closure::wrap(Box::new(move |event: web_sys::KeyboardEvent| {
        let key = event.key();
        if !(event.ctrl_key() || event.meta_key()) || event.alt_key() || !key.eq_ignore_ascii_case("k") {
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
}

fn merge_library(
    mut library: Signal<Vec<MediaItem>>,
    mut active: Signal<Option<String>>,
    remote: Vec<MediaItem>,
) {
    let mut merged = library.peek().clone();
    for item in remote {
        if let Some(existing) = merged.iter_mut().find(|m| m.id == item.id) {
            if !item.url.is_empty() && (existing.url.is_empty() || existing.url.starts_with("blob:"))
            {
                existing.url = item.url;
            }
            if existing.duration <= 0.05 && item.duration > 0.05 {
                existing.duration = item.duration;
            }
        } else {
            merged.push(item);
        }
    }
    library.set(merged);
    if active.peek().is_none() {
        active.set(library.peek().first().map(|item| item.url.clone()));
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

fn commit_mask(
    save: WorkspaceSave,
    library: &[MediaItem],
    evt: &Event<dioxus::html::PointerData>,
) {
    let Some((clip_id, shape)) = media::mask_up(evt) else {
        return;
    };
    live_note(
        save,
        library,
        tools::set_mask_at(save, Some(&clip_id), "", playhead_now(), Some(shape)),
    );
}

fn stop_chat(mut messages: Signal<Vec<ChatMsg>>, mut busy: Signal<bool>) {
    api::request_chat_stop();
    finish_chat_stop(&mut messages.write());
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
    messages
        .write()
        .retain(|m| m.role != ChatRole::Status);
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
        if let Some(thought) = list.iter_mut().rev().find(|msg| {
            msg.role == ChatRole::Thought && traces_overlap(&msg.text, &trace)
        }) {
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
    if list.iter().any(|msg| msg.role == ChatRole::Bot && msg.text.contains(visible)) {
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
    err.split(" for url").next().unwrap_or(err).trim().to_string()
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
        "user" => Some(ChatMsg::user(msg.text)),
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
    messages.set(next);
}

fn merge_remote_chats(
    remote: Vec<api::ChatSummary>,
    local: &[api::ChatSummary],
    current: Option<&str>,
) -> Vec<api::ChatSummary> {
    let mut out = remote;
    if let Some(id) = current {
        if !out.iter().any(|row| row.id == id) {
            if let Some(found) = local.iter().find(|row| row.id == id) {
                out.insert(0, found.clone());
            }
        }
    }
    out
}

fn remember_summary(chats: &mut Signal<Vec<api::ChatSummary>>, chat: api::ChatSummary) {
    let mut list = chats.peek().clone();
    list.retain(|row| row.id != chat.id);
    list.insert(0, chat);
    chats.set(list);
}

async fn store_snap(
    project_id: &str,
    bound: &mut Option<String>,
    mut chat_id: Signal<Option<String>>,
    mut chats: Signal<Vec<api::ChatSummary>>,
    snap: Vec<api::StoredMsg>,
) -> Result<String, String> {
    let user = auth::current_email().ok_or_else(|| "Sign in to keep chats.".to_string())?;
    let id = if let Some(id) = bound.clone() {
        id
    } else {
        let chat = api::create_chat(project_id, &user).await?;
        let id = chat.id.clone();
        *bound = Some(id.clone());
        if chat_id.peek().is_none() {
            chat_id.set(Some(id.clone()));
        }
        remember_summary(&mut chats, chat);
        id
    };
    let chat = api::save_chat(project_id, &id, &user, &snap).await?;
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
    let Some(user) = auth::current_email() else {
        messages.write().push(ChatMsg::status("Sign in to keep chats."));
        return;
    };
    spawn(async move {
        match api::create_chat(&project_id, &user).await {
            Ok(chat) => {
                chat_id.set(Some(chat.id.clone()));
                remember_summary(&mut chats, chat);
                messages.set(Vec::new());
                draft.set(String::new());
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
    let Some(user) = auth::current_email() else {
        return;
    };
    let ticket = open_gen.peek().wrapping_add(1);
    open_gen.set(ticket);
    chat_id.set(Some(id.clone()));
    chat_menu.set(false);
    draft.set(String::new());
    spawn(async move {
        match api::get_chat(&project_id, &id, &user).await {
            Ok(detail) => {
                if *open_gen.peek() != ticket {
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
                messages.set(vec![ChatMsg::status(format!(
                    "Could not open that chat — {}",
                    short_api_error(&err)
                ))]);
            }
        }
    });
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
) {
    let text = draft.read().trim().to_string();
    if text.is_empty() || *busy.peek() {
        return;
    }
    messages.write().push(ChatMsg::user(text.clone()));
    draft.set(String::new());
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
            return;
        }
    }
    let turn = api::begin_chat();
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
    let library = use_context::<Signal<Vec<MediaItem>>>();
    let active = use_context::<CtxActive>().0;
    let bin: Vec<(String, String, String, f64, String)> = library
        .peek()
        .iter()
        .map(|item| {
            (
                item.id.clone(),
                item.name.clone(),
                item.content_type.clone(),
                item.duration,
                item.url.clone(),
            )
        })
        .collect();
    let opening = persistable(&messages.read());
    let mut bound = chat_id.peek().clone();
    spawn(async move {
        if let Ok(id) = store_snap(&pid, &mut bound, chat_id, chats, opening).await {
            bound = Some(id);
        }
        if !api::chat_current(turn) {
            if api::chat_generation() == turn {
                busy.set(false);
            }
            return;
        }
        for (id, name, ctype, dur, url) in &bin {
            if !api::chat_current(turn) {
                break;
            }
            let _ = api::register_media(&pid, id, name, ctype, *dur).await;
            if url.starts_with("blob:") {
                if let Ok(resp) = reqwest::Client::new().get(url).send().await {
                    if let Ok(bytes) = resp.bytes().await {
                        let _ = api::put_media_bytes(&pid, id, ctype, bytes.to_vec()).await;
                    }
                }
            }
        }
        if !api::chat_current(turn) {
            if api::chat_generation() == turn {
                busy.set(false);
            }
            return;
        }
        let reply = api::chat_stream(&pid, &provider, &model, &history, turn, |ev| {
            if !api::chat_current(turn) {
                return;
            }
            if let Some(tl) = ev.timeline.clone() {
                show_timeline(&mut save, &clock, tl);
            }
            apply_chat_event(messages, ev);
        })
        .await;
        if api::chat_generation() != turn {
            return;
        }
        if api::chat_stopped() {
            let snap = persistable(&messages.read());
            if api::chat_generation() == turn {
                let _ = store_snap(&pid, &mut bound, chat_id, chats, snap).await;
            }
            if api::chat_generation() == turn {
                busy.set(false);
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
                if let Ok(remote) = api::list_media(&pid).await {
                    merge_library(library, active, remote);
                }
                finish_bot_text(
                    messages,
                    if resp.text.trim().is_empty() {
                        resp.notes.join(" · ")
                    } else {
                        resp.text
                    },
                );
            }
            Err(err) => {
                clear_status(messages);
                messages.write().push(ChatMsg::bot(err));
            }
        }
        let snap = persistable(&messages.read());
        if let Err(err) = store_snap(&pid, &mut bound, chat_id, chats, snap).await {
            messages
                .write()
                .push(ChatMsg::status(format!(
                    "This chat was not saved — {}",
                    short_api_error(&err)
                )));
        }
        if api::chat_generation() == turn {
            busy.set(false);
        }
    });
}

fn commit_trim(
    save: WorkspaceSave,
    tracks: Signal<Vec<EditorTrack>>,
    mut trim: Signal<Option<(String, bool)>>,
) {
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

pub(crate) fn IconScissors() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "1.8",
            path { d: "M6 9a3 3 0 1 0 0-6 3 3 0 0 0 0 6Zm0 12a3 3 0 1 0 0-6 3 3 0 0 0 0 6Z" }
            path { d: "M20 4 8.12 15.88M14.47 14.48 20 20M8.12 8.12 12 12" }
        }
    }
}

fn IconFilm() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "1.8",
            rect { x: "3", y: "5", width: "18", height: "14", rx: "2" }
            path { d: "M7 5v14M17 5v14M3 9h4M3 15h4M17 9h4M17 15h4" }
        }
    }
}

fn IconType() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "1.8",
            path { d: "M4 7V5h16v2M12 5v14M8 19h8" }
        }
    }
}

fn IconCaptions() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "1.8",
            rect { x: "3", y: "6", width: "18", height: "12", rx: "2" }
            path { d: "M7 13h3M14 13h3M7 16h10" }
        }
    }
}

fn IconWave() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "1.8",
            path { d: "M4 12h2v0M8 8v8M12 5v14M16 9v6M20 11v2" }
        }
    }
}

fn IconShapes() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "1.8",
            rect { x: "3", y: "3", width: "8", height: "8", rx: "1" }
            circle { cx: "17", cy: "8", r: "4" }
            path { d: "M8 21 4 14h8l-4 7Zm8-1h5v-5" }
        }
    }
}

fn IconTransition() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "1.8",
            rect { x: "3", y: "6", width: "9", height: "12", rx: "1" }
            rect { x: "12", y: "6", width: "9", height: "12", rx: "1" }
            path { d: "M12 6v12" }
        }
    }
}

fn IconSliders() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "1.8",
            path { d: "M4 6h16M4 12h16M4 18h16M8 4v4M16 10v4M11 16v4" }
        }
    }
}

fn IconGear() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "1.8",
            circle { cx: "12", cy: "12", r: "3" }
            path { d: "M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.2a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.2a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.2a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9c.3.6.9 1 1.5 1.1H21a2 2 0 1 1 0 4h-.2a1.7 1.7 0 0 0-1.4 1Z" }
        }
    }
}

fn IconPlay() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "currentColor",
            path { d: "M8 5v14l11-7L8 5Z" }
        }
    }
}

fn IconPause() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "currentColor",
            rect { x: "6", y: "5", width: "4", height: "14" }
            rect { x: "14", y: "5", width: "4", height: "14" }
        }
    }
}

fn IconSkipBack() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "currentColor",
            path { d: "M6 5h2v14H6V5Zm3.5 7L18 19V5l-8.5 7Z" }
        }
    }
}

fn IconSkipFwd() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "currentColor",
            path { d: "M16 5h2v14h-2V5ZM6 5v14l8.5-7L6 5Z" }
        }
    }
}

fn IconSpark() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "1.8",
            path { d: "M12 3l1.6 5.2L19 10l-5.4 1.8L12 17l-1.6-5.2L5 10l5.4-1.8L12 3Zm6 10 0.8 2.4L21 16l-2.2.6L18 19l-.8-2.4L15 16l2.2-.6L18 13Z" }
        }
    }
}

fn IconChevRight() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "1.8",
            path { d: "M9 6l6 6-6 6" }
        }
    }
}

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
        let mixed = format!("{schema}\n\nthrough the drive, the newsroom, and home. I'm building that into one short.");
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
        assert!(!msgs.iter().any(|m| m.role == ChatRole::Bot && m.text.contains("$schema")));
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
        assert!(!msgs
            .iter()
            .any(|m| m.role == ChatRole::Bot && looks_like_tool_trace(&m.text)));
        append_model_chunk(
            &mut msgs,
            "I'll cut a 40 second vlog from the clips you imported.",
            false,
        );
        assert!(msgs.iter().any(|m| {
            m.role == ChatRole::Bot && m.text.contains("40 second")
        }));
        settle_reply(&mut msgs, &format!("{dump}\n\nI'll cut a 40 second vlog from the clips you imported."));
        assert_eq!(
            msgs.iter().filter(|m| m.role == ChatRole::Thought).count(),
            1
        );
        assert!(msgs.iter().any(|m| m.role == ChatRole::Bot && m.text.contains("40 second")));
        assert!(!msgs.iter().any(|m| m.role == ChatRole::Bot && m.text.contains("$schema")));
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
        assert!(msgs.iter().any(|m| m.role == ChatRole::Bot && m.text.contains("clips")));
        assert!(!msgs
            .iter()
            .any(|m| m.role == ChatRole::Thought && m.text.contains("clips")));
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
            msgs.iter().any(|m| m.role == ChatRole::Thought && !m.open && m.text.contains("scheduler_create")),
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
        assert!(msgs.iter().any(|m| m.role == ChatRole::Bot && m.text.contains("40 second")));
        fold_messages(&mut msgs);
        schema_is_hidden(&msgs);
        assert_eq!(msgs.iter().filter(|m| m.role == ChatRole::Thought).count(), 1);
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
        assert!(msgs.iter().any(|m| m.role == ChatRole::Bot && m.text.contains("40 second")));
        assert_eq!(msgs.iter().filter(|m| m.role == ChatRole::Thought).count(), 1);
    }

    #[test]
    fn acp_logs_are_dropped_from_the_chat() {
        let log = "acp: [2m2026-10-01T08:59:20Z[0m [32m INFO[0m sampling.request model=grok-4.7 auth_prefix=hidden sse_chunk encrypted_content api_backend=responses";
        assert!(is_acp_log(log));
        assert!(!is_acp_log("Sending to xai · grok-4.7…"));
        assert!(!is_acp_log("I'll cut a 40 second vlog from the clips you imported."));
        let mut msgs = vec![
            ChatMsg::user("edit like a pro editor"),
            ChatMsg::status(log.to_string()),
            ChatMsg::bot("I'll cut a 40 second vlog from the clips you imported."),
        ];
        fold_messages(&mut msgs);
        assert!(msgs.iter().all(|m| !m.text.contains("sampling.request") && !m.text.contains("auth_prefix")));
        assert!(msgs.iter().any(|m| m.role == ChatRole::Bot && m.text.contains("40 second")));
        assert!(msgs.iter().any(|m| m.role == ChatRole::User));
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
        assert!(saved.iter().all(|row| !row.text.contains("sampling.request")));
        let restored: Vec<_> = saved
            .into_iter()
            .enumerate()
            .filter_map(|(n, row)| msg_from_stored(row, n))
            .collect();
        assert!(restored.iter().any(|msg| msg.role == ChatRole::User));
        assert!(restored.iter().any(|msg| msg.role == ChatRole::Bot && msg.text.contains("40 second")));
    }

    #[test]
    fn the_open_chat_survives_a_stale_list() {
        let local = vec![api::ChatSummary {
            id: "new".into(),
            title: "New chat".into(),
        }];
        let remote = vec![api::ChatSummary {
            id: "old".into(),
            title: "Yesterday".into(),
        }];
        let merged = merge_remote_chats(remote, &local, Some("new"));
        assert_eq!(merged[0].id, "new");
        assert!(merged.iter().any(|row| row.id == "old"));
    }
}
