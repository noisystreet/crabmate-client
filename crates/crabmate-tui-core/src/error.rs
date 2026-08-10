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
    #[error("{0}")]
    Message(String),
}
