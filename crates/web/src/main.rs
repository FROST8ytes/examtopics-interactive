#[allow(unused_imports, dead_code)]
mod components;
mod data;
mod pages;
mod pdf;
mod supabase;
mod ui;

use components::toast::ToastProvider;
use dioxus::prelude::*;
use pages::*;
use supabase::auth::SESSION;

const FAVICON: Asset = asset!("/assets/favicon.svg");
const THEME_CSS: Asset = asset!("/assets/dx-components-theme.css");
const APP_CSS: Asset = asset!("/assets/app.css");

#[derive(Debug, Clone, Routable, PartialEq)]
#[rustfmt::skip]
pub enum Route {
    #[route("/login?:next")]
    Login { next: String },
    #[route("/auth/callback")]
    AuthCallback {},
    #[layout(AppShell)]
        #[route("/")]
        Dashboard {},
        #[route("/certifications/:id")]
        CertificationPage { id: String },
        #[route("/sets/:id?:q")]
        Quiz { id: String, q: String },
        #[route("/upload")]
        Upload {},
        #[route("/account?:reset")]
        Account { reset: String },
        #[route("/questions/:id/edit")]
        EditQuestion { id: String },
    #[end_layout]
    #[route("/:..segments")]
    NotFound { segments: Vec<String> },
}

fn main() {
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    rsx! {
        document::Link { rel: "icon", href: FAVICON }
        document::Link { rel: "stylesheet", href: THEME_CSS }
        document::Link { rel: "stylesheet", href: APP_CSS }
        document::Title { "ExamTopics Interactive" }
        ToastProvider {
            if supabase::configured() {
                Router::<Route> {}
            } else {
                MissingConfig {}
            }
        }
    }
}

#[component]
fn MissingConfig() -> Element {
    rsx! {
        div { class: "center container narrow",
            div { class: "stack",
                h1 { "Supabase is not configured" }
                p { class: "muted",
                    "Build the app with the "
                    code { "SUPABASE_URL" }
                    " and "
                    code { "SUPABASE_PUBLISHABLE_KEY" }
                    " environment variables set. See README.md."
                }
            }
        }
    }
}

/// Layout for every signed-in page: redirects to the login page without a session.
#[component]
fn AppShell() -> Element {
    let nav = use_navigator();
    let signed_in = SESSION.read().is_some();

    use_effect(move || {
        if SESSION.read().is_none() {
            nav.replace(Route::Login {
                next: ui::current_path(),
            });
        }
    });

    if !signed_in {
        return rsx! {};
    }
    rsx! {
        ui::Header {}
        ui::GuestBanner {}
        Outlet::<Route> {}
    }
}

#[component]
fn NotFound(segments: Vec<String>) -> Element {
    rsx! {
        div { class: "center container",
            div { class: "stack",
                h1 { "Page not found" }
                p { class: "muted", "/{segments.join(\"/\")}" }
                Link { class: "link", to: Route::Dashboard {}, "Back to the library" }
            }
        }
    }
}
