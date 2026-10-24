//! Account: sign-in methods (link / unlink Google & GitHub), set a password so OAuth users
//! can also sign in with email, and turn a guest into a permanent account.

use super::login::{GithubIcon, GoogleIcon, friendly};
use crate::Route;
use crate::components::badge::{Badge, BadgeVariant};
use crate::components::button::{Button, ButtonSize, ButtonVariant};
use crate::components::card::*;
use crate::components::input::Input;
use crate::components::label::Label;
use crate::supabase::auth::{self, Provider, SESSION};
use crate::ui::{Banner, Spinner};
use dioxus::prelude::*;
use dioxus_icons::lucide::{KeyRound, LogOut, Mail};
use dioxus_primitives::toast::{ToastOptions, use_toast};

#[component]
pub fn Account(reset: String) -> Element {
    let nav = use_navigator();
    let toast = use_toast();
    let session = SESSION.read().clone();
    let Some(session) = session else {
        return rsx! {};
    };
    let user = session.user.clone();
    let is_reset = !reset.is_empty();

    // Refresh identities when the page opens (e.g. right after linking).
    use_future(|| async {
        let _ = auth::refresh_user().await;
    });

    let mut password = use_signal(String::new);
    let mut confirm = use_signal(String::new);
    let mut pw_busy = use_signal(|| false);
    let mut pw_error = use_signal(|| None::<String>);

    let mut new_email = use_signal(String::new);
    let mut email_busy = use_signal(|| false);
    let mut email_msg = use_signal(|| None::<Result<String, String>>);

    let mut name = use_signal(|| user.display_name());
    let mut link_error = use_signal(|| None::<String>);

    let save_password = move |e: FormEvent| async move {
        e.prevent_default();
        pw_error.set(None);
        if password().len() < 8 {
            pw_error.set(Some("Use at least 8 characters.".into()));
            return;
        }
        if password() != confirm() {
            pw_error.set(Some("Passwords don't match.".into()));
            return;
        }
        pw_busy.set(true);
        let res = auth::update_password(&password()).await;
        pw_busy.set(false);
        match res {
            Ok(_) => {
                password.set(String::new());
                confirm.set(String::new());
                toast.success(
                    "Password saved".into(),
                    ToastOptions::new()
                        .description("You can now sign in with your email and this password."),
                );
                if is_reset {
                    nav.replace(Route::Account {
                        reset: String::new(),
                    });
                }
            }
            Err(e) => pw_error.set(Some(
                if e.code.as_deref() == Some("reauthentication_needed") {
                    "For security, sign out and sign in again before changing your password.".into()
                } else {
                    friendly(&e.message, e.code.as_deref())
                },
            )),
        }
    };

    let save_email = move |e: FormEvent| async move {
        e.prevent_default();
        email_busy.set(true);
        let res = auth::update_email(new_email().trim()).await;
        email_busy.set(false);
        email_msg.set(Some(match res {
            Ok(_) => Ok(format!(
                "We sent a confirmation link to {}. Open it to finish — then set a password here.",
                new_email().trim()
            )),
            Err(e) => Err(friendly(&e.message, e.code.as_deref())),
        }));
    };

    let link = move |p: Provider| async move {
        link_error.set(None);
        if let Err(e) = auth::link_identity(p).await {
            link_error.set(Some(friendly(&e.message, e.code.as_deref())));
        }
    };

    let identities = user.identities.clone();
    let can_unlink = identities.len() > 1;

    rsx! {
        div { class: "container narrow stack-lg",
            div { class: "page-head",
                div {
                    h1 { "Account" }
                    p { class: "muted",
                        if user.is_anonymous { "You're using a guest account." }
                        else { "Signed in as {user.email.clone().unwrap_or_default()}" }
                    }
                }
                Button {
                    variant: ButtonVariant::Outline,
                    onclick: move |_| async move {
                        auth::sign_out().await;
                        nav.replace(Route::Login { next: String::new() });
                    },
                    LogOut { size: "16px" } "Sign out"
                }
            }

            if is_reset {
                Banner { kind: "info", span { class: "small", "Choose a new password below to finish resetting it." } }
            }

            if user.is_anonymous {
                Card {
                    CardHeader {
                        CardTitle { "Keep your progress" }
                        CardDescription { "Link a sign-in method to turn this guest account into a permanent one. Nothing is lost." }
                    }
                    CardContent {
                        div { class: "stack",
                            div { class: "row",
                                Button { variant: ButtonVariant::Outline, onclick: move |_| link(Provider::Google), GoogleIcon {} "Link Google" }
                                Button { variant: ButtonVariant::Outline, onclick: move |_| link(Provider::Github), GithubIcon {} "Link GitHub" }
                            }
                            if let Some(e) = link_error() { p { class: "error-text", "{e}" } }
                            div { class: "or", "or use email" }
                            form { class: "row", onsubmit: save_email,
                                Input {
                                    r#type: "email", required: true, placeholder: "you@company.com", style: "flex:1;min-width:14rem",
                                    value: new_email(), oninput: move |e: FormEvent| new_email.set(e.value()),
                                }
                                Button { r#type: "submit", disabled: email_busy(), Mail { size: "16px" } "Send link" }
                            }
                            match email_msg() {
                                Some(Ok(m)) => rsx! { Banner { kind: "ok", span { class: "small", "{m}" } } },
                                Some(Err(m)) => rsx! { p { class: "error-text", "{m}" } },
                                None => rsx! {},
                            }
                            p { class: "small muted",
                                "Already have an account? "
                                Link { class: "link", to: Route::Login { next: "/".into() }, "Sign in to it" }
                                " — your guest progress will be merged into it."
                            }
                        }
                    }
                }
            } else {
                Card {
                    CardHeader {
                        CardTitle { "Profile" }
                    }
                    CardContent {
                        form {
                            class: "row",
                            onsubmit: move |e: FormEvent| async move {
                                e.prevent_default();
                                match auth::update_display_name(name().trim()).await {
                                    Ok(_) => toast.success("Saved".into(), ToastOptions::new()),
                                    Err(e) => toast.error("Couldn't save".into(), ToastOptions::new().description(e.to_string())),
                                }
                            },
                            Input { style: "flex:1;min-width:14rem", value: name(), oninput: move |e: FormEvent| name.set(e.value()) }
                            Button { r#type: "submit", variant: ButtonVariant::Secondary, "Save name" }
                        }
                    }
                }
                Card {
                    CardHeader {
                        CardTitle { "Sign-in methods" }
                        CardDescription {
                            "Accounts with the same verified email are linked automatically — signing in with GitHub after Google (same email) opens this same account. "
                            "You can also connect a provider that uses a different email."
                        }
                    }
                    CardContent {
                        div { class: "stack",
                            for provider in [Provider::Google, Provider::Github] {
                                {
                                    let identity = identities.iter().find(|i| i.provider == provider.id()).cloned();
                                    rsx! {
                                        div { key: "{provider.id()}", class: "row-between",
                                            div { class: "row",
                                                if provider == Provider::Google { GoogleIcon {} } else { GithubIcon {} }
                                                span { class: "bold", "{provider.label()}" }
                                                if let Some(i) = &identity {
                                                    Badge { variant: BadgeVariant::Secondary, "Connected" }
                                                    if let Some(email) = i.email() { span { class: "small muted", "{email}" } }
                                                }
                                            }
                                            match identity {
                                                Some(i) => rsx! {
                                                    Button {
                                                        size: ButtonSize::Sm,
                                                        variant: ButtonVariant::Ghost,
                                                        disabled: !can_unlink,
                                                        title: if can_unlink { "Disconnect" } else { "You need at least one other sign-in method" },
                                                        onclick: move |_| {
                                                            let id = i.identity_id.clone();
                                                            async move {
                                                                if let Err(e) = auth::unlink_identity(&id).await {
                                                                    link_error.set(Some(friendly(&e.message, e.code.as_deref())));
                                                                }
                                                            }
                                                        },
                                                        "Disconnect"
                                                    }
                                                },
                                                None => rsx! {
                                                    Button { size: ButtonSize::Sm, variant: ButtonVariant::Outline, onclick: move |_| link(provider), "Connect" }
                                                },
                                            }
                                        }
                                    }
                                }
                            }
                            div { class: "row-between",
                                div { class: "row",
                                    Mail { size: "16px" }
                                    span { class: "bold", "Email" }
                                    span { class: "small muted", "{user.email.clone().unwrap_or_default()}" }
                                    if user.has_provider("email") { Badge { variant: BadgeVariant::Secondary, "Connected" } }
                                }
                            }
                            if let Some(e) = link_error() { p { class: "error-text", "{e}" } }
                        }
                    }
                }
            }

            if !user.is_anonymous || user.email.is_some() {
                Card {
                    CardHeader {
                        CardTitle { span { class: "row", style: "gap:.5rem", KeyRound { size: "18px" } "Password" } }
                        CardDescription {
                            "Set a password to sign in with {user.email.clone().unwrap_or_else(|| \"your email\".into())} — even if you signed up with Google or GitHub."
                        }
                    }
                    CardContent {
                        form { class: "stack", onsubmit: save_password,
                            div { class: "field-row",
                                div { class: "field",
                                    Label { html_for: "pw", "New password" }
                                    Input { id: "pw", r#type: "password", autocomplete: "new-password", value: password(), oninput: move |e: FormEvent| password.set(e.value()) }
                                }
                                div { class: "field",
                                    Label { html_for: "pw2", "Confirm password" }
                                    Input { id: "pw2", r#type: "password", autocomplete: "new-password", value: confirm(), oninput: move |e: FormEvent| confirm.set(e.value()) }
                                }
                            }
                            if let Some(e) = pw_error() { p { class: "error-text", "{e}" } }
                            div {
                                Button { r#type: "submit", disabled: pw_busy(),
                                    if pw_busy() { Spinner { size: "16px" } }
                                    "Save password"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
