//! Editor surfaces modeled on Kdenlive: audio mixer, curves, alpha shapes,
//! generators, time remap, undo history, multicam, and the rendered file.

use crate::WorkspaceSave;
use crate::media::{self, Clock, MediaItem, capture_pointer};
use crate::tools;
use dioxus::prelude::*;
use oc_core::{AlphaShape, CurvePoint, Curves, Generator, MaskShape, Mix, SpeedKey, TrackKind};
use oc_tools::ToolId;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, HtmlVideoElement};

thread_local! {
    static WAVES: RefCell<HashMap<String, Vec<u8>>> = RefCell::new(HashMap::new());
    static FILM: RefCell<FilmCache> = RefCell::new(FilmCache::default());
    static FILM_HOLD: Cell<bool> = const { Cell::new(false) };
}

/// Pause filmstrip seeks while a trim is dragging or the monitor is playing.
pub fn hold_film(on: bool) {
    FILM_HOLD.with(|cell| cell.set(on));
}

fn film_held() -> bool {
    FILM_HOLD.with(Cell::get)
}

struct FilmCache {
    frames: HashMap<String, Vec<(f64, String)>>,
    tried: HashMap<String, Vec<f64>>,
    failed: HashSet<String>,
    /// One JPEG repeated across the clip. The browser could not decode the file.
    stills: HashSet<String>,
    /// Blob URL to the media id, so a still can be cached per clip.
    media: HashMap<String, String>,
    wishes: Vec<(String, f64)>,
    decoding: bool,
    listeners: HashMap<u64, Rc<RefCell<dyn FnMut()>>>,
    next_listener: u64,
}

impl Default for FilmCache {
    fn default() -> Self {
        Self {
            frames: HashMap::new(),
            tried: HashMap::new(),
            failed: HashSet::new(),
            stills: HashSet::new(),
            media: HashMap::new(),
            wishes: Vec::new(),
            decoding: false,
            listeners: HashMap::new(),
            next_listener: 1,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Picture {
    /// At least one frame was drawn from the video element.
    Ready,
    /// Metadata arrived and the element has no picture. Use a still.
    Still,
    /// No picture and no reason to upload the file.
    Missing,
}

/// Chrome on this machine reports HEVC as `videoWidth == 0` even when duration is known.
#[must_use]
pub fn browser_needs_still(video_width: u32) -> bool {
    video_width == 0
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

/// Still frames for one timeline clip. The pictures are cached JPEGs, so a
/// later render does not wipe them the way a canvas draw did. A file the
/// browser cannot decode uses one still instead.
#[component]
pub fn Filmstrip(
    url: String,
    media_id: String,
    tiles: i32,
    source_in: f64,
    source_out: f64,
) -> Element {
    let rev = use_signal(|| 0u32);
    dioxus_core::use_hook_with_cleanup(
        move || {
            let mut rev = rev;
            FILM.with(|film| {
                let mut film = film.borrow_mut();
                let token = film.next_listener;
                film.next_listener = token.wrapping_add(1);
                film.listeners.insert(
                    token,
                    Rc::new(RefCell::new(move || {
                        let next = rev.peek().wrapping_add(1);
                        rev.set(next);
                    })),
                );
                token
            })
        },
        |token| {
            FILM.with(|film| {
                film.borrow_mut().listeners.remove(&token);
            });
        },
    );
    let _paint = *rev.read();
    let still_src = if film_is_still(&url) {
        film_picks_owned(&url, source_in, source_out, tiles)
            .into_iter()
            .next()
            .unwrap_or_default()
    } else {
        String::new()
    };
    let frames = if still_src.is_empty() {
        film_picks_owned(&url, source_in, source_out, tiles)
    } else {
        Vec::new()
    };
    use_effect(use_reactive(
        (&url, &media_id, &source_in, &source_out, &tiles),
        |(url, media_id, source_in, source_out, tiles)| {
            if url.is_empty() {
                return;
            }
            film_request(
                &url,
                &media_id,
                &film_sample_times(source_in, source_out, tiles),
            );
        },
    ));
    rsx! {
        div { class: "nle-strip",
            if !still_src.is_empty() {
                img {
                    class: "nle-frame wait",
                    src: "{still_src}",
                    alt: "",
                    draggable: "false",
                }
            } else if frames.is_empty() {
                div { class: "nle-frame wait" }
            } else {
                for (i, src) in frames.into_iter().enumerate() {
                    img {
                        key: "{i}",
                        class: "nle-frame",
                        src: "{src}",
                        alt: "",
                        draggable: "false",
                    }
                }
            }
        }
    }
}

/// Where to grab stills inside one clip. At most six, so a long clip does not
/// decode a frame per column.
pub fn film_sample_times(start: f64, end: f64, tiles: i32) -> Vec<f64> {
    let start = if start.is_finite() {
        start.max(0.0)
    } else {
        0.0
    };
    let end = if end.is_finite() {
        end.max(start + 0.05)
    } else {
        start + 0.05
    };
    let n = tiles.clamp(1, 6) as usize;
    let span = end - start;
    (0..n)
        .map(|i| {
            let at = if n == 1 {
                start + span * 0.2
            } else {
                start + span * (i as f64 + 0.5) / n as f64
            };
            at.clamp(start, (end - 0.04).max(start))
        })
        .collect()
}

/// Distinct stills across the clip. The same JPEG is not copied into the next column.
pub fn film_picks(frames: &[(f64, &str)], start: f64, end: f64, tiles: i32) -> Vec<String> {
    if frames.is_empty() {
        return Vec::new();
    }
    let mut picks = Vec::new();
    for at in film_sample_times(start, end, tiles) {
        let Some(src) = nearest_frame(frames, at) else {
            continue;
        };
        if picks.last().is_some_and(|prev: &String| prev == src) {
            continue;
        }
        picks.push(src.to_string());
    }
    picks
}

fn nearest_frame<'a>(frames: &'a [(f64, &'a str)], at: f64) -> Option<&'a str> {
    frames
        .iter()
        .min_by(|a, b| {
            (a.0 - at)
                .abs()
                .partial_cmp(&(b.0 - at).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|frame| frame.1)
}

fn film_picks_owned(url: &str, start: f64, end: f64, tiles: i32) -> Vec<String> {
    if url.is_empty() {
        return Vec::new();
    }
    let frames = FILM.with(|film| film.borrow().frames.get(url).cloned());
    let Some(frames) = frames else {
        return Vec::new();
    };
    let borrowed = frames
        .iter()
        .map(|(at, src)| (*at, src.as_str()))
        .collect::<Vec<_>>();
    film_picks(&borrowed, start, end, tiles)
}

fn film_request(url: &str, media_id: &str, times: &[f64]) {
    if url.is_empty() || times.is_empty() {
        return;
    }
    if !media_id.is_empty() {
        FILM.with(|film| {
            film.borrow_mut()
                .media
                .insert(url.to_string(), media_id.to_string());
        });
    }
    let missing = FILM.with(|film| {
        let film = film.borrow();
        if film.failed.contains(url) || film.stills.contains(url) {
            return Vec::new();
        }
        let have = film.frames.get(url).map(Vec::as_slice).unwrap_or(&[]);
        let tried = film.tried.get(url).map(Vec::as_slice).unwrap_or(&[]);
        times
            .iter()
            .copied()
            .filter(|at| {
                !have.iter().any(|(got, _)| (got - at).abs() < 0.35)
                    && !tried.iter().any(|got| (got - at).abs() < 0.35)
            })
            .collect::<Vec<_>>()
    });
    if missing.is_empty() {
        return;
    }
    let start = FILM.with(|film| {
        let mut film = film.borrow_mut();
        let unseen = !film
            .frames
            .get(url)
            .is_some_and(|frames| !frames.is_empty());
        if unseen {
            if let Some(first) = missing.first().copied() {
                film.wishes.insert(0, (url.to_string(), first));
            }
            for at in missing.into_iter().skip(1) {
                film.wishes.push((url.to_string(), at));
            }
        } else {
            for at in missing {
                film.wishes.push((url.to_string(), at));
            }
        }
        if film.decoding {
            false
        } else {
            film.decoding = true;
            true
        }
    });
    if start {
        spawn(async move {
            film_pump().await;
        });
    }
}

const FILM_FRAMES: usize = 4;

async fn film_pump() {
    loop {
        if film_held() || monitor_is_playing() {
            gloo_timers::future::TimeoutFuture::new(180).await;
            let pending = FILM.with(|film| !film.borrow().wishes.is_empty());
            if !pending {
                break;
            }
            continue;
        }
        let loaded = film_loaded_url();
        let next = FILM.with(|film| take_next_wish(&mut film.borrow_mut(), loaded.as_deref()));
        let Some((url, at)) = next else {
            break;
        };
        if film_is_still(&url) || film_failed(&url) {
            continue;
        }
        if frame_near(&url, at) {
            continue;
        }
        if stored_count(&url) >= FILM_FRAMES {
            drop_wishes(&url);
            continue;
        }
        if !film_has_picture(&url) {
            let _ = remember_poster(&url).await;
            if frame_near(&url, at) {
                continue;
            }
        }
        match decode_url(&url, &[at]).await {
            Picture::Ready => {}
            Picture::Still => {
                if !fetch_poster(&url).await && !film_has_picture(&url) {
                    mark_undecoded(&url);
                }
            }
            Picture::Missing => {
                if !film_has_picture(&url) {
                    mark_undecoded(&url);
                }
            }
        }
        // Yield so a seek does not freeze the monitor and the trim drag.
        gloo_timers::future::TimeoutFuture::new(40).await;
    }
    FILM.with(|film| film.borrow_mut().decoding = false);
}

fn monitor_is_playing() -> bool {
    crate::media::preview_video().is_some_and(|video| !video.paused() && !video.ended())
}

fn film_failed(url: &str) -> bool {
    FILM.with(|film| film.borrow().failed.contains(url))
}

fn stored_count(url: &str) -> usize {
    FILM.with(|film| film.borrow().frames.get(url).map(Vec::len).unwrap_or(0))
}

fn frame_near(url: &str, at: f64) -> bool {
    FILM.with(|film| {
        film.borrow()
            .frames
            .get(url)
            .is_some_and(|frames| frames.iter().any(|(got, _)| (got - at).abs() < 0.35))
    })
}

fn drop_wishes(url: &str) {
    FILM.with(|film| {
        film.borrow_mut().wishes.retain(|(wish, _)| wish != url);
    });
}

fn film_has_picture(url: &str) -> bool {
    FILM.with(|film| {
        let film = film.borrow();
        film.stills.contains(url)
            || film
                .frames
                .get(url)
                .is_some_and(|frames| !frames.is_empty())
    })
}

fn film_is_still(url: &str) -> bool {
    FILM.with(|film| film.borrow().stills.contains(url))
}

fn mark_undecoded(url: &str) {
    FILM.with(|film| {
        let mut film = film.borrow_mut();
        film.failed.insert(url.to_string());
        film.wishes.retain(|(wish, _)| wish != url);
    });
    notify_film();
}

fn clip_ids(url: &str) -> Option<(String, String)> {
    let media = FILM.with(|film| film.borrow().media.get(url).cloned())?;
    if media.is_empty() {
        return None;
    }
    let project = super::page_project_id()?;
    if project.is_empty() {
        return None;
    }
    Some((project, media))
}

fn store_still(url: &str, jpeg: &[u8]) -> bool {
    let Some(src) = crate::media::blob_url(jpeg, "image/jpeg") else {
        return false;
    };
    FILM.with(|film| {
        let mut film = film.borrow_mut();
        film.frames.insert(url.to_string(), vec![(0.0, src)]);
        film.stills.insert(url.to_string());
        film.failed.remove(url);
        film.wishes.retain(|(wish, _)| wish != url);
    });
    notify_film();
    true
}

async fn remember_poster(url: &str) -> bool {
    let Some((project, media)) = clip_ids(url) else {
        return false;
    };
    let Ok(Some(bytes)) = crate::hold::load_poster(&project, &media).await else {
        return false;
    };
    // A poster is the first still, not a reason to tile it or to stop sampling.
    store_poster_frame(url, &bytes)
}

fn store_poster_frame(url: &str, jpeg: &[u8]) -> bool {
    if film_has_picture(url) {
        return false;
    }
    let Some(src) = crate::media::blob_url(jpeg, "image/jpeg") else {
        return false;
    };
    FILM.with(|film| {
        let mut film = film.borrow_mut();
        let frames = film.frames.entry(url.to_string()).or_default();
        if frames.is_empty() {
            frames.push((0.0, src));
        }
        film.failed.remove(url);
    });
    notify_film();
    true
}

async fn fetch_poster(url: &str) -> bool {
    if !url.starts_with("blob:") {
        return false;
    }
    let Some((project, media)) = clip_ids(url) else {
        return false;
    };
    let Ok(bytes) = crate::api::request_poster(&project, &media, url).await else {
        return false;
    };
    let _ = crate::hold::save_poster(&project, &media, &bytes).await;
    store_still(url, &bytes)
}

/// One still at a time. A file with no picture yet goes before a second frame
/// of a file that already has one, and one loaded file is not dropped to seek
/// a different file.
fn take_next_wish(film: &mut FilmCache, loaded: Option<&str>) -> Option<(String, f64)> {
    if film.wishes.is_empty() {
        return None;
    }
    let idx = film
        .wishes
        .iter()
        .position(|(url, _)| Some(url.as_str()) == loaded)
        .or_else(|| {
            film.wishes.iter().position(|(url, _)| {
                !film
                    .frames
                    .get(url)
                    .is_some_and(|frames| !frames.is_empty())
                    && !film.failed.contains(url)
            })
        })
        .unwrap_or(0);
    let (url, at) = film.wishes.remove(idx);
    film.wishes
        .retain(|(wish, other)| wish != &url || (other - at).abs() >= 0.05);
    Some((url, at))
}

fn film_loaded_url() -> Option<String> {
    let video = film_video()?;
    let url = video.get_attribute("data-film-src")?;
    if url.is_empty() || video.ready_state() < 1 {
        None
    } else {
        Some(url)
    }
}

fn notify_film() {
    let listeners = FILM.with(|film| {
        film.borrow()
            .listeners
            .values()
            .cloned()
            .collect::<Vec<_>>()
    });
    for listener in listeners {
        if let Ok(mut callback) = listener.try_borrow_mut() {
            callback();
        }
    }
}

fn film_video() -> Option<HtmlVideoElement> {
    let doc = web_sys::window()?.document()?;
    if let Some(found) = doc.get_element_by_id("oc-film-decoder") {
        return found.dyn_into().ok();
    }
    let video: HtmlVideoElement = doc.create_element("video").ok()?.dyn_into().ok()?;
    video.set_id("oc-film-decoder");
    video.set_muted(true);
    let _ = video.set_attribute("muted", "true");
    let _ = video.set_attribute("playsinline", "true");
    video.set_preload("auto");
    let _ = video.set_attribute(
        "style",
        "position:fixed;width:2px;height:2px;left:0;top:0;opacity:0;pointer-events:none;z-index:-1",
    );
    doc.body()?.append_child(&video).ok()?;
    Some(video)
}

async fn decode_url(url: &str, times: &[f64]) -> Picture {
    let Some(video) = film_video() else {
        return Picture::Missing;
    };
    if video.get_attribute("data-film-src").as_deref() != Some(url) {
        if url.starts_with("http://") || url.starts_with("https://") {
            let _ = video.set_cross_origin(Some("anonymous"));
        } else {
            let _ = video.set_cross_origin(None);
        }
        let _ = video.set_attribute("data-film-src", url);
        video.set_src(url);
        video.load();
        if !wait_ready(&video).await {
            return if element_has_no_picture(&video) {
                Picture::Still
            } else {
                Picture::Missing
            };
        }
    } else if video.ready_state() < 1 && !wait_ready(&video).await {
        return if element_has_no_picture(&video) {
            Picture::Still
        } else {
            Picture::Missing
        };
    }
    if browser_needs_still(video.video_width()) {
        return Picture::Still;
    }
    let dur = video.duration();
    let mut stored = false;
    for at in times {
        let mut at = *at;
        if dur.is_finite() && dur > 0.05 {
            at = at.clamp(0.0, dur - 0.05);
        }
        FILM.with(|film| {
            film.borrow_mut()
                .tried
                .entry(url.to_string())
                .or_default()
                .push(at);
        });
        if !seek_to(&video, at).await {
            continue;
        }
        if let Some(jpeg) = grab_jpeg(&video).await {
            FILM.with(|film| {
                let mut film = film.borrow_mut();
                let frames = film.frames.entry(url.to_string()).or_default();
                if !frames.iter().any(|(got, _)| (got - at).abs() < 0.2) {
                    frames.push((at, jpeg));
                    frames
                        .sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                }
            });
            stored = true;
            notify_film();
        }
    }
    if stored
        || FILM.with(|film| {
            film.borrow()
                .frames
                .get(url)
                .is_some_and(|frames| !frames.is_empty())
        })
    {
        Picture::Ready
    } else {
        Picture::Missing
    }
}

fn element_has_no_picture(video: &HtmlVideoElement) -> bool {
    // Duration with no dimensions is a file the element parsed and cannot decode.
    browser_needs_still(video.video_width())
        && video.duration().is_finite()
        && video.duration() > 0.0
}

async fn wait_ready(video: &HtmlVideoElement) -> bool {
    if video.ready_state() >= 1 && video.duration().is_finite() && video.duration() > 0.0 {
        return true;
    }
    let hit = Rc::new(Cell::new(false));
    let flag = hit.clone();
    let closure = Closure::wrap(Box::new(move || flag.set(true)) as Box<dyn FnMut()>);
    let _ = video.add_event_listener_with_callback("loadeddata", closure.as_ref().unchecked_ref());
    for _ in 0..160 {
        if hit.get()
            || (video.ready_state() >= 1 && video.duration().is_finite() && video.duration() > 0.0)
        {
            break;
        }
        gloo_timers::future::TimeoutFuture::new(50).await;
    }
    let _ =
        video.remove_event_listener_with_callback("loadeddata", closure.as_ref().unchecked_ref());
    video.ready_state() >= 1 && video.duration().is_finite() && video.duration() > 0.0
}

async fn seek_to(video: &HtmlVideoElement, at: f64) -> bool {
    if video.ready_state() >= 2 && (video.current_time() - at).abs() < 0.08 {
        return true;
    }
    let hit = Rc::new(Cell::new(false));
    let flag = hit.clone();
    let closure = Closure::wrap(Box::new(move || flag.set(true)) as Box<dyn FnMut()>);
    let _ = video.add_event_listener_with_callback("seeked", closure.as_ref().unchecked_ref());
    video.set_current_time(at);
    for _ in 0..50 {
        if hit.get() && video.ready_state() >= 2 {
            break;
        }
        gloo_timers::future::TimeoutFuture::new(50).await;
    }
    let _ = video.remove_event_listener_with_callback("seeked", closure.as_ref().unchecked_ref());
    gloo_timers::future::TimeoutFuture::new(80).await;
    video.ready_state() >= 2
}

async fn grab_jpeg(video: &HtmlVideoElement) -> Option<String> {
    let (w, h) = thumb_px(video);
    let doc = web_sys::window()?.document()?;
    let canvas: HtmlCanvasElement = doc.create_element("canvas").ok()?.dyn_into().ok()?;
    canvas.set_width(w);
    canvas.set_height(h);
    let ctx = canvas
        .get_context("2d")
        .ok()
        .flatten()?
        .dyn_into::<CanvasRenderingContext2d>()
        .ok()?;
    ctx.draw_image_with_html_video_element_and_dw_and_dh(
        video,
        0.0,
        0.0,
        f64::from(w),
        f64::from(h),
    )
    .ok()?;
    if frame_luma(&ctx, w, h).is_some_and(|luma| luma < 12) {
        gloo_timers::future::TimeoutFuture::new(220).await;
        ctx.draw_image_with_html_video_element_and_dw_and_dh(
            video,
            0.0,
            0.0,
            f64::from(w),
            f64::from(h),
        )
        .ok()?;
    }
    canvas.to_data_url_with_type("image/jpeg").ok()
}

fn thumb_px(video: &HtmlVideoElement) -> (u32, u32) {
    let w = video.video_width().max(1);
    let h = video.video_height().max(1);
    // Small JPEGs. A 160px grab of every column was enough to hitch the editor.
    if h > w {
        let height = 96u32;
        let width = ((96.0 * f64::from(w) / f64::from(h)).round() as u32).clamp(36, 96);
        (width, height)
    } else {
        let width = 96u32;
        let height = ((96.0 * f64::from(h) / f64::from(w)).round() as u32).clamp(36, 96);
        (width, height)
    }
}

fn frame_luma(ctx: &CanvasRenderingContext2d, w: u32, h: u32) -> Option<u32> {
    let data = ctx
        .get_image_data(
            f64::from(w / 2).max(0.0),
            f64::from(h / 2).max(0.0),
            8.0,
            8.0,
        )
        .ok()?;
    let px = data.data();
    if px.len() < 4 {
        return None;
    }
    let mut sum = 0u32;
    let mut n = 0u32;
    let mut i = 0;
    while i + 2 < px.len() {
        sum += u32::from(px[i]) + u32::from(px[i + 1]) + u32::from(px[i + 2]);
        n += 1;
        i += 4;
    }
    Some(sum / n.max(1))
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

#[cfg(test)]
mod tests {
    use super::{curve_ends, nearest_point, remap_speeds, shape_name};
    use oc_core::{Clip, ClipId, ClipKind, ClipLook, Duration, MaskShape, SpeedKey, Time};

    fn clip(speed: f32, speed_to: Option<f32>) -> Clip {
        let mut look = ClipLook::default();
        look.speed_to = speed_to;
        Clip {
            id: ClipId::new(),
            media_id: None,
            kind: ClipKind::Video {
                transform: Default::default(),
            },
            start: Time::ZERO,
            duration: Duration::from_seconds(4.0),
            source_in: Time::ZERO,
            speed,
            group_id: None,
            link_id: None,
            disabled: false,
            look,
        }
    }

    #[test]
    fn remap_reads_keys_and_otherwise_averages_the_ends() {
        let mut keyed = clip(1.0, None);
        keyed.look.speed_keys = vec![
            SpeedKey {
                at: 0.0,
                speed: 0.5,
            },
            SpeedKey {
                at: 0.5,
                speed: 2.0,
            },
            SpeedKey {
                at: 1.0,
                speed: 1.0,
            },
        ];
        assert_eq!(remap_speeds(&keyed), (0.5, 2.0, 1.0));
        let (start, mid, end) = remap_speeds(&clip(1.0, Some(3.0)));
        assert_eq!((start, end), (1.0, 3.0));
        assert!((mid - 2.0).abs() < 1e-4);
        assert_eq!(remap_speeds(&clip(f32::NAN, None)).0, 1.0);
    }

    #[test]
    fn nearest_curve_point_ignores_a_far_click() {
        let points = curve_ends();
        assert_eq!(points.len(), 2);
        assert_eq!(nearest_point(&points, 0.0, 0.0), Some(0));
        assert_eq!(nearest_point(&points, 1.0, 1.0), Some(1));
        assert_eq!(nearest_point(&points, 0.5, 0.5), None);
    }

    #[test]
    fn every_mask_has_a_name() {
        assert_eq!(shape_name(MaskShape::Rectangle), "Rectangle");
        assert_eq!(shape_name(MaskShape::Ellipse), "Ellipse");
        assert_eq!(shape_name(MaskShape::Triangle), "Triangle");
        assert_eq!(shape_name(MaskShape::Diamond), "Diamond");
    }

    #[test]
    fn a_clip_samples_a_few_frames_across_its_source() {
        let one = super::film_sample_times(0.0, 10.0, 1);
        assert_eq!(one.len(), 1);
        assert!((one[0] - 2.0).abs() < 1e-6, "{one:?}");
        let many = super::film_sample_times(1.0, 7.0, 40);
        assert_eq!(many.len(), 6);
        assert!(many[0] >= 1.0 && many[5] < 7.0, "{many:?}");
        assert!(many.windows(2).all(|pair| pair[1] > pair[0]));
        let flipped = super::film_sample_times(5.0, 1.0, 2);
        assert!(flipped.iter().all(|at| *at >= 5.0), "{flipped:?}");
    }

    #[test]
    fn a_clip_the_browser_cannot_decode_uses_one_still() {
        assert!(super::browser_needs_still(0));
        assert!(!super::browser_needs_still(1920));
        let frames = [(0.4, "still")];
        let picks = super::film_picks(&frames, 0.0, 12.0, 5);
        assert_eq!(picks, vec!["still".to_string()]);
    }

    #[test]
    fn one_decoded_frame_does_not_tile_the_strip() {
        let frames = [(1.5, "early"), (8.0, "late")];
        let picks = super::film_picks(&frames, 0.0, 10.0, 2);
        assert_eq!(picks, vec!["early".to_string(), "late".to_string()]);
        let only = [(1.5, "early")];
        let filled = super::film_picks(&only, 0.0, 10.0, 4);
        assert_eq!(filled, vec!["early".to_string()]);
        assert!(super::film_picks(&[], 0.0, 10.0, 3).is_empty());
    }
}
