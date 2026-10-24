//! Supabase Storage: uploads page snapshots and signs URLs to display them.

use super::{Result, api_key, auth::access_token, base_url, check, http};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;

pub const BUCKET: &str = "question-images";

pub async fn upload(path: &str, bytes: Vec<u8>, content_type: &str) -> Result<()> {
    let token = access_token().await?;
    let resp = http()
        .post(format!("{}/storage/v1/object/{BUCKET}/{path}", base_url()))
        .header("apikey", api_key())
        .header("x-upsert", "true")
        .header("content-type", content_type)
        .header("cache-control", "max-age=31536000")
        .bearer_auth(token)
        .body(bytes)
        .send()
        .await?;
    check(resp).await.map(|_| ())
}

/// Removes objects (used when a certification is deleted).
pub async fn remove(paths: &[String]) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    let token = access_token().await?;
    let resp = http()
        .delete(format!("{}/storage/v1/object/{BUCKET}", base_url()))
        .header("apikey", api_key())
        .bearer_auth(token)
        .json(&json!({ "prefixes": paths }))
        .send()
        .await?;
    check(resp).await.map(|_| ())
}

/// Signs many paths at once. Returns path → absolute URL.
pub async fn signed_urls(paths: &[String], expires_in: u32) -> Result<HashMap<String, String>> {
    let mut out = HashMap::new();
    if paths.is_empty() {
        return Ok(out);
    }
    #[derive(Deserialize)]
    struct Signed {
        path: Option<String>,
        #[serde(rename = "signedURL", alias = "signedUrl")]
        signed_url: Option<String>,
    }
    let token = access_token().await?;
    for chunk in paths.chunks(500) {
        let resp = http()
            .post(format!("{}/storage/v1/object/sign/{BUCKET}", base_url()))
            .header("apikey", api_key())
            .bearer_auth(&token)
            .json(&json!({ "expiresIn": expires_in, "paths": chunk }))
            .send()
            .await?;
        let signed: Vec<Signed> = check(resp).await?.json().await?;
        for s in signed {
            if let (Some(path), Some(url)) = (s.path, s.signed_url) {
                out.insert(path, format!("{}/storage/v1{}", base_url(), url));
            }
        }
    }
    Ok(out)
}
