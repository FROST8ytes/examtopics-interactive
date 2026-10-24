//! Minimal Supabase client (Auth, PostgREST, Storage) for the browser.

pub mod auth;
pub mod db;
pub mod storage;

use serde::Deserialize;
use std::fmt;

/// Compile-time configuration. Set these env vars when running `dx serve` / `dx bundle`
/// (Vercel: Project Settings → Environment Variables).
pub const SUPABASE_URL: Option<&str> = option_env!("SUPABASE_URL");
/// The publishable key (`sb_publishable_…`) or the legacy anon key.
pub const SUPABASE_KEY: Option<&str> = option_env!("SUPABASE_PUBLISHABLE_KEY");

pub fn configured() -> bool {
    SUPABASE_URL.is_some_and(|u| !u.is_empty()) && SUPABASE_KEY.is_some_and(|k| !k.is_empty())
}

pub fn base_url() -> &'static str {
    SUPABASE_URL.unwrap_or_default().trim_end_matches('/')
}

pub fn api_key() -> &'static str {
    SUPABASE_KEY.unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApiError {
    pub status: u16,
    pub code: Option<String>,
    pub message: String,
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ApiError {}

impl ApiError {
    pub fn msg(message: impl Into<String>) -> Self {
        ApiError {
            status: 0,
            code: None,
            message: message.into(),
        }
    }
}

impl From<reqwest::Error> for ApiError {
    fn from(e: reqwest::Error) -> Self {
        ApiError::msg(format!("Network error: {e}"))
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(e: serde_json::Error) -> Self {
        ApiError::msg(format!("Unexpected response: {e}"))
    }
}

pub type Result<T> = std::result::Result<T, ApiError>;

/// Supabase services return slightly different error shapes (Auth sends both `code`
/// and `error_code`), so every variant is its own field rather than an alias.
#[derive(Deserialize, Default)]
struct ErrorBody {
    error_code: Option<String>,
    code: Option<serde_json::Value>,
    message: Option<String>,
    msg: Option<String>,
    error_description: Option<String>,
    error: Option<String>,
}

pub(crate) async fn check(resp: reqwest::Response) -> Result<reqwest::Response> {
    if resp.status().is_success() {
        return Ok(resp);
    }
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    let body: ErrorBody = serde_json::from_str(&text).unwrap_or_default();
    let code = body.error_code.or(body.code.map(|c| match c {
        serde_json::Value::String(s) => s,
        other => other.to_string(),
    }));
    let message = body
        .msg
        .or(body.message)
        .or(body.error_description)
        .or(body.error)
        .unwrap_or_else(|| {
            if text.is_empty() {
                format!("Request failed ({status})")
            } else {
                text
            }
        });
    Err(ApiError {
        status,
        code,
        message,
    })
}

pub(crate) fn http() -> reqwest::Client {
    thread_local! {
        static CLIENT: reqwest::Client = reqwest::Client::new();
    }
    CLIENT.with(Clone::clone)
}

pub fn encode(s: &str) -> String {
    String::from(js_sys::encode_uri_component(s))
}
