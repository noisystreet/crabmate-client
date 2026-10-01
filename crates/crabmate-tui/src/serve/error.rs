use serde_json::Value;
use thiserror::Error;

/// 远程终端客户端错误。
#[derive(Debug, Error)]
pub enum TermError {
    #[error("invalid API base URL: {0}")]
    InvalidApiBase(String),
    #[error("HTTP {status}: {body}")]
    Http { status: u16, body: String },
    #[error("request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("stream error: {0}")]
    Stream(String),
    #[error("server run error: {0}")]
    RunError(String),
    #[error("interrupted")]
    Interrupted,
    /// 流在完成前中断（网络/传输错误），且已拿到 job 句柄可续传。
    #[error("stream interrupted at seq {after_seq} (job {job_id}): {cause}")]
    InterruptedStream {
        job_id: u64,
        after_seq: u64,
        cause: String,
    },
    #[error("{0}")]
    Message(String),
}

/// 非 2xx 响应 → 错误：优先取 JSON 体的 `error` 字段文本，否则回落为截断后的原始响应体。
///
/// `json_api`（通用 JSON 请求）与 `client`（health / approval / cancel）共用，
/// 避免同一错误体在两处呈现不一致。
pub(crate) fn http_error(status: u16, text: &str) -> TermError {
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
