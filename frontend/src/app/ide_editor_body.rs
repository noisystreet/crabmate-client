//! IDE 编辑器体：查找 / 跳转条 + 标签栏 + CodeMirror 编辑区。
//!
//! 从 `ide_layout` 抽出。动机不是「文件太长」，而是**挂载位置变了**：编辑器只替换
//! 聊天主区（与 TUI `/editor` 视图同一位置口径），因此状态宿主与视图不能再住在同一个
//! 组件里 —— 视图必须是能被别处（聊天列的 body 区）挂载的独立组件。
//!
//! 信号全部是句柄（`Copy`）：宿主 [`super::ide_session::IdeSession`] 建一份、这里复制一份，
//! 状态只有一处，不会出现「两套 tabs」。

use leptos::prelude::*;

use crate::app::app_signals::{IdeChromeSignals, IdeEditorSignals};
use crate::i18n::Locale;
use crate::ide_codemirror::IdeEditorHost;
use crate::ide_confirm::IdeConfirmSignals;
use crate::ide_tabs::{IdeTabsEditorSignals, IdeTabsHandle};

use super::ide_editor_pane::IdeEditorPane;
use super::ide_find_bar::{IdeFindBar, IdeFindBarInput, IdeGotoLineBar};
use super::ide_tabs_bar::{IdeTabsBar, IdeTabsBarInput};

/// 编辑器体需要的信号束。
///
/// 字段全是信号句柄，`Copy` 不会复制状态。
#[derive(Clone, Copy)]
pub struct IdeEditorBodySignals {
    pub locale: RwSignal<Locale>,
    pub chrome: IdeChromeSignals,
    pub editor: IdeEditorSignals,
    pub tabs: IdeTabsHandle,
    pub confirm: IdeConfirmSignals,
    /// 编辑器视图是否可见：CodeMirror 的按需挂载与尺寸刷新都按它判定。
    pub editor_visible: RwSignal<bool>,
    pub tab_editor: IdeTabsEditorSignals,
    pub editor_host: IdeEditorHost,
}

#[component]
pub fn IdeEditorBody(signals: IdeEditorBodySignals) -> impl IntoView {
    let IdeEditorBodySignals {
        locale,
        chrome,
        editor,
        tabs,
        confirm,
        editor_visible,
        tab_editor,
        editor_host,
    } = signals;
    let find_input = IdeFindBarInput {
        locale,
        chrome,
        ide_text: tab_editor.ide_text,
        editor_host,
    };
    // 加载中 / 错误这两条横幅与标签栏同源，都读 `tabs` 上的那对信号。
    let ide_err = tabs.err;
    let ide_load_busy = tabs.load_busy;
    view! {
        <div class="ide-editor-body" data-testid="ide-editor-body">
            <IdeFindBar input=find_input />
            <IdeGotoLineBar input=find_input />
            <Show when=move || ide_err.get().is_some()>
                <div class="msg-error ide-editor-err">
                    {move || ide_err.get().unwrap_or_default()}
                </div>
            </Show>
            <Show when=move || ide_load_busy.get()>
                <p class="ide-editor-loading" role="status">
                    "…"
                </p>
            </Show>
            <IdeTabsBar input=IdeTabsBarInput {
                locale,
                tabs,
                confirm,
                editor: tab_editor,
            } />
            <IdeEditorPane
                locale=locale
                editor=editor
                host=editor_host
                editor_visible=editor_visible
                ide_path=tab_editor.ide_path
                ide_text=tab_editor.ide_text
                ide_load_busy=ide_load_busy
            />
        </div>
    }
}
