//! Editor surfaces modeled on Kdenlive: audio mixer, curves, alpha shapes,
//! scopes, generators, time remap, undo history, multicam, and the rendered file.

use crate::media::{Clock, MediaItem, preview_video};
use crate::tools;
use crate::WorkspaceSave;
use dioxus::prelude::*;
use oc_core::{
    AlphaShape, CurvePoint, Curves, Generator, MaskShape, Mix, SpeedKey, TrackKind,
};
use oc_tools::ToolId;
use std::cell::RefCell;
use std::collections::HashMap;
use wasm_bindgen::JsCast;
use web_sys::HtmlCanvasElement;

thread_local! {
    static WAVES: RefCell<HashMap<String, Vec<u8>>> = RefCell::new(HashMap::new());
}

#[component]
pub fn Mixer() -> Element {
    let save = use_context::<WorkspaceSave>();
    let timeline = save.engine.read();
    let tracks: Vec<_> = timeline
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Audio)
        .map(|t| (t.id, t.name.clone(), t.muted, t.mix))
        .collect();
    let master = timeline.master.gain_db;
    rsx! {
        div { class: "mixer",
            div { class: "mixer-title", "Audio mixer" }
            div { class: "mixer-strips",
                for (id, name, muted, mix) in tracks {
                    MixerStrip {
                        key: "{id}",
                        track_id: id.to_string(),
                        name,
                        muted,
                        mix,
                    }
                }
                div { class: "mixer-strip master",
                    span { class: "mixer-name", "Master" }
                    span { class: "mixer-db", "{master:+.1} dB" }
                    input {
                        r#type: "range",
                        min: "-60",
                        max: "12",
                        step: "0.5",
                        value: "{master}",
                        title: "Master volume. Right-click resets to 0 dB.",
                        oninput: move |evt| {
                            let db = evt.value().parse().unwrap_or(0.0);
                            let _ = tools::set_mix(save, None, Mix { gain_db: db, ..Mix::default() });
                        },
                        oncontextmenu: move |evt| {
                            evt.prevent_default();
                            let _ = tools::set_mix(save, None, Mix::default());
                        },
                    }
                }
            }
            p { class: "mixer-note",
                "Solo is exclusive. Shift-click adds a solo. Right-click a fader to reset it."
            }
        }
    }
}

#[component]
fn MixerStrip(track_id: String, name: String, muted: bool, mix: Mix) -> Element {
    let save = use_context::<WorkspaceSave>();
    let id_gain = track_id.clone();
    let id_pan = track_id.clone();
    let id_reset = track_id.clone();
    let id_solo = track_id.clone();
    let id_mute = track_id;
    let gain = mix.gain_db;
    let pan = mix.pan;
    let solo = mix.solo;
    rsx! {
        div { class: "mixer-strip",
            span { class: "mixer-name", "{name}" }
            div { class: "mixer-flags",
                button {
                    class: if muted { "mix-flag on" } else { "mix-flag" },
                    title: "Mute",
                    onclick: move |_| {
                        let _ = tools::run_ops(save, vec![oc_core::Op::SetTrackFlags {
                            track_id: parse_track(&id_mute),
                            muted: !muted,
                            hidden: false,
                        }]);
                    },
                    "M"
                }
                button {
                    class: if solo { "mix-flag on" } else { "mix-flag" },
                    title: "Solo. Shift-click adds another solo.",
                    onclick: move |evt| {
                        let shift = evt.modifiers().shift();
                        let tl = save.engine.peek();
                        let me = parse_track(&id_solo);
                        for track in tl.tracks.iter().filter(|t| t.kind == TrackKind::Audio) {
                            let on = if track.id == me {
                                !track.mix.solo
                            } else if shift {
                                track.mix.solo
                            } else {
                                false
                            };
                            if on != track.mix.solo {
                                let _ = tools::set_mix(save, Some(track.id), Mix { solo: on, ..track.mix });
                            }
                        }
                    },
                    "S"
                }
            }
            span { class: "mixer-db", "{gain:+.1} dB" }
            input {
                r#type: "range",
                min: "-60",
                max: "12",
                step: "0.5",
                value: "{gain}",
                title: "Volume. Right-click resets to 0 dB.",
                oninput: move |evt| {
                    let db = evt.value().parse().unwrap_or(0.0);
                    let _ = tools::set_mix(save, Some(parse_track(&id_gain)), Mix { gain_db: db, pan, solo, });
                },
                oncontextmenu: move |evt| {
                    evt.prevent_default();
                    let _ = tools::set_mix(save, Some(parse_track(&id_reset)), Mix { gain_db: 0.0, pan, solo });
                },
            }
            input {
                r#type: "range",
                min: "-1",
                max: "1",
                step: "0.05",
                value: "{pan}",
                title: "Balance. Left is −1, right is +1.",
                oninput: move |evt| {
                    let pan = evt.value().parse().unwrap_or(0.0);
                    let _ = tools::set_mix(save, Some(parse_track(&id_pan)), Mix { gain_db: gain, pan, solo });
                },
            }
        }
    }
}

fn parse_track(raw: &str) -> oc_core::TrackId {
    uuid::Uuid::parse_str(raw)
        .map(oc_core::TrackId::from_uuid)
        .unwrap_or_else(|_| oc_core::TrackId::new())
}

#[component]
pub fn CurvesPanel(selected: String, track: String, at: f64) -> Element {
    let save = use_context::<WorkspaceSave>();
    let mut channel = use_signal(|| "all".to_string());
    let mut mid = use_signal(|| 0.5_f32);
    let ch = channel.read().clone();
    let selected_reset = selected.clone();
    let track_reset = track.clone();
    rsx! {
        div { class: "card-list",
            div { class: "mixer-title", "Curves" }
            div { class: "mixer-flags",
                for name in ["all", "red", "green", "blue"] {
                    button {
                        class: if ch == name { "mix-flag on" } else { "mix-flag" },
                        onclick: move |_| channel.set(name.to_string()),
                        "{name}"
                    }
                }
            }
            input {
                r#type: "range",
                min: "0",
                max: "1",
                step: "0.02",
                value: "{mid}",
                title: "Output of the midpoint. 0.5 is a straight line.",
                oninput: move |evt| {
                    let y = evt.value().parse().unwrap_or(0.5);
                    mid.set(y);
                    let points = vec![
                        CurvePoint { x: 0.0, y: 0.0 },
                        CurvePoint { x: 0.5, y },
                        CurvePoint { x: 1.0, y: 1.0 },
                    ];
                    let curves = match channel.peek().as_str() {
                        "red" => Curves { red: points, ..Curves::default() },
                        "green" => Curves { green: points, ..Curves::default() },
                        "blue" => Curves { blue: points, ..Curves::default() },
                        _ => Curves { all: points, ..Curves::default() },
                    };
                    let _ = tools::set_curves_at(save, Some(&selected), &track, at, curves);
                },
            }
            button {
                class: "btn btn-ghost",
                onclick: move |_| {
                    mid.set(0.5);
                    let _ = tools::set_curves_at(save, Some(&selected_reset), &track_reset, at, Curves::default());
                },
                "Reset curve"
            }
        }
    }
}

#[component]
pub fn MaskPanel(selected: String, track: String, at: f64) -> Element {
    let save = use_context::<WorkspaceSave>();
    let mut shape = use_signal(|| MaskShape::Rectangle);
    let mut feather = use_signal(|| 0.0_f32);
    let mut invert = use_signal(|| false);
    let selected_clear = selected.clone();
    let track_clear = track.clone();
    rsx! {
        div { class: "card-list",
            div { class: "mixer-title", "Alpha shapes" }
            div { class: "mixer-flags",
                for item in [MaskShape::Rectangle, MaskShape::Ellipse, MaskShape::Triangle, MaskShape::Diamond] {
                    button {
                        class: if *shape.read() == item { "mix-flag on" } else { "mix-flag" },
                        onclick: move |_| shape.set(item),
                        "{shape_name(item)}"
                    }
                }
            }
            label { class: "mixer-note",
                "Feather"
                input {
                    r#type: "range",
                    min: "0",
                    max: "1",
                    step: "0.05",
                    value: "{feather}",
                    oninput: move |evt| feather.set(evt.value().parse().unwrap_or(0.0)),
                }
            }
            button {
                class: if *invert.read() { "mix-flag on" } else { "mix-flag" },
                onclick: move |_| {
                    let next = !*invert.peek();
                    invert.set(next);
                },
                "Invert"
            }
            button {
                class: "btn btn-primary",
                onclick: move |_| {
                    let mask = AlphaShape {
                        shape: *shape.peek(),
                        x: 0.5,
                        y: 0.5,
                        w: 0.5,
                        h: 0.5,
                        feather: *feather.peek(),
                        invert: *invert.peek(),
                    };
                    let _ = tools::set_mask_at(save, Some(&selected), &track, at, Some(mask));
                },
                "Apply mask"
            }
            button {
                class: "btn btn-ghost",
                onclick: move |_| {
                    let _ = tools::set_mask_at(save, Some(&selected_clear), &track_clear, at, None);
                },
                "Clear mask"
            }
        }
    }
}

fn shape_name(shape: MaskShape) -> &'static str {
    match shape {
        MaskShape::Rectangle => "Rectangle",
        MaskShape::Ellipse => "Ellipse",
        MaskShape::Triangle => "Triangle",
        MaskShape::Diamond => "Diamond",
    }
}

#[component]
pub fn TimeRemap(selected: String, track: String, at: f64) -> Element {
    let save = use_context::<WorkspaceSave>();
    let mut mid_speed = use_signal(|| 1.0_f32);
    let mut end_speed = use_signal(|| 1.0_f32);
    rsx! {
        div { class: "card-list",
            div { class: "mixer-title", "Time remap" }
            label { class: "mixer-note",
                "Speed at the middle"
                input {
                    r#type: "range", min: "0.25", max: "4", step: "0.05", value: "{mid_speed}",
                    oninput: move |evt| mid_speed.set(evt.value().parse().unwrap_or(1.0)),
                }
            }
            label { class: "mixer-note",
                "Speed at the end"
                input {
                    r#type: "range", min: "0.25", max: "4", step: "0.05", value: "{end_speed}",
                    oninput: move |evt| end_speed.set(evt.value().parse().unwrap_or(1.0)),
                }
            }
            button {
                class: "btn btn-primary",
                onclick: move |_| {
                    let keys = vec![
                        SpeedKey { at: 0.0, speed: 1.0 },
                        SpeedKey { at: 0.5, speed: *mid_speed.peek() },
                        SpeedKey { at: 1.0, speed: *end_speed.peek() },
                    ];
                    let _ = tools::set_speed_keys_at(save, Some(&selected), &track, at, keys);
                },
                "Set keyframes"
            }
        }
    }
}

#[component]
pub fn Generators(at: f64) -> Element {
    let save = use_context::<WorkspaceSave>();
    rsx! {
        div { class: "card-list",
            div { class: "mixer-title", "Generators" }
            button {
                class: "card",
                onclick: move |_| {
                    let _ = tools::add_generator(save, Generator::Color { color: "#111111".into() }, at, 5.0);
                },
                b { "Color clip" }
                span { "A solid frame, like Kdenlive's color clip" }
            }
            button {
                class: "card",
                onclick: move |_| {
                    let _ = tools::add_generator(save, Generator::ColorBars, at, 5.0);
                },
                b { "Color bars" }
                span { "SMPTE bars" }
            }
            button {
                class: "card",
                onclick: move |_| {
                    let _ = tools::add_generator(save, Generator::WhiteNoise, at, 5.0);
                },
                b { "White noise" }
                span { "Snow, with a noise bed" }
            }
            button {
                class: "card",
                onclick: move |_| {
                    let _ = tools::add_generator(save, Generator::Counter, at, 8.0);
                },
                b { "Counter" }
                span { "A running clock and a 1 kHz tick" }
            }
        }
    }
}

#[component]
pub fn UndoHistory() -> Element {
    let save = use_context::<WorkspaceSave>();
    let stack = save.undo.read();
    let labels = stack.labels();
    let redo = stack.redo_labels();
    let depth = stack.depth();
    rsx! {
        div { class: "card-list",
            div { class: "mixer-title", "Undo history" }
            button { class: "btn btn-ghost", onclick: move |_| { let _ = tools::undo_edit(save); }, "Undo" }
            button { class: "btn btn-ghost", onclick: move |_| { let _ = tools::redo_edit(save); }, "Redo" }
            for (i, label) in labels.iter().enumerate() {
                button {
                    class: if i + 1 == depth { "card on" } else { "card" },
                    onclick: move |_| { let _ = tools::jump_history(save, i + 1); },
                    "{label}"
                }
            }
            if labels.is_empty() {
                p { class: "mixer-note", "No changes yet. Opening a project starts a fresh history." }
            }
            for label in redo.iter() {
                p { class: "mixer-note", "redo · {label}" }
            }
            button {
                class: "btn btn-ghost",
                title: "Clears the history. The timeline stays as it is.",
                onclick: move |_| tools::clear_history(save),
                "Clear history"
            }
        }
    }
}

#[component]
pub fn Scopes() -> Element {
    let mut open = use_signal(|| false);
    if *open.read() {
        ensure_scopes();
    }
    rsx! {
        div { class: "scopes",
            button {
                class: "btn btn-ghost",
                onclick: move |_| {
                    let next = !*open.peek();
                    open.set(next);
                },
                "Scopes"
            }
            if *open.read() {
                canvas {
                    id: "scope-canvas",
                    width: "280",
                    height: "120",
                    class: "scope-canvas",
                }
                p { class: "mixer-note", "Waveform, RGB parade, and vectorscope of the current monitor frame." }
            }
        }
    }
}

fn ensure_scopes() {
    use std::sync::Once;
    static STARTED: Once = Once::new();
    STARTED.call_once(|| {
        spawn(async move {
            loop {
                gloo_timers::future::TimeoutFuture::new(400).await;
                paint_scopes();
            }
        });
    });
}

pub fn paint_scopes() {
    let Some(video) = preview_video() else {
        return;
    };
    if video.ready_state() < 2 {
        return;
    }
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let Some(canvas) = doc
        .get_element_by_id("scope-canvas")
        .and_then(|el| el.dyn_into::<HtmlCanvasElement>().ok())
    else {
        return;
    };
    let Ok(ctx) = canvas.get_context("2d") else {
        return;
    };
    let Some(ctx) = ctx.and_then(|c| c.dyn_into::<web_sys::CanvasRenderingContext2d>().ok()) else {
        return;
    };
    let w = canvas.width() as f64;
    let h = canvas.height() as f64;
    let _ = ctx.draw_image_with_html_video_element_and_dw_and_dh(&video, 0.0, 0.0, 80.0, 45.0);
    let Ok(data) = ctx.get_image_data(0.0, 0.0, 80.0, 45.0) else {
        return;
    };
    let px = data.data();
    ctx.set_fill_style_str("#111");
    ctx.fill_rect(0.0, 0.0, w, h);
    // Waveform (luma) on the left third, parade in the middle, vectorscope on the right.
    for y in 0..45 {
        for x in 0..80 {
            let i = ((y * 80 + x) * 4) as usize;
            if i + 2 >= px.len() {
                continue;
            }
            let r = px[i] as f64;
            let g = px[i + 1] as f64;
            let b = px[i + 2] as f64;
            let luma = (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255.0;
            ctx.set_fill_style_str("rgba(180,220,180,0.35)");
            ctx.fill_rect(x as f64 * 0.9, (1.0 - luma) * (h - 4.0), 1.0, 1.0);
            ctx.set_fill_style_str("rgba(220,80,80,0.45)");
            ctx.fill_rect(90.0 + (r / 255.0) * 50.0, y as f64 * 2.4, 1.0, 1.0);
            ctx.set_fill_style_str("rgba(80,200,80,0.45)");
            ctx.fill_rect(90.0 + (g / 255.0) * 50.0, y as f64 * 2.4 + 1.0, 1.0, 1.0);
            ctx.set_fill_style_str("rgba(80,120,220,0.45)");
            ctx.fill_rect(90.0 + (b / 255.0) * 50.0, y as f64 * 2.4 + 2.0, 1.0, 1.0);
            let uu = (b - luma * 255.0) / 255.0;
            let vv = (r - luma * 255.0) / 255.0;
            ctx.set_fill_style_str("rgba(240,220,120,0.8)");
            ctx.fill_rect(210.0 + uu * 28.0, 60.0 - vv * 28.0, 1.5, 1.5);
        }
    }
}

#[component]
pub fn MulticamBank() -> Element {
    let save = use_context::<WorkspaceSave>();
    let tool = use_context::<Signal<ToolId>>();
    let library = use_context::<Signal<Vec<MediaItem>>>();
    let clock = use_context::<Clock>();
    if *tool.read() != ToolId::Multicam {
        return rsx! {};
    }
    let at = *clock.current.read();
    let tl = save.engine.read();
    let angles: Vec<_> = tl
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Video)
        .enumerate()
        .filter_map(|(i, track)| {
            let clip = track.clips.iter().find(|c| c.contains(oc_core::Time::from_seconds(at)) && !c.disabled)?;
            let url = clip.media_id.and_then(|id| {
                library.read().iter().find(|m| m.id == id.to_string()).map(|m| m.url.clone())
            }).unwrap_or_default();
            Some((i + 1, track.id.to_string(), track.name.clone(), url, clip.disabled))
        })
        .collect();
    rsx! {
        div { class: "multicam",
            div { class: "mixer-title", "Multitrack" }
            div { class: "multicam-grid",
                for (n, id, name, url, _off) in angles {
                    button {
                        class: "multicam-cell",
                        title: "Cut to this angle. Keys 1–9 do the same while this tool is on.",
                        onclick: move |_| {
                            let _ = tools::multicam_at(save, &id, at);
                        },
                        if !url.is_empty() {
                            video { src: "{url}", muted: true, preload: "metadata" }
                        }
                        span { "{n} {name}" }
                    }
                }
            }
            p { class: "mixer-note", "Audio stays on the track you already mixed. Cutting only changes the picture." }
        }
    }
}

#[component]
pub fn ExportPlayer() -> Element {
    let save = use_context::<WorkspaceSave>();
    let mut url = use_signal(String::new);
    let mut note = use_signal(|| String::new());
    rsx! {
        div { class: "export-play",
            button {
                class: "btn btn-primary",
                onclick: move |_| {
                    let tl = save.engine.peek();
                    let preset = if tl.height > tl.width {
                        oc_core::ExportPreset::Vertical1080
                    } else if tl.width == tl.height {
                        oc_core::ExportPreset::Square1080
                    } else {
                        oc_core::ExportPreset::Youtube1080
                    };
                    match tools::run_ops(save, vec![oc_core::Op::Export { preset }]) {
                        Ok(_) => {
                            note.set("Rendering. The file plays here when the worker finishes.".into());
                            let pid = save.project_id.peek().clone();
                            spawn(async move {
                                for _ in 0..60 {
                                    gloo_timers::future::TimeoutFuture::new(2000).await;
                                    let file = format!("http://127.0.0.1:8787/v1/projects/{pid}/export");
                                    if reqwest::Client::new().head(&file).send().await.ok().is_some_and(|r| r.status().is_success()) {
                                        url.set(file);
                                        note.set("Play after render".into());
                                        return;
                                    }
                                }
                                note.set("The render did not show up yet. Check the worker.".into());
                            });
                        }
                        Err(err) => note.set(err),
                    }
                },
                "Export"
            }
            if !note.read().is_empty() {
                span { class: "mixer-note", "{note}" }
            }
            if !url.read().is_empty() {
                video {
                    class: "export-video",
                    src: "{url}",
                    controls: true,
                    playsinline: true,
                }
            }
        }
    }
}

#[component]
pub fn Filmstrip(url: String, tiles: i32, ends_only: bool) -> Element {
    let n = if ends_only { 2 } else { tiles.max(1) };
    let host = use_signal(|| format!("film-{}", uuid::Uuid::now_v7().simple()));
    let host_id = host.read().clone();
    let src = url.clone();
    use_effect(move || {
        let src = src.clone();
        let host_id = host_id.clone();
        if src.is_empty() {
            return;
        }
        spawn(async move {
            paint_film(&src, n, &host_id).await;
        });
    });
    rsx! {
        div { class: "nle-strip", id: "{host}",
            for _ in 0..n {
                canvas { class: "nle-cell", width: "48", height: "36" }
            }
        }
    }
}

async fn paint_film(url: &str, tiles: i32, host: &str) {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let Ok(Some(root)) = doc.query_selector(&format!("#{host}")) else {
        return;
    };
    let Ok(list) = root.query_selector_all("canvas") else {
        return;
    };
    let Ok(video) = doc.create_element("video") else {
        return;
    };
    let Ok(video) = video.dyn_into::<web_sys::HtmlVideoElement>() else {
        return;
    };
    video.set_src(url);
    video.set_muted(true);
    let mut dur = 0.0;
    for _ in 0..30 {
        gloo_timers::future::TimeoutFuture::new(100).await;
        dur = video.duration();
        if dur.is_finite() && dur > 0.0 {
            break;
        }
    }
    if !dur.is_finite() || dur <= 0.0 {
        return;
    }
    let tiles = tiles.max(1) as u32;
    for i in 0..tiles.min(list.length()) {
        let Some(node) = list.item(i) else { continue };
        let Ok(canvas) = node.dyn_into::<HtmlCanvasElement>() else { continue };
        let at = if tiles == 1 {
            0.0
        } else {
            dur * f64::from(i) / f64::from(tiles - 1)
        };
        let _ = video.set_current_time(at.min(dur - 0.05).max(0.0));
        gloo_timers::future::TimeoutFuture::new(40).await;
        if let Some(ctx) = canvas
            .get_context("2d")
            .ok()
            .flatten()
            .and_then(|c| c.dyn_into::<web_sys::CanvasRenderingContext2d>().ok())
        {
            let _ = ctx.draw_image_with_html_video_element_and_dw_and_dh(&video, 0.0, 0.0, 48.0, 36.0);
        }
    }
}

#[component]
pub fn Waveform(url: String, bars: usize, seed: String) -> Element {
    let mut rev = use_signal(|| 0u32);
    let _redraw = *rev.read();
    let cached = WAVES.with(|m| m.borrow().get(&url).cloned());
    let peaks = cached.unwrap_or_else(|| crate::media::wave_bars(&seed, bars));
    let src = url.clone();
    use_effect(move || {
        let src = src.clone();
        if src.is_empty() || WAVES.with(|m| m.borrow().contains_key(&src)) {
            return;
        }
        spawn(async move {
            if let Some(peaks) = decode_peaks(&src, bars.max(8)).await {
                WAVES.with(|m| {
                    m.borrow_mut().insert(src, peaks);
                });
                let next = rev.peek().saturating_add(1);
                rev.set(next);
            }
        });
    });
    rsx! {
        div { class: "nle-wave",
            for bar in peaks.iter() {
                span { class: "nle-bar", style: "height: {bar}%" }
            }
        }
    }
}

async fn decode_peaks(url: &str, bars: usize) -> Option<Vec<u8>> {
    let win = web_sys::window()?;
    let resp = wasm_bindgen_futures::JsFuture::from(win.fetch_with_str(url)).await.ok()?;
    let resp: web_sys::Response = resp.dyn_into().ok()?;
    let buf = wasm_bindgen_futures::JsFuture::from(resp.array_buffer().ok()?).await.ok()?;
    let buf: js_sys::ArrayBuffer = buf.dyn_into().ok()?;
    let ctx = web_sys::AudioContext::new().ok()?;
    let audio = wasm_bindgen_futures::JsFuture::from(ctx.decode_audio_data(&buf).ok()?).await.ok()?;
    let audio: web_sys::AudioBuffer = audio.dyn_into().ok()?;
    let data = audio.get_channel_data(0).ok()?;
    let bars = bars.max(1);
    let step = (data.len() / bars).max(1);
    let mut peaks = Vec::with_capacity(bars);
    let mut i = 0;
    while peaks.len() < bars && i < data.len() {
        let end = (i + step).min(data.len());
        let mut peak = 0.0_f32;
        for s in i..end {
            peak = peak.max(data.get(s).copied().unwrap_or(0.0).abs());
        }
        peaks.push((peak * 100.0).clamp(8.0, 100.0) as u8);
        i = end;
    }
    Some(peaks)
}
