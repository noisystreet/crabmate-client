//! IDE 会话：编辑器状态与副作用的**唯一**宿主。
//!
//! 为什么不再是一个「布局组件」：编辑器只替换聊天列的主区（与 TUI `/editor` 视图同一个
//! 位置口径），视图因此挂在聊天列里。状态若跟着视图走，切回聊天一卸载就会连已打开的标签、
//! 未保存的缓冲与 CodeMirror 句柄一起丢掉。所以三件事分开：
//!
//! - 本模块：建状态、装副作用（刷新工作区 / 保存 / 磁盘同步 / 打开文件 / 关标签 / 菜单桥）；
//! - [`super::ide_editor_body::IdeEditorBody`]：只画，信号束由 [`IdeSession::body`] 给出；
//! - 顶栏菜单：只读 `IdeMenuBarBridge`（`wire_ide_menu_bar_bridge` 在本模块挂）。
//!
//! 两侧拿到的都是信号句柄（`Copy`）：状态只有一份，不会长出第二套 tabs。

use std::sync::Arc;

use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::app::app_signals::{IdeChromeSignals, IdeEditorSignals, ShellUISignals};
use crate::app::workspace_root_actions::WorkspaceRootPickHandle;
use crate::chat_session_state::ChatSessionSignals;
use crate::i18n::Locale;
use crate::ide_codemirror::IdeEditorHost;
use crate::ide_disk_sync::spawn_sync_ide_tabs_from_disk;
use crate::ide_save::{IdeSaveContext, spawn_save_active_tab, spawn_save_all_dirty_tabs};
use crate::ide_tabs::{
    IdeTabsEditorSignals, IdeTabsHandle, make_ide_open_file_handler, spawn_close_active_tab,
    wire_ide_editor_sync_to_active_tab,
};

use super::ide_editor_body::IdeEditorBodySignals;
use super::ide_layout_switch::{IdeLayoutToggleSignals, enter_editor_layout};
use super::ide_menu_bar::{IdeMenuBarSignals, wire_ide_menu_bar_bridge};
use super::ide_new_file_modal::{IdeNewFileModal, IdeNewFileModalInput};
use super::workspace_panel_state::WorkspacePanelSignals;

/// 建会话所需的输入（`Arc` 等非 `Copy` 项只在这里出现一次）。
#[derive(Clone)]
pub struct IdeSessionInput {
    pub locale: RwSignal<Locale>,
    pub shell_ui: ShellUISignals,
    pub chrome: IdeChromeSignals,
    pub editor: IdeEditorSignals,
    pub layout_toggle: IdeLayoutToggleSignals,
    pub ide_settings_page: RwSignal<bool>,
    pub chat: ChatSessionSignals,
    pub composer_draft: RwSignal<String>,
    pub workspace_panel: WorkspacePanelSignals,
    pub refresh_workspace: Arc<dyn Fn() + Send + Sync>,
    pub initialized: RwSignal<bool>,
    /// `shell_ui.editor_layout_mode`：主区当前是不是编辑器。
    pub editor_visible: RwSignal<bool>,
}

/// 建好的会话。字段全是句柄，`Copy` 不复制状态。
#[derive(Clone, Copy)]
pub struct IdeSession {
    body: IdeEditorBodySignals,
    save_ctx: IdeSaveContext,
    locale: RwSignal<Locale>,
    chrome: IdeChromeSignals,
    workspace_panel: WorkspacePanelSignals,
}

impl IdeSession {
    pub fn new(input: IdeSessionInput) -> Self {
        let IdeSessionInput {
            locale,
            shell_ui,
            chrome,
            editor,
            layout_toggle,
            ide_settings_page,
            chat,
            composer_draft,
            workspace_panel,
            refresh_workspace,
            initialized,
            editor_visible,
        } = input;

        let tabs = IdeTabsHandle::new();
        let ide_path = RwSignal::new(None::<String>);
        let ide_text = RwSignal::new(String::new());
        let ide_baseline = RwSignal::new(String::new());
        let editor_host = IdeEditorHost::new();
        let confirm = chrome.confirm_signals();

        wire_ide_editor_sync_to_active_tab(tabs, tabs.active, ide_text);

        let tab_editor = IdeTabsEditorSignals {
            ide_path,
            ide_text,
            ide_baseline,
        };
        let open_file = make_ide_open_file_handler(locale, tabs, tab_editor);
        let open_sv = StoredValue::new(open_file);

        let save_ctx = IdeSaveContext {
            tabs,
            ide_path,
            ide_text,
            ide_baseline,
            ide_err: tabs.err,
        };

        wire_session_effects(SessionEffectsInput {
            editor_visible,
            initialized,
            refresh_workspace: refresh_workspace.clone(),
            shell_ui,
            chrome,
            save_ctx,
            tab_editor,
            open_sv,
            layout_toggle,
            locale,
            tabs,
        });

        let menu_signals = IdeMenuBarSignals {
            locale,
            chrome,
            editor,
            layout_toggle,
            ide_settings_page,
            ide_menubar_dropdown_open: shell_ui.ide_menubar_dropdown_open,
            ide_path,
            ide_text,
            ide_baseline,
            ide_load_busy: tabs.load_busy,
            ide_save_busy: tabs.save_busy,
            editor_host,
            tabs,
            save_ctx,
            workspace_pick: WorkspaceRootPickHandle {
                locale,
                chat,
                composer_draft,
                ws: workspace_panel,
                side_panel_view: shell_ui.side_panel_view,
            },
        };
        wire_ide_menu_bar_bridge(shell_ui.ide_menu_bar_bridge, editor_visible, menu_signals);

        Self {
            body: IdeEditorBodySignals {
                locale,
                chrome,
                editor,
                tabs,
                confirm,
                editor_visible,
                tab_editor,
                editor_host,
            },
            save_ctx,
            locale,
            chrome,
            workspace_panel,
        }
    }

    /// 编辑器体视图的信号束：挂进聊天列主区。
    #[must_use]
    pub fn body(&self) -> IdeEditorBodySignals {
        self.body
    }

    /// 宿主自带的模态（新建文件）。常驻挂载，可见性由 `chrome.new_file_modal_open` 决定。
    ///
    /// `use<>` 是必要的：字段全是 `Copy` 句柄，视图不该借用 `&self`（否则 `App` 里
    /// 这个绑定会被要求活到 `'static`）。
    pub fn modals_view(&self) -> impl IntoView + use<> {
        view! {
            <IdeNewFileModal input=IdeNewFileModalInput {
                locale: self.locale,
                chrome: self.chrome,
                save_ctx: self.save_ctx,
                workspace_panel: self.workspace_panel,
            } />
        }
    }
}

/// 副作用所需的信号（从 [`IdeSession::new`] 拆出，避开 `fn-nloc` 与 CCN 门禁）。
struct SessionEffectsInput {
    editor_visible: RwSignal<bool>,
    initialized: RwSignal<bool>,
    refresh_workspace: Arc<dyn Fn() + Send + Sync>,
    shell_ui: ShellUISignals,
    chrome: IdeChromeSignals,
    save_ctx: IdeSaveContext,
    tab_editor: IdeTabsEditorSignals,
    open_sv: StoredValue<Arc<dyn Fn(String) + Send + Sync>>,
    layout_toggle: IdeLayoutToggleSignals,
    locale: RwSignal<Locale>,
    tabs: IdeTabsHandle,
}

/// 装全部副作用。**每条都以 `editor_visible` 作门闩**：主区在聊天时不该为编辑器做后台活。
fn wire_session_effects(input: SessionEffectsInput) {
    let SessionEffectsInput {
        editor_visible,
        initialized,
        refresh_workspace,
        shell_ui,
        chrome,
        save_ctx,
        tab_editor,
        open_sv,
        layout_toggle,
        locale,
        tabs,
    } = input;

    Effect::new(move |_| {
        if !editor_visible.get() {
            return;
        }
        if initialized.get() {
            refresh_workspace();
        }
    });

    Effect::new(move |_| {
        if !editor_visible.get() {
            return;
        }
        let _ = shell_ui.ide_save_active_nonce.get();
        spawn_save_active_tab(save_ctx, locale);
    });

    Effect::new(move |_| {
        if !editor_visible.get() {
            return;
        }
        let _ = shell_ui.ide_save_all_nonce.get();
        spawn_save_all_dirty_tabs(save_ctx, locale);
    });

    Effect::new(move |_| {
        if !editor_visible.get() {
            return;
        }
        let _ = shell_ui.ide_sync_disk_nonce.get();
        spawn_sync_ide_tabs_from_disk(tabs, tab_editor, locale, chrome.confirm_signals());
    });

    // 工具卡「打开此文件」：nonce 递增时打开配对的工作区相对文件（消费即清空路径，
    // 防止下次切进编辑器时重放旧目标）。点击发生在聊天布局，故不能以 `editor_visible`
    // 作门闩 —— 聊天布局先切进编辑器再打开。
    Effect::new(move |_| {
        let _ = shell_ui.ide_open_file_nonce.get();
        let Some(path) = shell_ui.ide_open_file_path.get_untracked() else {
            return;
        };
        shell_ui.ide_open_file_path.set(None);
        if !editor_visible.get_untracked() {
            enter_editor_layout(layout_toggle);
        }
        spawn_local(async move {
            (open_sv.get_value())(path);
        });
    });

    Effect::new(move |_| {
        if !editor_visible.get() {
            return;
        }
        let _ = chrome.close_active_tab_nonce.get();
        spawn_close_active_tab(tabs, locale, tab_editor, chrome.confirm_signals());
    });
}
