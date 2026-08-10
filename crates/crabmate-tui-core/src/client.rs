//! `reqwest` 封装：鉴权头 + 健康探测。

use reqwest::Client;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};

use crate::config::ConnectionConfig;
use crate::error::TermError;
use crate::url::api_url;

/// 已配置的远程 `serve` 客户端。
#[derive(Debug, Clone)]
pub struct ServeClient {
    http: Client,
    cfg: ConnectionConfig,
}

impl ServeClient {
    pub fn new(cfg: ConnectionConfig) -> Result<Self, TermError> {
        let http = Client::builder()
            .user_agent(concat!("crabmate-tui/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self { http, cfg })
    }

    #[must_use]
    pub fn config(&self) -> &ConnectionConfig {
        &self.cfg
    }

    #[must_use]
    pub fn http(&self) -> &Client {
        &self.http
    }

    pub fn auth_headers(&self) -> Result<HeaderMap, TermError> {
        let mut h = HeaderMap::new();
        let t = self.cfg.bearer_token.trim();
        if !t.is_empty() {
            let v = HeaderValue::from_str(&format!("Bearer {t}"))
                .map_err(|e| TermError::Message(format!("invalid bearer token header: {e}")))?;
            h.insert(AUTHORIZATION, v);
            h.insert(
                HeaderName::from_static("x-api-key"),
                HeaderValue::from_str(t)
                    .map_err(|e| TermError::Message(format!("invalid X-API-Key header: {e}")))?,
            );
        }
        Ok(h)
    }

    pub fn url(&self, path: &str) -> Result<String, TermError> {
        api_url(&self.cfg.api_base, path)
    }

    /// `GET /health`：连通性探测。
    pub async fn probe_health(&self) -> Result<(), TermError> {
        let url = self.url("/health")?;
        let resp = self
            .http
            .get(&url)
            .headers(self.auth_headers()?)
            .send()
            .await?;
        let status = resp.status();
        if status.is_success() {
            return Ok(());
        }
        let body = resp.text().await.unwrap_or_default();
        Err(TermError::Http {
            status: status.as_u16(),
            body: body.trim().chars().take(400).collect(),
        })
    }
}
