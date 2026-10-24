//! PostgREST helpers. Queries use PostgREST syntax, e.g. `select=*&set_id=eq.<id>`.

use super::{Result, api_key, auth::access_token, base_url, check, http};
use serde::Serialize;
use serde::de::DeserializeOwned;

fn rest_url(path: &str) -> String {
    format!("{}/rest/v1/{}", base_url(), path)
}

async fn request(method: reqwest::Method, path: &str) -> Result<reqwest::RequestBuilder> {
    let token = access_token().await?;
    Ok(http()
        .request(method, rest_url(path))
        .header("apikey", api_key())
        .bearer_auth(token))
}

pub async fn select<T: DeserializeOwned>(table: &str, query: &str) -> Result<Vec<T>> {
    let resp = request(reqwest::Method::GET, &format!("{table}?{query}"))
        .await?
        .send()
        .await?;
    Ok(check(resp).await?.json().await?)
}

/// Fetches exactly one row (`None` when there is no match).
pub async fn select_one<T: DeserializeOwned>(table: &str, query: &str) -> Result<Option<T>> {
    Ok(select::<T>(table, &format!("{query}&limit=1"))
        .await?
        .into_iter()
        .next())
}

pub async fn insert<B: Serialize + ?Sized>(table: &str, body: &B) -> Result<()> {
    let resp = request(reqwest::Method::POST, table)
        .await?
        .header("Prefer", "return=minimal")
        .json(body)
        .send()
        .await?;
    check(resp).await.map(|_| ())
}

pub async fn insert_returning<T: DeserializeOwned, B: Serialize + ?Sized>(
    table: &str,
    body: &B,
    columns: &str,
) -> Result<Vec<T>> {
    let resp = request(reqwest::Method::POST, &format!("{table}?select={columns}"))
        .await?
        .header("Prefer", "return=representation")
        .json(body)
        .send()
        .await?;
    Ok(check(resp).await?.json().await?)
}

pub async fn update<B: Serialize + ?Sized>(table: &str, filter: &str, body: &B) -> Result<()> {
    let resp = request(reqwest::Method::PATCH, &format!("{table}?{filter}"))
        .await?
        .header("Prefer", "return=minimal")
        .json(body)
        .send()
        .await?;
    check(resp).await.map(|_| ())
}

pub async fn delete(table: &str, filter: &str) -> Result<()> {
    let resp = request(reqwest::Method::DELETE, &format!("{table}?{filter}"))
        .await?
        .send()
        .await?;
    check(resp).await.map(|_| ())
}

/// Calls a Postgres function. Functions returning `void` deserialize to `()`/`Value::Null`.
pub async fn rpc<T: DeserializeOwned>(function: &str, args: &serde_json::Value) -> Result<T> {
    let resp = request(reqwest::Method::POST, &format!("rpc/{function}"))
        .await?
        .json(args)
        .send()
        .await?;
    let text = check(resp).await?.text().await?;
    Ok(serde_json::from_str(if text.trim().is_empty() {
        "null"
    } else {
        &text
    })?)
}

/// `in.(a,b,c)` filter value.
pub fn in_list<'a>(ids: impl IntoIterator<Item = &'a str>) -> String {
    let joined: Vec<String> = ids.into_iter().map(|id| format!("\"{id}\"")).collect();
    format!("in.({})", joined.join(","))
}
