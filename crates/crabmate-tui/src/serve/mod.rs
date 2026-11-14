//! 远程 serve 访问层（原 `crabmate-tui-core` crate 并入）：连接已运行的 `crabmate serve`（HTTP + SSE）。
//!
//! **不**内嵌 Agent，**不** spawn `serve`。

mod approval;
// `POST /chat/branch`（issue #422）：回合重试 / 再生的服务端截断入口。
mod branch;
pub(crate) mod chat_classify;
mod chat_dispatch;
mod chat_stream;
mod client;
mod config;
mod conversation;
mod ctl;
mod error;
mod json_api;
mod session_store;
mod sessions;
// 流回调 trait 单独成文件：`chat_stream` 贴着 920 行门禁，而加一个事件就要在里面占十来行。
mod stream_sink;
// 文本模式的旁注文案与 `context_trim` 提示文案（issue #478）：同上，文案与「怎么写出去」
// 分开，`chat_stream` 里只剩一行调用。
mod text_notes;
pub(crate) mod trim_line;
mod url;
pub(crate) mod user_data;
mod workspace;

pub use approval::{
    ApprovalDecision, ApprovalGate, AutoAllowOnce, CommandApprovalData, new_approval_session_id,
};
pub use chat_stream::{
    ChatStreamArgs, ChatStreamOptions, ChatStreamOutcome, ClientLlm, ClientLlmFields, StreamCancel,
    StreamResume, run_chat_stream, run_chat_stream_sink,
};
pub use client::ServeClient;
pub use stream_sink::StreamSink;
// 澄清问卷（issue #424）：题面类型与作答状态都来自共享层，这里转出给 `tui_mode` 用 ——
// 前者是 SSE 事件载荷，后者承载「草稿 + 必填校验 + 序列化」。
pub use config::{BearerSource, ConnectionConfig};
pub use conversation::{
    ConversationMessagesResponse, fetch_all_pages, fetch_conversation_messages,
    write_permission_level,
};
pub use crabmate_client_api::clarify_form::PendingClarification;
pub use crabmate_client_api::sse_dispatch::ClarificationQuestionnaireInfo;
pub use ctl::{
    add_instance, instance_status, list_instances, primary_binding, restart_instance,
    same_api_base, set_instance_enabled,
};
pub use error::TermError;
pub use session_store::set_session_conversation_store;
pub use sessions::{
    DEFAULT_SESSION_TITLE, SessionIndexApplied, SessionIndexOp, SessionListRow, WebSessionsList,
    apply_web_session_op, conversation_id_for_resume, fetch_web_sessions, is_default_session_title,
    title_from_user_prompt,
};
pub use user_data::{
    LlmOverridesDto, McpServersFileDto, UserPrefsDto, fetch_llm_overrides, fetch_mcp_servers,
    fetch_user_data_prefs, put_llm_overrides, put_mcp_servers, put_user_data_prefs,
};
pub use workspace::{
    FILE_VIEW_MAX_LINES, WorkspaceDirData, WorkspaceDirEntry, WorkspaceFile, WorkspaceProjectsData,
    WorkspaceSetError, fetch_workspace, fetch_workspace_dir, fetch_workspace_file,
    fetch_workspace_projects, set_workspace, switch_workspace_project,
};
