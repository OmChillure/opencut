use crate::Route;
use crate::api::{self, ProjectSummary};
use crate::auth;
use dioxus::prelude::*;
use oc_core::{ExportPreset, Op};

pub fn confirm_delete(name: &str) -> bool {
    let msg = format!(
        "Delete “{name}”? This permanently removes the project, timeline, and all media from storage."
    );
    web_sys::window()
        .and_then(|w| w.confirm_with_message(&msg).ok())
        .unwrap_or(false)
}

fn try_login(email: Signal<String>, password: Signal<String>, mut error: Signal<Option<String>>) {
    let mail = email.read().trim().to_string();
    let pass = password.read().clone();
    if mail.is_empty() || !mail.contains('@') {
        error.set(Some("Enter a valid email".into()));
        return;
    }
    if pass.len() < 4 {
        error.set(Some("Password must be at least 4 characters".into()));
        return;
    }
    auth::sign_in(&mail);
    navigator().replace(Route::Projects {});
}

#[component]
pub fn Login() -> Element {
    let nav = navigator();
    let mut email = use_signal(String::new);
    let mut password = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);

    use_effect(move || {
        if auth::is_signed_in() {
            nav.replace(Route::Projects {});
        }
    });

    rsx! {
        document::Title { "OpenCut — Sign in" }
        div { class: "shell",
            div { class: "auth-card",
                div { class: "auth-brand",
                    crate::IconScissors {}
                    h1 { "OpenCut" }
                }
                p { class: "muted", "Sign in to open your projects." }
                label { "Email"
                    input {
                        r#type: "email",
                        placeholder: "you@studio.com",
                        value: "{email}",
                        oninput: move |e| email.set(e.value()),
                    }
                }
                label { "Password"
                    input {
                        r#type: "password",
                        placeholder: "••••••••",
                        value: "{password}",
                        oninput: move |e| password.set(e.value()),
                        onkeydown: move |e| {
                            if e.key() == Key::Enter {
                                try_login(email, password, error);
                            }
                        },
                    }
                }
                if let Some(msg) = error.read().as_ref() {
                    p { class: "form-error", "{msg}" }
                }
                button {
                    class: "btn btn-primary auth-submit",
                    onclick: move |_| try_login(email, password, error),
                    "Continue"
                }
                p { class: "hint", "Local sign-in for now. Any email + password works." }
            }
        }
    }
}

#[component]
pub fn Projects() -> Element {
    let nav = navigator();
    let mut projects = use_signal(Vec::<ProjectSummary>::new);
    let mut status = use_signal(|| "Loading…".to_string());
    let mut deleting = use_signal(|| None::<String>);

    use_effect(move || {
        if !auth::is_signed_in() {
            nav.replace(Route::Login {});
        }
    });

    use_future(move || async move {
        match api::list_projects().await {
            Ok(list) => {
                let msg = if list.is_empty() {
                    "No projects yet".into()
                } else {
                    format!("{} projects", list.len())
                };
                status.set(msg);
                projects.set(list);
            }
            Err(err) => status.set(format!("API offline — {err}")),
        }
    });

    let email = auth::current_email().unwrap_or_default();

    rsx! {
        document::Title { "OpenCut — Projects" }
        div { class: "shell",
            header { class: "shell-bar",
                div { class: "header-left",
                    span { class: "logo", crate::IconScissors {} }
                    strong { "Projects" }
                    span { class: "muted", "{email}" }
                }
                div { class: "header-right",
                    button {
                        class: "btn-ghost",
                        onclick: move |_| {
                            auth::sign_out();
                            nav.replace(Route::Login {});
                        },
                        "Sign out"
                    }
                    Link { to: Route::NewProject {}, class: "btn btn-primary", "New project" }
                }
            }
            div { class: "shell-body",
                div { class: "projects-head",
                    h1 { "Projects" }
                    p { class: "muted", "{status}" }
                }
                div { class: "project-grid",
                    Link { to: Route::NewProject {}, class: "project-card project-new",
                        div { class: "project-thumb new-thumb",
                            span { class: "plus", "+" }
                        }
                        b { "New project" }
                        span { "Start a blank timeline" }
                    }
                    for project in projects.read().iter() {
                        {
                            let id = project.id.clone();
                            let delete_id = id.clone();
                            let name = project.name.clone();
                            let delete_name = name.clone();
                            let updated = project.updated_at.clone().unwrap_or_default();
                            let busy = deleting.read().as_deref() == Some(id.as_str());
                            rsx! {
                                div { class: "project-card",
                                    Link {
                                        to: Route::Workspace { id: id.clone() },
                                        class: "project-open",
                                        div { class: "project-thumb" }
                                        b { "{name}" }
                                        span { "{updated}" }
                                    }
                                    button {
                                        class: "project-del",
                                        title: "Delete project",
                                        disabled: busy,
                                        onclick: move |evt| {
                                            evt.stop_propagation();
                                            if deleting.read().is_some() {
                                                return;
                                            }
                                            if !confirm_delete(&delete_name) {
                                                return;
                                            }
                                            let pid = delete_id.clone();
                                            deleting.set(Some(pid.clone()));
                                            projects.write().retain(|p| p.id != pid);
                                            let n = projects.read().len();
                                            status.set(if n == 0 {
                                                "Deleting…".into()
                                            } else {
                                                format!("{n} projects")
                                            });
                                            spawn(async move {
                                                if let Err(err) = api::delete_project(&pid).await {
                                                    status.set(format!("Delete failed — {err}"));
                                                } else {
                                                    let n = projects.read().len();
                                                    status.set(if n == 0 {
                                                        "No projects yet".into()
                                                    } else {
                                                        format!("{n} projects")
                                                    });
                                                }
                                                deleting.set(None);
                                            });
                                        },
                                        if busy { "…" } else { "Delete" }
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

#[component]
pub fn Export(id: String) -> Element {
    let nav = navigator();
    let project_id = id.clone();
    let mut name = use_signal(|| "Project".to_string());
    let mut preset = use_signal(|| ExportPreset::Youtube1080);
    let mut note = use_signal(String::new);
    let mut url = use_signal(String::new);
    let mut busy = use_signal(|| false);

    use_effect(move || {
        if !auth::is_signed_in() {
            nav.replace(Route::Login {});
        }
    });
    let load_id = project_id.clone();
    use_future(move || {
        let project_id = load_id.clone();
        async move {
            let Ok(project) = api::get_project(&project_id).await else {
                return;
            };
            name.set(project.name);
            let tl = &project.timeline;
            if tl.height > tl.width {
                preset.set(ExportPreset::Vertical1080);
            } else if tl.width == tl.height {
                preset.set(ExportPreset::Square1080);
            } else {
                preset.set(ExportPreset::Youtube1080);
            }
        }
    });

    let choices = [
        (ExportPreset::Youtube1080, "YouTube", "1920 × 1080"),
        (ExportPreset::Vertical1080, "Vertical", "1080 × 1920"),
        (ExportPreset::Square1080, "Square", "1080 × 1080"),
    ];
    let selected = *preset.read();
    let back = id.clone();

    rsx! {
        document::Title { "OpenCut — Export" }
        div { class: "shell",
            header { class: "shell-bar",
                div { class: "header-left",
                    Link {
                        to: Route::Workspace { id: back.clone() },
                        class: "logo",
                        title: "Back to the edit",
                        crate::IconScissors {}
                    }
                    strong { "{name}" }
                }
            }
            div { class: "shell-body",
                div { class: "export-page",
                    h1 { "Export" }
                    p { class: "export-lead", "Render the current timeline. The file plays here when the worker finishes." }
                    div { class: "export-options",
                        for (kind, label, size) in choices {
                            button {
                                class: if selected == kind { "export-opt on" } else { "export-opt" },
                                onclick: move |_| preset.set(kind),
                                strong { "{label}" }
                                span { "{size}" }
                            }
                        }
                    }
                    button {
                        class: "btn btn-primary",
                        disabled: *busy.read(),
                        onclick: move |_| {
                            let kind = *preset.peek();
                            let pid = project_id.clone();
                            busy.set(true);
                            note.set("Rendering. This page updates when the file is ready.".into());
                            url.set(String::new());
                            spawn(async move {
                                if let Err(err) = api::apply_ops(&pid, vec![Op::Export { preset: kind }]).await {
                                    note.set(err);
                                    busy.set(false);
                                    return;
                                }
                                for _ in 0..300 {
                                    gloo_timers::future::TimeoutFuture::new(2000).await;
                                    if api::export_is_ready(&pid).await {
                                        let stamp = js_sys::Date::now() as u64;
                                        let file = api::export_file_url(&pid);
                                        let join = if file.contains('?') { '&' } else { '?' };
                                        url.set(format!("{file}{join}v={stamp}"));
                                        note.set("Ready to play.".into());
                                        busy.set(false);
                                        return;
                                    }
                                }
                                note.set("The render did not show up yet. Check the worker.".into());
                                busy.set(false);
                            });
                        },
                        if *busy.read() { "Rendering…" } else { "Render" }
                    }
                    if !note.read().is_empty() {
                        p { class: "export-note", "{note}" }
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
    }
}

#[component]
pub fn NewProject() -> Element {
    let nav = navigator();
    let mut name = use_signal(|| "Untitled".to_string());
    let mut error = use_signal(|| None::<String>);
    let mut busy = use_signal(|| false);

    use_effect(move || {
        if !auth::is_signed_in() {
            nav.replace(Route::Login {});
        }
    });

    rsx! {
        document::Title { "OpenCut — New project" }
        div { class: "shell",
            header { class: "shell-bar",
                div { class: "header-left",
                    Link { to: Route::Projects {}, class: "logo", crate::IconScissors {} }
                    strong { "New project" }
                }
            }
            div { class: "shell-body",
                div { class: "create-form",
                    h1 { "New project" }
                    label { "Name"
                        input {
                            value: "{name}",
                            oninput: move |e| name.set(e.value()),
                        }
                    }
                    if let Some(msg) = error.read().as_ref() {
                        p { class: "form-error", "{msg}" }
                    }
                    button {
                        class: "btn btn-primary auth-submit",
                        disabled: *busy.read(),
                        onclick: move |_| {
                            let title = name.read().trim().to_string();
                            if title.is_empty() {
                                error.set(Some("Give the project a name".into()));
                                return;
                            }
                            busy.set(true);
                            spawn(async move {
                                match api::create_project(&title).await {
                                    Ok(project) => {
                                        nav.replace(Route::Workspace { id: project.id });
                                    }
                                    Err(err) => {
                                        error.set(Some(err));
                                        busy.set(false);
                                    }
                                }
                            });
                        },
                        if *busy.read() { "Creating…" } else { "Create and open" }
                    }
                }
            }
        }
    }
}
