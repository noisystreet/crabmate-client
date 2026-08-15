//! 无障碍辅助：模态内焦点、Tab 循环、菜单方向键。

use gloo_timers::future::TimeoutFuture;
use leptos::ev::MouseEvent;
use leptos::task::spawn_local;
use wasm_bindgen::JsCast;

const TABBABLE_SELECTOR: &str = "button:not([disabled]), [href], input:not([disabled]):not([type='hidden']), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex='-1'])";
const MENU_ITEM_SELECTOR: &str = "button:not([disabled])[role='menuitem'], button:not([disabled])[role='menuitemradio'], button:not([disabled])[role='menuitemcheckbox']";

/// 是否为「直接点在遮罩层上」（非对话框面板或子节点）。
#[must_use]
pub fn mouse_event_target_is_current_target(ev: &MouseEvent) -> bool {
    let Some(target) = ev.target() else {
        return false;
    };
    let Some(current) = ev.current_target() else {
        return false;
    };
    let Ok(t) = target.dyn_into::<web_sys::Element>() else {
        return false;
    };
    let Ok(c) = current.dyn_into::<web_sys::Element>() else {
        return false;
    };
    t == c
}

/// `Shift+F10` 或 `ContextMenu`：打开自定义上下文菜单。
#[must_use]
pub fn is_context_menu_open_key(key: &str, shift: bool) -> bool {
    key == "ContextMenu" || (shift && key == "F10")
}

/// `Shift+F10` / `ContextMenu`：返回锚点坐标并 `preventDefault`。
#[must_use]
pub fn context_menu_keydown_anchor(ev: &web_sys::KeyboardEvent) -> Option<(i32, i32)> {
    if !is_context_menu_open_key(&ev.key(), ev.shift_key()) {
        return None;
    }
    ev.prevent_default();
    let target = ev.current_target()?;
    let el = target.dyn_into::<web_sys::Element>().ok()?;
    let r = el.get_bounding_client_rect();
    Some((r.left() as i32, r.bottom() as i32))
}

/// 环形索引（空切片时返回 0）。
#[must_use]
pub fn wrapping_index(len: usize, current: usize, delta: i32) -> usize {
    if len == 0 {
        return 0;
    }
    let n = i32::try_from(len).unwrap_or(1);
    let cur = i32::try_from(current.min(len.saturating_sub(1))).unwrap_or(0);
    usize::try_from((cur + delta).rem_euclid(n)).unwrap_or(0)
}

/// 标签栏方向键 / Home / End 的下一活动下标。
#[must_use]
pub fn tablist_index_after_key(len: usize, active: Option<usize>, key: &str) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let cur = active.unwrap_or(0).min(len - 1);
    match key {
        "ArrowRight" | "ArrowDown" => Some(wrapping_index(len, cur, 1)),
        "ArrowLeft" | "ArrowUp" => Some(wrapping_index(len, cur, -1)),
        "Home" => Some(0),
        "End" => Some(len - 1),
        _ => None,
    }
}

/// 当前项用 `aria-current="true"`，否则 `"false"`（等价未设置）。
#[must_use]
pub fn aria_current_true_or_false(active: bool) -> &'static str {
    if active { "true" } else { "false" }
}

/// 在容器内聚焦第一个可 Tab 停驻的子节点（若无则聚焦容器自身）。
pub fn focus_first_in_modal_container(container: &web_sys::Element) {
    let els = collect_matching_html_elements(container, TABBABLE_SELECTOR);
    if let Some(el) = els.first() {
        let _ = el.focus();
        return;
    }
    let _ = container
        .dyn_ref::<web_sys::HtmlElement>()
        .map(|h| h.focus());
}

/// 聚焦菜单内第一项（若无则聚焦容器）。
pub fn focus_first_menu_item(container: &web_sys::Element) {
    let els = collect_matching_html_elements(container, MENU_ITEM_SELECTOR);
    if let Some(el) = els.first() {
        let _ = el.focus();
        return;
    }
    focus_first_in_modal_container(container);
}

/// 菜单刚打开时把焦点移到第一项（下一帧，等 DOM 挂上）。
pub fn schedule_focus_first_menu_item(container: &web_sys::Element) {
    let container = container.clone();
    spawn_local(async move {
        TimeoutFuture::new(0).await;
        focus_first_menu_item(&container);
    });
}

/// 对话框刚打开时把焦点移到第一个可 Tab 停驻节点（下一帧，等 DOM 挂上）。
pub fn schedule_focus_first_in_modal(container: &web_sys::Element) {
    let container = container.clone();
    spawn_local(async move {
        TimeoutFuture::new(0).await;
        focus_first_in_modal_container(&container);
    });
}

fn is_hidden_from_a11y_tree(el: &web_sys::HtmlElement) -> bool {
    let mut cur: Option<web_sys::Element> = Some(el.clone().unchecked_into());
    while let Some(node) = cur {
        if node.tag_name().eq_ignore_ascii_case("BODY") {
            break;
        }
        if node.get_attribute("aria-hidden").as_deref() == Some("true") {
            return true;
        }
        cur = node.parent_element();
    }
    false
}

fn collect_matching_html_elements(
    container: &web_sys::Element,
    selector: &str,
) -> Vec<web_sys::HtmlElement> {
    let Ok(list) = container.query_selector_all(selector) else {
        return Vec::new();
    };
    let mut els: Vec<web_sys::HtmlElement> = Vec::new();
    let n = list.length();
    for i in 0..n {
        if let Some(node) = list.item(i)
            && let Ok(el) = node.dyn_into::<web_sys::HtmlElement>()
            && !is_hidden_from_a11y_tree(&el)
        {
            els.push(el);
        }
    }
    els
}

fn active_tabbable_index(
    els: &[web_sys::HtmlElement],
    active_el: &web_sys::HtmlElement,
) -> Option<usize> {
    els.iter()
        .position(|el| active_el.is_same_node(Some(el.as_ref())))
}

fn focus_next_in_tab_trap(els: &[web_sys::HtmlElement], idx_opt: Option<usize>, shift: bool) {
    match idx_opt {
        None => {
            let _ = els[0].focus();
        }
        Some(idx) if !shift && idx + 1 < els.len() => {
            let _ = els[idx + 1].focus();
        }
        Some(_) if !shift => {
            let _ = els[0].focus();
        }
        Some(idx) if idx > 0 => {
            let _ = els[idx - 1].focus();
        }
        Some(_) => {
            let _ = els.last().expect("non-empty").focus();
        }
    }
}

/// `Tab` / `Shift+Tab` 时将焦点限制在 `container` 内。
pub fn trap_tab_in_container(ev: &web_sys::KeyboardEvent, container: &web_sys::Element) {
    if ev.key() != "Tab" {
        return;
    }
    let els = collect_matching_html_elements(container, TABBABLE_SELECTOR);
    if els.is_empty() {
        return;
    }

    let doc = leptos_dom::helpers::document();
    let Some(active) = doc.active_element() else {
        return;
    };
    let Ok(active_el) = active.dyn_into::<web_sys::HtmlElement>() else {
        return;
    };

    let idx_opt = active_tabbable_index(&els, &active_el);
    ev.prevent_default();
    focus_next_in_tab_trap(&els, idx_opt, ev.shift_key());
}

fn document_active_html_element() -> Option<web_sys::HtmlElement> {
    leptos_dom::helpers::document()
        .active_element()?
        .dyn_into::<web_sys::HtmlElement>()
        .ok()
}

fn focus_menu_item_at(els: &[web_sys::HtmlElement], idx: usize) {
    if let Some(el) = els.get(idx) {
        let _ = el.focus();
    }
}

fn move_menu_focus(els: &[web_sys::HtmlElement], delta: i32) {
    if els.is_empty() {
        return;
    }
    let active = document_active_html_element();
    let cur = active
        .as_ref()
        .and_then(|el| active_tabbable_index(els, el))
        .unwrap_or(0);
    focus_menu_item_at(els, wrapping_index(els.len(), cur, delta));
}

/// 菜单内 `ArrowUp`/`ArrowDown`/`Home`/`End` 移动焦点；`Tab` 仍循环陷阱。
pub fn handle_menu_keyboard(ev: &web_sys::KeyboardEvent, container: &web_sys::Element) {
    match ev.key().as_str() {
        "Tab" => trap_tab_in_container(ev, container),
        "ArrowDown" => {
            ev.prevent_default();
            move_menu_focus(
                &collect_matching_html_elements(container, MENU_ITEM_SELECTOR),
                1,
            );
        }
        "ArrowUp" => {
            ev.prevent_default();
            move_menu_focus(
                &collect_matching_html_elements(container, MENU_ITEM_SELECTOR),
                -1,
            );
        }
        "Home" => {
            ev.prevent_default();
            let els = collect_matching_html_elements(container, MENU_ITEM_SELECTOR);
            focus_menu_item_at(&els, 0);
        }
        "End" => {
            ev.prevent_default();
            let els = collect_matching_html_elements(container, MENU_ITEM_SELECTOR);
            if let Some(last) = els.len().checked_sub(1) {
                focus_menu_item_at(&els, last);
            }
        }
        _ => {}
    }
}

/// 对话框 `keydown`：Tab 陷阱；Escape 交给 `on_escape`（调用方决定 deny / 取消）。
pub fn handle_modal_layer_keydown(
    ev: &web_sys::KeyboardEvent,
    container: &web_sys::Element,
    on_escape: impl FnOnce(),
) {
    if ev.key() == "Escape" {
        ev.prevent_default();
        ev.stop_propagation();
        on_escape();
        return;
    }
    trap_tab_in_container(ev, container);
}

#[cfg(test)]
mod a11y_pure_tests {
    use super::{
        aria_current_true_or_false, is_context_menu_open_key, tablist_index_after_key,
        wrapping_index,
    };

    #[test]
    fn wrapping_index_steps_and_wraps() {
        assert_eq!(wrapping_index(3, 0, 1), 1);
        assert_eq!(wrapping_index(3, 2, 1), 0);
        assert_eq!(wrapping_index(3, 0, -1), 2);
        assert_eq!(wrapping_index(0, 0, 1), 0);
    }

    #[test]
    fn tablist_index_after_key_arrows_home_end() {
        assert_eq!(tablist_index_after_key(3, Some(1), "ArrowRight"), Some(2));
        assert_eq!(tablist_index_after_key(3, Some(2), "ArrowRight"), Some(0));
        assert_eq!(tablist_index_after_key(3, Some(0), "ArrowLeft"), Some(2));
        assert_eq!(tablist_index_after_key(4, Some(2), "Home"), Some(0));
        assert_eq!(tablist_index_after_key(4, Some(0), "End"), Some(3));
        assert_eq!(tablist_index_after_key(2, Some(0), "Enter"), None);
        assert_eq!(tablist_index_after_key(0, None, "ArrowRight"), None);
    }

    #[test]
    fn context_menu_open_key_shift_f10_or_contextmenu() {
        assert!(is_context_menu_open_key("ContextMenu", false));
        assert!(is_context_menu_open_key("F10", true));
        assert!(!is_context_menu_open_key("F10", false));
        assert!(!is_context_menu_open_key("Escape", true));
    }

    #[test]
    fn aria_current_true_or_false_matches_active() {
        assert_eq!(aria_current_true_or_false(true), "true");
        assert_eq!(aria_current_true_or_false(false), "false");
    }

    #[test]
    fn ide_tab_select_aria_includes_unsaved_and_pinned() {
        use crate::i18n::{self, Locale};
        let s = i18n::ide_tab_select_aria("main.rs", true, true, Locale::En);
        assert!(s.contains("main.rs"));
        assert!(s.contains("Unsaved"));
        assert!(s.contains("Pinned"));
    }
}
