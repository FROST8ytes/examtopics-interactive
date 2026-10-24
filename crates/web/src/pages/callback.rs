//! `/auth/callback`: finishes OAuth (PKCE `?code=`), identity linking and email links
//! (`#access_token=…&type=signup|recovery|email_change`).

use super::login::friendly;
use crate::Route;
use crate::components::button::Button;
use crate::supabase::auth::{self, Provider, SESSION};
use crate::ui::{Banner, Loading};
use dioxus::prelude::*;

#[component]
pub fn AuthCallback() -> Element {
    let nav = use_navigator();
    let mut error = use_signal(|| None::<(String, Option<String>)>);

    use_future(move || async move {
        let (search, hash) = auth::landing_url();
        let query = web_sys::UrlSearchParams::new_with_str(&search).ok();
        let fragment = web_sys::UrlSearchParams::new_with_str(hash.trim_start_matches('#')).ok();
        let get = |k: &str| {
            query
                .as_ref()
                .and_then(|q| q.get(k))
                .or_else(|| fragment.as_ref().and_then(|f| f.get(k)))
        };

        if let Some(description) = get("error_description").or_else(|| get("error")) {
            let code = get("error_code");
            error.set(Some((
                friendly(&description.replace('+', " "), code.as_deref()),
                code,
            )));
            return;
        }

        let result = if let Some(code) = get("code") {
            auth::exchange_code(&code).await.map(|_| None)
        } else if hash.contains("access_token=") {
            auth::session_from_fragment(&hash)
                .await
                .map(|(_, kind)| kind)
        } else if SESSION.peek().is_some() {
            Ok(None)
        } else {
            Err(crate::supabase::ApiError::msg(
                "The sign-in link is incomplete or has expired.",
            ))
        };

        match result {
            Ok(Some(kind)) if kind == "recovery" => {
                nav.replace(Route::Account { reset: "1".into() });
            }
            Ok(_) => {
                nav.replace(auth::take_next());
            }
            Err(e) => error.set(Some((friendly(&e.message, e.code.as_deref()), e.code))),
        }
    });

    match error() {
        None => rsx! { Loading { label: "Signing you in…" } },
        Some((message, code)) => {
            let identity_taken = code.as_deref() == Some("identity_already_exists");
            rsx! {
                div { class: "auth-wrap",
                    div { class: "auth-card stack",
                        Banner { kind: "bad",
                            div { class: "stack-sm",
                                span { class: "bold", "Couldn't complete sign-in" }
                                span { class: "small", "{message}" }
                            }
                        }
                        if identity_taken {
                            p { class: "small muted",
                                "Sign in to that existing account instead. Your guest progress will be merged into it."
                            }
                            div { class: "row",
                                Button { onclick: move |_| async move { auth::sign_in_with(Provider::Google).await }, "Sign in with Google" }
                                Button { onclick: move |_| async move { auth::sign_in_with(Provider::Github).await }, "Sign in with GitHub" }
                            }
                        }
                        Link { class: "link small", to: Route::Login { next: String::new() }, "Back to sign in" }
                    }
                }
            }
        }
    }
}
