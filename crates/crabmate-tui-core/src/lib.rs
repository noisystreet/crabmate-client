//! 远程终端客户端核心：连接已运行的 `crabmate serve`（HTTP + SSE）。
//!
//! **不**内嵌 Agent，**不** spawn `serve`。

mod approval;
mod chat_stream;
mod client;
mod config;
mod error;
mod json_api;
mod sessions;
mod url;
mod workspace;

pub use approval::{
    ApprovalDecision, ApprovalGate, AutoAllowOnce, CommandApprovalRequest, new_approval_session_id,
    parse_command_approval_data,
};
pub use chat_stream::{
    ChatStreamArgs, ChatStreamOptions, ChatStreamOutcome, ClientLlm, ClientLlmFields, StreamCancel,
    StreamResume, StreamSink, run_chat_stream, run_chat_stream_sink,
};
pub use client::ServeClient;
pub use config::ConnectionConfig;
pub use error::TermError;
pub use sessions::{
    SessionListItem, WebSessionsList, conversation_id_for_resume, fetch_web_sessions,
};
pub use url::{api_url, normalize_api_base};
pub use workspace::{WorkspaceInfo, fetch_workspace, set_workspace};
