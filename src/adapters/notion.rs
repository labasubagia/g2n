//! Notion API adapter: database page creation and duplicate checks.

use std::collections::HashSet;
use std::path::Path;

use async_trait::async_trait;
use reqwest::header::{CONTENT_TYPE, HeaderMap};
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};

use crate::shared::constants::NOTION_BASE_URL;
use crate::shared::http::{HttpError, build_client, ensure_success, retry_http};

#[derive(Debug, thiserror::Error)]
pub enum NotionError {
    #[error("request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("invalid header value: {0}")]
    InvalidHeader(#[from] reqwest::header::InvalidHeaderValue),
    #[error(transparent)]
    Http(#[from] HttpError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("file not found: {0}")]
    FileNotFound(String),
    #[error("Notion file upload response is missing an id")]
    MissingUploadId,
    #[error("invalid response while reconciling Notion page: {0}")]
    InvalidResponse(String),
}

#[derive(Debug, Clone)]
pub struct NotionContext {
    pub api_key: String,
    pub database_id: String,
}

#[async_trait]
pub trait NotionApi: Send + Sync {
    /// Return the set of page titles already present in the database.
    async fn list_existing_names(
        &self,
        on_page: &(dyn Fn(usize) + Send + Sync),
    ) -> Result<HashSet<String>, NotionError>;
    async fn add_page(
        &self,
        file_path: &Path,
        prompt: &str,
        created_time: Option<&str>,
    ) -> Result<Value, NotionError>;
}

pub struct NotionClient {
    http: Client,
    headers: HeaderMap,
    database_id: String,
    base_url: String,
}

impl std::fmt::Debug for NotionClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NotionClient")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

impl NotionClient {
    pub fn new(context: NotionContext) -> Result<Self, NotionError> {
        Self::with_base_url(context, NOTION_BASE_URL)
    }

    pub fn with_base_url(
        context: NotionContext,
        base_url: impl Into<String>,
    ) -> Result<Self, NotionError> {
        let mut headers = HeaderMap::new();
        headers.insert(
            "Authorization",
            format!("Bearer {}", context.api_key).parse()?,
        );
        headers.insert("Notion-Version", "2022-06-28".parse()?);
        Ok(Self {
            http: build_client()?,
            headers,
            database_id: context.database_id,
            base_url: base_url.into().trim_end_matches('/').to_owned(),
        })
    }

    /// Fetch all page titles in the database (paginated, 100/page).
    async fn list_existing_names(
        &self,
        on_page: &(dyn Fn(usize) + Send + Sync),
    ) -> Result<HashSet<String>, NotionError> {
        let url = format!("{}/v1/databases/{}/query", self.base_url, self.database_id);
        let mut names = HashSet::new();
        let mut start_cursor: Option<String> = None;
        loop {
            let mut payload = json!({"page_size": 100});
            if let Some(cursor) = start_cursor.as_ref() {
                payload["start_cursor"] = Value::String(cursor.clone());
            }
            let data = retry_http(|| async {
                let response = self
                    .http
                    .post(&url)
                    .headers(self.headers.clone())
                    .json(&payload)
                    .send()
                    .await?;
                let response = ensure_success(response, Method::POST).await?;
                Ok(response.json::<Value>().await?)
            })
            .await?;
            let results = data
                .get("results")
                .and_then(Value::as_array)
                .ok_or_else(|| NotionError::InvalidResponse("missing results array".into()))?;
            for page in results {
                if let Some(name) = page
                    .pointer("/properties/Name/title/0/text/content")
                    .and_then(Value::as_str)
                {
                    names.insert(name.to_owned());
                }
            }
            on_page(names.len());
            match data.get("has_more").and_then(Value::as_bool) {
                Some(true) => {
                    start_cursor = data
                        .get("next_cursor")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                }
                _ => break,
            }
        }
        Ok(names)
    }
    async fn create_upload(
        &self,
        file_path: &Path,
        content_type: &str,
    ) -> Result<Value, NotionError> {
        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| NotionError::InvalidResponse(file_path.display().to_string()))?
            .to_owned();
        let url = format!("{}/v1/file_uploads", self.base_url);
        retry_http(|| async {
            let response = self
                .http
                .post(&url)
                .headers(self.headers.clone())
                .json(&json!({
                    "mode": "single_part",
                    "filename": file_name,
                    "content_type": content_type,
                }))
                .send()
                .await?;
            let response = ensure_success(response, Method::POST).await?;
            Ok(response.json::<Value>().await?)
        })
        .await
        .map_err(NotionError::from)
    }

    async fn send_upload(
        &self,
        upload_id: &str,
        file_path: &Path,
        content_type: &str,
    ) -> Result<Value, NotionError> {
        let url = format!("{}/v1/file_uploads/{}/send", self.base_url, upload_id);
        let mut headers = self.headers.clone();
        headers.remove(CONTENT_TYPE);
        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("image")
            .to_owned();
        retry_http(|| async {
            let part = reqwest::multipart::Part::file(file_path)
                .await?
                .file_name(file_name.clone())
                .mime_str(content_type)?;
            let form = reqwest::multipart::Form::new().part("file", part);
            let response = self
                .http
                .post(&url)
                .headers(headers.clone())
                .multipart(form)
                .send()
                .await?;
            if response.status() == StatusCode::BAD_REQUEST {
                return Ok(json!({"id": upload_id, "status": "complete"}));
            }
            let response = ensure_success(response, Method::POST).await?;
            Ok(response.json::<Value>().await?)
        })
        .await
        .map_err(NotionError::from)
    }
}

#[async_trait]
impl NotionApi for NotionClient {
    async fn list_existing_names(
        &self,
        on_page: &(dyn Fn(usize) + Send + Sync),
    ) -> Result<HashSet<String>, NotionError> {
        self.list_existing_names(on_page).await
    }

    async fn add_page(
        &self,
        file_path: &Path,
        prompt: &str,
        created_time: Option<&str>,
    ) -> Result<Value, NotionError> {
        if !file_path.exists() {
            return Err(NotionError::FileNotFound(file_path.display().to_string()));
        }
        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| NotionError::InvalidResponse(file_path.display().to_string()))?
            .to_owned();
        let content_type = content_type_for_path(file_path);
        let create = self.create_upload(file_path, &content_type).await?;
        let upload_id = create
            .get("id")
            .and_then(Value::as_str)
            .ok_or(NotionError::MissingUploadId)?
            .to_owned();
        let send = self
            .send_upload(&upload_id, file_path, &content_type)
            .await?;
        let final_upload_id = send
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or(upload_id.as_str());

        let normalized_prompt = prompt.trim().replace("\r\n", "\n").replace('\r', "\n");
        let mut body = format!("**Prompt:**\n\n```\n{normalized_prompt}\n```");

        let mut properties = json!({
            "Name": {"title": [{"text": {"content": file_name}}]},
            "Image": {
                "files": [
                    {
                        "type": "file_upload",
                        "file_upload": {"id": final_upload_id}
                    }
                ]
            }
        });
        if let Some(created) = created_time {
            properties["Created Time"] = json!({"date": {"start": created}});
            body.push_str(&format!("\n\n**Date:** {created}"));
        }

        let payload = json!({
            "parent": {"database_id": self.database_id},
            "properties": properties,
            "markdown": body,
        });
        let url = format!("{}/v1/pages", self.base_url);
        let response = retry_http(|| async {
            let response = self
                .http
                .post(&url)
                .headers(self.headers.clone())
                .json(&payload)
                .send()
                .await?;
            let response = ensure_success(response, Method::POST).await?;
            Ok(response.json::<Value>().await?)
        })
        .await?;
        Ok(response)
    }
}

fn content_type_for_path(path: &Path) -> String {
    mime_guess::from_path(path)
        .first_or_octet_stream()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_type_for_common_extensions() {
        assert_eq!(content_type_for_path(Path::new("a.png")), "image/png");
        assert_eq!(content_type_for_path(Path::new("a.jpg")), "image/jpeg");
        assert_eq!(content_type_for_path(Path::new("a.webp")), "image/webp");
    }
}
