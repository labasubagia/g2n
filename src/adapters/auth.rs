//! Google OAuth 2.0 loopback (localhost) and token storage.

use std::path::{Path, PathBuf};

use reqwest::{Client, Method};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use url::Url;

use crate::domain::models::AppConfig;
use crate::shared::constants::{OAUTH_AUTHORIZE_URL, OAUTH_TOKEN_URL};
use crate::shared::http::{HttpError, build_client, ensure_success, retry_http};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: Option<u64>,
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error(transparent)]
    Http(#[from] HttpError),
    #[error("OAuth error: {0}")]
    OAuth(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid URL: {0}")]
    Url(#[from] url::ParseError),
    #[error("timed out waiting for authorization")]
    Timeout,
}

/// Open the system browser (best effort; prints the URL as a fallback).
fn open_browser(url: &str) {
    println!("Opening browser for authorization…\n  {url}");
    #[cfg(unix)]
    if let Ok(mut child) = std::process::Command::new("xdg-open").arg(url).spawn() {
        let _ = child.wait();
    }
    #[cfg(windows)]
    if let Ok(mut child) = std::process::Command::new("cmd")
        .args(["/c", "start", "", url])
        .spawn()
    {
        let _ = child.wait();
    }
}

/// Run the loopback OAuth flow: open a local listener, ask the user to
/// authorize in the browser, then exchange the returned code for tokens.
async fn loopback_auth(
    http: &Client,
    config: &AppConfig,
    timeout_secs: u64,
) -> Result<TokenPair, AuthError> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}");

    let auth_url = authorize_url(
        &config.google.client_id,
        &config.google.scope,
        &redirect_uri,
    )?;
    open_browser(auth_url.as_str());

    let code = await_authorization_code(&listener, timeout_secs).await?;
    exchange_code_for_tokens(
        http,
        OAUTH_TOKEN_URL,
        &config.google.client_id,
        &config.google.client_secret,
        &code,
        &redirect_uri,
    )
    .await
}

/// Build the Google authorization URL with the loopback redirect.
fn authorize_url(client_id: &str, scope: &str, redirect_uri: &str) -> Result<Url, AuthError> {
    let mut url = Url::parse(OAUTH_AUTHORIZE_URL)?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("client_id", client_id);
        query.append_pair("redirect_uri", redirect_uri);
        query.append_pair("response_type", "code");
        query.append_pair("scope", scope);
        query.append_pair("access_type", "offline");
        query.append_pair("prompt", "consent");
    }
    Ok(url)
}

async fn await_authorization_code(
    listener: &TcpListener,
    timeout_secs: u64,
) -> Result<String, AuthError> {
    let mut buf = [0u8; 8192];
    tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        async {
            let (mut stream, _) = listener.accept().await?;
            let n = stream.read(&mut buf).await?;
            let text = String::from_utf8_lossy(&buf[..n]);
            let code = extract_authorization_code(&text)
                .ok_or_else(|| AuthError::OAuth("no code in callback request".to_owned()))?;
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 63\r\nConnection: close\r\n\r\n<html><body>Authorization complete. Close this window.</body></html>",
                )
                .await?;
            Ok::<String, AuthError>(code)
        },
    )
    .await
    .map_err(|_| AuthError::Timeout)?
}

/// Parse `code` out of the first line of the HTTP request, e.g.
/// `GET /oauth2callback?code=…&scope=… HTTP/1.1`.
fn extract_authorization_code(request: &str) -> Option<String> {
    let line = request.lines().next()?;
    let uri = line.split_whitespace().nth(1)?;
    let parsed = Url::parse(&format!("http://127.0.0.1{uri}")).ok()?;
    if parsed.path() != "/" && parsed.path() != "/oauth2callback" {
        return None;
    }
    for (key, value) in parsed.query_pairs() {
        if key == "code" {
            return Some(value.into_owned());
        }
    }
    None
}

/// Exchange an authorization code for tokens.
async fn exchange_code_for_tokens(
    http: &Client,
    token_url: &str,
    client_id: &str,
    client_secret: &str,
    code: &str,
    redirect_uri: &str,
) -> Result<TokenPair, AuthError> {
    let response = retry_http(|| async {
        let response = http
            .post(token_url)
            .form(&[
                ("client_id", client_id),
                ("client_secret", client_secret),
                ("code", code),
                ("redirect_uri", redirect_uri),
                ("grant_type", "authorization_code"),
            ])
            .send()
            .await?;
        ensure_success(response, Method::POST).await
    })
    .await?;
    Ok(response.json().await?)
}

pub async fn refresh_access_token(
    http: &Client,
    token_url: &str,
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
) -> Result<TokenPair, AuthError> {
    let response = retry_http(|| async {
        let response = http
            .post(token_url)
            .form(&[
                ("client_id", client_id),
                ("client_secret", client_secret),
                ("refresh_token", refresh_token),
                ("grant_type", "refresh_token"),
            ])
            .send()
            .await?;
        ensure_success(response, Method::POST).await
    })
    .await?;
    Ok(response.json().await?)
}

pub fn load_tokens(path: &Path) -> Result<Option<TokenPair>, AuthError> {
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(path)?;
    Ok(Some(serde_json::from_str(&content)?))
}

pub fn save_tokens(path: &Path, tokens: &TokenPair) -> Result<(), AuthError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(tokens)?;
    std::fs::write(path, content)?;
    Ok(())
}

/// Ensure a usable access token, refreshing or running loopback OAuth as needed.
pub async fn ensure_access_token(
    token_path: &Path,
    config: &AppConfig,
    timeout_secs: u64,
    interactive: bool,
) -> Result<String, AuthError> {
    let http = build_client()?;
    if let Some(token) = refresh_existing(&http, token_path, config).await {
        return Ok(token);
    }
    if !interactive {
        return Err(AuthError::OAuth(
            "no cached token and --interactive is false".to_owned(),
        ));
    }
    let tokens = loopback_auth(&http, config, timeout_secs).await?;
    save_tokens(token_path, &tokens)?;
    Ok(tokens.access_token)
}

async fn refresh_existing(http: &Client, token_path: &Path, config: &AppConfig) -> Option<String> {
    let stored = load_tokens(token_path).ok()??;
    let refresh = stored.refresh_token?;
    let refreshed = refresh_access_token(
        http,
        OAUTH_TOKEN_URL,
        &config.google.client_id,
        &config.google.client_secret,
        &refresh,
    )
    .await
    .ok()?;
    save_tokens(token_path, &refreshed).ok()?;
    Some(refreshed.access_token)
}

pub fn default_token_path() -> PathBuf {
    crate::shared::constants::data_dir().join("tokens.json")
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorize_url_contains_all_params() {
        let url = authorize_url("client-123", "scope-a", "http://127.0.0.1:9999").unwrap();
        let query = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect::<Vec<_>>();
        for (key, value) in [
            ("client_id", "client-123"),
            ("redirect_uri", "http://127.0.0.1:9999"),
            ("response_type", "code"),
            ("scope", "scope-a"),
            ("access_type", "offline"),
            ("prompt", "consent"),
        ] {
            assert!(
                query.contains(&(key.to_owned(), value.to_owned())),
                "missing {key}={value}"
            );
        }
    }

    #[test]
    fn parses_authorization_code() {
        let root_req = "GET /?code=rootcode&scope=foo HTTP/1.1\r\nHost: 127.0.0.1\r\n";
        assert_eq!(
            extract_authorization_code(root_req).as_deref(),
            Some("rootcode")
        );
        assert_eq!(
            extract_authorization_code("GET /other?code=x HTTP/1.1\r\n"),
            None
        );
        assert_eq!(
            extract_authorization_code("GET /oauth2callback HTTP/1.1\r\n"),
            None
        );
    }
}
