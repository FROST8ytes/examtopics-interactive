//! Runs Claude Code in headless mode (`claude -p`) with structured output.
//!
//! Claude Code authenticates with the Claude subscription it is logged in with
//! (`claude` → `/login`), or with `CLAUDE_CODE_OAUTH_TOKEN` from `claude setup-token`
//! on servers / CI. `ANTHROPIC_API_KEY` is removed from the child environment so an API key
//! that happens to be set is not used instead of the subscription.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::Value;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

#[derive(Debug, Clone)]
pub struct ClaudeConfig {
    pub bin: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub web: bool,
    pub allow_api_key: bool,
    pub timeout: Duration,
}

pub struct ClaudeOutput {
    pub structured: Value,
    pub model: String,
}

#[derive(Deserialize)]
struct ResultMessage {
    #[serde(default)]
    is_error: bool,
    #[serde(default)]
    subtype: String,
    #[serde(default)]
    result: Option<String>,
    #[serde(default)]
    structured_output: Option<Value>,
    #[serde(default, rename = "modelUsage")]
    model_usage: Option<serde_json::Map<String, Value>>,
}

pub async fn run(
    cfg: &ClaudeConfig,
    prompt: &str,
    schema: &Value,
    workdir: &Path,
    can_read_files: bool,
) -> Result<ClaudeOutput> {
    let mut tools: Vec<&str> = Vec::new();
    if can_read_files {
        tools.push("Read");
    }
    if cfg.web {
        tools.extend(["WebSearch", "WebFetch"]);
    }
    let tools = tools.join(",");

    let mut cmd = Command::new(&cfg.bin);
    cmd.arg("-p")
        .args(["--output-format", "json"])
        .args(["--json-schema", &schema.to_string()])
        .args(["--tools", &tools])
        .args(["--permission-mode", "dontAsk"])
        .current_dir(workdir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if !tools.is_empty() {
        cmd.args(["--allowedTools", &tools]);
    }
    if let Some(model) = &cfg.model {
        cmd.args(["--model", model]);
    }
    if let Some(effort) = &cfg.effort {
        cmd.args(["--effort", effort]);
    }
    if !cfg.allow_api_key {
        cmd.env_remove("ANTHROPIC_API_KEY");
    }

    let mut child = cmd.spawn().with_context(|| {
        format!(
            "could not start `{}` — is Claude Code installed and on PATH?",
            cfg.bin
        )
    })?;
    // The prompt goes through stdin: question texts can exceed command-line limits.
    let mut stdin = child.stdin.take().context("no stdin")?;
    stdin.write_all(prompt.as_bytes()).await?;
    drop(stdin);

    let output = tokio::time::timeout(cfg.timeout, child.wait_with_output())
        .await
        .context("Claude Code timed out")??;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.trim().is_empty() {
        bail!(
            "Claude Code exited with {} and no output: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let msg: ResultMessage = serde_json::from_str(stdout.trim())
        .with_context(|| format!("unexpected Claude Code output: {}", truncate(&stdout, 500)))?;
    if msg.is_error || msg.subtype != "success" {
        bail!(
            "Claude Code failed ({}): {}",
            msg.subtype,
            truncate(msg.result.as_deref().unwrap_or_default(), 500)
        );
    }
    let structured = match msg.structured_output {
        Some(v) => v,
        None => serde_json::from_str(msg.result.as_deref().unwrap_or_default())
            .context("Claude Code returned no structured output")?,
    };
    let model = msg
        .model_usage
        .and_then(|m| m.keys().max_by_key(|k| k.len()).cloned())
        .unwrap_or_else(|| "Claude".into());
    Ok(ClaudeOutput { structured, model })
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() > n {
        format!("{}…", s.chars().take(n).collect::<String>())
    } else {
        s.to_string()
    }
}
