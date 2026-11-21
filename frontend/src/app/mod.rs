//! 主界面：单根 `App`（导航、对话、侧栏、状态栏、模态框与偏好接线）。
//!
//! 首启会话加载、`/user-data` / DOM 偏好同步、全局 `Escape` 等壳级副作用见 `app_shell_effects`。聊天主路径（滚动、查找、输入/流式）见 `chat` 子模块；Workspace / 变更集、`/status` 与任务等接线已部分迁至 **`wire_workspace_domain`**、**`status_tasks_wiring`**、**`chat::wire_chat_session_lifecycle`**（阶段 B）。

mod app_body;
mod app_bootstrap_phase;
mod app_shell_bootstrap;
mod app_shell_ctx;
pub(crate) mod app_shell_effects;
mod app_shell_init;
mod app_shell_wire_phases;
pub(crate) mod app_signals;
mod approval_modal;
mod changelist_modal;
mod chat;
pub(crate) use chat::turn_lifecycle;
mod connection_feed;
mod context_usage_page;
pub(crate) mod focusable_menu;
mod github_embed_page;
mod github_wiring;
mod ide_confirm_dialog;
mod ide_editor_body;
mod ide_editor_pane;
mod ide_find_bar;
mod ide_layout_switch;
mod ide_menu_bar;
mod ide_new_file_modal;
mod ide_session;
mod ide_settings_page;
mod ide_tabs_bar;
mod layout_toggle_hotkey;
mod mobile_shell_header;
mod nav_rail_connections;
mod session_list_modal;
pub(crate) mod session_mode_defaults;
mod settings_close_guard;
mod settings_commit;
mod settings_form_state;
mod settings_github_block;
mod settings_mcp_block;
mod settings_mcp_block_toolbar;
mod settings_mcp_json_import;
mod settings_mcp_server_row;
mod settings_mcp_server_row_actions;
mod settings_mcp_status;
mod settings_mcp_tools_list;
mod settings_models_registry;
mod settings_page;
mod settings_permission_block;
mod settings_sections;
mod settings_toggle_switch;
mod shell_confirm_dialog;
pub(crate) mod shell_prefs_storage;
mod shell_runtime_context;
mod side_column;
mod side_column_toolbar;
mod side_column_workspace_scroll;
mod sidebar_nav;
mod status_agent_role_menu;
mod status_bar;
mod status_context;
mod status_fetch_state;
mod status_permission_chip;
mod status_session_mode_seg;
mod status_tasks_state;
mod status_tasks_wiring;
mod status_view_chip;
mod tauri_window_controls;
mod web_api_bearer_recovery;
mod wire_workspace_domain;
mod workspace_browser_pick_modal;
mod workspace_clone_modal;
mod workspace_panel;
pub(crate) mod workspace_panel_state;
mod workspace_project_modal;
mod workspace_project_modal_body;
mod workspace_project_modal_parts;
mod workspace_root_actions;

use crate::i18n::{self, Locale};
use crate::icon::icon_chevron_right;
use app_body::app_body;
use app_shell_init::init_app_shell;

use leptos::prelude::*;

/// 根容器类名：随侧栏折叠 / 贴边变化（集中计算，减少 App 内 reactive 闭包数）。
///
/// 编辑器不再挂在根容器上：它只替换聊天列主区（与 TUI `/editor` 同一位置口径），
/// 侧栏与右列照旧，所以没有 `app-root--ide-layout` 这一档，也没有「切换视图时抑制
/// 侧栏宽度过渡」的 `sidebar-rail-snap` 了。
fn app_root_class_name(collapsed: bool) -> String {
    let mut c = String::from("app-root app-shell-ds");
    if collapsed {
        c.push_str(" sidebar-rail-collapsed");
    }
    c
}

/// 主区是否被全屏页遮挡：设置 / IDE 设置 / 上下文用量三页任一打开即隐藏（集中计算，减少 App 内 reactive 闭包数）。
fn shell_main_hidden(
    settings_page: RwSignal<bool>,
    ide_settings_page: RwSignal<bool>,
    context_usage_page: RwSignal<bool>,
) -> bool {
    settings_page.get() || ide_settings_page.get() || context_usage_page.get()
}

#[component]
fn SidebarRailRevealBtn(
    sidebar_rail_collapsed: RwSignal<bool>,
    locale: RwSignal<Locale>,
) -> impl IntoView {
    view! {
        <Show when=move || sidebar_rail_collapsed.get()>
            <button
                type="button"
                class="btn btn-secondary sidebar-rail-reveal-btn"
                prop:aria-label=move || i18n::nav_sidebar_expand_aria(locale.get())
                on:click=move |_| sidebar_rail_collapsed.set(false)
            >
                {icon_chevron_right("")}
            </button>
        </Show>
    }
}

#[component]
pub fn App() -> impl IntoView {
    let app_ctx = init_app_shell();
    // `app_ctx` 只用来「取一次当前 lane」：视图捕获的是 `Rc<LaneRuntime>` 与其中的 Copy
    // 信号句柄，**不持有** ctx，所以它随本函数返回被 drop 是安全的（N=1 时没有第二个读者）。
    // **S2 的 `add_lane` 必须让 `App` 长期持有 `app_ctx`**，否则事件回调里拿不到 `LaneSet`。
    // #561 S1b-3：`App` 只做「取当前前台那条 lane 的运行时 → 交给视图主体」。视图与
    // `provide_context` 都搬进了 `app_body`，于是 S2 用 `mount_to` 挂第二条 lane 时 owner
    // 就是新子树的，ctx 不会串到第一条（§5.2「已知风险」）。
    let rt = app_ctx.current_runtime();
    app_body(rt)
}
