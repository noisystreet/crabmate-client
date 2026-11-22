//! `App` 级响应式信号的**按域聚合**。
//!
//! 减少 `App` 中平铺的 40+ `RwSignal` 声明，使 `App` 更贴近「声明信号 + 调用 `wire_*`」的壳层职责。
//! 子模块按域拆分文件；[`AppSignals`] 与 [`ChatDomainWiringSignals`] 仍在本 `mod` 根聚合导出。
//! 已有的聚合类型：[`ChatSessionSignals`](crate::chat_session_state::ChatSessionSignals)、
//! [`StatusTasksSignals`](super::status_tasks_state::StatusTasksSignals)、
//! [`WorkspacePanelSignals`](super::workspace_panel_state::WorkspacePanelSignals)。

mod approval;
mod chat_composer;
mod connection;
mod ide_chrome;
mod ide_editor;
mod lane_set;
mod lane_signals;
mod lane_view_inputs;
mod llm_settings;
mod modal;
mod resize;
mod shell_ui;
mod sidebar;
mod status;
mod stream_control;
mod window_signals;
mod workspace;

use leptos::prelude::*;

use crate::api::connection_scope;

pub use approval::ApprovalSignals;
pub use chat_composer::ChatComposerSignals;
pub use connection::ConnSignals;
pub use ide_chrome::IdeChromeSignals;
pub use ide_editor::IdeEditorSignals;
pub use lane_signals::LaneSignals;
// 窗口内的 lane 容器（#561 阶段 3）：crate 内可见 —— 壳层用它取「当前前台那条」。
pub(crate) use lane_set::{LaneRailRow, LaneRuntime, LaneSet};
pub use llm_settings::LLMSettingsSignals;
pub use modal::ModalSignals;
pub use resize::ResizeSignals;
pub use shell_ui::ShellUISignals;
pub use sidebar::SidebarSignals;
pub use status::StatusSignals;
pub use stream_control::StreamControlSignals;
pub use window_signals::WindowSignals;
pub use workspace::WorkspaceSignals;

pub use super::status_tasks_state::StatusTasksSignals;
pub use super::workspace_panel_state::WorkspacePanelSignals;
pub use crate::chat_session_state::ChatSessionSignals;

use crate::clarification_form::PendingClarificationForm;
use crate::i18n::Locale;

#[derive(Clone)]
pub struct AppSignals {
    pub initialized: RwSignal<bool>,
    /// **窗口级**状态（#561）：切 lane 不该变的那组值（locale / 主题 / 列宽 / 设置面板 …）。
    pub shell_ui: ShellUISignals,
    pub sidebar: SidebarSignals,
    pub modal: ModalSignals,
    pub ide_editor: IdeEditorSignals,
    pub ide_chrome: IdeChromeSignals,
    pub resize: ResizeSignals,
    // ---- 以下是 **lane 级**（#561）：切 lane 该跟着变的那组值 ----
    //
    // 它们在这里**平铺**而不是嵌成 `lane: LaneSignals`，理由是闭包捕获粒度：
    // 一旦经 `Deref` 访问，`move ||` 闭包捕获的会是整个 `AppSignals`，把
    // `ResizeSignals` 的 `Rc` 与 `StreamControlSignals` 的 `Arc<Mutex<AbortController>>`
    // 一并拖进 Leptos 的 `Send + Sync` 要求里（实测报 `Rc cannot be sent between threads`）。
    // 平铺则保持「闭包只捕获那个 Copy 句柄」的现状。
    //
    // `LaneSignals` 仍是这组字段的唯一构造来源（见 [`Self::new_lane`]），也是将来
    // 「每 lane 一份」的载体。
    /// 这条 lane 的连接作用域（#558 项 1b）：`api_base` / Web API Bearer 的真源。
    ///
    /// **每 lane 一份**：`new_lane` 收一份 holder 存进来，lane 容器造第二条 lane 时传它
    /// 自己那份。`ACTIVE` 仍只是「当前前台用哪一份」的指针，这里存的才是「**这条** lane
    /// 用哪一份」—— 跨 lane 路径读这里，不读 `ACTIVE`。
    pub(crate) scope: connection_scope::ScopeHolder,
    pub connection: ConnSignals,
    pub chat: ChatSessionSignals,
    pub chat_composer: ChatComposerSignals,
    pub approval: ApprovalSignals,
    pub stream: StreamControlSignals,
    pub workspace: WorkspaceSignals,
    pub status: StatusSignals,
    pub llm_settings: LLMSettingsSignals,
    /// 这条 lane 的生命周期阶段（#561）：由 `crabmate_client_api::lane_phase` 的转移表
    /// 决定，本字段是它的响应式镜像。单 lane 时恒 `Foreground` —— 所以所有
    /// 「只在前台才做」的门控都退化成改动前的行为。
    pub phase: RwSignal<crabmate_client_api::lane_phase::LanePhase>,
}

impl AppSignals {
    /// 聊天域接线用信号切片（[`ChatDomainWiringSignals`]）；含一份 [`ChatComposerSignals`] 克隆，**仅**宜在初始化等低频路径调用。
    #[inline]
    #[must_use]
    pub fn chat_domain_wiring(&self) -> ChatDomainWiringSignals {
        ChatDomainWiringSignals::from_app_signals(self)
    }

    pub fn new() -> Self {
        Self::new_lane(&WindowSignals::new(), connection_scope::active_holder())
    }

    /// 建一条 lane 的 [`AppSignals`]：连接级全新建，**窗口级复用 `window` 的那批句柄**。
    ///
    /// `RwSignal` 是 Copy、`ResizeSignals` 内部是 `Rc`，所以复制出来的每一份 lane 共享同一个
    /// 窗口级 signal —— 切 lane 时主题 / 语言 / 列宽不会跳，而连接级状态天然隔离。
    /// 单 lane 用法（当前的全部用法）与 `Self::new()` 完全等价。
    ///
    /// `scope` 是**这一条 lane 自己的**连接作用域（#558 项 1b）：`new()` 传前台默认那份，
    /// lane 容器造第二条 lane 时传它新建的那份 —— 于是「这条 lane 连的是谁」跟着 lane 走，
    /// 而不是跟着 thread_local 指针走。
    pub(crate) fn new_lane(window: &WindowSignals, scope: connection_scope::ScopeHolder) -> Self {
        // 先建 lane 内容、再取窗口级句柄：两边都会碰 `localStorage`（`ConnSignals::new`
        // 读基址分域键，`ShellUISignals::new` 读 shell prefs），但**键不重叠、顺序无关**。
        // 若将来某一边开始读对方的键，这里就要重新排。
        let lane = LaneSignals::new(&scope);
        Self {
            initialized: RwSignal::new(false),
            shell_ui: window.shell_ui,
            sidebar: window.sidebar,
            modal: window.modal,
            ide_editor: window.ide_editor,
            ide_chrome: window.ide_chrome,
            resize: window.resize.clone(),
            scope,
            connection: lane.connection,
            chat: lane.chat,
            chat_composer: lane.chat_composer,
            approval: lane.approval,
            stream: lane.stream,
            workspace: lane.workspace,
            status: lane.status,
            llm_settings: lane.llm_settings,
            phase: lane.phase,
        }
    }

    pub fn to_status_tasks(&self) -> StatusTasksSignals {
        StatusTasksSignals {
            status_data: self.status.status_data,
            status_loading: self.status.status_loading,
            status_fetch_phase: self.status.status_fetch_phase,
            status_fetch_err: self.status.status_fetch_err,
            tasks_data: self.status.tasks_data,
            tasks_err: self.status.tasks_err,
            tasks_loading: self.status.tasks_loading,
            github_repo: self.status.github_repo,
        }
    }

    pub fn to_workspace_panel(&self) -> WorkspacePanelSignals {
        WorkspacePanelSignals {
            workspace_data: self.workspace.workspace_data,
            workspace_subtree_expanded: self.workspace.workspace_subtree_expanded,
            workspace_subtree_cache: self.workspace.workspace_subtree_cache,
            workspace_subtree_loading: self.workspace.workspace_subtree_loading,
            workspace_err: self.workspace.workspace_err,
            workspace_loading: self.workspace.workspace_loading,
            workspace_path_draft: self.workspace.workspace_path_draft,
            workspace_set_err: self.workspace.workspace_set_err,
            workspace_set_busy: self.workspace.workspace_set_busy,
            workspace_pick_busy: self.workspace.workspace_pick_busy,
            workspace_project_modal_open: self.workspace.workspace_project_modal_open,
            workspace_clone_modal_open: self.workspace.workspace_clone_modal_open,
            workspace_pool_enabled: self.workspace.workspace_pool_enabled,
            workspace_browser_pick_modal_open: self.workspace.workspace_browser_pick_modal_open,
            recent_workspace_roots: self.workspace.recent_workspace_roots,
            workspace_context_menu: self.workspace.workspace_context_menu,
            workspace_pending_create: self.workspace.workspace_pending_create,
        }
    }
}

impl Default for AppSignals {
    fn default() -> Self {
        Self::new()
    }
}

/// 聊天列 `wire_chat_domain_effects` 所需信号的**单点切片**（[`AppSignals`] 子域组合）。
///
/// 目的：壳层初始化只调用 [`ChatDomainWiringSignals::from_app_signals`]，避免向
/// [`super::chat::wire_chat_domain::WireChatDomainEffectsArgs`] 重复传入与 [`ChatSessionSignals`] 同源的
/// `sessions` / `active_id`；接线内部一律经 [`Self::chat`] 访问会话向量与活动 id。
#[derive(Clone)]
pub struct ChatDomainWiringSignals {
    pub initialized: RwSignal<bool>,
    pub chat: ChatSessionSignals,
    pub composer: ChatComposerSignals,
    pub pending_clarification: RwSignal<Option<PendingClarificationForm>>,
    pub locale: RwSignal<Locale>,
    pub apply_assistant_display_filters: RwSignal<bool>,
    pub selected_agent_role: RwSignal<Option<String>>,
    pub agent_role_user_override: RwSignal<bool>,
    pub selected_session_mode: RwSignal<String>,
    pub session_mode_user_override: RwSignal<bool>,
}

impl ChatDomainWiringSignals {
    /// 从整份 [`AppSignals`] 抽取聊天域接线子集（与 [`crate::app::chat::ComposerStreamShell::from_app_signals`] 对称的「单点组装」）。
    #[must_use]
    pub fn from_app_signals(app: &AppSignals) -> Self {
        Self {
            initialized: app.initialized,
            chat: app.chat,
            composer: app.chat_composer,
            pending_clarification: app.approval.pending_clarification,
            locale: app.shell_ui.locale,
            apply_assistant_display_filters: app.shell_ui.apply_assistant_display_filters,
            selected_agent_role: app.llm_settings.selected_agent_role,
            agent_role_user_override: app.llm_settings.agent_role_user_override,
            selected_session_mode: app.llm_settings.selected_session_mode,
            session_mode_user_override: app.llm_settings.session_mode_user_override,
        }
    }
}
