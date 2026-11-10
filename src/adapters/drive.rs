//! Google Drive API adapter: list image files and download blobs.

use async_trait::async_trait;
use reqwest::{Client, Method, Response};
use serde_json::Value;

use crate::domain::models::DriveImage;
use crate::shared::constants::DRIVE_BASE_URL;
use crate::shared::http::{HttpError, auth_bearer, build_client, ensure_success, retry_http};

#[derive(Debug, thiserror::Error)]
pub enum DriveError {
    #[error(transparent)]
    Http(#[from] HttpError),
    #[error("request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("invalid response from Drive: {0}")]
    InvalidResponse(String),
}

#[async_trait]
pub trait DriveApi: Send + Sync {
    async fn list_images(&self, folder_id: &str) -> Result<Vec<DriveImage>, DriveError>;
    async fn download(&self, file_id: &str, file_name: &str) -> Result<Vec<u8>, DriveError>;
}

pub struct DriveClient {
    http: Client,
    access_token: String,
    base_url: String,
}

impl std::fmt::Debug for DriveClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DriveClient")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

impl DriveClient {
    pub fn new(access_token: String) -> Result<Self, DriveError> {
        Self::with_base_url(access_token, DRIVE_BASE_URL)
    }

    pub fn with_base_url(
        access_token: String,
        base_url: impl Into<String>,
    ) -> Result<Self, DriveError> {
        Ok(Self {
            http: build_client()?,
            access_token,
            base_url: base_url.into().trim_end_matches('/').to_owned(),
        })
    }

    async fn get(&self, url: &str) -> Result<Response, DriveError> {
        let headers = auth_bearer(&self.access_token);
        let response = retry_http(|| async {
            let response = self.http.get(url).headers(headers.clone()).send().await?;
            ensure_success(response, Method::GET).await
        })
        .await?;
        Ok(response)
    }
}

impl DriveClient {
    fn parse_file(item: &Value) -> Option<DriveImage> {
        let id = item.get("id")?.as_str()?.to_owned();
        let name = item.get("name")?.as_str()?.to_owned();
        let mime_type = item
            .get("mimeType")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let prompt = item
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let created_time = item
            .get("createdTime")
            .and_then(Value::as_str)
            .map(str::to_owned);
        Some(DriveImage {
            id,
            name,
            prompt,
            created_time,
            mime_type,
        })
    }
    /// Collect image files under a folder, descending into nested folders.
    async fn collect_images(
        &self,
        root_folder: &str,
        out: &mut Vec<DriveImage>,
    ) -> Result<(), DriveError> {
        let mut pending = vec![root_folder.to_owned()];
        while let Some(folder_id) = pending.pop() {
            let mut page_token: Option<String> = None;
            loop {
                let query = format!("'{folder_id}' in parents and trashed = false");
                let mut url = format!(
                    "{}/drive/v3/files?q={}&includeItemsFromAllDrives=true&supportsAllDrives=true&pageSize=1000&fields=nextPageToken,files(id,name,description,mimeType,createdTime)",
                    self.base_url,
                    urlencoding(&query)
                );
                if let Some(token) = &page_token {
                    url.push_str(&format!("&pageToken={}", urlencoding(token)));
                }
                let response = self.get(&url).await?;
                let data: Value = response
                    .json()
                    .await
                    .map_err(|e| DriveError::InvalidResponse(format!("body decode failed: {e}")))?;
                let files = data
                    .get("files")
                    .and_then(Value::as_array)
                    .ok_or_else(|| DriveError::InvalidResponse("missing files array".into()))?;
                for file in files {
                    let mime = file.get("mimeType").and_then(Value::as_str).unwrap_or("");
                    if mime == "application/vnd.google-apps.folder" {
                        if let Some(id) = file.get("id").and_then(Value::as_str) {
                            pending.push(id.to_owned());
                        }
                    } else if mime.contains("image/")
                        && let Some(image) = Self::parse_file(file)
                    {
                        out.push(image);
                    }
                }
                page_token = data
                    .get("nextPageToken")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                if page_token.is_none() {
                    break;
                }
            }
        }
        Ok(())
    }
}

#[async_trait]
impl DriveApi for DriveClient {
    async fn list_images(&self, folder_id: &str) -> Result<Vec<DriveImage>, DriveError> {
        let mut images = Vec::new();
        self.collect_images(folder_id, &mut images).await?;
        Ok(images)
    }

    async fn download(&self, file_id: &str, file_name: &str) -> Result<Vec<u8>, DriveError> {
        let url = format!(
            "{}/drive/v3/files/{}?alt=media&supportsAllDrives=true",
            self.base_url, file_id
        );
        let response = self.get(&url).await?;
        let bytes = response.bytes().await.map_err(|e| {
            DriveError::InvalidResponse(format!("download body failed for {file_name}: {e}"))
        })?;
        Ok(bytes.to_vec())
    }
}

fn urlencoding(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{:02X}", b),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urlencoding_quotes_spaces() {
        assert_eq!(
            urlencoding("'abc' in parents and x='a b'"),
            "%27abc%27%20in%20parents%20and%20x%3D%27a%20b%27"
        );
    }
}
