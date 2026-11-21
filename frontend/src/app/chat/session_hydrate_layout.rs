//! 水合页的 `layout` 映射与 v2 projection 追加（[`super::session_hydrate`] 的辅助实现）。
//!
//! 拆出本文件只因 `session_hydrate.rs` 触到文件行数门禁（≤920 行）；逻辑上仍属尾部合并。

use std::collections::HashSet;

use crabmate_client_api::layout_projection::{LayoutProjection, project_hydrated_rows};

use super::ConversationMessagesResponse;
use crate::storage::{ChatSession, StoredMessage};

/// 尾页合并结果：新消息列表 + 本页是否按 `layout` 映射成了 v2 投影行。
pub(crate) struct TailMergeOutcome {
    pub(crate) messages: Vec<StoredMessage>,
    /// 本页历史行已按会话级 `layout` 映射成稳定 v2 行键（调用方据此 stamp v2）。
    pub(crate) layout_projected: bool,
}

/// 按会话级 `layout` 把本页历史行映射成稳定 v2 行键；对不齐则原样返回（legacy）。
pub(super) fn project_layout_rows(
    hydrated: Vec<StoredMessage>,
    resp: &ConversationMessagesResponse,
) -> (Vec<StoredMessage>, bool) {
    let projection = project_hydrated_rows(
        &hydrated,
        resp.layout.as_ref(),
        resp.window_start_index == 0,
    );
    match projection {
        LayoutProjection::Mapped(rows) => (rows, true),
        // 回落原因只进计数：`layout_debug_counters` 在非 wasm32 上是空操作，
        // 这里不能直接调 `web_sys::console`（宿主单测会 panic）。
        LayoutProjection::Fallback(_reason) => (hydrated, false),
    }
}

/// 本页覆盖的本地行之前的前缀长度（按 `history_window_start` 与响应窗口对齐）。
///
/// v2 projection 会话也要知道**本页**覆盖了哪些本地行：追加后缀要按「本页内的 local user 数」
/// 对齐，否则翻过更早页后新回合永远追加不进来。
pub(super) fn window_prefix_len(
    session: &ChatSession,
    resp: &ConversationMessagesResponse,
) -> usize {
    match session.history_window_start {
        Some(local_start) if resp.window_start_index >= local_start => {
            ((resp.window_start_index - local_start) as usize).min(session.messages.len())
        }
        _ => 0,
    }
}

/// v2 projection 只追加服务端新回合：按**本页覆盖的本地 user 数**对齐，翻过更早页后仍能追加。
pub(super) fn append_server_only_turns_to_v2_projection(
    session: &ChatSession,
    resp: &ConversationMessagesResponse,
    hydrated: Vec<StoredMessage>,
) -> Vec<StoredMessage> {
    let keep = window_prefix_len(session, resp);
    let local_page_users = session.messages[keep..]
        .iter()
        .filter(|message| message.role == "user" && !message.is_tool)
        .count();
    let Some(suffix_start) = find_new_turn_suffix_start(&hydrated, local_page_users) else {
        return session.messages.to_vec();
    };

    let mut combined = session.messages.to_vec();
    let mut existing_ids: HashSet<String> =
        combined.iter().map(|message| message.id.clone()).collect();
    combined.extend(
        hydrated
            .iter()
            .skip(suffix_start)
            .filter(|message| existing_ids.insert(message.id.clone()))
            .cloned(),
    );
    combined
}

/// 服务端快照里第 `local_users` 个 user 之后的第一条行下标（即「新回合」起点）。
fn find_new_turn_suffix_start(hydrated: &[StoredMessage], local_users: usize) -> Option<usize> {
    let mut seen = 0usize;
    let mut start = None;
    for (index, message) in hydrated.iter().enumerate() {
        if message.role != "user" || message.is_tool {
            continue;
        }
        if seen == local_users && start.is_none() {
            start = Some(index);
        }
        seen += 1;
    }
    start
}
