//! `Ctrl+T` 切换主区（对话 ⇄ 编辑器）：与 TUI 的 `Ctrl+T`（`TopHotkey::ToggleView`）同键。
//!
//! 顶栏的布局按钮已删（与 TUI 一样只剩状态栈里的 `view` chip），而右列在窄屏 / rail 收起时
//! 是视觉隐藏的 —— 快捷键是那时不必开抽屉的入口。TUI 也是这两个入口并存。
//!
//! 浏览器标签页里 `Ctrl+T` 是「新标签」且不归页面管（Chrome 不接受页面拦这类组合键）；
//! 桌面壳与 Android WebView 没有这层浏览器官能，`preventDefault` 生效。所以这里不假装
//! 两端一样：纯浏览器下它只是多按了一次浏览器快捷键，`view` chip 仍然可用。

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use super::ide_layout_switch::{IdeLayoutToggleSignals, toggle_editor_layout};

/// 是否 `Ctrl+T`：排除 `Shift` / `Alt` / `Meta` 与输入法组合中，避免与相邻组合键撞车。
fn is_toggle_view_key(ev: &web_sys::KeyboardEvent) -> bool {
    ev.ctrl_key()
        && !ev.alt_key()
        && !ev.meta_key()
        && !ev.shift_key()
        && !ev.is_composing()
        && ev.key().eq_ignore_ascii_case("t")
}

/// 装 window 级 `keydown` 监听。
///
/// 监听随页面存活（`forget`），与壳层其它全局监听同一处置：它绑的是壳层级信号，
/// 没有「某个组件卸载就该摘掉」的语义。
pub fn wire_layout_toggle_hotkey(layout_toggle: IdeLayoutToggleSignals) {
    let Some(win) = web_sys::window() else {
        return;
    };
    let cb =
        Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(move |ev: web_sys::KeyboardEvent| {
            if !is_toggle_view_key(&ev) {
                return;
            }
            ev.prevent_default();
            toggle_editor_layout(layout_toggle);
        });
    let _ = win.add_event_listener_with_callback("keydown", cb.as_ref().unchecked_ref());
    cb.forget();
}
