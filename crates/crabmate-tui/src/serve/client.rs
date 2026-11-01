//! `reqwest` 封装：鉴权头 + 健康探测 + 审批提交 + 回合取消。

use std::time::Duration;

use crabmate_client_api::auth::{HEADER_X_API_KEY, web_api_credential_pair};
use crabmate_client_api::{
    ApprovalDecision, ApprovalDecisionApi, ChatApprovalRequestBody, health_degraded_note, paths,
};
use reqwest::Client;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use serde::Deserialize;

use crate::serve::config::ConnectionConfig;
use crate::serve::error::{TermError, http_error};
use crate::serve::url::api_url;

/// 建连（TCP + TLS）超时：serve 不可达 / 被防火墙黑洞时快速失败。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// 非流式请求总超时。**不**设在 [`Client`] 上——同一 client 也承载 `/chat/stream`
/// 长流，总超时会掐断进行中的回合。新增非流式请求须自行 `.timeout(REQUEST_TIMEOUT)`。
pub(super) const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// 空闲连接在池中的保留时长：短于 serve / 中间代理的 keep-alive 上限，
/// 避免复用已被对端关闭的连接（首次写入即报错）。
const POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// `/chat/stream/{job}/cancel` 的超时：交互打断路径，短于 [`REQUEST_TIMEOUT`]，
/// 失败时尽快回落到「后台回合可能仍在跑，可 /resume」提示。
const CANCEL_TIMEOUT: Duration = Duration::from_secs(5);

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
            .connect_timeout(CONNECT_TIMEOUT)
            .pool_idle_timeout(POOL_IDLE_TIMEOUT)
            // 交互式逐 token 流：禁用 Nagle，避免小帧被合并延迟。
            .tcp_nodelay(true)
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
        let url = self.url(paths::HEALTH)?;
        let resp = self
            .http
            .get(&url)
            .headers(self.auth_headers()?)
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(self.error_for(status.as_u16(), &body));
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
        let url = self.url(paths::CHAT_APPROVAL)?;
        let mut headers = self.auth_headers()?;
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let body = ChatApprovalRequestBody {
            approval_session_id: approval_session_id.to_string(),
            decision: decision.as_api_str().to_string(),
        };
        let resp = self
            .http
            .post(&url)
            .headers(headers)
            .json(&body)
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await?;
        self.ensure_success(resp).await
    }

    /// `POST /chat/stream/{job_id}/cancel`：让 serve 停掉该回合（仅 abort SSE 不够）。
    ///
    /// 任务已结束（410 `STREAM_JOB_GONE`）视为成功；其余失败留给调用方提示。
    pub async fn cancel_chat_stream(&self, job_id: u64) -> Result<(), TermError> {
        let url = self.url(&paths::chat_stream_cancel(&job_id.to_string()))?;
        let mut headers = self.auth_headers()?;
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let resp = self
            .http
            .post(&url)
            .headers(headers)
            .body("{}")
            // 取消是交互中打断路径：失败要尽快回落到「后台可能仍在跑」提示，而不是卡 30s。
            .timeout(CANCEL_TIMEOUT)
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
        Err(self.error_for(status.as_u16(), &body))
    }

    async fn ensure_success(&self, resp: reqwest::Response) -> Result<(), TermError> {
        let status = resp.status();
        if status.is_success() {
            return Ok(());
        }
        let body = resp.text().await.unwrap_or_default();
        Err(self.error_for(status.as_u16(), &body))
    }

    /// 非 2xx → `TermError`，并在「401 且本连接没有 Bearer」时补上成因。
    ///
    /// 两个条件同时成立时，只写 `UNAUTHORIZED` 等于让用户去查服务端：真因在客户端没拿到
    /// Bearer（[`crate::shell_keyring`] 的提示只落在 stderr，全屏 TUI 起来后看不见）。
    /// 带了 Bearer 的 401 **不**补这句 —— 那时是「有凭据但不匹配」，处置方向完全不同。
    pub(super) fn error_for(&self, status: u16, text: &str) -> TermError {
        let err = http_error(status, text);
        if status != 401 || !self.cfg.bearer_token.is_empty() {
            return err;
        }
        TermError::Message(format!(
            "{err}（本次请求未携带 Web API Bearer：--bearer / CM_WEB_API_BEARER_TOKEN 未提供，桌面壳钥匙串槽也未命中）"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client_with_bearer(bearer: &str) -> ServeClient {
        ServeClient::new(ConnectionConfig::new("http://127.0.0.1:9", bearer)).expect("build client")
    }

    const UNAUTHORIZED: &str = r#"{"code":"UNAUTHORIZED","message":"缺少或无效的 Web API 凭证"}"#;

    /// 401 + 本连接没有 Bearer：文案要同时说清「凭证缺失」和「是客户端没带」。
    #[test]
    fn unauthorized_without_bearer_names_the_missing_credential() {
        let msg = client_with_bearer("")
            .error_for(401, UNAUTHORIZED)
            .to_string();
        assert!(msg.contains("UNAUTHORIZED"), "{msg}");
        assert!(msg.contains("未携带 Web API Bearer"), "{msg}");
    }

    /// 401 + 有 Bearer：那是「有凭据但不匹配」，不该再劝用户去配 Bearer。
    #[test]
    fn unauthorized_with_bearer_keeps_the_server_text() {
        assert_eq!(
            client_with_bearer("secret")
                .error_for(401, UNAUTHORIZED)
                .to_string(),
            "UNAUTHORIZED: 缺少或无效的 Web API 凭证"
        );
    }

    /// 其它状态码与 Bearer 无关，别把提示塞进 502 / 403。
    #[test]
    fn other_statuses_never_mention_the_bearer() {
        assert_eq!(
            client_with_bearer("")
                .error_for(403, r#"{"error":"forbidden"}"#)
                .to_string(),
            "forbidden"
        );
    }
}
