//! lane 级视图入参组装（#561 **S1b**）：`App` 的 14 个 `*_signals()` / `*_view_input()`。
//!
//! # 为什么从 `AppShellCtx` 搬到这里
//!
//! S1 要把 `App` 的视图抽成 `app_body(rt)`，入参必须由「**指定的那条** lane」生成。
//! `AppShellCtx::session_list_modal_signals(&self)` 只能读 `self.signals()`
//! （= 当前前台 lane，经 `active` 指针），签名里没有「哪条 lane」的位置；
//! 挂到 [`LaneRuntime`] 上之后签名天然就是「这条 lane 的」—— `rt.xxx_signals()`
//! 读 `rt.signals` 字段，与 `active` 指向谁无关。
//!
//! N=1 等价：容器恒一条，`AppShellCtx::signals()` 与 `rt.signals` 是同一份。
//!
//! # 边界
//!
//! 取法只有两种：`self.signals`（信号）与 `self.wiring` / `self.chat_column`
//! （lane 级接线产物）。**窗口级**对象（`IdeSession`、`ChatShellLeptosContext`）不在这里 ——
//! 它们不随 lane 走，仍由 `App` 自己建。

use std::rc::Rc;
use std::sync::Arc;

use super::lane_set::LaneRuntime;

use super::super::app_shell_ctx::{
    ChangelistModalSignals, MobileShellHeaderSignals, SessionListModalSignals,
    SideColumnViewSignals, SidebarNavSignals, StatusBarFooterSignals,
};
use super::super::approval_modal::ApprovalModalSignals;
use super::super::context_usage_page::ContextUsagePageViewInput;
use super::super::ide_settings_page::IdeSettingsPageViewInput;
use super::super::settings_page::{SettingsPageFormSignals, SettingsPageViewInput};
use super::super::workspace_browser_pick_modal::WorkspaceBrowserPickModalSignals;
use super::super::workspace_clone_modal::WorkspaceCloneModalSignals;
use super::super::workspace_project_modal::WorkspaceProjectModalSignals;
use super::super::workspace_root_actions::WorkspaceRootPickHandle;

impl LaneRuntime {
    /// 「选工作区」句柄：三处模态框入参与顶栏共用一份构造。
    ///
    /// 改造前这段在三处逐字重复（`workspace_project_modal_signals` /
    /// `workspace_clone_modal_signals` / `workspace_browser_pick_modal_signals`），
    /// 搬到这里时收成一个方法，免得第四条入口再加一份。
    pub(crate) fn workspace_pick(&self) -> WorkspaceRootPickHandle {
        let signals = &self.signals;
        WorkspaceRootPickHandle {
            locale: signals.shell_ui.locale,
            chat: signals.chat,
            composer_draft: signals.chat_composer.draft,
            ws: signals.to_workspace_panel(),
            side_panel_view: signals.shell_ui.side_panel_view,
        }
    }

    pub(crate) fn session_list_modal_signals(&self) -> SessionListModalSignals {
        SessionListModalSignals::from_app_signals(&self.signals)
    }

    pub(crate) fn workspace_project_modal_signals(&self) -> WorkspaceProjectModalSignals {
        WorkspaceProjectModalSignals {
            open: self.signals.workspace.workspace_project_modal_open,
            workspace_pick: self.workspace_pick(),
        }
    }

    pub(crate) fn workspace_clone_modal_signals(&self) -> WorkspaceCloneModalSignals {
        WorkspaceCloneModalSignals {
            open: self.signals.workspace.workspace_clone_modal_open,
            settings_page: self.signals.modal.settings_page,
            workspace_pick: self.workspace_pick(),
        }
    }

    pub(crate) fn workspace_browser_pick_modal_signals(&self) -> WorkspaceBrowserPickModalSignals {
        WorkspaceBrowserPickModalSignals {
            open: self.signals.workspace.workspace_browser_pick_modal_open,
            workspace_pick: self.workspace_pick(),
        }
    }

    pub(crate) fn changelist_modal_signals(&self) -> ChangelistModalSignals {
        ChangelistModalSignals {
            changelist_modal_open: self.signals.modal.changelist_modal_open,
            locale: self.signals.shell_ui.locale,
            changelist_modal_body: self.signals.modal.changelist_modal_body,
            changelist_fetch_nonce: self.signals.modal.changelist_fetch_nonce,
            changelist_body_ref: self.signals.modal.changelist_body_ref,
        }
    }

    pub(crate) fn approval_modal_signals(&self) -> ApprovalModalSignals {
        ApprovalModalSignals {
            pending_approval: self.signals.approval.pending_approval,
            busy: self.signals.approval.approval_busy,
            // #561 §8.4：审批弹窗按 lane 阶段门控（单 lane 恒 Foreground，不改变行为）。
            phase: self.signals.phase,
            error: self.signals.approval.approval_error,
            locale: self.signals.shell_ui.locale,
        }
    }

    /// 设置页表单所需 `RwSignal` 聚合（阶段 B：避免在 `App` 的 `view!` 中重复罗列字段）。
    pub(crate) fn settings_page_form_signals(&self) -> SettingsPageFormSignals {
        SettingsPageFormSignals::from_app_signals(&self.signals)
    }

    /// IDE 编辑器设置全屏页入参。
    pub(crate) fn ide_settings_page_view_input(&self) -> IdeSettingsPageViewInput {
        IdeSettingsPageViewInput {
            ide_settings_page: self.signals.modal.ide_settings_page,
            locale: self.signals.shell_ui.locale,
            editor: self.signals.ide_editor,
        }
    }

    /// 上下文用量可视化全屏页入参（与状态栏「会话」芯片同源信号）。
    pub(crate) fn context_usage_page_view_input(&self) -> ContextUsagePageViewInput {
        ContextUsagePageViewInput {
            context_usage_page: self.signals.modal.context_usage_page,
            st: self.signals.to_status_tasks(),
            chat: self.signals.chat,
            client_llm_storage_tick: self.signals.llm_settings.client_llm_storage_tick,
            selected_agent_role: self.signals.llm_settings.selected_agent_role,
            locale: self.signals.shell_ui.locale,
        }
    }

    pub(crate) fn sidebar_nav_signals(&self) -> SidebarNavSignals {
        let signals = &self.signals;
        SidebarNavSignals {
            locale: signals.shell_ui.locale,
            mobile_nav_open: signals.sidebar.mobile_nav_open,
            session_modal: signals.modal.session_modal,
            new_session: Rc::clone(&self.wiring.chat_wires.new_session),
            sidebar_session_query: signals.sidebar.sidebar_session_query,
            global_message_query: signals.sidebar.global_message_query,
            sidebar_search_panel_open: signals.sidebar.sidebar_search_panel_open,
            sidebar_rail_ctx_menu: signals.sidebar.sidebar_rail_ctx_menu,
            chat_find_panel_open: signals.chat_composer.chat_find_panel_open,
            session_context_menu: signals.sidebar.session_context_menu,
            sidebar_rail_collapsed: signals.sidebar.sidebar_rail_collapsed,
            settings_page: signals.modal.settings_page,
        }
    }

    pub(crate) fn side_column_view_signals(&self) -> SideColumnViewSignals {
        let signals = &self.signals;
        SideColumnViewSignals {
            locale: signals.shell_ui.locale,
            side_resize_dragging: signals.resize.side_resize_dragging,
            side_panel_view: signals.shell_ui.side_panel_view,
            side_width: signals.shell_ui.side_width,
            side_resize_session: Rc::clone(&signals.resize.side_resize_session),
            side_resize_handles: Rc::clone(&signals.resize.side_resize_handles),
            view_menu_open: signals.shell_ui.view_menu_open,
            settings_page: signals.modal.settings_page,
            workspace_panel: signals.to_workspace_panel(),
            status_tasks: signals.to_status_tasks(),
            refresh_workspace: Arc::clone(&self.wiring.refresh_workspace),
            refresh_tasks: Arc::clone(&self.wiring.refresh_tasks),
            toggle_task: Arc::clone(&self.wiring.toggle_task),
            changelist_modal_open: signals.modal.changelist_modal_open,
            changelist_fetch_nonce: signals.modal.changelist_fetch_nonce,
            insert_workspace_file_ref: self.wiring.insert_workspace_file_ref,
            thinking_trace_log: signals.approval.thinking_trace_log,
            is_narrow_viewport: signals.shell_ui.is_narrow_viewport,
            ide_open_file_nonce: signals.shell_ui.ide_open_file_nonce,
            ide_open_file_path: signals.shell_ui.ide_open_file_path,
            chat: signals.chat,
            status_bar: self.status_bar_footer_signals(),
        }
    }

    pub(crate) fn status_bar_footer_signals(&self) -> StatusBarFooterSignals {
        let signals = &self.signals;
        StatusBarFooterSignals {
            status_tasks: signals.to_status_tasks(),
            status_err: signals.stream.status_err,
            stream_busy_memos: self.chat_column.stream_busy_memos,
            client_llm_storage_tick: signals.llm_settings.client_llm_storage_tick,
            selected_agent_role: signals.llm_settings.selected_agent_role,
            agent_role_user_override: signals.llm_settings.agent_role_user_override,
            selected_session_mode: signals.llm_settings.selected_session_mode,
            session_mode_user_override: signals.llm_settings.session_mode_user_override,
            llm_thinking_mode: signals.llm_settings.llm_thinking_mode_draft,
            refresh_status: Arc::clone(&self.wiring.refresh_status),
            user_prefs_reload_nonce: signals.workspace.user_prefs_reload_nonce,
            user_prefs_sync_phase: signals.workspace.user_prefs_sync_phase,
            user_prefs_load_err: signals.workspace.user_prefs_load_err,
            user_prefs_save_err: signals.workspace.user_prefs_save_err,
            settings_page: signals.modal.settings_page,
            conversation_hydration_err: signals.chat.conversation_hydration_err,
            context_usage_page: signals.modal.context_usage_page,
        }
    }

    pub(crate) fn mobile_shell_header_signals(&self) -> MobileShellHeaderSignals {
        let signals = &self.signals;
        MobileShellHeaderSignals {
            locale: signals.shell_ui.locale,
            editor_layout_mode: signals.shell_ui.editor_layout_mode,
            is_narrow_viewport: signals.shell_ui.is_narrow_viewport,
            ide_menu_bar_bridge: signals.shell_ui.ide_menu_bar_bridge,
            workspace_pick: self.workspace_pick(),
            ide_menubar_dropdown_open: signals.shell_ui.ide_menubar_dropdown_open,
            side_panel_view: signals.shell_ui.side_panel_view,
            status_tasks: signals.to_status_tasks(),
            status_err: signals.stream.status_err,
            stream_busy_memos: self.chat_column.stream_busy_memos,
        }
    }

    /// 设置页全屏视图：`settings_page` 开关 + 表单信号（阶段 B：单行传入 `SettingsPageView`）。
    pub(crate) fn settings_page_view_input(&self) -> SettingsPageViewInput {
        SettingsPageViewInput {
            settings_page: self.signals.modal.settings_page,
            form: self.settings_page_form_signals(),
            status_data: self.signals.status.status_data,
            refresh_status: Arc::clone(&self.wiring.refresh_status),
        }
    }
}
