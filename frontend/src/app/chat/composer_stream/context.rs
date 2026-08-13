//! 单次 `/chat/stream` 回调共享的只读/句柄上下文（与 `callbacks` 分离，便于单测与浏览）。
//!
//! # 会话绑定
//!
//! [`ChatStreamCallbackCtx::bound_stream_session_id`] 为 **发起 attach 时** 的快照（见 [`super::stream_attach_lifecycle::prepare_stream_attach`]），并与 [`crate::chat_session_state::ChatSessionSignals::stream_transport`] 内 Bound 车道的 `session_id` 同步写入。
//! [`ChatStreamCallbackCtx::attach_generation`] 与 [`crate::chat_session_state::ChatStreamTransport::attach_generation`] 在发起时对齐，供各 `on_*` 丢弃陈旧回调。
//! 流式过程中即使用户切换 UI 的「当前会话」，SSE 仍应把增量写回**该场会话**在 `sessions` 中的那条记录；
//! 读写收口见 [`super::callbacks::stream_session_access`]。
//!
//! # 可变草稿
//!
//! [`ChatStreamCallbackCtx::scratch`] 承载本轮 attach 的可变草稿（[`super::stream_sse_scratch::StreamSseScratch`] → [`super::stream_turn_scratch_state::StreamTurnScratchState`]），与 Leptos `RwSignal` 解耦。

use leptos::prelude::*;

use crate::chat_session_state::ChatSessionSignals;
use crate::i18n::Locale;

use super::super::handles::ComposerStreamShell;
use super::stream_sse_scratch::StreamSseScratch;

/// 纯判定：`session_sync` 全局槽是否允许写入。
///
/// SSE 同步事件（`conversation_id` / `conversation_saved.revision`）只在该流**正被用户查看**且未过期时
/// 写全局槽；否则仅落绑定会话记录，切换回来时由切换 Effect 从记录重推导，避免后台流污染活跃会话的同步快照。
#[must_use]
pub(super) fn session_sync_global_gate(
    stale: bool,
    active_id: &str,
    bound_session_id: &str,
) -> bool {
    !stale && active_id == bound_session_id
}

/// 各 `Rc<dyn Fn>` 共享：避免在闭包树中重复 `Arc::clone` 同一组字段。
pub(super) struct ChatStreamCallbackCtx {
    pub(super) chat: ChatSessionSignals,
    pub(super) locale: RwSignal<Locale>,
    pub(super) bound_stream_session_id: String,
    /// 与 [`ChatStreamTransport::attach_generation`] 在发起 attach 时对齐；不一致表示本轮 SSE 已过期。
    pub(super) attach_generation: u64,
    pub(super) scratch: StreamSseScratch,
    pub(super) approval_session_store_id: String,
    pub(super) shell: ComposerStreamShell,
}

impl ChatStreamCallbackCtx {
    /// 当前闭包是否属于已过期的 attach（例如新一轮发送已 `abort` 上一轮但仍可能排队执行）。
    #[inline]
    pub(super) fn is_stale(&self) -> bool {
        self.chat.stream_transport.get_untracked().attach_generation != self.attach_generation
    }

    /// 本轮 SSE 是否正被用户查看（即 UI `active_id` 仍是绑定会话且未过期）。
    ///
    /// 决定 [`crate::chat_session_state::ChatSessionSignals::session_sync`] 全局槽是否同步写入；
    /// 后台流（用户已切到其它会话）只落绑定会话记录。
    #[inline]
    pub(super) fn is_bound_session_active(&self) -> bool {
        session_sync_global_gate(
            self.is_stale(),
            self.chat.active_id.get_untracked().as_str(),
            self.bound_stream_session_id.as_str(),
        )
    }

    /// 绑定会话记录中的服务端 `conversation_id`。
    ///
    /// 供回前台 resume 使用：后台流期间全局 `session_sync` 槽可能已切到其它会话，
    /// resume 的 `conversation_id` 必须取**绑定会话记录**，否则会把绑定会话的流续到错误会话。
    #[must_use]
    pub(super) fn bound_session_server_conversation_id(&self) -> Option<String> {
        let sid = self.bound_stream_session_id.as_str();
        self.chat.sessions.with_untracked(|list| {
            list.iter()
                .find(|s| s.id == sid)
                .and_then(|s| s.trimmed_server_conversation_id().map(str::to_string))
        })
    }

    /// 绑定会话的服务端 `conversation_id`（流式写回侧栏会话优先，其次 `session_sync`）。
    pub(super) fn server_conversation_id_for_tokens(&self) -> Option<String> {
        if self.is_stale() {
            return None;
        }
        self.bound_session_server_conversation_id().or_else(|| {
            self.chat
                .session_sync
                .with_untracked(|s| s.conversation_id.clone())
        })
    }
}

#[cfg(test)]
mod session_sync_global_gate_tests {
    use super::session_sync_global_gate;

    #[test]
    fn active_and_fresh_writes_global_slot() {
        assert!(session_sync_global_gate(false, "s1", "s1"));
    }

    #[test]
    fn background_stream_skips_global_slot() {
        assert!(!session_sync_global_gate(false, "s2", "s1"));
    }

    #[test]
    fn stale_never_writes_global_slot() {
        assert!(!session_sync_global_gate(true, "s1", "s1"));
    }
}
