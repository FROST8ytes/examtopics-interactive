use crate::components::button::{Button, ButtonVariant};
use crate::components::card::*;
use crate::components::input::Input;
use crate::components::label::Label;
use crate::supabase::auth::{self, Provider, SESSION, SignUpOutcome};
use crate::ui::{Banner, Spinner};
use dioxus::prelude::*;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    SignIn,
    SignUp,
    Reset,
}

fn target(next: &str) -> String {
    if next.starts_with('/') && !next.starts_with("//") && !next.starts_with("/login") {
        next.to_string()
    } else {
        "/".to_string()
    }
}

#[component]
pub fn Login(next: String) -> Element {
    let nav = use_navigator();
    let mut mode = use_signal(|| Mode::SignIn);
    let mut email = use_signal(String::new);
    let mut password = use_signal(String::new);
    let mut busy = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let mut info = use_signal(|| None::<String>);
    let next = target(&next);

    let is_guest = SESSION.read().as_ref().is_some_and(|s| s.user.is_anonymous);
    {
        let next = next.clone();
        use_effect(move || {
            if SESSION
                .read()
                .as_ref()
                .is_some_and(|s| !s.user.is_anonymous)
            {
                nav.replace(next.clone());
            }
        });
    }

    let submit = {
        let next = next.clone();
        move |evt: FormEvent| {
            evt.prevent_default();
            let next = next.clone();
            async move {
                busy.set(true);
                error.set(None);
                info.set(None);
                let email_v = email().trim().to_string();
                let password_v = password();
                let result = match mode() {
                    Mode::SignIn => auth::sign_in_password(&email_v, &password_v).await.map(|_| None),
                    Mode::SignUp => auth::sign_up(&email_v, &password_v).await.map(|o| match o {
                        SignUpOutcome::SignedIn => None,
                        SignUpOutcome::ConfirmationSent => Some(format!(
                            "Check {email_v} for a confirmation link. If you previously signed in with \
                             Google or GitHub using this email, sign in with that instead and set a \
                             password from your account page."
                        )),
                    }),
                    Mode::Reset => auth::send_password_reset(&email_v)
                        .await
                        .map(|_| Some(format!("If an account exists for {email_v}, a reset link is on its way."))),
                };
                busy.set(false);
                match result {
                    Ok(Some(message)) => info.set(Some(message)),
                    Ok(None) => {
                        nav.replace(next);
                    }
                    Err(e) => error.set(Some(friendly(&e.message, e.code.as_deref()))),
                }
            }
        }
    };

    let next_for_guest = next.clone();
    let oauth = move |provider: Provider| {
        let next = next.clone();
        async move {
            busy.set(true);
            auth::remember_next(&next);
            auth::sign_in_with(provider).await;
        }
    };
    let oauth_google = oauth.clone();
    let oauth_github = oauth.clone();

    let guest = {
        let next = next_for_guest;
        move |_| {
            let next = next.clone();
            async move {
                busy.set(true);
                error.set(None);
                match auth::sign_in_anonymously().await {
                    Ok(_) => {
                        nav.replace(next);
                    }
                    Err(e) => {
                        busy.set(false);
                        error.set(Some(friendly(&e.message, e.code.as_deref())));
                    }
                }
            }
        }
    };

    let title = match mode() {
        Mode::SignIn => "Welcome back",
        Mode::SignUp => "Create your account",
        Mode::Reset => "Reset your password",
    };

    rsx! {
        div { class: "auth-wrap",
            div { class: "auth-card stack",
                div { class: "row", style: "justify-content:center",
                    span { class: "brand-mark", "ET" }
                    span { class: "bold", "ExamTopics Interactive" }
                }
                Card {
                    CardHeader {
                        CardTitle { "{title}" }
                        CardDescription {
                            match mode() {
                                Mode::SignIn => "Pick up exactly where you left off.",
                                Mode::SignUp => "Track progress across devices.",
                                Mode::Reset => "We'll email you a link to choose a new password.",
                            }
                        }
                    }
                    CardContent {
                        div { class: "stack",
                            if is_guest {
                                Banner { kind: "info",
                                    span { class: "small",
                                        "You're currently a guest. Signing in moves your guest progress into that account."
                                    }
                                }
                            }
                            if mode() != Mode::Reset {
                                div { class: "stack-sm",
                                    Button {
                                        class: "oauth-btn",
                                        variant: ButtonVariant::Outline,
                                        disabled: busy(),
                                        onclick: move |_| oauth_google(Provider::Google),
                                        GoogleIcon {}
                                        "Continue with Google"
                                    }
                                    Button {
                                        class: "oauth-btn",
                                        variant: ButtonVariant::Outline,
                                        disabled: busy(),
                                        onclick: move |_| oauth_github(Provider::Github),
                                        GithubIcon {}
                                        "Continue with GitHub"
                                    }
                                }
                                div { class: "or", "or" }
                            }
                            form { class: "stack", onsubmit: submit,
                                div { class: "field",
                                    Label { html_for: "email", "Email" }
                                    Input {
                                        id: "email",
                                        r#type: "email",
                                        required: true,
                                        autocomplete: "email",
                                        placeholder: "you@company.com",
                                        value: email(),
                                        oninput: move |e: FormEvent| email.set(e.value()),
                                    }
                                }
                                if mode() != Mode::Reset {
                                    div { class: "field",
                                        div { class: "row-between",
                                            Label { html_for: "password", "Password" }
                                            if mode() == Mode::SignIn {
                                                button {
                                                    r#type: "button",
                                                    class: "link small",
                                                    onclick: move |_| mode.set(Mode::Reset),
                                                    "Forgot password?"
                                                }
                                            }
                                        }
                                        Input {
                                            id: "password",
                                            r#type: "password",
                                            required: true,
                                            minlength: if mode() == Mode::SignUp { "8" } else { "1" },
                                            autocomplete: if mode() == Mode::SignUp { "new-password" } else { "current-password" },
                                            value: password(),
                                            oninput: move |e: FormEvent| password.set(e.value()),
                                        }
                                    }
                                }
                                if let Some(err) = error() {
                                    p { class: "error-text", "{err}" }
                                }
                                if let Some(msg) = info() {
                                    Banner { kind: "ok", span { class: "small", "{msg}" } }
                                }
                                Button { r#type: "submit", class: "full", disabled: busy(),
                                    if busy() { Spinner { size: "16px" } }
                                    match mode() {
                                        Mode::SignIn => "Sign in",
                                        Mode::SignUp => "Create account",
                                        Mode::Reset => "Send reset link",
                                    }
                                }
                            }
                            div { class: "small muted", style: "text-align:center",
                                match mode() {
                                    Mode::SignIn => rsx! {
                                        "No account yet? "
                                        button { class: "link", onclick: move |_| mode.set(Mode::SignUp), "Create one" }
                                    },
                                    _ => rsx! {
                                        "Already have an account? "
                                        button { class: "link", onclick: move |_| mode.set(Mode::SignIn), "Sign in" }
                                    },
                                }
                            }
                        }
                    }
                }
                if !is_guest {
                    div { class: "stack-sm", style: "text-align:center",
                        Button { variant: ButtonVariant::Ghost, disabled: busy(), onclick: guest,
                            "Continue as guest"
                        }
                        p { class: "tiny muted",
                            "Guests can study and track progress in this browser. You can link an account later without losing anything."
                        }
                    }
                }
            }
        }
    }
}

/// Maps common Supabase Auth errors to clearer messages.
pub fn friendly(message: &str, code: Option<&str>) -> String {
    match code.unwrap_or_default() {
        "invalid_credentials" => "Wrong email or password. If you usually sign in with Google or GitHub, use that button — you can add a password afterwards on the Account page.".into(),
        "email_not_confirmed" => "Please confirm your email first — check your inbox for the link.".into(),
        "anonymous_provider_disabled" => "Guest access is disabled in Supabase (Authentication → Sign In / Providers → Allow anonymous sign-ins).".into(),
        "user_already_exists" | "email_exists" => "An account with this email already exists. Sign in instead.".into(),
        "weak_password" => format!("Password is too weak: {message}"),
        "over_email_send_rate_limit" => "Too many emails sent. Please wait a minute and try again.".into(),
        "flow_state_not_found" | "flow_state_expired" | "bad_code_verifier" => {
            "This sign-in link has expired or was already used. Please sign in again.".into()
        }
        "identity_already_exists" => "That account is already linked to another user. Sign in with it instead — your guest progress will be merged.".into(),
        "manual_linking_disabled" => "Account linking is disabled in Supabase (Authentication → Settings → Allow manual linking).".into(),
        _ => message.to_string(),
    }
}

#[component]
pub fn GoogleIcon() -> Element {
    rsx! {
        svg { width: "16", height: "16", view_box: "0 0 24 24",
            path { fill: "#4285F4", d: "M22.56 12.25c0-.78-.07-1.53-.2-2.25H12v4.26h5.92a5.06 5.06 0 0 1-2.2 3.32v2.77h3.57c2.08-1.92 3.27-4.74 3.27-8.1z" }
            path { fill: "#34A853", d: "M12 23c2.97 0 5.46-.98 7.28-2.66l-3.57-2.77c-.98.66-2.23 1.06-3.71 1.06-2.86 0-5.29-1.93-6.16-4.53H2.18v2.84A11 11 0 0 0 12 23z" }
            path { fill: "#FBBC05", d: "M5.84 14.1A6.6 6.6 0 0 1 5.5 12c0-.73.13-1.44.34-2.1V7.06H2.18A11 11 0 0 0 1 12c0 1.78.43 3.46 1.18 4.94l3.66-2.84z" }
            path { fill: "#EA4335", d: "M12 5.38c1.62 0 3.06.56 4.21 1.64l3.15-3.15C17.45 2.09 14.97 1 12 1A11 11 0 0 0 2.18 7.06l3.66 2.84C6.71 7.31 9.14 5.38 12 5.38z" }
        }
    }
}

#[component]
pub fn GithubIcon() -> Element {
    rsx! {
        svg { width: "16", height: "16", view_box: "0 0 24 24", fill: "currentColor",
            path { d: "M12 .5a11.5 11.5 0 0 0-3.64 22.41c.58.1.79-.25.79-.56v-2c-3.2.7-3.88-1.37-3.88-1.37-.52-1.33-1.28-1.69-1.28-1.69-1.05-.71.08-.7.08-.7 1.16.08 1.77 1.19 1.77 1.19 1.03 1.77 2.7 1.26 3.36.96.1-.75.4-1.26.73-1.55-2.55-.29-5.24-1.28-5.24-5.69 0-1.26.45-2.29 1.19-3.1-.12-.29-.52-1.46.11-3.05 0 0 .97-.31 3.17 1.18a11 11 0 0 1 5.77 0c2.2-1.49 3.17-1.18 3.17-1.18.63 1.59.23 2.76.11 3.05.74.81 1.19 1.84 1.19 3.1 0 4.42-2.7 5.4-5.26 5.68.41.36.78 1.06.78 2.14v3.17c0 .31.21.67.8.56A11.5 11.5 0 0 0 12 .5z" }
        }
    }
}
