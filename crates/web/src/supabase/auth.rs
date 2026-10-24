//! Supabase Auth (GoTrue) over REST.
//!
//! * Email + password, Google and GitHub (PKCE), anonymous guests.
//! * Supabase links identities that share a verified email automatically, so signing in
//!   with GitHub after Google (same email) lands in the same account.
//! * OAuth users can add a password (`update_password`) to sign in with email later.
//! * Guests can link a provider / email to keep their progress, or sign in to an existing
//!   account — then their progress is merged through a one-time merge token.

use super::{ApiError, Result, api_key, base_url, check, encode, http};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use dioxus::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const SESSION_KEY: &str = "et.session";
const VERIFIER_KEY: &str = "et.pkce_verifier";
const MERGE_KEY: &str = "et.guest_merge_token";
const NEXT_KEY: &str = "et.after_login";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix seconds.
    pub expires_at: i64,
    pub user: User,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    #[serde(default, deserialize_with = "empty_as_none")]
    pub email: Option<String>,
    #[serde(default, deserialize_with = "empty_as_none")]
    pub new_email: Option<String>,
    #[serde(default)]
    pub is_anonymous: bool,
    #[serde(default)]
    pub identities: Vec<Identity>,
    #[serde(default)]
    pub user_metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Identity {
    #[serde(default)]
    pub identity_id: String,
    pub provider: String,
    #[serde(default)]
    pub identity_data: Value,
}

impl Identity {
    pub fn email(&self) -> Option<&str> {
        self.identity_data.get("email").and_then(Value::as_str)
    }
}

/// Supabase returns `""` instead of `null` for users without an email (guests).
fn empty_as_none<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<String>, D::Error> {
    Ok(Option::<String>::deserialize(d)?.filter(|s| !s.trim().is_empty()))
}

impl User {
    pub fn display_name(&self) -> String {
        let meta = |k: &str| {
            self.user_metadata
                .get(k)
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .map(str::to_string)
        };
        meta("full_name")
            .or_else(|| meta("name"))
            .or_else(|| meta("user_name"))
            .or_else(|| self.email.clone())
            .unwrap_or_else(|| {
                if self.is_anonymous {
                    "Guest".into()
                } else {
                    "User".into()
                }
            })
    }

    pub fn avatar_url(&self) -> Option<String> {
        self.user_metadata
            .get("avatar_url")
            .and_then(Value::as_str)
            .map(str::to_string)
    }

    pub fn has_provider(&self, provider: &str) -> bool {
        self.identities.iter().any(|i| i.provider == provider)
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    #[serde(default)]
    expires_in: Option<i64>,
    #[serde(default)]
    expires_at: Option<i64>,
    user: User,
}

impl From<TokenResponse> for Session {
    fn from(t: TokenResponse) -> Self {
        let expires_at = t
            .expires_at
            .unwrap_or_else(|| now_secs() + t.expires_in.unwrap_or(3600));
        Session {
            access_token: t.access_token,
            refresh_token: t.refresh_token,
            expires_at,
            user: t.user,
        }
    }
}

/// The signed-in session, shared by the whole app.
pub static SESSION: GlobalSignal<Option<Session>> = Signal::global(load_session);

pub fn now_secs() -> i64 {
    (js_sys::Date::now() / 1000.0) as i64
}

// ---- local storage ----------------------------------------------------------

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

pub fn store_get(key: &str) -> Option<String> {
    storage()?.get_item(key).ok()?
}

pub fn store_set(key: &str, value: &str) {
    if let Some(s) = storage() {
        let _ = s.set_item(key, value);
    }
}

pub fn store_remove(key: &str) {
    if let Some(s) = storage() {
        let _ = s.remove_item(key);
    }
}

fn load_session() -> Option<Session> {
    serde_json::from_str(&store_get(SESSION_KEY)?).ok()
}

fn set_session(session: Option<Session>) {
    match &session {
        Some(s) => store_set(SESSION_KEY, &serde_json::to_string(s).unwrap_or_default()),
        None => store_remove(SESSION_KEY),
    }
    *SESSION.write() = session;
}

/// `(search, hash)` of the page URL at startup. The router rewrites the URL to its
/// canonical form on load, which drops the `?code=` / `#access_token=` of auth redirects,
/// so `main` captures them before launching.
static LANDING: std::sync::OnceLock<(String, String)> = std::sync::OnceLock::new();

pub fn capture_landing_url() {
    if let Some(location) = web_sys::window().map(|w| w.location()) {
        let _ = LANDING.set((
            location.search().unwrap_or_default(),
            location.hash().unwrap_or_default(),
        ));
    }
}

pub fn landing_url() -> (String, String) {
    LANDING.get().cloned().unwrap_or_default()
}

pub fn origin() -> String {
    web_sys::window()
        .and_then(|w| w.location().origin().ok())
        .unwrap_or_default()
}

fn callback_url() -> String {
    format!("{}/auth/callback", origin())
}

/// Where to go after a redirect-based sign-in finishes.
pub fn remember_next(path: &str) {
    store_set(NEXT_KEY, path);
}

pub fn take_next() -> String {
    let next = store_get(NEXT_KEY).unwrap_or_else(|| "/".into());
    store_remove(NEXT_KEY);
    // Only allow local paths.
    if next.starts_with('/') && !next.starts_with("//") {
        next
    } else {
        "/".into()
    }
}

// ---- requests ------------------------------------------------------------------

fn auth_url(path: &str) -> String {
    format!("{}/auth/v1{}", base_url(), path)
}

async fn post_token(grant: &str, body: Value) -> Result<Session> {
    let resp = http()
        .post(auth_url(&format!("/token?grant_type={grant}")))
        .header("apikey", api_key())
        .json(&body)
        .send()
        .await?;
    let token: TokenResponse = check(resp).await?.json().await?;
    Ok(token.into())
}

/// Returns a valid access token, refreshing the session when it is about to expire.
pub async fn access_token() -> Result<String> {
    let Some(session) = SESSION.peek().clone() else {
        return Err(ApiError {
            status: 401,
            code: Some("no_session".into()),
            message: "Not signed in".into(),
        });
    };
    if session.expires_at - now_secs() > 60 {
        return Ok(session.access_token);
    }
    match post_token(
        "refresh_token",
        json!({ "refresh_token": session.refresh_token }),
    )
    .await
    {
        Ok(fresh) => {
            let token = fresh.access_token.clone();
            set_session(Some(fresh));
            Ok(token)
        }
        Err(e) if e.status == 400 || e.status == 401 || e.status == 403 => {
            set_session(None);
            Err(e)
        }
        Err(e) => Err(e),
    }
}

async fn finish(session: Session) -> Result<Session> {
    set_session(Some(session.clone()));
    merge_guest_progress().await;
    Ok(session)
}

pub async fn sign_in_password(email: &str, password: &str) -> Result<Session> {
    prepare_guest_merge().await;
    let session = post_token("password", json!({ "email": email, "password": password })).await?;
    finish(session).await
}

pub enum SignUpOutcome {
    SignedIn,
    ConfirmationSent,
}

pub async fn sign_up(email: &str, password: &str) -> Result<SignUpOutcome> {
    prepare_guest_merge().await;
    let resp = http()
        .post(auth_url(&format!(
            "/signup?redirect_to={}",
            encode(&callback_url())
        )))
        .header("apikey", api_key())
        .json(&json!({ "email": email, "password": password }))
        .send()
        .await?;
    let body: Value = check(resp).await?.json().await?;
    if body.get("access_token").is_some() {
        let session: TokenResponse = serde_json::from_value(body)?;
        finish(session.into()).await?;
        Ok(SignUpOutcome::SignedIn)
    } else {
        Ok(SignUpOutcome::ConfirmationSent)
    }
}

/// Creates an anonymous guest account (requires "Allow anonymous sign-ins").
pub async fn sign_in_anonymously() -> Result<Session> {
    let resp = http()
        .post(auth_url("/signup"))
        .header("apikey", api_key())
        .json(&json!({ "data": {} }))
        .send()
        .await?;
    let token: TokenResponse = check(resp).await?.json().await?;
    let session: Session = token.into();
    set_session(Some(session.clone()));
    Ok(session)
}

fn new_pkce() -> (String, String) {
    let mut bytes = [0u8; 48];
    if let Some(crypto) = web_sys::window().and_then(|w| w.crypto().ok()) {
        let _ = crypto.get_random_values_with_u8_array(&mut bytes);
    }
    let verifier = URL_SAFE_NO_PAD.encode(bytes);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    store_set(VERIFIER_KEY, &verifier);
    (verifier, challenge)
}

fn navigate_external(url: &str) {
    if let Some(w) = web_sys::window() {
        let _ = w.location().set_href(url);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Provider {
    Google,
    Github,
}

impl Provider {
    pub fn id(self) -> &'static str {
        match self {
            Provider::Google => "google",
            Provider::Github => "github",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Provider::Google => "Google",
            Provider::Github => "GitHub",
        }
    }
}

/// Starts the OAuth sign-in redirect.
pub async fn sign_in_with(provider: Provider) {
    prepare_guest_merge().await;
    let (_, challenge) = new_pkce();
    let url = auth_url(&format!(
        "/authorize?provider={}&redirect_to={}&code_challenge={}&code_challenge_method=s256",
        provider.id(),
        encode(&callback_url()),
        challenge
    ));
    navigate_external(&url);
}

/// Links another provider to the signed-in account (also converts guests to real accounts).
pub async fn link_identity(provider: Provider) -> Result<()> {
    let token = access_token().await?;
    let (_, challenge) = new_pkce();
    let resp = http()
        .get(auth_url(&format!(
            "/user/identities/authorize?provider={}&redirect_to={}&code_challenge={}&code_challenge_method=s256&skip_http_redirect=true",
            provider.id(),
            encode(&callback_url()),
            challenge
        )))
        .header("apikey", api_key())
        .bearer_auth(token)
        .send()
        .await?;
    let body: Value = check(resp).await?.json().await?;
    let url = body
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::msg("Supabase did not return an authorization URL"))?;
    navigate_external(url);
    Ok(())
}

pub async fn unlink_identity(identity_id: &str) -> Result<()> {
    let token = access_token().await?;
    let resp = http()
        .delete(auth_url(&format!("/user/identities/{identity_id}")))
        .header("apikey", api_key())
        .bearer_auth(token)
        .send()
        .await?;
    check(resp).await?;
    refresh_user().await.map(|_| ())
}

/// Completes a PKCE redirect (`/auth/callback?code=…`).
pub async fn exchange_code(code: &str) -> Result<Session> {
    let verifier = store_get(VERIFIER_KEY).ok_or_else(|| {
        ApiError::msg("This sign-in link was opened in a different browser. Please sign in again.")
    })?;
    store_remove(VERIFIER_KEY);
    let session = post_token(
        "pkce",
        json!({ "auth_code": code, "code_verifier": verifier }),
    )
    .await?;
    finish(session).await
}

/// Completes an implicit redirect (`/auth/callback#access_token=…`), used by email links.
pub async fn session_from_fragment(fragment: &str) -> Result<(Session, Option<String>)> {
    let params = web_sys::UrlSearchParams::new_with_str(fragment.trim_start_matches('#'))
        .map_err(|_| ApiError::msg("Invalid callback URL"))?;
    let access_token = params
        .get("access_token")
        .ok_or_else(|| ApiError::msg("Missing access token"))?;
    let refresh_token = params.get("refresh_token").unwrap_or_default();
    let expires_at = params
        .get("expires_at")
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| now_secs() + 3600);
    let link_type = params.get("type");
    let user = get_user(&access_token).await?;
    let session = Session {
        access_token,
        refresh_token,
        expires_at,
        user,
    };
    Ok((finish(session).await?, link_type))
}

async fn get_user(token: &str) -> Result<User> {
    let resp = http()
        .get(auth_url("/user"))
        .header("apikey", api_key())
        .bearer_auth(token)
        .send()
        .await?;
    Ok(check(resp).await?.json().await?)
}

/// Reloads the user (identities, email) into the session.
pub async fn refresh_user() -> Result<User> {
    let token = access_token().await?;
    let user = get_user(&token).await?;
    replace_session_user(&user);
    Ok(user)
}

async fn update_user(body: Value, redirect: bool) -> Result<User> {
    let token = access_token().await?;
    let url = if redirect {
        auth_url(&format!("/user?redirect_to={}", encode(&callback_url())))
    } else {
        auth_url("/user")
    };
    let resp = http()
        .put(url)
        .header("apikey", api_key())
        .bearer_auth(token)
        .json(&body)
        .send()
        .await?;
    let user: User = check(resp).await?.json().await?;
    replace_session_user(&user);
    Ok(user)
}

fn replace_session_user(user: &User) {
    // Bind the clone first: in `if let … = SESSION.peek().clone()` the read guard would
    // live through the body and `set_session`'s write would panic with AlreadyBorrowed.
    let current = SESSION.peek().clone();
    if let Some(mut s) = current {
        s.user = user.clone();
        set_session(Some(s));
    }
}

/// Sets or changes the password. Lets Google / GitHub users sign in with email too.
pub async fn update_password(password: &str) -> Result<User> {
    update_user(json!({ "password": password }), false).await
}

/// Adds / changes the email address; a confirmation link is sent.
pub async fn update_email(email: &str) -> Result<User> {
    update_user(json!({ "email": email }), true).await
}

pub async fn update_display_name(name: &str) -> Result<User> {
    update_user(json!({ "data": { "full_name": name } }), false).await
}

pub async fn send_password_reset(email: &str) -> Result<()> {
    let resp = http()
        .post(auth_url(&format!(
            "/recover?redirect_to={}",
            encode(&callback_url())
        )))
        .header("apikey", api_key())
        .json(&json!({ "email": email }))
        .send()
        .await?;
    check(resp).await.map(|_| ())
}

pub async fn sign_out() {
    if let Ok(token) = access_token().await {
        let _ = http()
            .post(auth_url("/logout?scope=local"))
            .header("apikey", api_key())
            .bearer_auth(token)
            .send()
            .await;
    }
    store_remove(MERGE_KEY);
    set_session(None);
}

// ---- guest progress merge ---------------------------------------------------------

/// Called before a guest signs in to another account: lets the new account
/// adopt the guest's progress afterwards.
async fn prepare_guest_merge() {
    let is_guest = SESSION.peek().as_ref().is_some_and(|s| s.user.is_anonymous);
    if !is_guest {
        return;
    }
    #[derive(Deserialize)]
    struct Row {
        token: String,
    }
    let created: Result<Vec<Row>> =
        super::db::insert_returning("guest_merge_tokens", &json!({}), "token").await;
    if let Ok(Some(row)) = created.map(|mut r| r.pop()) {
        store_set(MERGE_KEY, &row.token);
    }
}

async fn merge_guest_progress() {
    let Some(token) = store_get(MERGE_KEY) else {
        return;
    };
    if SESSION.peek().as_ref().is_some_and(|s| s.user.is_anonymous) {
        return;
    }
    store_remove(MERGE_KEY);
    let _ = super::db::rpc::<Value>("merge_guest_progress", &json!({ "p_token": token })).await;
}
