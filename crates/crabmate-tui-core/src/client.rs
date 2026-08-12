//! `reqwest` 封装：鉴权头 + 健康探测 + 审批提交。

use crabmate_client_api::auth::{HEADER_X_API_KEY, web_api_credential_pair};
use reqwest::Client;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use serde_json::json;

use crate::approval::ApprovalDecision;
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
        if let Some(creds) = web_api_credential_pair(&self.cfg.bearer_token) {
            let v = HeaderValue::from_str(&creds.authorization)
                .map_err(|e| TermError::Message(format!("invalid bearer token header: {e}")))?;
            h.insert(AUTHORIZATION, v);
            let name = HeaderName::from_bytes(HEADER_X_API_KEY.as_bytes())
                .map_err(|e| TermError::Message(format!("invalid X-API-Key header name: {e}")))?;
            h.insert(
                name,
                HeaderValue::from_str(&creds.api_key)
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
        Self::ensure_success(resp).await
    }

    /// `POST /chat/approval`：放行/拒绝非白名单命令（流仍挂在 `/chat/stream`）。
    pub async fn submit_chat_approval(
        &self,
        approval_session_id: &str,
        decision: ApprovalDecision,
    ) -> Result<(), TermError> {
        let url = self.url("/chat/approval")?;
        let mut headers = self.auth_headers()?;
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let body = json!({
            "approval_session_id": approval_session_id,
            "decision": decision.as_api_str(),
        });
        let resp = self
            .http
            .post(&url)
            .headers(headers)
            .json(&body)
            .send()
            .await?;
        Self::ensure_success(resp).await
    }

    async fn ensure_success(resp: reqwest::Response) -> Result<(), TermError> {
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
