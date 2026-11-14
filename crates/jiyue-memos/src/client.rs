//! HTTP client for Memos REST API v1.

use std::fmt;
use std::time::Duration;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};

use crate::model::Memo;

/// Error type for Memos API operations.
#[derive(Debug)]
pub enum MemosError {
    Http(reqwest::Error),
    InvalidUrl(String),
    Api { status: u16, message: String },
    Serde(serde_json::Error),
}

impl fmt::Display for MemosError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http(e) => write!(f, "HTTP request error: {}", e),
            Self::InvalidUrl(s) => write!(f, "Invalid base URL: {}", s),
            Self::Api { status, message } => write!(f, "API error (status {}): {}", status, message),
            Self::Serde(e) => write!(f, "Serialization/Deserialization error: {}", e),
        }
    }
}

impl std::error::Error for MemosError {}

impl From<reqwest::Error> for MemosError {
    fn from(e: reqwest::Error) -> Self {
        Self::Http(e)
    }
}

impl From<serde_json::Error> for MemosError {
    fn from(e: serde_json::Error) -> Self {
        Self::Serde(e)
    }
}

/// Raw response item from Memos API v1.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawMemo {
    name: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    create_time: Option<String>,
    #[serde(default)]
    update_time: Option<String>,
    #[serde(default)]
    visibility: Option<String>,
    #[serde(default)]
    pinned: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListMemosResponse {
    #[serde(default)]
    memos: Vec<RawMemo>,
    #[serde(default)]
    next_page_token: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateMemoRequest {
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    visibility: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateMemoRequest {
    content: String,
}

/// A lightweight asynchronous client for Memos.
#[derive(Clone)]
pub struct MemosClient {
    base_url: String,
    client: reqwest::Client,
}

impl MemosClient {
    /// Constructs a new client with `base_url` and optional `token`.
    pub fn new(base_url: impl Into<String>, token: Option<&str>) -> Result<Self, MemosError> {
        let mut base = base_url.into().trim().trim_end_matches('/').to_string();
        if !base.starts_with("http://") && !base.starts_with("https://") {
            base = format!("https://{}", base);
        }

        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Some(tok) = token {
            let tok = tok.trim();
            if !tok.is_empty() {
                let auth_val = format!("Bearer {}", tok);
                if let Ok(hv) = HeaderValue::from_str(&auth_val) {
                    headers.insert(AUTHORIZATION, hv);
                }
            }
        }

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(10))
            .build()?;

        Ok(Self {
            base_url: base,
            client,
        })
    }

    /// Fetches a list of recent memos.
    pub async fn list_memos(&self, page_size: usize, page_token: Option<&str>) -> Result<(Vec<Memo>, Option<String>), MemosError> {
        let mut url = format!("{}/api/v1/memos?pageSize={}", self.base_url, page_size);
        if let Some(token) = page_token {
            url.push_str("&pageToken=");
            url.push_str(token);
        }

        let res = self.client.get(&url).send().await?;
        let status = res.status();
        if !status.is_success() {
            let text = res.text().await.unwrap_or_default();
            return Err(MemosError::Api {
                status: status.as_u16(),
                message: text,
            });
        }

        let resp: ListMemosResponse = res.json().await?;
        let memos = resp
            .memos
            .into_iter()
            .map(|raw| Memo::from_raw(raw.name, raw.content, raw.create_time, raw.update_time, raw.visibility, raw.pinned))
            .collect();

        Ok((memos, resp.next_page_token))
    }

    /// Creates a new memo with the given Markdown content.
    pub async fn create_memo(&self, content: &str, visibility: Option<&str>) -> Result<Memo, MemosError> {
        let url = format!("{}/api/v1/memos", self.base_url);
        let req_body = CreateMemoRequest {
            content: content.to_string(),
            visibility: visibility.map(|v| v.to_string()),
        };

        let res = self.client.post(&url).json(&req_body).send().await?;
        let status = res.status();
        if !status.is_success() {
            let text = res.text().await.unwrap_or_default();
            return Err(MemosError::Api {
                status: status.as_u16(),
                message: text,
            });
        }

        let raw: RawMemo = res.json().await?;
        Ok(Memo::from_raw(raw.name, raw.content, raw.create_time, raw.update_time, raw.visibility, raw.pinned))
    }

    /// Updates the content of a memo (e.g. for checking off a todo item).
    pub async fn update_memo_content(&self, memo_name: &str, new_content: &str) -> Result<Memo, MemosError> {
        let name = memo_name.trim_start_matches('/');
        let url = format!("{}/api/v1/{}?updateMask=content", self.base_url, name);
        let req_body = UpdateMemoRequest {
            content: new_content.to_string(),
        };

        let res = self.client.patch(&url).json(&req_body).send().await?;
        let status = res.status();
        if !status.is_success() {
            let text = res.text().await.unwrap_or_default();
            return Err(MemosError::Api {
                status: status.as_u16(),
                message: text,
            });
        }

        let raw: RawMemo = res.json().await?;
        Ok(Memo::from_raw(raw.name, raw.content, raw.create_time, raw.update_time, raw.visibility, raw.pinned))
    }

    /// Deletes a memo.
    pub async fn delete_memo(&self, memo_name: &str) -> Result<(), MemosError> {
        let name = memo_name.trim_start_matches('/');
        let url = format!("{}/api/v1/{}", self.base_url, name);
        let res = self.client.delete(&url).send().await?;
        let status = res.status();
        if !status.is_success() {
            let text = res.text().await.unwrap_or_default();
            return Err(MemosError::Api {
                status: status.as_u16(),
                message: text,
            });
        }
        Ok(())
    }
}
