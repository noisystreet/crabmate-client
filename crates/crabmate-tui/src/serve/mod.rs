//! 远程 serve 访问层（原 `crabmate-tui-core` crate 并入）：连接已运行的 `crabmate serve`（HTTP + SSE）。
//!
//! **不**内嵌 Agent，**不** spawn `serve`。

mod approval;
mod chat_classify;
mod chat_dispatch;
mod chat_stream;
mod client;
mod config;
mod conversation;
mod error;
mod json_api;
mod sessions;
mod url;
pub(crate) mod user_data;
mod workspace;

pub use approval::{
    ApprovalDecision, ApprovalGate, AutoAllowOnce, CommandApprovalData, new_approval_session_id,
};
pub use chat_stream::{
    ChatStreamArgs, ChatStreamOptions, ChatStreamOutcome, ClientLlm, ClientLlmFields, StreamCancel,
    StreamResume, StreamSink, run_chat_stream, run_chat_stream_sink,
};
pub use client::ServeClient;
pub use config::ConnectionConfig;
pub use conversation::{
    ConversationMessagesResponse, fetch_conversation_messages, write_permission_level,
};
pub use error::TermError;
pub use sessions::{
    DEFAULT_SESSION_TITLE, SessionIndexApplied, SessionIndexOp, SessionListRow, WebSessionsList,
    apply_web_session_op, conversation_id_for_resume, fetch_web_sessions, is_default_session_title,
    title_from_user_prompt,
};
pub use user_data::{
    LlmOverridesDto, UserPrefsDto, fetch_llm_overrides, fetch_user_data_prefs, put_llm_overrides,
    put_user_data_prefs,
};
pub use workspace::{
    WorkspaceDirData, WorkspaceDirEntry, WorkspaceProjectsData, WorkspaceSetError, fetch_workspace,
    fetch_workspace_dir, fetch_workspace_projects, set_workspace, switch_workspace_project,
};
