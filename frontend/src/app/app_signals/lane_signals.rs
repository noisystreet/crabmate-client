//! **lane 级**状态（#561）：一条连接自己的那一组信号的**构造清单**。
//!
//! 判据只有一条：**切到另一条 lane 时，这个值该不该变？** 该变 → 属于这里；不该变 →
//! 留在 [`super::WindowSignals`]。
//!
//! # 它现在只是「构造清单」，不是「每 lane 一份的载体」
//!
//! 说清楚免得误用：[`super::AppSignals`] 把这 8 个字段**平铺**在自己身上（不是嵌一个
//! `lane` 字段），构造时调本结构的 `new()` 再逐个拆出去，之后**没有任何地方持有本结构
//! 的实例**。之所以平铺而不是嵌套，是因为 `Deref` 会让 `move ||` 闭包捕获整个
//! `AppSignals`，把 `Rc` / `Arc<Mutex<AbortController>>` 拖进 Leptos 的 `Send + Sync`
//! 要求（实测报错）；不加 `Deref` 则 284 处 `app.chat` 都要改成 `app.lane.chat`。
//!
//! 所以真要做「每 lane 一份」时，正确的做法是**新增 lane 容器**去持有 N 份本结构，
//! 并给 `AppSignals` 加具名访问器，而不是 `vec![app.lane]`（那不成立）。
//!
//! # 与 TUI 的对应关系
//!
//! 对应 `crates/crabmate-tui/src/tui_mode/mod.rs` 的 `LaneApp`：那边的 `client` /
//! `overrides`（`/model` `/mode` `/role` `/perm`）/ `st` / `drafts` / `panel` / `persisted`
//! / `body` 全在这一层，理由照抄它结构体上的注释 ——「`/model` 只影响当前 lane；`&mut`
//! 借用会让第二条 lane 无法构造」。
//!
//! 这里**刻意不含**的两类：
//! - 窗口级偏好（locale / 主题 / 列宽 / 设置面板）→ [`super::WindowSignals`]；
//! - 跨 lane 的协作设施（当前 lane 索引、SSE 按 lane 分发）→ 外层 lane 容器。
//!
//! TUI 的 `rx` 由 `TuiApp` 独占（所有 lane 共用一个通道，避免抢消息）也是同一情形：
//! 它不属于任何一条 lane。Web 侧对应的是「SSE 事件按连接标识归属」，不是共享通道。

use std::collections::HashMap;

use leptos::prelude::*;

use super::{
    ApprovalSignals, ChatComposerSignals, ConnSignals, LLMSettingsSignals, StatusSignals,
    StreamControlSignals, WorkspaceSignals,
};
use crate::chat_session_state::ChatSessionSignals;
use crate::session_sync::SessionSyncState;

#[derive(Clone)]
pub struct LaneSignals {
    pub connection: ConnSignals,
    pub chat: ChatSessionSignals,
    pub chat_composer: ChatComposerSignals,
    pub approval: ApprovalSignals,
    pub stream: StreamControlSignals,
    pub workspace: WorkspaceSignals,
    pub status: StatusSignals,
    pub llm_settings: LLMSettingsSignals,
}

impl LaneSignals {
    pub fn new() -> Self {
        let session_sync = RwSignal::new(SessionSyncState::local_only());
        Self {
            connection: ConnSignals::new(),
            chat: ChatSessionSignals {
                sessions: RwSignal::new(Vec::new()),
                active_id: RwSignal::new(String::new()),
                session_sync,
                session_hydrate_nonce: RwSignal::new(0),
                stream_transport: RwSignal::new(
                    crate::chat_session_state::ChatStreamTransport::default(),
                ),
                stream_last_sse_event_seq: RwSignal::new(0),
                reasoning_preserved: RwSignal::new(HashMap::new()),
                stream_text_overlay: RwSignal::new(None),
                stream_overlay_display_mid: RwSignal::new(None),
                stream_overlay_revision: RwSignal::new(0),
                tool_output_chunks: RwSignal::new(HashMap::new()),
                tool_job_states: RwSignal::new(HashMap::new()),
                tool_file_paths: RwSignal::new(HashMap::new()),
                conversation_prompt_tokens: RwSignal::new(None),
                history_loading_older: RwSignal::new(false),
                conversation_hydration_err: RwSignal::new(None),
                context_used_watermark: RwSignal::new(
                    crate::chat_session_state::ContextUsedWatermark::default(),
                ),
            },
            chat_composer: ChatComposerSignals::new(),
            approval: ApprovalSignals::new(),
            stream: StreamControlSignals::new(),
            workspace: WorkspaceSignals::new(),
            status: StatusSignals::new(),
            llm_settings: LLMSettingsSignals::new(),
        }
    }
}

impl Default for LaneSignals {
    fn default() -> Self {
        Self::new()
    }
}
