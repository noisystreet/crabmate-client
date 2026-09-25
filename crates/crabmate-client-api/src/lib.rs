//! 多端 Client 共用纯逻辑（无 Tauri / `web-sys` / `reqwest` / `tokio`）。
//!
//! 见 `docs/design/client_shared_logic.md`（S1–S4 + hash 交接 + health JSON 子集）。

#![forbid(unsafe_code)]

pub mod ag_ui_parser;
pub mod approval;
pub mod auth;
pub mod chat_body;
pub mod handoff;
pub mod health;
pub mod markdown_inline;
pub mod markdown_normalize;
pub mod messages;
pub mod paths;
pub mod prompt_tokens;
pub mod secrets;
pub mod sessions;
pub mod slash;
pub mod sse_dispatch;
pub mod url;
pub mod workspace;

pub use ag_ui_parser::{format_user_error_with_meta, parse_ag_ui_line};
pub use approval::{
    ApprovalDecision, ApprovalDecisionApi, ChatApprovalRequestBody, CommandApprovalData,
    approval_session_id_is_valid,
};
pub use auth::{
    HEADER_AUTHORIZATION, HEADER_GITHUB_TOKEN, HEADER_X_API_KEY, github_token_header_value,
    web_api_credential_pair,
};
pub use chat_body::{
    ChatStreamCoreFields, build_chat_stream_core_body, insert_trimmed_str,
    llm_context_tokens_for_chat_body, merge_chat_stream_core_fields,
    readonly_tool_ttl_cache_secs_for_chat_body, temperature_for_chat_body,
};
pub use handoff::{
    API_BASE_HASH_KEY, BEARER_HASH_KEY, handoff_hash_fragment, is_handoff_hash_key,
    percent_encode_unreserved,
};
pub use health::health_degraded_note;
pub use markdown_inline::{InlineSpan, parse_inline_markdown};
pub use markdown_normalize::normalize_markdown_for_render;
pub use messages::{http_error_message, http_error_text};
pub use prompt_tokens::{
    TiktokenPromptTokensSnapshot, parse_tiktoken_prompt_tokens_value, tiktoken_from_ag_ui_object,
};
pub use secrets::{KEYRING_SERVICE, SecretSlot, WEB_API_BEARER_KEYRING_ACCOUNT};
pub use sessions::{
    SessionListRow, conversation_id_for_resume, session_row_conversation_id_for_resume,
};
pub use url::normalize_api_base;
pub use workspace::{
    WorkspaceDirData, WorkspaceDirEntry, WorkspaceInfo, WorkspaceProjectsData,
    WorkspaceSetErrorKind, parse_workspace_project_open_body, parse_workspace_set_ok_body,
};
