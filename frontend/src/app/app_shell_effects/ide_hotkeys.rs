//! IDE 布局全局快捷键：保存、查找、跳转行、关闭标签等。

use leptos::prelude::*;
use leptos_dom::helpers::window_event_listener;

use crate::app::app_signals::IdeChromeSignals;
use crate::app::app_signals::ShellUISignals;

#[derive(Clone, Copy)]
pub struct IdeEditorHotkeySignals {
    pub shell_ui: ShellUISignals,
    pub chrome: IdeChromeSignals,
    pub ide_settings_page: RwSignal<bool>,
}

fn handle_meta_save(shell_ui: ShellUISignals, ev: &web_sys::KeyboardEvent) -> bool {
    if !ev.key().eq_ignore_ascii_case("s") {
        return false;
    }
    ev.prevent_default();
    if ev.shift_key() {
        shell_ui
            .ide_save_all_nonce
            .update(|n| *n = n.saturating_add(1));
    } else {
        shell_ui
            .ide_save_active_nonce
            .update(|n| *n = n.saturating_add(1));
    }
    true
}

fn handle_meta_find(chrome: IdeChromeSignals, ev: &web_sys::KeyboardEvent) -> bool {
    if !ev.key().eq_ignore_ascii_case("f") || ev.shift_key() {
        return false;
    }
    ev.prevent_default();
    chrome.goto_panel_open.set(false);
    chrome.find_panel_open.set(true);
    true
}

fn handle_meta_goto(chrome: IdeChromeSignals, ev: &web_sys::KeyboardEvent) -> bool {
    if !ev.key().eq_ignore_ascii_case("g") || ev.shift_key() {
        return false;
    }
    ev.prevent_default();
    chrome.find_panel_open.set(false);
    chrome.goto_panel_open.set(true);
    true
}

fn handle_meta_close_tab(chrome: IdeChromeSignals, ev: &web_sys::KeyboardEvent) -> bool {
    if !ev.key().eq_ignore_ascii_case("w") || ev.shift_key() {
        return false;
    }
    ev.prevent_default();
    chrome
        .close_active_tab_nonce
        .update(|n| *n = n.saturating_add(1));
    true
}

/// IDE 快捷键生效条件：编辑器布局可见、不在设置页、无模态/新建文件弹窗、按下 Meta/Ctrl。
fn ide_hotkey_allowed(
    shell_ui: ShellUISignals,
    chrome: IdeChromeSignals,
    ide_settings_page: RwSignal<bool>,
    ev: &web_sys::KeyboardEvent,
) -> bool {
    if !shell_ui.editor_layout_mode.get_untracked() {
        return false;
    }
    if ide_settings_page.get_untracked() {
        return false;
    }
    if chrome.confirm_pending.get_untracked().is_some()
        || chrome.new_file_modal_open.get_untracked()
    {
        return false;
    }
    if !(ev.meta_key() || ev.ctrl_key()) {
        return false;
    }
    true
}

/// 在编辑器布局可见时注册 IDE 快捷键（文本框内同样生效）。
pub fn wire_ide_editor_hotkeys(signals: IdeEditorHotkeySignals) {
    let IdeEditorHotkeySignals {
        shell_ui,
        chrome,
        ide_settings_page,
    } = signals;
    Effect::new(move |_| {
        let h = window_event_listener(leptos::ev::keydown, move |ev: web_sys::KeyboardEvent| {
            if !ide_hotkey_allowed(shell_ui, chrome, ide_settings_page, &ev) {
                return;
            }
            // 各 handler 按自身按键判定并短路消费；无匹配即返回 false，无需用 `||` 串联。
            handle_meta_save(shell_ui, &ev);
            handle_meta_find(chrome, &ev);
            handle_meta_goto(chrome, &ev);
            handle_meta_close_tab(chrome, &ev);
        });
        on_cleanup(move || h.remove());
    });
}
