//! Sync pipeline: list Drive images, dedupe against Notion, upload each.

use std::io::IsTerminal;
use std::path::PathBuf;

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

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
}

pub fn load_app_config(path: &Option<PathBuf>) -> Result<AppConfig, SyncError> {
    Ok(match path {
        Some(p) => crate::config::load_config_from(p)?,
        None => crate::config::load_config(None)?,
    })
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
    let bar = mp.as_ref().map(|mp| {
        let bar = mp.add(ProgressBar::new(total as u64));
        bar.set_style(
            ProgressStyle::with_template("{bar:34} {pos}/{len} {msg}")
                .unwrap()
                .progress_chars("=> "),
        );
        bar.set_message(format!("found {total} images in Drive"));
        bar
    });

    // Report a progress line above the bar (TTY) or via tracing (pipes/tests).
    let notify = |msg: String| {
        if let Some(mp) = &mp {
            let _ = mp.println(msg);
        } else {
            tracing::info!("{msg}");
        }
    };

    let mut uploaded = 0usize;
    let mut skipped = 0usize;
    for image in &images {
        notify(format!("processing: {}", image.name));
        if notion.page_exists(&image.name).await? {
            notify(format!("exists in Notion; skipping: {}", image.name));
            skipped += 1;
        } else {
            notify(format!("downloading from Drive: {}", image.name));
            tokio::fs::create_dir_all(&options.download_dir).await?;
            let file_path = options.download_dir.join(&image.name);
            let bytes = drive.download(&image.id, &image.name).await?;
            notify(format!(
                "downloaded: {} ({} bytes)",
                image.name,
                bytes.len()
            ));
            tokio::fs::write(&file_path, bytes).await?;
            notify(format!("uploading to Notion: {}", image.name));
            notion
                .add_page(&file_path, &image.prompt, image.created_time.as_deref())
                .await?;
            uploaded += 1;
            notify(format!("uploaded: {}", image.name));
        }
        if let Some(bar) = &bar {
            bar.inc(1);
        }
    }

    if let Some(bar) = &bar {
        bar.finish_with_message(format!(
            "sync finished: {uploaded} uploaded, {skipped} skipped"
        ));
    } else {
        tracing::info!(uploaded, skipped, "sync finished");
    }
    Ok(())
}
