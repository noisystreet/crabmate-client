//! `reqwest` 封装：鉴权头 + 健康探测 + 审批提交 + 回合取消。

use crabmate_client_api::auth::{HEADER_X_API_KEY, web_api_credential_pair};
use crabmate_client_api::{ApprovalDecision, ApprovalPostBody, health_degraded_note};
use reqwest::Client;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use serde::Deserialize;

use crate::config::ConnectionConfig;
use crate::error::TermError;
use crate::url::api_url;

/// `POST /chat/stream/{job_id}/cancel` 的响应体。
#[derive(Deserialize)]
struct CancelChatStreamBody {
    #[serde(default)]
    cancelled: bool,
}

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

    /// `GET /health`：连通性探测。`degraded` 时在 stderr 打印失败检查摘要，仍视为成功。
    pub async fn probe_health(&self) -> Result<(), TermError> {
        let url = self.url("/health")?;
        let resp = self
            .http
            .get(&url)
            .headers(self.auth_headers()?)
            .send()
            .await?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(TermError::Http {
                status: status.as_u16(),
                body: body.trim().chars().take(400).collect(),
            });
        }
        if let Some(note) = health_degraded_note(&body) {
            eprintln!("[crabmate-tui] /health degraded (optional checks failed): {note}");
        }
        Ok(())
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
        let body = ApprovalPostBody::new(approval_session_id, decision);
        let resp = self
            .http
            .post(&url)
            .headers(headers)
            .json(&body)
            .send()
            .await?;
        Self::ensure_success(resp).await
    }

    /// `POST /chat/stream/{job_id}/cancel`：让 serve 停掉该回合（仅 abort SSE 不够）。
    ///
    /// 任务已结束（410 `STREAM_JOB_GONE`）视为成功；其余失败留给调用方提示。
    pub async fn cancel_chat_stream(&self, job_id: u64) -> Result<(), TermError> {
        let url = self.url(&format!("/chat/stream/{job_id}/cancel"))?;
        let mut headers = self.auth_headers()?;
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let resp = self
            .http
            .post(&url)
            .headers(headers)
            .body("{}")
            .send()
            .await?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if status.is_success() {
            let cancelled = serde_json::from_str::<CancelChatStreamBody>(&body)
                .map(|b| b.cancelled)
                .unwrap_or(false);
            if cancelled {
                return Ok(());
            }
            return Err(TermError::Message("stream cancel rejected".into()));
        }
        // 410 = job 已结束；旧 serve 无此路由（404）等失败由调用方决定如何处理。
        if status.as_u16() == 410 {
            return Ok(());
        }
        Err(TermError::Http {
            status: status.as_u16(),
            body: body.trim().chars().take(400).collect(),
        })
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
