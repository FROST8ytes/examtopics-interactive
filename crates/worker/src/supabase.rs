//! Service-role access to Supabase (bypasses RLS). Never ship this key to browsers.

use anyhow::{Context, Result, bail};
use reqwest::{Client, RequestBuilder};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

#[derive(Clone)]
pub struct Supabase {
    http: Client,
    url: String,
    key: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Job {
    pub id: i64,
    pub question_id: String,
    pub attempts: i32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CertInfo {
    pub code: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct QuestionRecord {
    pub data: examcore::Question,
    pub source: examcore::Question,
    pub certification: Option<CertInfo>,
}

impl Supabase {
    pub fn new(url: &str, key: &str) -> Self {
        Supabase {
            http: Client::new(),
            url: url.trim_end_matches('/').to_string(),
            key: key.to_string(),
        }
    }

    fn auth(&self, req: RequestBuilder) -> RequestBuilder {
        let req = req.header("apikey", &self.key);
        // Legacy service_role keys are JWTs and also go in the Authorization header;
        // new `sb_secret_…` keys must only be sent as `apikey`.
        if self.key.starts_with("sb_") {
            req
        } else {
            req.bearer_auth(&self.key)
        }
    }

    async fn send<T: DeserializeOwned>(&self, req: RequestBuilder) -> Result<T> {
        let resp = self.auth(req).send().await?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            bail!("Supabase returned {status}: {text}");
        }
        serde_json::from_str(if text.trim().is_empty() {
            "null"
        } else {
            &text
        })
        .with_context(|| format!("unexpected response: {text}"))
    }

    pub async fn rpc<T: DeserializeOwned>(&self, function: &str, args: Value) -> Result<T> {
        self.send(
            self.http
                .post(format!("{}/rest/v1/rpc/{function}", self.url))
                .json(&args),
        )
        .await
    }

    pub async fn claim_jobs(&self, limit: usize) -> Result<Vec<Job>> {
        self.rpc("claim_ai_jobs", json!({ "p_limit": limit })).await
    }

    pub async fn complete_job(
        &self,
        job: i64,
        content: &Value,
        model: &str,
        kind: Option<&Value>,
    ) -> Result<()> {
        self.rpc::<Value>(
            "complete_ai_job",
            json!({ "p_job_id": job, "p_content": content, "p_model": model, "p_kind": kind }),
        )
        .await
        .map(|_| ())
    }

    /// Returns a job to the queue without counting the attempt.
    pub async fn release_job(&self, job: i64, reason: &str) -> Result<()> {
        self.rpc::<Value>(
            "release_ai_job",
            json!({ "p_job_id": job, "p_reason": reason }),
        )
        .await
        .map(|_| ())
    }

    pub async fn fail_job(&self, job: i64, error: &str) -> Result<()> {
        self.rpc::<Value>("fail_ai_job", json!({ "p_job_id": job, "p_error": error }))
            .await
            .map(|_| ())
    }

    pub async fn question(&self, id: &str) -> Result<QuestionRecord> {
        let rows: Vec<QuestionRecord> = self
            .send(self.http.get(format!(
                "{}/rest/v1/questions?select=data,source,certification:certifications(code,name)&id=eq.{id}",
                self.url
            )))
            .await?;
        rows.into_iter().next().context("question not found")
    }

    pub async fn download(&self, path: &str) -> Result<Vec<u8>> {
        let resp = self
            .auth(self.http.get(format!(
                "{}/storage/v1/object/question-images/{path}",
                self.url
            )))
            .send()
            .await?;
        if !resp.status().is_success() {
            bail!("downloading {path} failed: {}", resp.status());
        }
        Ok(resp.bytes().await?.to_vec())
    }
}
