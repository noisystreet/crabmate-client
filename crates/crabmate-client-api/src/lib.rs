//! 多端 Client 共用纯逻辑（无 Tauri / `web-sys` / `reqwest` / `tokio`）。
//!
//! 见 `docs/design/client_shared_logic.md`（S1：`url` / `auth` / `secrets`；S2：`approval`）。

#![forbid(unsafe_code)]

pub mod approval;
pub mod auth;
pub mod secrets;
pub mod url;

pub use approval::{
    ApprovalDecision, ApprovalPostBody, CommandApprovalRequest, approval_post_body_json,
    approval_session_id_is_valid, is_approval_session_id_char, parse_command_approval_data,
};
pub use auth::{
    HEADER_AUTHORIZATION, HEADER_GITHUB_TOKEN, HEADER_X_API_KEY, WebApiCredentialPair,
    github_token_header_value, web_api_credential_pair,
};
pub use secrets::{KEYRING_SERVICE, SecretSlot, WEB_API_BEARER_KEYRING_ACCOUNT, secret_slot_names};
pub use url::{ApiUrlError, join_api_path, normalize_api_base};
