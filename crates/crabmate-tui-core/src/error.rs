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
