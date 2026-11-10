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
    /// Notion connection request budget per second (default 3, safe for
    /// non-Business plans; Business/Enterprise allow 10). Used to pace uploads.
    #[serde(default = "default_requests_per_second")]
    pub requests_per_second: f64,
}

fn default_requests_per_second() -> f64 {
    3.0
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
