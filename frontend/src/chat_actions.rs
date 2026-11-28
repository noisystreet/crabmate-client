//! 聊天侧「服务端分支」与会话 revision 对齐（无 UI、无 `view!`）。
//!
//! 与 `crate::api::http::post_chat_branch` 配合：HTTP 在 `api`，成功后对 `SessionSyncState` / `ChatSession` 的写入集中在此模块。

use leptos::prelude::*;

use crate::chat_session_state::ChatSessionSignals;
use crate::domain::session::apply_branch_revision;

/// `POST /chat/branch` 成功后，更新 `session_sync` 与活动会话的 `server_revision`。
pub fn apply_branch_success_revision(
    chat: ChatSessionSignals,
    active_session_id: &str,
    new_revision: u64,
) {
    chat.session_sync
        .update(|s| s.set_revision_after_branch(new_revision));
    chat.update_sessions_branch(|list| {
        // 规则在 `crate::domain::session`（可 host 单测）；这里只负责把它挂到信号上。
        let _ = apply_branch_revision(list, active_session_id, new_revision);
    });
}
