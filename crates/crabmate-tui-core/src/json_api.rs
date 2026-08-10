//! 通用 JSON GET/POST（Bearer 与 health/approval 同源鉴权）。

use reqwest::header::{CONTENT_TYPE, HeaderValue};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::client::ServeClient;
use crate::error::TermError;

impl ServeClient {
    pub async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T, TermError> {
        let url = self.url(path)?;
        let resp = self
            .http()
            .get(&url)
            .headers(self.auth_headers()?)
            .send()
            .await?;
        let text = Self::read_success_text(resp).await?;
        serde_json::from_str(&text)
            .map_err(|e| TermError::Message(format!("decode JSON from {path}: {e}")))
    }

    pub async fn post_json(&self, path: &str, body: &Value) -> Result<Value, TermError> {
        let url = self.url(path)?;
        let mut headers = self.auth_headers()?;
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let resp = self
            .http()
            .post(&url)
            .headers(headers)
            .json(body)
            .send()
            .await?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status.is_success() {
            if text.trim().is_empty() {
                return Ok(Value::Null);
            }
            return serde_json::from_str(&text)
                .map_err(|e| TermError::Message(format!("decode JSON from {path}: {e}")));
        }
        Err(http_error_from_body(status.as_u16(), &text))
    }

    async fn read_success_text(resp: reqwest::Response) -> Result<String, TermError> {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status.is_success() {
            return Ok(text);
        }
        Err(http_error_from_body(status.as_u16(), &text))
    }
}

fn http_error_from_body(status: u16, text: &str) -> TermError {
    if let Ok(v) = serde_json::from_str::<Value>(text)
        && let Some(err) = v
            .get("error")
            .and_then(|e| e.as_str())
            .filter(|s| !s.is_empty())
    {
        return TermError::Message(err.to_string());
    }
    TermError::Http {
        status,
        body: text.trim().chars().take(400).collect(),
    }
}
