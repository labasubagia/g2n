//! Sync pipeline: list Drive images, dedupe against Notion, upload each.

use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use tokio::sync::Semaphore;

use crate::adapters::auth::AuthError;
use crate::adapters::drive::DriveError;
use crate::adapters::notion::NotionError;
use crate::config::ConfigError;
use crate::domain::models::AppConfig;

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error(transparent)]
    Drive(#[from] DriveError),
    #[error(transparent)]
    Notion(#[from] NotionError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("no images found in Drive folder {0}")]
    NoImages(String),
}

#[derive(Debug, Clone)]
pub struct SyncOptions {
    pub download_dir: PathBuf,
    pub concurrency: usize,
}

pub fn load_app_config(path: &Option<PathBuf>) -> Result<AppConfig, SyncError> {
    Ok(match path {
        Some(p) => crate::config::load_config_from(p)?,
        None => crate::config::load_config(None)?,
    })
}

/// A simple token-bucket rate limiter used to pace Notion uploads so the
/// connection stays under its per-second request budget.
struct RateLimiter {
    rate_per_sec: f64,
    state: tokio::sync::Mutex<LimiterState>,
}

struct LimiterState {
    tokens: f64,
    last_refill: Instant,
}

impl RateLimiter {
    fn new(rate_per_sec: f64) -> Self {
        Self {
            rate_per_sec: rate_per_sec.max(f64::EPSILON),
            state: tokio::sync::Mutex::new(LimiterState {
                tokens: rate_per_sec.max(f64::EPSILON),
                last_refill: Instant::now(),
            }),
        }
    }

    /// Reserve `n` tokens, sleeping until the budget allows.
    async fn acquire(&self, n: u32) {
        let n = n as f64;
        let wait = {
            let mut s = self.state.lock().await;
            let now = Instant::now();
            let elapsed = now.duration_since(s.last_refill).as_secs_f64();
            s.tokens = (s.tokens + elapsed * self.rate_per_sec).min(self.rate_per_sec * 2.0);
            s.last_refill = now;
            if s.tokens >= n {
                s.tokens -= n;
                None
            } else {
                let deficit = n - s.tokens;
                s.tokens = 0.0;
                Some(Duration::from_secs_f64(deficit / self.rate_per_sec))
            }
        };
        if let Some(wait) = wait {
            tokio::time::sleep(wait).await;
        }
    }
}

pub async fn run_sync(
    options: &SyncOptions,
    config: &AppConfig,
    drive: &dyn crate::adapters::drive::DriveApi,
    notion: &dyn crate::adapters::notion::NotionApi,
) -> Result<(), SyncError> {
    let images = drive.list_images(&config.google.folder_id).await?;
    if images.is_empty() {
        return Err(SyncError::NoImages(config.google.folder_id.clone()));
    }
    let total = images.len();

    let use_bar = std::io::stderr().is_terminal();
    let mp = use_bar.then(MultiProgress::new);
    // Print a settled line above the bar (TTY) or via tracing (pipes/tests).
    let report = |line: String| {
        if let Some(mp) = &mp {
            let _ = mp.println(line);
        } else {
            tracing::info!("{line}");
        }
    };

    report(format!("🔍 found {total} images in Drive"));

    // Spinner during the one-time bulk dedup fetch so it never looks hung.
    let fetch_bar = mp.as_ref().map(|mp| {
        let bar = mp.add(ProgressBar::new_spinner());
        bar.set_style(
            ProgressStyle::with_template("{spinner:.cyan} {msg}")
                .unwrap()
                .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"),
        );
        bar.set_message("🔎 Fetching existing Notion pages for dedup…".to_owned());
        bar
    });
    report("🔎 Fetching existing Notion pages for dedup…".to_owned());

    // One-time bulk dedup: fetch existing titles once instead of one query per
    // image (data-source queries are throttled to ~3 req/s).
    let existing = notion
        .list_existing_names(&|found| {
            if let Some(bar) = &fetch_bar {
                bar.set_message(format!(
                    "🔎 Fetching existing Notion pages… {found} collected"
                ));
            }
        })
        .await?;

    if let (Some(mp), Some(bar)) = (&mp, &fetch_bar) {
        mp.remove(bar);
    }

    let pending: Vec<_> = images
        .iter()
        .filter(|image| !existing.contains(&image.name))
        .collect();
    let pending_len = pending.len();
    let skipped = total - pending_len;

    let bar = mp.as_ref().map(|mp| {
        let bar = mp.add(ProgressBar::new(pending_len as u64));
        bar.set_style(
            ProgressStyle::with_template("{bar:34} {pos}/{len} {msg}")
                .unwrap()
                .progress_chars("=> "),
        );
        bar.set_message(format!(
            "🔍 {skipped}/{total} already in Notion, {pending_len} to upload"
        ));
        bar
    });

    report(format!(
        "🔍 {skipped}/{total} already in Notion, {pending_len} to upload"
    ));
    let limiter = Arc::new(RateLimiter::new(config.notion.requests_per_second));
    let semaphore = Arc::new(Semaphore::new(options.concurrency.max(1)));
    let downloaded_dir = options.download_dir.clone();

    let results: Vec<Result<(), SyncError>> = futures_util::stream::iter(pending)
        .map(|image| {
            let (limiter, semaphore) = (limiter.clone(), semaphore.clone());
            let (mp, bar) = (mp.clone(), bar.clone());
            let downloaded_dir = downloaded_dir.clone();
            async move {
                let _permit = semaphore
                    .acquire()
                    .await
                    .map_err(|_| SyncError::Io(std::io::Error::other("semaphore closed")))?;
                if let Some(bar) = &bar {
                    bar.set_message(format!("⏳ {}", image.name));
                }
                tokio::fs::create_dir_all(&downloaded_dir).await?;
                let file_path = downloaded_dir.join(&image.name);
                let bytes = drive.download(&image.id, &image.name).await?;
                let sized = format!("{} ({} bytes)", image.name, bytes.len());
                if let Some(bar) = &bar {
                    bar.set_message(format!("💾 {sized}"));
                }
                tokio::fs::write(&file_path, &bytes).await?;
                // Each add_page is 3 API requests (create upload, send, create page).
                limiter.acquire(3).await;
                notion
                    .add_page(&file_path, &image.prompt, image.created_time.as_deref())
                    .await?;
                if let Some(mp) = &mp {
                    let _ = mp.println(format!("✅ {sized}"));
                } else {
                    tracing::info!("✅ {sized}");
                }
                if let Some(bar) = &bar {
                    bar.inc(1);
                }
                Ok(())
            }
        })
        .buffer_unordered(options.concurrency.max(1))
        .collect()
        .await;

    let failures: Vec<_> = results.into_iter().filter_map(Result::err).collect();
    let uploaded = pending_len - failures.len();

    if let Some(bar) = &bar {
        bar.finish_with_message(format!(
            "sync finished: {uploaded} uploaded, {skipped} skipped"
        ));
    } else {
        tracing::info!(uploaded, skipped, "sync finished");
    }

    match failures.into_iter().next() {
        Some(err) => Err(err),
        None => Ok(()),
    }
}
