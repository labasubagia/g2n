//! Constants and path helpers shared across the crate.

use std::path::PathBuf;

use directories::ProjectDirs;

pub const MAX_HTTP_ATTEMPTS: usize = 10;
pub const HTTP_TIMEOUT_SECONDS: u64 = 120;
pub const MIN_RETRY_DELAY_SECONDS: u64 = 2;
pub const MAX_RETRY_DELAY_SECONDS: u64 = 60;
pub const OUTPUT_PATH: &str = "output";
pub const CONFIG_FILE_NAME: &str = "config.toml";
pub const NOTION_BASE_URL: &str = "https://api.notion.com";
pub const DRIVE_BASE_URL: &str = "https://www.googleapis.com";
pub const OAUTH_AUTHORIZE_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub const OAUTH_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const CONFIG_DIR_ENV: &str = "G2N_CONFIG_DIR";
const DATA_DIR_ENV: &str = "G2N_DATA_DIR";

fn project_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from("", "", "g2n")
}

pub fn config_dir() -> PathBuf {
    if let Ok(dir) = std::env::var(CONFIG_DIR_ENV) {
        return PathBuf::from(dir);
    }
    project_dirs()
        .map(|dirs| dirs.config_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(OUTPUT_PATH))
}

pub fn data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var(DATA_DIR_ENV) {
        return PathBuf::from(dir);
    }
    project_dirs()
        .map(|dirs| dirs.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(OUTPUT_PATH))
}

pub fn config_path() -> PathBuf {
    config_dir().join(CONFIG_FILE_NAME)
}

pub fn ensure_dirs() -> std::io::Result<()> {
    std::fs::create_dir_all(config_dir())?;
    std::fs::create_dir_all(data_dir())?;
    Ok(())
}
