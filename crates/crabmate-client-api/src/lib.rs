//! 多端 Client 共用纯逻辑（无 Tauri / `web-sys` / `reqwest` / `tokio`）。
//!
//! 见 `docs/design/client_shared_logic.md`（S1–S4 + hash 交接键名）。

#![forbid(unsafe_code)]

pub mod approval;
pub mod auth;
pub mod chat_body;
pub mod handoff;
pub mod secrets;
pub mod sessions;
pub mod url;
pub mod workspace;

pub use approval::{
    ApprovalDecision, ApprovalPostBody, CommandApprovalRequest, approval_post_body_json,
    approval_session_id_is_valid, is_approval_session_id_char, parse_command_approval_data,
};
pub use auth::{
    HEADER_AUTHORIZATION, HEADER_GITHUB_TOKEN, HEADER_X_API_KEY, WebApiCredentialPair,
    github_token_header_value, web_api_credential_pair,
};
pub use chat_body::{
    ChatStreamCoreFields, build_chat_stream_core_body, merge_chat_stream_core_fields,
};
pub use handoff::{
    API_BASE_HASH_KEY, BEARER_HASH_KEY, handoff_hash_fragment, is_handoff_hash_key,
    percent_encode_unreserved,
};
pub use secrets::{KEYRING_SERVICE, SecretSlot, WEB_API_BEARER_KEYRING_ACCOUNT, secret_slot_names};
pub use sessions::{
    SessionListItem, conversation_id_for_resume, parse_session_list_rows,
    session_item_conversation_id_for_resume,
};
pub use url::{ApiUrlError, join_api_path, normalize_api_base};
pub use workspace::{
    WorkspaceInfo, WorkspaceSetError, WorkspaceSetErrorKind, parse_workspace_set_ok_body,
    workspace_set_http_error_message,
};
