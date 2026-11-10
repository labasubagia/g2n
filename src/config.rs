//! Loading and validating the TOML configuration.

use std::path::{Path, PathBuf};

use crate::domain::models::AppConfig;
use crate::shared::constants::config_path;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("missing configuration: {0} not found")]
    MissingFile(String),
    #[error("failed to read configuration: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse configuration: {0}")]
    Parse(#[from] toml::de::Error),
}

/// Load configuration from the default path or an explicit path.
pub fn load_config(explicit: Option<&Path>) -> Result<AppConfig, ConfigError> {
    let path = match explicit {
        Some(path) => path.to_path_buf(),
        None => config_path(),
    };
    let content = std::fs::read_to_string(&path)
        .map_err(|_| ConfigError::MissingFile(path.display().to_string()))?;
    let config: AppConfig = toml::from_str(&content)?;
    Ok(config)
}

pub fn load_config_from(path: &Path) -> Result<AppConfig, ConfigError> {
    let content = std::fs::read_to_string(path)
        .map_err(|_| ConfigError::MissingFile(path.display().to_string()))?;
    Ok(toml::from_str(&content)?)
}

pub fn config_path_for(dir: &Path) -> PathBuf {
    dir.join("config.toml")
}
