//! Sync pipeline: list Drive images, dedupe against Notion, upload each.

use std::path::PathBuf;

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
    tracing::info!(count = images.len(), "found images in Drive");

    let mut uploaded = 0usize;
    let mut skipped = 0usize;
    for image in &images {
        tracing::info!(name = %image.name, "processing image");
        if notion.page_exists(&image.name).await? {
            tracing::info!(name = %image.name, "exists in Notion; skipping");
            skipped += 1;
            continue;
        }
        tracing::info!(name = %image.name, "downloading from Drive");
        tokio::fs::create_dir_all(&options.download_dir).await?;
        let file_path = options.download_dir.join(&image.name);
        let bytes = drive.download(&image.id, &image.name).await?;
        tracing::info!(name = %image.name, bytes = bytes.len(), "downloaded");
        tokio::fs::write(&file_path, bytes).await?;
        tracing::info!(name = %image.name, "uploading to Notion");
        notion
            .add_page(&file_path, &image.prompt, image.created_time.as_deref())
            .await?;
        uploaded += 1;
        tracing::info!(name = %image.name, "uploaded");
    }
    tracing::info!(uploaded, skipped, "sync finished");
    Ok(())
}
