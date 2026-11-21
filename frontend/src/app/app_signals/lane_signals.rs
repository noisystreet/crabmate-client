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

use crate::api::connection_scope;

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
    /// 这条 lane 的生命周期阶段（#561）：前台 / 后台 / 关闭中 / 已关闭。
    ///
    /// 判据同其余字段 —— 切 lane 时它**该**变。单 lane 时恒 `Foreground`，所以
    /// 所有「只在前台才做」的门控都退化成改动前的行为。
    ///
    /// 转移规则与查询在 `crabmate_client_api::lane_phase`（纯逻辑 + 单测），
    /// 本字段只是它的响应式镜像。
    pub phase: RwSignal<crabmate_client_api::lane_phase::LanePhase>,
}

impl LaneSignals {
    /// `scope` 是这条 lane 的连接作用域（#558 项 1b）：8 个字段里唯一读它的是
    /// [`ConnSignals::new`]，其余 lane 级字段与连接作用域无关。
    pub fn new(scope: &connection_scope::ScopeHolder) -> Self {
        let session_sync = RwSignal::new(SessionSyncState::local_only());
        Self {
            connection: ConnSignals::new(scope),
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
            // 新建的 lane 就是用户正要看的那条（`LanePhase::default()` = Foreground）。
            phase: RwSignal::new(Default::default()),
        }
    }
}

impl Default for LaneSignals {
    fn default() -> Self {
        Self::new(&connection_scope::active_holder())
    }
}
