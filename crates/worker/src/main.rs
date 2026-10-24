//! `explainer` — processes the `ai_jobs` queue: for each question it asks Claude Code
//! (running on your Claude subscription) for an explanation, and turns image-only
//! hotspot / drag & drop questions into interactive ones.
//!
//! Run it anywhere Claude Code is logged in:
//!
//! ```sh
//! cargo run --release -p explainer            # keep watching the queue
//! cargo run --release -p explainer -- --once  # drain the queue and exit (CI / cron)
//! ```

mod claude;
mod prompt;
mod supabase;

use anyhow::{Context, Result};
use clap::Parser;
use claude::ClaudeConfig;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use supabase::{Job, Supabase};
use tokio::sync::Semaphore;
use tracing::{error, info, warn};

/// How long the watching worker waits after hitting the subscription's usage limit.
const LIMIT_PAUSE: Duration = Duration::from_secs(15 * 60);

#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// Supabase project URL.
    #[arg(long, env = "SUPABASE_URL", required_unless_present = "sample")]
    supabase_url: Option<String>,
    /// Secret key (`sb_secret_…`) or legacy service_role key. Keep it out of the web app.
    #[arg(
        long,
        env = "SUPABASE_SECRET_KEY",
        hide_env_values = true,
        required_unless_present = "sample"
    )]
    secret_key: Option<String>,
    /// Claude Code executable.
    #[arg(long, env = "CLAUDE_BIN", default_value = "claude")]
    claude_bin: String,
    /// Model alias or id passed to Claude Code. Sonnet stretches subscription limits further
    /// than Opus; pass an empty value to use Claude Code's own default.
    #[arg(long, env = "EXPLAINER_MODEL", default_value = "sonnet")]
    model: String,
    /// Effort level passed to Claude Code (low, medium, high, xhigh, max).
    #[arg(long, env = "EXPLAINER_EFFORT")]
    effort: Option<String>,
    /// Let Claude search the web / fetch documentation while explaining (slower, more accurate).
    #[arg(long, env = "EXPLAINER_WEB")]
    web: bool,
    /// Questions processed in parallel.
    #[arg(long, env = "EXPLAINER_CONCURRENCY", default_value_t = 2)]
    concurrency: usize,
    /// Seconds between queue polls when idle.
    #[arg(long, default_value_t = 15)]
    poll_seconds: u64,
    /// Per-question timeout in seconds.
    #[arg(long, default_value_t = 600)]
    timeout_seconds: u64,
    /// Drain the queue once and exit.
    #[arg(long)]
    once: bool,
    /// Don't strip ANTHROPIC_API_KEY from Claude Code's environment.
    #[arg(long)]
    allow_api_key: bool,
    /// Explain a question from a JSON file (examcore::Question) and print the result,
    /// without touching Supabase. Handy for tuning the prompt.
    #[arg(long, value_name = "QUESTION_JSON")]
    sample: Option<std::path::PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let args = Args::parse();
    let cfg = Arc::new(ClaudeConfig {
        bin: args.claude_bin.clone(),
        model: Some(args.model.clone()).filter(|m| !m.is_empty()),
        effort: args.effort.clone(),
        web: args.web,
        allow_api_key: args.allow_api_key,
        timeout: Duration::from_secs(args.timeout_seconds),
    });
    if let Some(path) = &args.sample {
        return explain_sample(&cfg, path).await;
    }
    let db = Supabase::new(
        args.supabase_url.as_deref().unwrap_or_default(),
        args.secret_key.as_deref().unwrap_or_default(),
    );
    let slots = Arc::new(Semaphore::new(args.concurrency.max(1)));
    info!(
        concurrency = args.concurrency,
        once = args.once,
        "explainer started"
    );

    let paused = Arc::new(AtomicBool::new(false));
    loop {
        if paused.load(Ordering::SeqCst) {
            // Usage limit reached: let in-flight jobs finish, then stop (CI) or wait.
            let busy = args.concurrency.max(1) - slots.available_permits();
            if busy > 0 {
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
            if args.once {
                warn!(
                    "Claude usage limit reached; stopping. Queued jobs stay pending for the next run."
                );
                return Ok(());
            }
            warn!(
                "Claude usage limit reached; waiting {} minutes",
                LIMIT_PAUSE.as_secs() / 60
            );
            tokio::select! {
                _ = tokio::time::sleep(LIMIT_PAUSE) => {}
                _ = tokio::signal::ctrl_c() => { info!("stopping"); return Ok(()); }
            }
            paused.store(false, Ordering::SeqCst);
            continue;
        }
        let free = slots.available_permits();
        let jobs = if free == 0 {
            Vec::new()
        } else {
            db.claim_jobs(free).await.unwrap_or_else(|e| {
                error!("claiming jobs failed: {e:#}");
                Vec::new()
            })
        };

        if jobs.is_empty() {
            let busy = args.concurrency.max(1) - slots.available_permits();
            if args.once && busy == 0 {
                info!("queue is empty, exiting");
                return Ok(());
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(if busy > 0 { 2 } else { args.poll_seconds })) => {}
                _ = tokio::signal::ctrl_c() => { info!("stopping"); return Ok(()); }
            }
            continue;
        }

        for job in jobs {
            let permit = slots.clone().acquire_owned().await?;
            let db = db.clone();
            let cfg = cfg.clone();
            let paused = paused.clone();
            tokio::spawn(async move {
                let _permit = permit;
                let started = std::time::Instant::now();
                match process(&db, &cfg, &job).await {
                    Ok(model) => {
                        info!(job = job.id, question = %job.question_id, %model, secs = started.elapsed().as_secs(), "explained")
                    }
                    Err(e) if claude::is_usage_limit(&format!("{e:#}")) => {
                        // Not the question's fault: hand it back without using up an attempt.
                        paused.store(true, Ordering::SeqCst);
                        if let Err(e2) = db.release_job(job.id, &format!("{e:#}")).await {
                            error!("could not release job: {e2:#}");
                        }
                    }
                    Err(e) => {
                        warn!(job = job.id, attempt = job.attempts, "failed: {e:#}");
                        if let Err(e2) = db.fail_job(job.id, &format!("{e:#}")).await {
                            error!("could not record failure: {e2:#}");
                        }
                    }
                }
            });
        }
    }
}

async fn process(db: &Supabase, cfg: &ClaudeConfig, job: &Job) -> Result<String> {
    let record = db.question(&job.question_id).await?;
    let q = &record.data;
    let dir = tempfile::tempdir().context("creating a temp dir")?;

    // Pictures matter for exhibits and for image-only questions.
    let wants_images = q.has_exhibit
        || prompt::is_visual(q)
        || !matches!(q.kind, examcore::QuestionKind::Choice { .. });
    let mut question_image = None;
    let mut answer_image = None;
    if wants_images {
        for (path, slot, name) in [
            (&q.images.question, &mut question_image, "question"),
            (&q.images.answer, &mut answer_image, "answer"),
        ] {
            let Some(path) = path else { continue };
            let ext = path.rsplit('.').next().unwrap_or("webp");
            let file = format!("{name}.{ext}");
            match db.download(path).await {
                Ok(bytes) => {
                    tokio::fs::write(dir.path().join(&file), bytes).await?;
                    *slot = Some(file);
                }
                Err(e) => warn!("{e:#}"),
            }
        }
    }

    let cert = record
        .certification
        .as_ref()
        .map(|c| (c.code.as_str(), c.name.as_str()));
    let text = prompt::build(&prompt::PromptInput {
        question: q,
        source: &record.source,
        cert,
        question_image: question_image.as_deref(),
        answer_image: answer_image.as_deref(),
    });
    let visual = prompt::is_visual(q);
    let has_files = question_image.is_some() || answer_image.is_some();
    let out = claude::run(cfg, &text, &prompt::schema(visual), dir.path(), has_files).await?;
    let (explanation, kind) = prompt::parse(&out.structured)?;
    let content = serde_json::to_value(&explanation)?;
    let kind = kind.map(serde_json::to_value).transpose()?;
    db.complete_job(job.id, &content, &out.model, kind.as_ref())
        .await?;
    Ok(out.model)
}

async fn explain_sample(cfg: &ClaudeConfig, path: &std::path::Path) -> Result<()> {
    let q: examcore::Question = serde_json::from_str(&std::fs::read_to_string(path)?)
        .with_context(|| format!("{} is not a question JSON", path.display()))?;
    let dir = tempfile::tempdir()?;
    let text = prompt::build(&prompt::PromptInput {
        question: &q,
        source: &q,
        cert: None,
        question_image: None,
        answer_image: None,
    });
    let out = claude::run(
        cfg,
        &text,
        &prompt::schema(prompt::is_visual(&q)),
        dir.path(),
        false,
    )
    .await?;
    let (explanation, kind) = prompt::parse(&out.structured)?;
    println!("model: {}", out.model);
    println!("{}", serde_json::to_string_pretty(&explanation)?);
    if let Some(kind) = kind {
        println!("interactive: {}", serde_json::to_string_pretty(&kind)?);
    }
    Ok(())
}
