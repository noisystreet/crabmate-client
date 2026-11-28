//! 一棵 lane 子树的视图主体（#561 **S1b-3**）。
//!
//! # 为什么是普通函数，而不是 `#[component]`
//!
//! S2 要用 `leptos::mount::mount_to(host, || app_body(rt))` 手工挂载新 lane，参数里有
//! `Rc<LaneRuntime>`。`#[component]` 的 props 走 `Attribute`，要求 `Send + Sync`，`Rc`
//! 过不了那道门 —— 就是 #561 §5.2 实测出来的那 9 处 `Rc<(dyn Fn)> cannot be sent between
//! threads safely`。普通函数不经过 props，`Rc` 随便传。
//!
//! # `provide_context` 为什么在这里，而不是留在 `App` 顶部
//!
//! 它按 **owner** 生效。`mount_to` 会为新子树建 owner，在它的闭包里调用本函数，ctx 就属于
//! 那条 lane 自己；反过来若留在 `App` 顶部，第二条 lane 会读到第一条的 ctx（§5.2「已知
//! 风险」）。N=1 时两者等价 —— 只有一条 lane，owner 就是 `App` 的。
//!
//! # 归属（S2 落地时已按这条口径分好）
//!
//! - `wire_layout_toggle_hotkey` **不在**这里：热键是窗口级的，`App` 只注册一次。放进本函数会
//!   让每条 lane 各注册一份全局监听，第二条 lane 建出来后一次按键就 toggle 两次。
//! - `IdeSession` 暂按 lane 建（它绑着这条 lane 的 chat 信号）。若确认编辑器状态该是窗口级
//!   （切 lane 不该丢编辑器标签），再上移 —— §5.2 要求这类对象逐个判定归属。

use std::rc::Rc;

use leptos::prelude::*;

use super::app_signals::LaneRuntime;
use super::approval_modal::ApprovalModal;
use super::changelist_modal::changelist_modal_view;
use super::chat::ChatFindBar;
use super::chat::chat_column_view;
use super::context_usage_page::ContextUsagePageView;
use super::ide_confirm_dialog::IdeConfirmDialog;
use super::ide_editor_body::IdeEditorBody;
use super::ide_layout_switch::IdeLayoutToggleSignals;
use super::ide_session::{IdeSession, IdeSessionInput};
use super::ide_settings_page::IdeSettingsPageView;
use super::session_list_modal::session_list_modal_view;
use super::settings::page::SettingsPageView;
use super::sidebar_nav::sidebar_nav_view;
use super::workspace::browser_pick_modal::workspace_browser_pick_modal_view;
use super::workspace::clone_modal::workspace_clone_modal_view;
use super::workspace::project_modal::workspace_project_modal_view;
use super::{SidebarRailRevealBtn, app_root_class_name, shell_main_hidden};
use crate::app::shell::confirm_dialog::ShellConfirmDialog;
use crate::app::shell::mobile_header::mobile_shell_header_view;
use crate::app::shell::runtime_context::ChatShellLeptosContext;
use crate::app::shell::side_column::side_column_view;

/// 一条 lane 的完整视图：侧栏 + 主区 + 各模态 + 全屏页。
///
/// `App` 现在传「当前前台那条」进来；S2 的 `mount_to` 传「刚建的那条」。
pub(crate) fn app_body(rt: Rc<LaneRuntime>) -> AnyView {
    // 闭包只捕获其中的 Copy 句柄：写成 `rt.signals.chat` 会让 `move ||` 捕获整个
    // `LaneRuntime`（含 `Rc`），当场被 `Send + Sync` 打回 —— 与 #563 记录的 `Deref` 坑同源。
    let signals = rt.signals.clone();
    provide_context(ChatShellLeptosContext::from_app_signals(&signals));

    // 入参全部由「这条 lane」生成（S1b：见 `app_signals/lane_view_inputs.rs`），不再经
    // `LaneSet::active` 指针 —— 同一份 `rt` 传给 14 个方法，靠的是 `Rc` 句柄不持借用。
    let approval_modal_signals = rt.approval_modal_signals();
    let settings_page_view_input = rt.settings_page_view_input();
    let ide_settings_page_view_input = rt.ide_settings_page_view_input();
    let context_usage_page_view_input = rt.context_usage_page_view_input();
    let mobile_shell_header_signals = rt.mobile_shell_header_signals();
    let changelist_modal_signals = rt.changelist_modal_signals();
    let session_list_modal_signals = rt.session_list_modal_signals();
    let workspace_project_modal_signals = rt.workspace_project_modal_signals();
    let workspace_clone_modal_signals = rt.workspace_clone_modal_signals();
    let workspace_browser_pick_modal_signals = rt.workspace_browser_pick_modal_signals();
    let sidebar_nav_signals = rt.sidebar_nav_signals();
    let side_column_view_signals = rt.side_column_view_signals();

    // IDE 会话（状态 + 副作用）在这里建一次：视图挂在聊天列主区里，切回聊天也不丢状态。
    let layout_toggle = IdeLayoutToggleSignals::from_app_signals(&signals);
    let ide_session = IdeSession::new(IdeSessionInput {
        locale: signals.shell_ui.locale,
        shell_ui: signals.shell_ui,
        chrome: signals.ide_chrome,
        editor: signals.ide_editor,
        layout_toggle,
        ide_settings_page: signals.modal.ide_settings_page,
        chat: signals.chat,
        composer_draft: signals.chat_composer.draft,
        workspace_panel: signals.to_workspace_panel(),
        refresh_workspace: rt.wiring.refresh_workspace.clone(),
        initialized: signals.initialized,
        editor_visible: signals.shell_ui.editor_layout_mode,
    });
    let ide_body_view: AnyView = view! { <IdeEditorBody signals=ide_session.body() /> }.into_any();

    view! {
        <div
            class=move || {
                app_root_class_name(signals.sidebar.sidebar_rail_collapsed.get())
            }
        >
            {sidebar_nav_view(sidebar_nav_signals)}

            <SidebarRailRevealBtn
                sidebar_rail_collapsed=signals.sidebar.sidebar_rail_collapsed
                locale=signals.shell_ui.locale
            />

            <div
                class="shell-main"
                id="layout-mode-panel-main"
                class:settings-page-hidden=move || {
                    shell_main_hidden(
                        signals.modal.settings_page,
                        signals.modal.ide_settings_page,
                        signals.modal.context_usage_page,
                    )
                }
            >
                {mobile_shell_header_view(mobile_shell_header_signals)}

                <Show when=move || signals.chat_composer.chat_find_panel_open.get()>
                    <ChatFindBar />
                </Show>

                <div
                    class:main-row-resizing=move || signals.resize.side_resize_dragging.get()
                    class="main-row"
                >
                    <div class="main-row-chat-layer">
                        {chat_column_view(rt.chat_column.clone(), ide_body_view)}
                        {side_column_view(side_column_view_signals.clone())}
                    </div>
                </div>
            </div>

            {session_list_modal_view(session_list_modal_signals)}

            {workspace_project_modal_view(workspace_project_modal_signals)}

            {workspace_clone_modal_view(workspace_clone_modal_signals)}

            {workspace_browser_pick_modal_view(workspace_browser_pick_modal_signals)}

            {changelist_modal_view(changelist_modal_signals)}

            {ide_session.modals_view()}

            <ApprovalModal signals=approval_modal_signals />

            <SettingsPageView input=settings_page_view_input />
            <IdeSettingsPageView input=ide_settings_page_view_input />
            <ContextUsagePageView input=context_usage_page_view_input />
            <IdeConfirmDialog
                locale=signals.shell_ui.locale
                chrome=signals.ide_chrome
            />
            <ShellConfirmDialog
                locale=signals.shell_ui.locale
                modal=signals.modal
            />
        </div>
    }
    .into_any()
}
