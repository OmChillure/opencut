//! Fleet-style toaster: high-contrast card + progress line, then auto-close.
//!
//! ```ignore
//! show_toast().success("Dissolve applied.");
//! show_toast().error("no clip at the playhead");
//! ```

use std::cell::Cell;

use dioxus::prelude::*;

const TOAST_MS: u32 = 4000;
const MAX_TOASTS: usize = 4;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Warn,
    Error,
}

#[derive(Clone, PartialEq)]
pub struct ToastItem {
    pub id: u64,
    pub kind: ToastKind,
    pub message: String,
}

#[derive(Clone, Copy)]
pub struct Toaster {
    items: Signal<Vec<ToastItem>>,
    next_id: Signal<u64>,
}

thread_local! {
    static GLOBAL_TOASTER: Cell<Option<Toaster>> = const { Cell::new(None) };
}

pub fn show_toast() -> Toaster {
    GLOBAL_TOASTER
        .with(Cell::get)
        .expect("show_toast() used before <ToastProvider> mounted")
}

pub fn try_toast() -> Option<Toaster> {
    GLOBAL_TOASTER.with(Cell::get)
}

impl Toaster {
    pub fn info(self, message: impl Into<String>) {
        self.push(ToastKind::Info, message.into());
    }

    pub fn success(self, message: impl Into<String>) {
        self.push(ToastKind::Success, message.into());
    }

    pub fn warn(self, message: impl Into<String>) {
        self.push(ToastKind::Warn, message.into());
    }

    pub fn error(self, message: impl Into<String>) {
        self.push(ToastKind::Error, message.into());
    }

    pub fn dismiss(self, id: u64) {
        let mut items = self.items;
        items.write().retain(|t| t.id != id);
    }

    fn push(self, kind: ToastKind, message: String) {
        let message = message.trim().to_string();
        if message.is_empty() {
            return;
        }
        let mut items = self.items;
        let mut next_id = self.next_id;
        let id = *next_id.peek();
        next_id.set(id.wrapping_add(1));
        {
            let mut list = items.write();
            list.push(ToastItem { id, kind, message });
            while list.len() > MAX_TOASTS {
                list.remove(0);
            }
        }
        spawn(async move {
            gloo_timers::future::TimeoutFuture::new(TOAST_MS).await;
            items.write().retain(|t| t.id != id);
        });
    }
}

#[component]
pub fn ToastProvider(children: Element) -> Element {
    let items = use_signal(Vec::<ToastItem>::new);
    let next_id = use_signal(|| 1u64);
    let toaster = Toaster { items, next_id };
    use_context_provider(|| toaster);
    GLOBAL_TOASTER.with(|c| c.set(Some(toaster)));
    rsx! {
        {children}
        ToastHost {}
    }
}

#[component]
fn ToastHost() -> Element {
    let toaster = use_context::<Toaster>();
    let items = toaster.items;
    rsx! {
        div { class: "toaster", "aria-live": "polite",
            for t in items() {
                {
                    let id = t.id;
                    let kind = match t.kind {
                        ToastKind::Info => "info",
                        ToastKind::Success => "ok",
                        ToastKind::Warn => "warn",
                        ToastKind::Error => "err",
                    };
                    rsx! {
                        div { key: "{id}", class: "toast toast-{kind}", role: "status",
                            div { class: "toast-row",
                                span { class: "toast-ico", "aria-hidden": "true",
                                    ToastIcon { kind: t.kind }
                                }
                                p { class: "toast-msg", "{t.message}" }
                                button {
                                    class: "toast-dismiss",
                                    r#type: "button",
                                    "aria-label": "Dismiss",
                                    onclick: move |_| toaster.dismiss(id),
                                    "OK"
                                }
                            }
                            div { class: "toast-track",
                                div { class: "toast-line" }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn ToastIcon(kind: ToastKind) -> Element {
    match kind {
        ToastKind::Info => rsx! {
            svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "1.8",
                path { d: "M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9" }
                path { d: "M10.3 21a1.94 1.94 0 0 0 3.4 0" }
            }
        },
        ToastKind::Success => rsx! {
            svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "2",
                circle { cx: "12", cy: "12", r: "10" }
                path { d: "m9 12 2 2 4-4" }
            }
        },
        ToastKind::Warn => rsx! {
            svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "2",
                path { d: "m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3" }
                path { d: "M12 9v4M12 17h.01" }
            }
        },
        ToastKind::Error => rsx! {
            svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "2",
                circle { cx: "12", cy: "12", r: "10" }
                path { d: "m15 9-6 6M9 9l6 6" }
            }
        },
    }
}
