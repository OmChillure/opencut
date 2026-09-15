mod api;
mod auth;
mod bind;
mod media;
mod pages;
mod tools;

use dioxus::prelude::*;
use media::{
    Clock, DragSession, DragSource, EditMode, EditTool, EditorTrack, MediaItem, MediaKind,
    TimelineClip, TrackKindUi, advance_playhead, clip_duration, commit_drag,
    display_tracks, film_tiles, fit_scale, format_clock, format_tc_short,
    item_from_bytes_id, lane_height, next_track_name, paint_clock, paint_playhead, place_clip,
    playhead_now, preview_video, reset_tick_clock, ruler_marks_nle, scroll_left,
    seek_by, max_timeline_h, set_media_duration, sync_monitor, timeline_end,
    timeline_viewport_h, timeline_viewport_w, update_drag, wave_bars,
    capture_pointer, clamp_pps, video_duration_from_src,
};
use oc_core::{Op, Timeline as EngineTimeline, TrackKind};
use oc_core::TimelineEditMode;
use oc_tools::{ToolId, actions as cut_actions, modes as edit_tools, track_actions};
use pages::{Login, NewProject, Projects};

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
}

#[derive(Clone, Copy, PartialEq)]
enum AssetTab {
    Media,
    Text,
    Captions,
    Audio,
    Elements,
    Visuals,
    Settings,
}

impl AssetTab {
    const ALL: [Self; 7] = [
        Self::Media,
        Self::Text,
        Self::Captions,
        Self::Audio,
        Self::Elements,
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

#[derive(Clone, Copy)]
pub(crate) struct WorkspaceSave {
    pub project_id: Signal<String>,
    pub tracks: Signal<Vec<EditorTrack>>,
    pub engine: Signal<EngineTimeline>,
    pub aspect: Signal<Aspect>,
    pub persist_q: Signal<Vec<Vec<Op>>>,
    pub persist_busy: Signal<bool>,
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
        Router::<Route> {}
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
    };

    use_context_provider(|| library);
    use_context_provider(|| tracks);
    use_context_provider(|| active);
    use_context_provider(|| project_id);
    use_context_provider(|| clock);
    use_context_provider(|| edit_mode);
    use_context_provider(|| edit_tool);
    use_context_provider(|| target_track);
    use_context_provider(|| drag);
    use_context_provider(|| pps);
    use_context_provider(|| suppress_seek);
    use_context_provider(|| tl_h);
    use_context_provider(|| tl_drag);
    use_context_provider(|| save);

    use_effect(move || {
        if !auth::is_signed_in() {
            nav.replace(Route::Login {});
        }
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
            }
            Err(_) => {
                if let Some(track) = engine.peek().first_track(TrackKind::Video) {
                    target_track.set(track.id.to_string());
                }
            }
        }
        if let Ok(remote) = api::list_media(&pid).await {
            let mut library = library;
            let mut merged = library.peek().clone();
            for item in remote {
                if let Some(existing) = merged.iter_mut().find(|m| m.id == item.id) {
                    if existing.url.is_empty() {
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
                let mut active = active;
                active.set(library.peek().first().map(|item| item.url.clone()));
            }
        }
    });

    use_future(move || async move {
        loop {
            gloo_timers::future::TimeoutFuture::new(33).await;
            if !*clock.playing.peek() {
                reset_tick_clock();
                continue;
            }
            let span = timeline_end(&tracks.peek()).max(*clock.duration.peek()).max(0.1);
            let before = playhead_now();
            sync_monitor(&library.peek(), &tracks.peek(), before, true);
            let now = if let Some(shot) = crate::media::clip_under(
                &tracks.peek(),
                &library.peek(),
                playhead_now().max(before),
            ) {
                if shot.kind == MediaKind::Video {
                    playhead_now()
                } else {
                    advance_playhead(span)
                }
            } else {
                advance_playhead(span)
            };
            paint_playhead(now);
            if now >= span - 0.001 {
                let mut playing = clock.playing;
                playing.set(false);
                let mut current = clock.current;
                current.set(now);
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
    let project_id = use_context::<Signal<String>>();
    rsx! {
        header { class: "header",
            div { class: "header-left",
                Link { to: Route::Projects {}, class: "logo", title: "Projects",
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
                    class: "btn-ghost",
                    title: "Delete this project",
                    onclick: move |_| {
                        let pid = project_id.peek().clone();
                        let title = name.peek().clone();
                        if !crate::pages::confirm_delete(&title) {
                            return;
                        }
                        let nav = navigator();
                        nav.replace(Route::Projects {});
                        spawn(async move {
                            let _ = api::delete_project(&pid).await;
                        });
                    },
                    "Delete"
                }
                button { class: "btn btn-primary", "Export" }
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
                    {asset_view(current)}
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
        AssetTab::Visuals => rsx! { IconSliders {} },
        AssetTab::Settings => rsx! { IconGear {} },
    }
}

fn asset_view(tab: AssetTab) -> Element {
    match tab {
        AssetTab::Media => rsx! { MediaPanel {} },
        AssetTab::Text => rsx! {
            div { class: "card-list",
                button { class: "card",
                    b { "Title" }
                    span { "Large heading overlay" }
                }
                button { class: "card",
                    b { "Subtitle" }
                    span { "Secondary line of text" }
                }
                button { class: "card",
                    b { "Lower third" }
                    span { "Name and title card" }
                }
            }
        },
        AssetTab::Captions => rsx! {
            div { class: "drop",
                IconCaptions {}
                strong { "Captions" }
                small { "Transcribe speech and drop a caption track on the timeline." }
                button { class: "btn btn-primary", "Generate captions" }
            }
        },
        AssetTab::Audio => rsx! {
            div { class: "card-list",
                button { class: "card",
                    b { "Voiceover" }
                    span { "Sarvam Bulbul TTS" }
                }
                button { class: "card",
                    b { "Music" }
                    span { "Import a soundtrack" }
                }
                button { class: "card",
                    b { "Duck music" }
                    span { "Lower bed under speech" }
                }
            }
        },
        AssetTab::Elements => rsx! {
            div { class: "card-list",
                button { class: "card",
                    b { "Stickers" }
                    span { "Emojis and shapes" }
                }
                button { class: "card",
                    b { "Transitions" }
                    span { "Cuts, dissolves, slides" }
                }
            }
        },
        AssetTab::Visuals => rsx! {
            div { class: "card-list",
                button { class: "card",
                    b { "Color" }
                    span { "Grade and exposure" }
                }
                button { class: "card",
                    b { "Effects" }
                    span { "Blur, grain, vignette" }
                }
            }
        },
        AssetTab::Settings => rsx! {
            div { class: "card-list",
                button { class: "card",
                    b { "Frame rate" }
                    span { "30 fps" }
                }
                button { class: "card",
                    b { "Background" }
                    span { "Black" }
                }
            }
        },
    }
}

#[component]
fn MediaPanel() -> Element {
    let mut library = use_context::<Signal<Vec<MediaItem>>>();
    let mut tracks = use_context::<Signal<Vec<EditorTrack>>>();
    let mut active = use_context::<Signal<Option<String>>>();
    let project_id = use_context::<Signal<String>>();
    let clock = use_context::<Clock>();
    let target_track = use_context::<Signal<String>>();
    let edit_mode = use_context::<Signal<EditMode>>();
    let mut drag = use_context::<Signal<Option<DragSession>>>();
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
                                    let pid = project_id.peek().clone();
                                    let ctype = MediaKind::mime(&name).to_string();
                                    let media_id = match crate::api::upload_media(
                                        &pid,
                                        &name,
                                        &ctype,
                                        bytes.to_vec(),
                                    )
                                    .await
                                    {
                                        Ok(id) => id,
                                        Err(_) => uuid::Uuid::now_v7().to_string(),
                                    };
                                    let Some(item) = item_from_bytes_id(name, &bytes, media_id)
                                    else {
                                        continue;
                                    };
                                    if matches!(item.kind, MediaKind::Video | MediaKind::Image)
                                        && active.read().is_none()
                                    {
                                        active.set(Some(item.url.clone()));
                                    }
                                    let pid2 = pid.clone();
                                    let sync = item.clone();
                                    let file_bytes = bytes.to_vec();
                                    library.write().push(item);
                                    spawn(async move {
                                        let _ = crate::api::register_media(
                                            &pid2,
                                            &sync.id,
                                            &sync.name,
                                            &sync.content_type,
                                            sync.duration,
                                        )
                                        .await;
                                        let _ = crate::api::put_media_bytes(
                                            &pid2,
                                            &sync.id,
                                            &sync.content_type,
                                            file_bytes,
                                        )
                                        .await;
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
    let _ = playing;

    use_effect(move || {
        let playing = *clock.playing.read();
        if playing {
            return;
        }
        let now = *clock.current.read();
        sync_monitor(&library.read(), &tracks.read(), now, false);
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
                div { class: current.class(),
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
                                let end = timeline_end(&tracks.read());
                                if end > *clock.duration.peek() {
                                    let mut duration = clock.duration;
                                    duration.set(end);
                                }
                                let now = playhead_now();
                                if let Some(shot) = crate::media::clip_under(
                                    &tracks.read(),
                                    &library.read(),
                                    now,
                                ) {
                                    video.set_current_time(
                                        (shot.source_in + (now - shot.start)).max(0.0),
                                    );
                                }
                            }
                            if changed {
                                persist(save);
                            }
                            paint_clock();
                        },
                    }
                    img { class: "preview-image off", alt: "" }
                    div { class: "monitor-blank",
                        span { class: "monitor-meta", "{size}" }
                    }
                }
            }
        }
    }
}

#[component]
fn Timeline() -> Element {
    let library = use_context::<Signal<Vec<MediaItem>>>();
    let mut tracks = use_context::<Signal<Vec<EditorTrack>>>();
    let clock = use_context::<Clock>();
    let mut target_track = use_context::<Signal<String>>();
    let mut edit_mode = use_context::<Signal<EditMode>>();
    let mut edit_tool = use_context::<Signal<EditTool>>();
    let mut selected_clip = use_signal(|| None::<String>);
    let mut drag = use_context::<Signal<Option<DragSession>>>();
    let mut pps = use_context::<Signal<f64>>();
    let mut suppress_seek = use_context::<Signal<bool>>();
    let mut tl_h = use_context::<Signal<f64>>();
    let mut tl_drag = use_context::<Signal<Option<(f64, f64)>>>();
    let save = use_context::<WorkspaceSave>();
    let mut view_h = use_signal(|| 280.0_f64);
    let mut view_w = use_signal(|| 800.0_f64);
    let mut trim = use_signal(|| None::<(String, bool)>);
    let now = *clock.current.read();
    let video_dur = *clock.duration.read();
    let span = timeline_end(&tracks.read()).max(video_dur).max(8.0);
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
                                                let name = item.as_ref().map(|m| m.name.clone()).unwrap_or_else(|| "Clip".into());
                                                let url = item.as_ref().map(|m| m.url.clone()).unwrap_or_default();
                                                let media_kind = item.as_ref().map(|m| m.kind).unwrap_or(MediaKind::Video);
                                                let is_video = media_kind == MediaKind::Video;
                                                let is_image = media_kind == MediaKind::Image;
                                                let is_audio = media_kind == MediaKind::Audio
                                                    || track.kind == TrackKindUi::Audio;
                                                let selected = selected_clip.read().as_deref() == Some(clip.id.as_str());
                                                let lifted = lifted_id.as_deref() == Some(clip.id.as_str());
                                                let clip_class = if lifted {
                                                    format!("nle-clip {kind_class} lifted")
                                                } else if clip.disabled {
                                                    format!("nle-clip {kind_class} dim")
                                                } else if selected {
                                                    format!("nle-clip {kind_class} on")
                                                } else {
                                                    format!("nle-clip {kind_class}")
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
                                                let bars = if is_audio {
                                                    wave_bars(&name, (tiles as usize * 5).max(12))
                                                } else {
                                                    Vec::new()
                                                };
                                                rsx! {
                                                    div {
                                                        class: "{clip_class}",
                                                        style: "left: {left}px; width: {width}px",
                                                        title: "{name}",
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
                                                        div { class: "nle-strip",
                                                            if is_audio {
                                                                div { class: "nle-wave",
                                                                    for bar in bars.iter() {
                                                                        span {
                                                                            class: "nle-bar",
                                                                            style: "height: {bar}%"
                                                                        }
                                                                    }
                                                                }
                                                            } else {
                                                                for _ in 0..tiles {
                                                                    div { class: "nle-cell",
                                                                        if is_image {
                                                                            img { src: "{url}", alt: "" }
                                                                        } else if is_video {
                                                                            video {
                                                                                src: "{url}",
                                                                                muted: true,
                                                                                preload: "metadata"
                                                                            }
                                                                        }
                                                                    }
                                                                }
                                                            }
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
    match kind {
        TrackKindUi::Video | TrackKindUi::Caption => tracks.write().insert(0, track),
        TrackKindUi::Audio => tracks.write().push(track),
    }
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
                        let label = library
                            .read()
                            .iter()
                            .find(|item| item.id == clip.media_id)
                            .map(|item| item.name.clone())
                            .unwrap_or_else(|| "Clip".into());
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
    messages: Signal<Vec<ChatMsg>>,
    ai_open: Signal<bool>,
    ai_width: Signal<f64>,
    mut ai_drag: Signal<Option<(f64, f64)>>,
) -> Element {


    let save = use_context::<WorkspaceSave>();
    let clock = use_context::<Clock>();
    let target_track = use_context::<Signal<String>>();
    let mut picker_open = use_signal(|| false);
    let mut providers = use_signal(Vec::<api::AiProvider>::new);
    let mut provider_id = use_signal(|| "xai".to_string());
    let mut model_id = use_signal(|| "grok-4.6".to_string());
    let mut model_name = use_signal(|| "Grok 4.6".to_string());
    let mut busy = use_signal(|| false);

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

    rsx! {
        aside {
            class: if *ai_open.read() { "ai" } else { "ai collapsed" },
            style: if *ai_open.read() {
                format!("width: {}px", *ai_width.read())
            } else {
                String::new()
            },
            if *ai_open.read() {
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
                    title: if *ai_open.read() { "Collapse AI" } else { "Open AI Studio" },
                    onclick: move |_| {
                        let next = !*ai_open.read();
                        ai_open.set(next);
                    },
                    if *ai_open.read() {
                        IconChevRight {}
                    } else {
                        IconSpark {}
                    }
                }
                div { class: "ai-title ai-copy",
                    span { "AI Studio" }
                    span { class: "badge", "Beta" }
                }
                span { class: "kbd", "Ctrl+K" }
            }
            div { class: "ai-body",
                if messages.read().is_empty() {
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
                        for msg in messages.read().iter() {
                            match msg.role {
                                ChatRole::User => rsx! {
                                    div { class: "bubble user", "{msg.text}" }
                                },
                                ChatRole::Bot => rsx! {
                                    div { class: "bubble bot", "{msg.text}" }
                                },
                                ChatRole::Status => rsx! {
                                    div { class: "bubble status", "{msg.text}" }
                                },
                                ChatRole::Tool => rsx! {
                                    div { class: "bubble tool",
                                        div { class: "tool-head",
                                            span { class: "tool-k", "tool" }
                                            span { class: "tool-name", "{msg.tool_name}" }
                                            span {
                                                class: "tool-status {msg.tool_status}",
                                                "{msg.tool_status}"
                                            }
                                        }
                                        if !msg.tool_args.is_empty() {
                                            pre { class: "tool-args", "{msg.tool_args}" }
                                        }
                                        if !msg.tool_result.is_empty() {
                                            pre { class: "tool-result", "{msg.tool_result}" }
                                        }
                                    }
                                },
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
                                );
                            }
                        },
                    }
                    button {
                        class: "send",
                        disabled: draft.read().trim().is_empty() || *busy.read(),
                        onclick: move |_| send_prompt(
                            draft,
                            messages,
                            save,
                            clock,
                            target_track,
                            provider_id,
                            model_id,
                            busy,
                        ),
                        IconSend {}
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
            if ev.text.trim().is_empty() {
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
            let text = strip_tool_lines(&ev.text);
            if text.is_empty() {
                return;
            }
            let mut list = messages.write();
            if let Some(last) = list.last_mut().filter(|m| m.role == ChatRole::Bot) {
                last.text.push_str(&text);
            } else {
                list.push(ChatMsg::bot(text));
            }
        }
        "tool" => upsert_tool(messages, ev),
        "note" => {
            if !ev.text.is_empty() {
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
        .find(|m| m.role == ChatRole::Tool && (m.tool_id == id || (id.is_empty() && m.tool_name == ev.name)))
    {
        existing.tool_status = ev.status;
        if !args.is_empty() {
            existing.tool_args = args;
        }
        if !result.is_empty() {
            existing.tool_result = result;
        }
        return;
    }
    list.push(ChatMsg {
        role: ChatRole::Tool,
        text: ev.name.clone(),
        tool_id: id,
        tool_name: ev.name,
        tool_status: if ev.status.is_empty() {
            "pending".into()
        } else {
            ev.status
        },
        tool_args: args,
        tool_result: result,
    });
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

fn clear_status(mut messages: Signal<Vec<ChatMsg>>) {
    messages
        .write()
        .retain(|m| m.role != ChatRole::Status);
}

fn finish_bot_text(mut messages: Signal<Vec<ChatMsg>>, text: String) {
    clear_status(messages);
    if text.trim().is_empty() {
        return;
    }
    let mut list = messages.write();
    if let Some(last) = list.last_mut().filter(|m| m.role == ChatRole::Bot) {
        if last.text.trim().is_empty() {
            last.text = text;
        }
        return;
    }
    list.push(ChatMsg::bot(text));
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
            return;
        }
    }
    busy.set(true);
    let pid = save.project_id.peek().clone();
    let provider = provider_id.peek().clone();
    let model = model_id.peek().clone();
    let history: Vec<(bool, String)> = messages
        .peek()
        .iter()
        .filter(|m| matches!(m.role, ChatRole::User | ChatRole::Bot))
        .map(|m| (m.role == ChatRole::User, m.text.clone()))
        .collect();
    let bin: Vec<(String, String, String, f64, String)> = use_context::<Signal<Vec<MediaItem>>>()
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
    spawn(async move {
        for (id, name, ctype, dur, url) in &bin {
            let _ = api::register_media(&pid, id, name, ctype, *dur).await;
            if url.starts_with("blob:") {
                if let Ok(resp) = reqwest::Client::new().get(url).send().await {
                    if let Ok(bytes) = resp.bytes().await {
                        let _ = api::put_media_bytes(&pid, id, ctype, bytes.to_vec()).await;
                    }
                }
            }
        }
        let reply = api::chat_stream(&pid, &provider, &model, &history, |ev| {
            apply_chat_event(messages, ev);
        })
        .await;
        match reply {
            Ok(resp) => {
                save.engine.set(resp.timeline.clone());
                save.tracks.set(bind::tracks_from_timeline(&resp.timeline));
                let end = resp.timeline.duration().as_seconds();
                if end > 0.0 {
                    let mut duration = clock.duration;
                    duration.set(end);
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
        busy.set(false);
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
