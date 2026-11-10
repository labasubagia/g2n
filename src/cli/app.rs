//! Command-line interface.

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::adapters::auth::{default_token_path, ensure_access_token};
use crate::adapters::drive::DriveClient;
use crate::adapters::notion::{NotionClient, NotionContext};
use crate::services::sync::{SyncOptions, load_app_config, run_sync};

const AUTH_TIMEOUT_SECS: u64 = 300;

#[derive(Debug, Parser)]
#[command(name = "g2n", about = "Upload Google Drive images to Notion")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Sync images from the Drive folder into Notion.
    Sync(SyncArgs),
}

#[derive(Debug, clap::Args)]
pub struct SyncArgs {
    /// Path to config.toml (default: platform config dir).
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Directory to write downloaded images (default: platform data dir/images).
    #[arg(long)]
    pub download_dir: Option<PathBuf>,
    /// Authorize via browser when no token is cached (default: true).
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub interactive: bool,
}

pub async fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Sync(args) => {
            let config = load_app_config(&args.config)?;
            let token_path = default_token_path();
            let access_token =
                ensure_access_token(&token_path, &config, AUTH_TIMEOUT_SECS, args.interactive)
                    .await?;

            let drive = DriveClient::new(access_token)?;
            let notion = NotionClient::new(NotionContext {
                api_key: config.notion.api_key.clone(),
                database_id: config.notion.database_id.clone(),
            })?;

            let options = SyncOptions {
                download_dir: args
                    .download_dir
                    .unwrap_or_else(|| crate::shared::constants::data_dir().join("images")),
            };
            run_sync(&options, &config, &drive, &notion).await?;
            Ok(())
        }
    }
}
