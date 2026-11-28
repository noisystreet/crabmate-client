//! 对话 / 编辑器主区切换：翻一个 `editor_layout_mode`，没有别的副作用。
//!
//! 侧栏**不参与**切换：编辑器只替换聊天列主区（`layout-chat.css` 的 `.chat-body`），
//! 会话侧栏与右列一直可见可操作。所以这里既不该在进入时「记住」侧栏状态，也不该在退出时
//! 「恢复」它 —— 那会把用户在编辑器视图里做的展开/收起悄悄回退掉。旧整屏 IDE 布局才需要
//! 这对动作，因为那时侧栏是被 CSS 压成 0 宽隐藏的（见 `0.8.0-alpha.2` 的
//! `app-root--ide-layout` 规则）。

use leptos::prelude::*;

/// 主区切换信号（同源 `AppSignals::shell_ui.editor_layout_mode`）。
#[derive(Clone, Copy)]
pub struct IdeLayoutToggleSignals {
    pub editor_layout_mode: RwSignal<bool>,
}

impl IdeLayoutToggleSignals {
    pub fn from_app_signals(app: &crate::app::app_signals::AppSignals) -> Self {
        Self {
            editor_layout_mode: app.shell_ui.editor_layout_mode,
        }
    }
}

/// 退出编辑器视图：只翻标志位，侧栏 / 右列 / composer 都没被动过。
pub fn exit_editor_layout(s: IdeLayoutToggleSignals) {
    s.editor_layout_mode.set(false);
}

pub fn enter_editor_layout(s: IdeLayoutToggleSignals) {
    s.editor_layout_mode.set(true);
}

pub fn toggle_editor_layout(s: IdeLayoutToggleSignals) {
    if s.editor_layout_mode.get_untracked() {
        exit_editor_layout(s);
    } else {
        enter_editor_layout(s);
    }
}
