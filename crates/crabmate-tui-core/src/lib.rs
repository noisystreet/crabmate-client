//! 远程终端客户端核心：连接已运行的 `crabmate serve`（HTTP + SSE）。
//!
//! **不**内嵌 Agent，**不** spawn `serve`。

mod approval;
mod chat_stream;
mod client;
mod config;
mod error;
mod url;

pub use approval::{
    ApprovalDecision, ApprovalGate, AutoAllowOnce, CommandApprovalRequest, new_approval_session_id,
    parse_command_approval_data,
};
pub use chat_stream::{ChatStreamArgs, ChatStreamOutcome, run_chat_stream};
pub use client::ServeClient;
pub use config::ConnectionConfig;
pub use error::TermError;
pub use url::{api_url, normalize_api_base};
