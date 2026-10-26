//! 多端 Client 共用纯逻辑（无 Tauri / `web-sys` / `reqwest` / `tokio`）。
//!
//! 见 `docs/design/client_shared_logic.md`（S1–S4 + hash 交接 + health JSON 子集）。

#![forbid(unsafe_code)]

pub mod ag_ui_parser;
pub mod approval;
pub mod auth;
pub mod chat_body;
pub mod client_llm_presets;
pub mod context_axis;
pub mod conversation_messages;
pub mod conversation_state;
pub mod handoff;
pub mod health;
pub mod markdown_inline;
pub mod markdown_normalize;
pub mod messages;
pub mod paths;
pub mod permission;
pub mod prompt_tokens;
pub mod secrets;
pub mod session_model;
pub mod sessions;
pub mod slash;
pub mod sse_dispatch;
pub mod tool_display;
pub mod url;
pub mod workspace;
pub mod workspace_rel;

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
pub use conversation_messages::{
    CONVERSATION_MESSAGES_PAGE_LIMIT, ConversationMessage, ConversationMessagesResponse,
    FIRST_TURN_WORKSPACE_CONTEXT_NAME, ReplayRow,
};
pub use conversation_state::{
    ConversationStateOutcome, ConversationStatePatch, ConversationStateView,
    classify_conversation_state_response,
};
pub use handoff::{
    API_BASE_HASH_KEY, BEARER_HASH_KEY, handoff_hash_fragment, is_handoff_hash_key,
    percent_encode_unreserved,
};
pub use health::health_degraded_note;
pub use markdown_inline::{InlineSpan, parse_inline_markdown};
pub use markdown_normalize::normalize_markdown_for_render;
pub use messages::{api_error_code, http_error_message, http_error_text};
pub use permission::{
    CLAMP_ORDER, DEFAULT_LEVEL, LEVELS, is_known_level, normalize_level, ordered_clamped_axes,
};
pub use prompt_tokens::{
    TiktokenPromptTokensSnapshot, parse_tiktoken_prompt_tokens_value, tiktoken_from_ag_ui_object,
};
pub use secrets::{KEYRING_SERVICE, SecretSlot, WEB_API_BEARER_KEYRING_ACCOUNT};
pub use session_model::{
    CURRENT_LAYOUT_SCHEMA_VERSION, ChatSession, DEFAULT_CHAT_SESSION_TITLE,
    LEGACY_LAYOUT_SCHEMA_VERSION, StoredMessage, StoredMessageState, TIMELINE_UI_STATE_KEY,
    V2_COMMENTARY_ROW_ID_PREFIX, V2_FINAL_ANSWER_ROW_ID, V2_THINK_ROW_ID_PREFIX,
    is_default_session_title, new_empty_session, new_session_id, prefer_or_create_empty_session,
    reclaim_disposable_blank_session, session_is_blank_chat, session_is_disposable_blank,
    session_is_unbound_empty_chat, title_from_user_prompt,
};
pub use sessions::{
    SessionListRow, conversation_id_for_resume, session_row_conversation_id_for_resume,
};
pub use tool_display::{ToolDisplayInput, is_placeholder_summary, tool_display_line};
pub use url::normalize_api_base;
pub use workspace::{
    WorkspaceDirData, WorkspaceDirEntry, WorkspaceInfo, WorkspaceProjectsData,
    WorkspaceSetErrorKind, parse_workspace_project_open_body, parse_workspace_set_ok_body,
};
