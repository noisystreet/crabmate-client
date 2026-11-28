//! 壳层 `*_view` 聚合为 [`AppShellCtx`]，压缩 `App` 内 `view!` 的长实参列表。
//! **#561 S1**：子组件入参的组装方法已搬到 `LaneRuntime`（`app_signals::lane_view_inputs`），
//! 本结构只剩「窗口级容器 + 一个取用口」—— [`Self::current_runtime`]（当前前台那条 lane
//! 的运行时句柄）。
//! **未使用** Leptos `Context` / `<Provider>` 承载整份壳层：状态含 `Rc<RefCell<…>>`、`Rc<dyn Fn()>` 等，
//! 不满足 `provide_context` 的 `Send + Sync + 'static` 约束；以结构体 **`Clone`**（内部多为
//! `Copy` / `Rc::clone` / `Arc::clone`）在 `*_view` 间传递即可。
//! 聊天域中已满足边界的切片见 [`super::shell_runtime_context::ChatShellLeptosContext`]（由 [`super::App`] 根
//! [`provide_context`]，侧栏等用 [`super::shell_runtime_context::expect_chat_shell_ctx`] 读取）。
//!
//! 根壳层 `wire_*` 的**注册顺序**影响隐式时序依赖；见 [`app_shell_wire_phases`](super::app_shell_wire_phases)
//! 与 [`bootstrap_app_shell`](super::app_shell_bootstrap::bootstrap_app_shell)（[`init_app_shell`](super::app_shell_init::init_app_shell)）；聊天列内部顺序见 [`wire_chat_domain`](super::chat::wire_chat_domain)。
//! 聊天域信号切片见 [`ChatDomainWiringSignals`](super::app_signals::ChatDomainWiringSignals)（与 [`AppSignals`] 同源，供接线单点组装）。

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use leptos::html::Div;
use leptos::prelude::*;

use crate::chat_session_state::ChatStreamBusyMemos;
use crate::i18n::Locale;
use crate::session_ops::SessionContextAnchor;
use crate::sse_dispatch::ThinkingTraceInfo;

use crate::app_prefs::SidePanelView;

use super::app_signals::AppSignals;
use super::workspace::panel_state::WorkspacePanelSignals;
use crate::app::shell::status::tasks_state::StatusTasksSignals;

use super::changelist_modal::ChangelistModalBodyState;

/// 壳顶栏所需句柄（阶段 B：避免向 `mobile_shell_header_view` 传递整份 [`AppShellCtx`]）。
#[derive(Clone, Copy)]
pub struct MobileShellHeaderSignals {
    pub locale: RwSignal<Locale>,
    pub editor_layout_mode: RwSignal<bool>,
    pub is_narrow_viewport: RwSignal<bool>,
    pub ide_menu_bar_bridge: RwSignal<Option<super::ide_menu_bar::IdeMenuBarBridge>>,
    pub workspace_pick: super::workspace::root_actions::WorkspaceRootPickHandle,
    pub ide_menubar_dropdown_open: RwSignal<bool>,
    /// 迷你运行点兜底：右列抽屉收起时在顶栏展示运行态色点，点击打开工作区抽屉。
    pub side_panel_view: RwSignal<SidePanelView>,
    pub status_tasks: StatusTasksSignals,
    pub status_err: RwSignal<Option<String>>,
    pub stream_busy_memos: ChatStreamBusyMemos,
}

/// 变更集预览模态所需句柄（阶段 B：避免向 `changelist_modal_view` 传递整份 [`AppShellCtx`]）。
#[derive(Clone, Copy)]
pub struct ChangelistModalSignals {
    pub changelist_modal_open: RwSignal<bool>,
    pub locale: RwSignal<Locale>,
    pub changelist_modal_body: RwSignal<ChangelistModalBodyState>,
    pub changelist_fetch_nonce: RwSignal<u64>,
    pub changelist_body_ref: NodeRef<Div>,
}

/// 会话列表模态所需句柄（阶段 B：避免向 `session_list_modal_view` 传递整份 [`AppShellCtx`]）。
/// 会话 / 草稿 / 语言 / 过滤开关经 [`super::shell_runtime_context::ChatShellLeptosContext`]。
#[derive(Clone, Copy)]
pub struct SessionListModalSignals {
    pub session_modal: RwSignal<bool>,
}

impl SessionListModalSignals {
    #[must_use]
    pub fn from_app_signals(app: &AppSignals) -> Self {
        Self {
            session_modal: app.modal.session_modal,
        }
    }
}

/// 底栏状态条所需句柄（阶段 B：避免向 `side_status_stack_view` 传递整份 [`AppShellCtx`]）。
/// 会话句柄与界面语言经 [`super::shell_runtime_context::ChatShellLeptosContext`]（角色切换时清流式句柄等）。
#[derive(Clone)]
pub struct StatusBarFooterSignals {
    pub status_tasks: StatusTasksSignals,
    pub status_err: RwSignal<Option<String>>,
    pub stream_busy_memos: ChatStreamBusyMemos,
    pub client_llm_storage_tick: RwSignal<u64>,
    pub selected_agent_role: RwSignal<Option<String>>,
    pub agent_role_user_override: RwSignal<bool>,
    pub selected_session_mode: RwSignal<String>,
    pub session_mode_user_override: RwSignal<bool>,
    /// 底栏「思考模式」开关（本机两态 `on` / `off`）。与设置页 `llm_thinking_mode` 同源。
    pub llm_thinking_mode: RwSignal<String>,
    pub refresh_status: Arc<dyn Fn() + Send + Sync>,
    pub user_prefs_reload_nonce: RwSignal<u64>,
    pub user_prefs_sync_phase: RwSignal<crate::user_prefs_sync_state::UserPrefsSyncPhase>,
    pub user_prefs_load_err: RwSignal<Option<String>>,
    pub user_prefs_save_err: RwSignal<Option<String>>,
    pub settings_page: RwSignal<bool>,
    pub conversation_hydration_err: RwSignal<Option<String>>,
    pub context_usage_page: RwSignal<bool>,
}

type SideResizeHandlesCell = Rc<
    RefCell<
        Option<(
            leptos_dom::helpers::WindowListenerHandle,
            leptos_dom::helpers::WindowListenerHandle,
        )>,
    >,
>;

/// 左侧导航轨所需句柄（阶段 B：避免向 `sidebar_nav_view` 传递整份 [`AppShellCtx`]）。
/// 会话列表 / 草稿 / 查找后焦点等经 [`super::shell_runtime_context::ChatShellLeptosContext`] 注入子树。
#[derive(Clone)]
pub struct SidebarNavSignals {
    pub locale: RwSignal<Locale>,
    pub mobile_nav_open: RwSignal<bool>,
    pub session_modal: RwSignal<bool>,
    pub new_session: Rc<dyn Fn()>,
    pub sidebar_session_query: RwSignal<String>,
    pub global_message_query: RwSignal<String>,
    pub sidebar_search_panel_open: RwSignal<bool>,
    pub sidebar_rail_ctx_menu: RwSignal<Option<(f64, f64)>>,
    pub chat_find_panel_open: RwSignal<bool>,
    pub session_context_menu: RwSignal<Option<SessionContextAnchor>>,
    pub sidebar_rail_collapsed: RwSignal<bool>,
    /// 「设置」页开关：左栏连接行在**单连接**时就是「改基址」的入口
    /// （右列那颗 chip 合并进左栏之后，这里是它唯一的去处）。
    pub settings_page: RwSignal<bool>,
}

/// 右列侧栏所需句柄（阶段 B：避免向 `side_column_view` 传递整份 [`AppShellCtx`]）。
#[derive(Clone)]
pub struct SideColumnViewSignals {
    pub locale: RwSignal<Locale>,
    pub side_resize_dragging: RwSignal<bool>,
    pub side_panel_view: RwSignal<SidePanelView>,
    pub side_width: RwSignal<f64>,
    pub side_resize_session: Rc<RefCell<Option<(f64, f64)>>>,
    pub side_resize_handles: SideResizeHandlesCell,
    pub view_menu_open: RwSignal<bool>,
    pub settings_page: RwSignal<bool>,
    pub workspace_panel: WorkspacePanelSignals,
    pub status_tasks: StatusTasksSignals,
    pub refresh_workspace: Arc<dyn Fn() + Send + Sync>,
    pub refresh_tasks: Arc<dyn Fn() + Send + Sync>,
    pub toggle_task: Arc<dyn Fn(String) + Send + Sync>,
    pub changelist_modal_open: RwSignal<bool>,
    pub changelist_fetch_nonce: RwSignal<u64>,
    pub insert_workspace_file_ref: StoredValue<Arc<dyn Fn(String) + Send + Sync>>,
    pub thinking_trace_log: RwSignal<Vec<ThinkingTraceInfo>>,
    pub is_narrow_viewport: RwSignal<bool>,
    /// 「在编辑器查看」桥接（工作区右键 → IDE 布局，复用工具卡同款 nonce / path）。
    pub ide_open_file_nonce: RwSignal<u64>,
    pub ide_open_file_path: RwSignal<Option<String>>,
    pub chat: crate::chat_session_state::ChatSessionSignals,
    /// 右列状态栈入参（原底栏；随右列展开挂载）。
    pub status_bar: StatusBarFooterSignals,
}

/// 根壳 `App` 与侧栏、底栏、各模态之间共享的一组句柄（由 `App` 构造一次，按需 `clone()`）。
///
/// **Step 1**：扁平字段已并入 [`AppSignals`]；此处仅保留初始化阶段构造的闭包与 [`ChatColumnShell`]，
/// 新增全局 `RwSignal` 时只需改 [`AppSignals`] 与（按需）[`bootstrap_app_shell`](super::app_shell_bootstrap::bootstrap_app_shell) /
/// [`init_app_shell`](super::app_shell_init::init_app_shell) 中的 wire 实参，不必再维护超长「逐字段拷贝」列表。
#[derive(Clone)]
pub struct AppShellCtx {
    /// 窗口内的 lane 容器（#561 阶段 3）：现在恒一条。
    ///
    /// **一条 lane 的全部运行时都在这里** —— 信号、接线产物（`refresh_*` /
    /// `new_session`）与聊天列壳（`ChatColumnShell`）。改造前它们是本结构上的单个
    /// 字段，只要再开一条 lane 就会互相覆盖；现在按 lane 各存一份，经
    /// [`Self::current_runtime`] 取当前那条。
    pub(crate) lanes: super::app_signals::LaneSet,
}

impl AppShellCtx {
    /// 取「当前前台那条 lane」的运行时句柄（`Rc`，常数开销、不持借用）。
    ///
    /// S1 起视图入参一律由它生成（`rt.xxx_signals()`），`active` 只在**取这一次**
    /// 生效 —— 拿到的是那条 lane 的句柄，切 lane 不会改写已取出的这一份。
    #[must_use]
    pub fn current_runtime(&self) -> Rc<super::app_signals::LaneRuntime> {
        self.lanes.current_runtime()
    }

    /// 新建一条 lane（#561 S2）：存储域后缀按 §7 取 `-l<n>`，`base` 给定时作为这条 lane 的
    /// 基址（左栏「连接」要从目标实例的地址起步，而不是「同 Origin」）。
    ///
    /// `name` 是**左栏那行的主体**（#569 对齐 TUI）：ctl 行给实例名、设置页给 `None`（回退基址）。
    ///
    /// **调用点必须在 `mount_to` 的闭包里**（见 [`super::app_signals::LaneSet::add_lane`]）：
    /// 接线里那约 29 个 `Effect::new` 需要一个新子树的 owner。
    pub fn add_lane(
        &self,
        name: Option<&str>,
        base: Option<&str>,
    ) -> Rc<super::app_signals::LaneRuntime> {
        let label = self.lanes.next_storage_label();
        let scope = crate::api::connection_scope::new_holder(label, base);
        // 行上显示规范化之后的基址（与请求实际用的同一份）。
        let api_base = scope.api_base();
        self.lanes.add_lane(scope, name, &api_base).0
    }
}
