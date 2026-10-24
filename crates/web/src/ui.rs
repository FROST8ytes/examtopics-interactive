//! Small shared UI pieces: header, banners, meters, loading and error states.

use crate::Route;
use crate::components::button::{Button, ButtonVariant};
use crate::supabase::auth::{self, SESSION};
use dioxus::prelude::*;
use dioxus_icons::lucide::{
    CircleAlert, CircleCheck, CircleX, Info, LibraryBig, LoaderCircle, LogOut, Upload, UserRound,
};

pub fn current_path() -> String {
    web_sys::window()
        .and_then(|w| {
            let l = w.location();
            Some(format!("{}{}", l.pathname().ok()?, l.search().ok()?))
        })
        .unwrap_or_else(|| "/".into())
}

#[component]
pub fn Header() -> Element {
    let route = use_route::<Route>();
    let nav = use_navigator();
    let mut menu_open = use_signal(|| false);
    let session = SESSION.read().clone();
    let Some(session) = session else {
        return rsx! {};
    };
    let user = session.user;
    let name = user.display_name();
    let initial = name
        .chars()
        .next()
        .unwrap_or('?')
        .to_uppercase()
        .to_string();
    let is_library = matches!(
        route,
        Route::Dashboard {} | Route::CertificationPage { .. } | Route::Quiz { .. }
    );

    rsx! {
        header { class: "app-header",
            Link { class: "brand", to: Route::Dashboard {},
                span { class: "brand-mark", "ET" }
                span { "ExamTopics Interactive" }
            }
            nav { class: "nav-links",
                Link {
                    class: if is_library { "nav-link active" } else { "nav-link" },
                    to: Route::Dashboard {},
                    span { class: "row", style: "gap:.4rem",
                        LibraryBig { size: "16px" }
                        span { class: "label", "Library" }
                    }
                }
                if !user.is_anonymous {
                    Link {
                        class: if matches!(route, Route::Upload {}) { "nav-link active" } else { "nav-link" },
                        to: Route::Upload {},
                        span { class: "row", style: "gap:.4rem",
                            Upload { size: "16px" }
                            span { class: "label", "Upload PDF" }
                        }
                    }
                }
            }
            div { class: "spacer" }
            div { style: "position:relative",
                button {
                    class: "user-btn",
                    onclick: move |_| menu_open.toggle(),
                    if let Some(url) = user.avatar_url() {
                        img { class: "avatar-img", src: "{url}", alt: "" }
                    } else {
                        span { class: "avatar-fallback", "{initial}" }
                    }
                    span { class: "small label", "{name}" }
                }
                if menu_open() {
                    div {
                        class: "menu",
                        onmouseleave: move |_| menu_open.set(false),
                        div { class: "menu-label",
                            if user.is_anonymous { "Guest account" } else { "{user.email.clone().unwrap_or_default()}" }
                        }
                        Link {
                            to: Route::Account { reset: String::new() },
                            onclick: move |_| menu_open.set(false),
                            span { class: "row", style: "gap:.5rem", UserRound { size: "15px" } "Account & sign-in methods" }
                        }
                        button {
                            onclick: move |_| async move {
                                menu_open.set(false);
                                auth::sign_out().await;
                                nav.replace(Route::Login { next: String::new() });
                            },
                            span { class: "row", style: "gap:.5rem", LogOut { size: "15px" } "Sign out" }
                        }
                    }
                }
            }
        }
    }
}

/// Reminds guests that their progress lives in an anonymous account.
#[component]
pub fn GuestBanner() -> Element {
    let is_guest = SESSION.read().as_ref().is_some_and(|s| s.user.is_anonymous);
    let route = use_route::<Route>();
    if !is_guest || matches!(route, Route::Account { .. }) {
        return rsx! {};
    }
    rsx! {
        div { class: "container", style: "padding-bottom:0",
            Banner { kind: "info",
                div { class: "row-between full",
                    span { class: "small",
                        "You're studying as a guest. Your progress is saved, but only in this browser. "
                        "Link Google, GitHub or an email to keep it everywhere."
                    }
                    Link { class: "link small bold", to: Route::Account { reset: String::new() }, "Save my progress →" }
                }
            }
        }
    }
}

#[component]
pub fn Banner(kind: &'static str, children: Element) -> Element {
    rsx! {
        div { class: "banner", "data-kind": kind,
            match kind {
                "ok" => rsx! { CircleCheck { size: "18px" } },
                "bad" => rsx! { CircleX { size: "18px" } },
                "warn" => rsx! { CircleAlert { size: "18px" } },
                _ => rsx! { Info { size: "18px" } },
            }
            div { class: "full", {children} }
        }
    }
}

#[component]
pub fn Spinner(#[props(default = "18px".to_string())] size: String) -> Element {
    rsx! { LoaderCircle { class: "spin", size } }
}

#[component]
pub fn Loading(#[props(default = "Loading…".to_string())] label: String) -> Element {
    rsx! {
        div { class: "center",
            div { class: "row muted", Spinner {} "{label}" }
        }
    }
}

#[component]
pub fn ErrorState(message: String, on_retry: Option<EventHandler<()>>) -> Element {
    rsx! {
        div { class: "container narrow",
            Banner { kind: "bad",
                div { class: "stack-sm",
                    span { class: "bold", "Something went wrong" }
                    span { class: "small", "{message}" }
                    if let Some(retry) = on_retry {
                        div {
                            Button { variant: ButtonVariant::Outline, onclick: move |_| retry.call(()), "Try again" }
                        }
                    }
                }
            }
        }
    }
}

/// Answered / correct meter.
#[component]
pub fn Meter(total: i32, answered: i32, correct: i32) -> Element {
    let total = total.max(1) as f64;
    let ok = correct as f64 / total * 100.0;
    let bad = (answered - correct).max(0) as f64 / total * 100.0;
    rsx! {
        div { class: "meter", title: "{correct} correct · {answered - correct} incorrect",
            span { class: "ok", style: "width:{ok}%" }
            span { class: "bad", style: "width:{bad}%" }
        }
    }
}

pub fn percent(part: i32, total: i32) -> i32 {
    if total <= 0 {
        0
    } else {
        ((part as f64 / total as f64) * 100.0).round() as i32
    }
}

/// Copies text with the async Clipboard API. Returns whether it worked.
pub async fn copy_to_clipboard(text: &str) -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };
    let promise = window.navigator().clipboard().write_text(text);
    wasm_bindgen_futures::JsFuture::from(promise).await.is_ok()
}
