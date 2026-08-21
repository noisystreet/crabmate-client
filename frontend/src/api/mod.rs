//! 浏览器 `fetch` + `/chat/stream` SSE 解析（单前端实现）。
//!
//! 子模块划分：[`browser`] 共享句柄、[`client_llm_storage`] 本机模型键值、[`http`] JSON API、[`chat_stream`] 流式聊天。

#![allow(clippy::collapsible_if)]

mod browser;
mod chat_stream;
pub(crate) mod client_llm_cache;
pub(crate) mod client_llm_storage;
mod connect_handoff;
mod github_oauth;
pub(crate) mod github_secrets_local;
mod http;
mod http_workspace_clone;
mod http_workspace_projects;
mod http_workspace_raw;
pub(crate) mod llm_secrets_local;
mod saved_models;
mod session_store;
pub mod user_data;
pub(crate) mod web_api_bearer_local;

pub use github_oauth::{
    GithubDeviceStartDto, fetch_github_oauth_device_status, post_github_oauth_device_cancel,
    post_github_oauth_device_logout, post_github_oauth_device_start,
};

#[allow(unused_imports)] // 对外 re-export；handoff / 设置页等按需引用
pub use browser::{
    api_base_host_is_loopback, api_base_url, api_url, is_web_api_credential_error,
    set_api_base_url, set_web_api_bearer_token, web_api_bearer_token_is_set,
};
pub use chat_stream::{ChatStreamCallbacks, OnToolCallFn, SendChatStreamParams, send_chat_stream};
#[allow(unused_imports)] // 对外 re-export；壳内设置页等按需引用
pub use client_llm_storage::{
    clear_client_llm_api_key_storage, clear_executor_llm_api_key_storage,
    client_llm_storage_has_api_key, executor_llm_storage_has_api_key,
    load_client_llm_text_fields_from_storage, load_executor_llm_text_fields_from_storage,
    load_readonly_tool_ttl_cache_follow_server_from_storage, persist_client_llm_to_storage,
    persist_executor_llm_to_storage, persist_readonly_tool_ttl_cache_follow_server,
};
pub use connect_handoff::consume_mobile_connect_handoff;
pub(crate) use http::fetch_auth_raster_image_blob_url;
#[allow(unused_imports)]
pub use http::{
    ChatBranchError, GithubRepoContextData, SkillListItem, SkillsListData, StatusData, TaskItem,
    TasksData, UploadedFileInfo, WebUiConfig, WorkspaceChangelogResponse, WorkspaceData,
    WorkspaceEntry, WorkspaceFileReadData, delete_workspace_dir, delete_workspace_file,
    fetch_conversation_messages, fetch_github_repo_context, fetch_skills, fetch_status,
    fetch_tasks, fetch_tool_job_status, fetch_web_ui_config, fetch_workspace,
    fetch_workspace_changelog, fetch_workspace_file, post_chat_branch, post_config_reload,
    post_tool_job_cancel, post_workspace_dir, post_workspace_file_write,
    post_workspace_file_write_opts, post_workspace_set, save_tasks, submit_chat_approval,
    upload_files_multipart,
};
pub use http_workspace_clone::{
    WorkspaceCloneRequest, WorkspaceCloneSseEvent, infer_project_name_from_clone_url,
    post_workspace_clone_stream,
};
pub use http_workspace_projects::{fetch_workspace_projects, post_workspace_project};
pub use http_workspace_raw::put_workspace_file_raw;
pub use llm_secrets_local::PersistKind;
pub use saved_models::{
    ExecutorLlmDraftSignals, MainLlmDraftSignals, SavedModelPreset,
    apply_saved_model_preset_to_executor_fields, apply_saved_model_preset_to_main_fields,
    load_saved_model_presets_from_storage, matching_saved_preset_index,
    persist_saved_model_presets_to_storage, persist_saved_model_presets_to_storage_async,
};
pub use session_store::post_session_conversation_store;
#[allow(unused_imports)] // 对外 re-export；壳内设置/断开等按需引用
pub use web_api_bearer_local::{
    clear_web_api_bearer_on_disconnect, ensure_web_api_bearer_hydrated,
    hydrate_web_api_bearer_from_secure_store, secure_web_api_bearer_backend_available,
    set_web_api_bearer_token_async,
};
