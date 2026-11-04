//! 进入编辑器视图时收起壳层临时面板（侧栏偏好不参与切换，见
//! [`crate::app::ide_layout_switch`]；持久化见 [`crate::user_prefs_sync`]）。

use leptos::prelude::*;

pub fn wire_close_shell_chrome_when_ide_layout(
    editor_layout_mode: RwSignal<bool>,
    mobile_nav_open: RwSignal<bool>,
    chat_find_panel_open: RwSignal<bool>,
) {
    Effect::new(move |_| {
        if !editor_layout_mode.get() {
            return;
        }
        mobile_nav_open.set(false);
        chat_find_panel_open.set(false);
    });
}
