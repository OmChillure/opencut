//! Editor surfaces modeled on Kdenlive: audio mixer, curves, alpha shapes,
//! scopes, generators, time remap, undo history, multicam, and the rendered file.

use crate::WorkspaceSave;
use crate::media::{self, Clock, MediaItem, capture_pointer};
use crate::tools;
use dioxus::prelude::*;
use oc_core::{AlphaShape, CurvePoint, Curves, Generator, MaskShape, Mix, SpeedKey, TrackKind};
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
                    MasterMeter {}
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
                        let me = parse_track(&id_solo);
                        let changes: Vec<_> = {
                            let tl = save.engine.peek();
                            tl.tracks
                                .iter()
                                .filter(|t| t.kind == TrackKind::Audio)
                                .filter_map(|track| {
                                    let on = if track.id == me {
                                        !track.mix.solo
                                    } else if shift {
                                        track.mix.solo
                                    } else {
                                        false
                                    };
                                    (on != track.mix.solo)
                                        .then_some((track.id, Mix { solo: on, ..track.mix }))
                                })
                                .collect()
                        };
                        for (id, mix) in changes {
                            let _ = tools::set_mix(save, Some(id), mix);
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

#[component]
fn MasterMeter() -> Element {
    let mut peak = use_signal(|| 0.0_f32);
    use_future(move || async move {
        loop {
            media::resume_meter();
            peak.set(media::meter_peak());
            gloo_timers::future::TimeoutFuture::new(80).await;
        }
    });
    let height = format!("height:{:.0}%", (*peak.read() * 100.0).clamp(0.0, 100.0));
    rsx! {
        div { class: "mixer-meter", title: "Level of what the monitor is playing",
            div { class: "mixer-meter-fill", style: "{height}" }
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
    let mut points = use_signal(|| curve_ends());
    let mut drag = use_signal(|| None::<usize>);
    let ch = channel.read().clone();
    let selected_load = selected.clone();
    let selected_reset = selected.clone();
    let track_reset = track.clone();
    use_effect(move || {
        let name = channel.read().clone();
        let clip = selected_load.clone();
        let loaded = save
            .engine
            .read()
            .tracks
            .iter()
            .flat_map(|track| track.clips.iter())
            .find(|clip_row| clip_row.id.to_string() == clip)
            .map(|clip_row| match name.as_str() {
                "red" => clip_row.look.curves.red.clone(),
                "green" => clip_row.look.curves.green.clone(),
                "blue" => clip_row.look.curves.blue.clone(),
                _ => clip_row.look.curves.all.clone(),
            })
            .filter(|pts| pts.len() >= 2)
            .unwrap_or_else(curve_ends);
        points.set(loaded);
    });
    let dots = points.read().clone();
    let poly = dots
        .iter()
        .map(|point| format!("{:.4},{:.4}", point.x, 1.0 - point.y))
        .collect::<Vec<_>>()
        .join(" ");
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
            svg {
                class: "curve-pad",
                view_box: "0 0 1 1",
                preserve_aspect_ratio: "none",
                onpointerdown: move |evt| {
                    capture_pointer(&evt);
                    let Some((x, y)) = curve_point(&evt) else { return };
                    let mut next = points.peek().clone();
                    if let Some(hit) = nearest_point(&next, x, y) {
                        drag.set(Some(hit));
                    } else {
                        next.push(CurvePoint { x, y });
                        next.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
                        let hit = nearest_point(&next, x, y).unwrap_or(0);
                        points.set(next);
                        drag.set(Some(hit));
                    }
                },
                onpointermove: move |evt| {
                    let Some(index) = *drag.peek() else { return };
                    let Some((x, y)) = curve_point(&evt) else { return };
                    let mut next = points.peek().clone();
                    let last = next.len().saturating_sub(1);
                    let Some(point) = next.get_mut(index) else { return };
                    if index == 0 {
                        point.x = 0.0;
                    } else if index == last {
                        point.x = 1.0;
                    } else {
                        point.x = x.clamp(0.02, 0.98);
                    }
                    point.y = y.clamp(0.0, 1.0);
                    points.set(next);
                },
                onpointerup: move |_| {
                    if drag.peek().is_none() {
                        return;
                    }
                    drag.set(None);
                    let mut next = points.peek().clone();
                    if let Some(first) = next.first_mut() {
                        first.x = 0.0;
                    }
                    if let Some(last) = next.last_mut() {
                        last.x = 1.0;
                    }
                    next.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
                    points.set(next.clone());
                    let mut curves = save
                        .engine
                        .peek()
                        .tracks
                        .iter()
                        .flat_map(|row| row.clips.iter())
                        .find(|clip_row| clip_row.id.to_string() == selected)
                        .map(|clip_row| clip_row.look.curves.clone())
                        .unwrap_or_default();
                    match channel.peek().as_str() {
                        "red" => curves.red = next,
                        "green" => curves.green = next,
                        "blue" => curves.blue = next,
                        _ => curves.all = next,
                    }
                    let _ = tools::set_curves_at(save, Some(&selected), &track, at, curves);
                },
                polyline { points: "{poly}", fill: "none", stroke: "currentColor", "stroke-width": "0.012" }
                for point in dots {
                    circle {
                        cx: "{point.x}",
                        cy: "{1.0 - point.y}",
                        r: "0.028",
                    }
                }
            }
            button {
                class: "btn btn-ghost",
                onclick: move |_| {
                    drag.set(None);
                    points.set(curve_ends());
                    let _ = tools::set_curves_at(save, Some(&selected_reset), &track_reset, at, Curves::default());
                },
                "Reset curve"
            }
        }
    }
}

fn curve_ends() -> Vec<CurvePoint> {
    vec![CurvePoint { x: 0.0, y: 0.0 }, CurvePoint { x: 1.0, y: 1.0 }]
}

fn nearest_point(points: &[CurvePoint], x: f32, y: f32) -> Option<usize> {
    points.iter().enumerate().find_map(|(index, point)| {
        let dx = point.x - x;
        let dy = point.y - y;
        (dx * dx + dy * dy < 0.08 * 0.08).then_some(index)
    })
}

fn curve_point(evt: &Event<PointerData>) -> Option<(f32, f32)> {
    let data = evt.data();
    let native = data.downcast::<web_sys::PointerEvent>()?;
    let target = native.target()?.dyn_into::<web_sys::Element>().ok()?;
    let svg = if target.tag_name().eq_ignore_ascii_case("svg") {
        target
    } else {
        target.closest("svg").ok().flatten()?
    };
    let rect = svg.get_bounding_client_rect();
    let point = evt.client_coordinates();
    let px = ((point.x - rect.left()) / rect.width().max(1.0)).clamp(0.0, 1.0) as f32;
    let py = (1.0 - ((point.y - rect.top()) / rect.height().max(1.0))).clamp(0.0, 1.0) as f32;
    Some((px, py))
}

#[component]
pub fn MaskPanel(selected: String, track: String, at: f64) -> Element {
    let save = use_context::<WorkspaceSave>();
    let mut shape = use_signal(|| MaskShape::Rectangle);
    let mut feather = use_signal(|| 0.0_f32);
    let mut invert = use_signal(|| false);
    let mut mask_x = use_signal(|| 0.5_f32);
    let mut mask_y = use_signal(|| 0.5_f32);
    let mut mask_w = use_signal(|| 0.5_f32);
    let mut mask_h = use_signal(|| 0.5_f32);
    let selected_load = selected.clone();
    let selected_clear = selected.clone();
    let track_clear = track.clone();
    use_effect(move || {
        let id = selected_load.clone();
        let mask = save
            .engine
            .read()
            .tracks
            .iter()
            .flat_map(|row| row.clips.iter())
            .find(|clip| clip.id.to_string() == id)
            .and_then(|clip| clip.look.mask);
        if let Some(mask) = mask {
            shape.set(mask.shape);
            feather.set(mask.feather);
            invert.set(mask.invert);
            mask_x.set(mask.x);
            mask_y.set(mask.y);
            mask_w.set(mask.w);
            mask_h.set(mask.h);
        }
    });
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
                        x: *mask_x.peek(),
                        y: *mask_y.peek(),
                        w: *mask_w.peek(),
                        h: *mask_h.peek(),
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

fn remap_speeds(clip: &oc_core::Clip) -> (f32, f32, f32) {
    let start = if clip.speed.is_finite() && clip.speed > 0.05 {
        clip.speed
    } else {
        1.0
    };
    if clip.look.speed_keys.len() >= 2 {
        let keys = &clip.look.speed_keys;
        let start = keys
            .iter()
            .min_by(|a, b| a.at.partial_cmp(&b.at).unwrap_or(std::cmp::Ordering::Equal))
            .map(|key| key.speed)
            .unwrap_or(start);
        let end = keys
            .iter()
            .max_by(|a, b| a.at.partial_cmp(&b.at).unwrap_or(std::cmp::Ordering::Equal))
            .map(|key| key.speed)
            .unwrap_or(start);
        let mid = keys
            .iter()
            .min_by(|a, b| {
                (a.at - 0.5)
                    .abs()
                    .partial_cmp(&(b.at - 0.5).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|key| key.speed)
            .unwrap_or((start + end) * 0.5);
        return (start, mid, end);
    }
    let end = clip.look.speed_to.unwrap_or(start);
    (start, (start + end) * 0.5, end)
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
    let mut start_speed = use_signal(|| 1.0_f32);
    let mut mid_speed = use_signal(|| 1.0_f32);
    let mut end_speed = use_signal(|| 1.0_f32);
    let selected_load = selected.clone();
    use_effect(move || {
        let id = selected_load.clone();
        let Some((start, mid, end)) = save
            .engine
            .read()
            .tracks
            .iter()
            .flat_map(|row| row.clips.iter())
            .find(|clip| clip.id.to_string() == id)
            .map(remap_speeds)
        else {
            return;
        };
        start_speed.set(start);
        mid_speed.set(mid);
        end_speed.set(end);
    });
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
                        SpeedKey { at: 0.0, speed: *start_speed.peek() },
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
                    height: "168",
                    class: "scope-canvas",
                }
                p { class: "mixer-note", "Waveform, RGB parade, vectorscope, and histogram of the current monitor frame." }
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
    let Some(px) = media::graded_frame() else {
        return;
    };
    let plot_h = (h - 42.0).max(48.0);
    ctx.set_fill_style_str("#111");
    ctx.fill_rect(0.0, 0.0, w, h);
    let mut bins = [0u32; 32];
    // Waveform (luma) on the left third, parade in the middle, vectorscope on the right.
    for y in (0..90).step_by(2) {
        for x in (0..160).step_by(2) {
            let i = ((y * 160 + x) * 4) as usize;
            if i + 2 >= px.len() {
                continue;
            }
            let x = x / 2;
            let y = y / 2;
            let r = px[i] as f64;
            let g = px[i + 1] as f64;
            let b = px[i + 2] as f64;
            let luma = (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255.0;
            let bin = (luma * 31.0).clamp(0.0, 31.0) as usize;
            bins[bin] = bins[bin].saturating_add(1);
            ctx.set_fill_style_str("rgba(180,220,180,0.35)");
            ctx.fill_rect(x as f64 * 0.9, (1.0 - luma) * (plot_h - 4.0), 1.0, 1.0);
            ctx.set_fill_style_str("rgba(220,80,80,0.45)");
            ctx.fill_rect(90.0 + (r / 255.0) * 50.0, y as f64 * 2.4, 1.0, 1.0);
            ctx.set_fill_style_str("rgba(80,200,80,0.45)");
            ctx.fill_rect(90.0 + (g / 255.0) * 50.0, y as f64 * 2.4 + 1.0, 1.0, 1.0);
            ctx.set_fill_style_str("rgba(80,120,220,0.45)");
            ctx.fill_rect(90.0 + (b / 255.0) * 50.0, y as f64 * 2.4 + 2.0, 1.0, 1.0);
            let uu = (b - luma * 255.0) / 255.0;
            let vv = (r - luma * 255.0) / 255.0;
            ctx.set_fill_style_str("rgba(240,220,120,0.8)");
            ctx.fill_rect(210.0 + uu * 28.0, (plot_h * 0.5) - vv * 28.0, 1.5, 1.5);
        }
    }
    let peak = bins.iter().copied().max().unwrap_or(1).max(1) as f64;
    ctx.set_fill_style_str("rgba(220,220,220,0.85)");
    for (i, count) in bins.iter().enumerate() {
        let bar = (*count as f64 / peak) * 32.0;
        ctx.fill_rect(6.0 + i as f64 * 8.4, h - 6.0 - bar, 6.0, bar);
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
            let clip = track
                .clips
                .iter()
                .find(|c| c.contains(oc_core::Time::from_seconds(at)))?;
            let url = clip
                .media_id
                .and_then(|id| {
                    library
                        .read()
                        .iter()
                        .find(|m| m.id == id.to_string())
                        .map(|m| m.url.clone())
                })
                .unwrap_or_default();
            let src = clip
                .source_time_at(oc_core::Time::from_seconds(at))
                .map(|t| t.as_seconds())
                .unwrap_or_else(|| clip.source_in.as_seconds());
            Some((i + 1, track.id.to_string(), track.name.clone(), url, src))
        })
        .collect();
    use_effect(move || {
        let _ = *clock.current.read();
        let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
            return;
        };
        let Ok(nodes) = doc.query_selector_all(".multicam-cell video") else {
            return;
        };
        for i in 0..nodes.length() {
            let Some(node) = nodes.item(i) else { continue };
            let Ok(video) = node.dyn_into::<web_sys::HtmlVideoElement>() else {
                continue;
            };
            let Some(raw) = video.get_attribute("data-src-time") else {
                continue;
            };
            let Ok(t) = raw.parse::<f64>() else { continue };
            if video.ready_state() >= 1 && (video.current_time() - t).abs() > 0.08 {
                video.set_current_time(t.max(0.0));
            }
        }
    });
    rsx! {
        div { class: "multicam",
            div { class: "mixer-title", "Multitrack" }
            div { class: "multicam-grid",
                for (n, id, name, url, src) in angles {
                    button {
                        class: "multicam-cell",
                        title: "Cut to this angle. Keys 1–9 do the same while this tool is on.",
                        onclick: move |_| {
                            let _ = tools::multicam_at(save, &id, at);
                        },
                        if !url.is_empty() {
                            video {
                                src: "{url}",
                                muted: true,
                                preload: "auto",
                                "data-src-time": "{src}",
                            }
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
        let Ok(canvas) = node.dyn_into::<HtmlCanvasElement>() else {
            continue;
        };
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
            let _ =
                ctx.draw_image_with_html_video_element_and_dw_and_dh(&video, 0.0, 0.0, 48.0, 36.0);
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
    let resp = wasm_bindgen_futures::JsFuture::from(win.fetch_with_str(url))
        .await
        .ok()?;
    let resp: web_sys::Response = resp.dyn_into().ok()?;
    let buf = wasm_bindgen_futures::JsFuture::from(resp.array_buffer().ok()?)
        .await
        .ok()?;
    let buf: js_sys::ArrayBuffer = buf.dyn_into().ok()?;
    let ctx = web_sys::AudioContext::new().ok()?;
    let audio = wasm_bindgen_futures::JsFuture::from(ctx.decode_audio_data(&buf).ok()?)
        .await
        .ok()?;
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
