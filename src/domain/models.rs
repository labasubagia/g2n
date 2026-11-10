//! Domain types shared across adapters and services.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoogleConfig {
    pub client_id: String,
    pub client_secret: String,
    pub folder_id: String,
    #[serde(default = "default_scope")]
    pub scope: String,
}

fn default_scope() -> String {
    "https://www.googleapis.com/auth/drive.readonly".to_owned()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotionConfig {
    pub api_key: String,
    pub database_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    pub google: GoogleConfig,
    pub notion: NotionConfig,
}

/// A Google Drive image file with its description as the prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveImage {
    pub id: String,
    pub name: String,
    pub prompt: String,
    pub created_time: Option<String>,
    pub mime_type: String,
}
