//! HTTP error handling and retry policy shared by adapters.

use std::time::Duration;

use reqwest::header::{HeaderMap, RETRY_AFTER};
use reqwest::{Client, Method, Response, StatusCode};

use crate::shared::constants::{
    HTTP_TIMEOUT_SECONDS, MAX_HTTP_ATTEMPTS, MAX_RETRY_DELAY_SECONDS, MIN_RETRY_DELAY_SECONDS,
};

const BODY_MAX_CHARS: usize = 500;

#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error("request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("HTTP {status} from {url}: {body}")]
    Status {
        status: StatusCode,
        url: String,
        body: String,
        retry_after_ms: Option<u64>,
    },
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

impl HttpError {
    pub fn status(&self) -> Option<StatusCode> {
        match self {
            HttpError::Status { status, .. } => Some(*status),
            _ => None,
        }
    }

    pub fn is_retryable(&self) -> bool {
        match self {
            HttpError::Request(e) => {
                e.is_timeout() || e.is_connect() || e.is_request() || e.is_body() || e.is_decode()
            }
            HttpError::Status { status, .. } => {
                *status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
            }
            HttpError::Io(_) => false,
        }
    }
}

pub fn build_client() -> Result<Client, reqwest::Error> {
    Client::builder()
        .timeout(Duration::from_secs(HTTP_TIMEOUT_SECONDS))
        .build()
}

/// Run an HTTP operation with exponential backoff and `Retry-After` support.
pub async fn retry_http<T, F, Fut>(operation: F) -> Result<T, HttpError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, HttpError>>,
{
    retry_http_with(operation, HttpError::is_retryable).await
}

pub async fn retry_http_with<T, F, Fut, P>(
    mut operation: F,
    should_retry: P,
) -> Result<T, HttpError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, HttpError>>,
    P: Fn(&HttpError) -> bool,
{
    let mut attempts = 0usize;
    loop {
        match operation().await {
            Ok(value) => return Ok(value),
            Err(error) => {
                attempts += 1;
                if !should_retry(&error) || attempts >= MAX_HTTP_ATTEMPTS {
                    return Err(error);
                }
                let delay = match &error {
                    HttpError::Status { retry_after_ms, .. } => retry_after_ms
                        .map(|ms| ms.min(MAX_RETRY_DELAY_SECONDS * 1000))
                        .unwrap_or_else(|| backoff_delay(attempts)),
                    _ => backoff_delay(attempts),
                };
                tokio::time::sleep(Duration::from_millis(delay)).await;
            }
        }
    }
}

fn backoff_delay(attempt: usize) -> u64 {
    let base = MIN_RETRY_DELAY_SECONDS * 1000;
    let exponent = (attempt - 1).min(5) as u32;
    (base << exponent).min(MAX_RETRY_DELAY_SECONDS * 1000)
}

/// Turn a non-success response into a detailed `HttpError`.
pub async fn ensure_success(response: Response, _method: Method) -> Result<Response, HttpError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let retry_after_ms = response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .map(|secs| secs * 1000);
    let url = response.url().as_str().to_owned();
    let body = truncate_body(response.text().await.unwrap_or_default());
    Err(HttpError::Status {
        status,
        url: safe_url(&url),
        body,
        retry_after_ms,
    })
}

pub fn safe_url(url: &str) -> String {
    url.split(['?', '#']).next().unwrap_or(url).to_owned()
}

fn truncate_body(mut body: String) -> String {
    if body.chars().count() > BODY_MAX_CHARS {
        body.truncate(BODY_MAX_CHARS);
        body.push('…');
    }
    body
}

/// Read the full response text, failing cleanly on decode errors.
pub async fn response_text(response: Response) -> Result<String, HttpError> {
    Ok(response.text().await?)
}

pub fn auth_bearer(token: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    if let Ok(value) = format!("Bearer {token}").parse() {
        headers.insert(reqwest::header::AUTHORIZATION, value);
    }
    headers
}
