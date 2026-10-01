//! 通用 JSON GET/POST（Bearer 与 health/approval 同源鉴权）。

use reqwest::header::{CONTENT_TYPE, HeaderValue};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::serve::client::{REQUEST_TIMEOUT, ServeClient};
use crate::serve::error::TermError;

impl ServeClient {
    pub async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T, TermError> {
        let url = self.url(path)?;
        let resp = self
            .http()
            .get(&url)
            .headers(self.auth_headers()?)
            .timeout(REQUEST_TIMEOUT)
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
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        ensure_ok(status, &text)?;
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&text)
            .map_err(|e| TermError::Message(format!("decode JSON from {path}: {e}")))
    }

    /// `PUT` 无内容响应端点（如 `/user-data/*` 全量保存）：2xx 即成功，不解析 body。
    pub async fn put_json_no_content(&self, path: &str, body: &Value) -> Result<(), TermError> {
        let url = self.url(path)?;
        let mut headers = self.auth_headers()?;
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let resp = self
            .http()
            .put(&url)
            .headers(headers)
            .json(body)
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        ensure_ok(status, &text)
    }

    /// `DELETE` 无内容响应端点（如 `/conversation/{id}` 幂等 204）：2xx 即成功，不解析 body。
    pub async fn delete_no_content(&self, path: &str) -> Result<(), TermError> {
        let url = self.url(path)?;
        let resp = self
            .http()
            .delete(&url)
            .headers(self.auth_headers()?)
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        ensure_ok(status, &text)
    }

    async fn read_success_text(resp: reqwest::Response) -> Result<String, TermError> {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        ensure_ok(status, &text)?;
        Ok(text)
    }
}

/// 2xx 视为成功；否则按响应体组装错误（`http_error_from_body` 会优先提取 JSON `error` 字段）。
fn ensure_ok(status: reqwest::StatusCode, text: &str) -> Result<(), TermError> {
    if status.is_success() {
        return Ok(());
    }
    Err(http_error_from_body(status.as_u16(), text))
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
