//! 聊天 / 变更集 innerHTML 里代码块 Copy 按钮的点击处理。

use wasm_bindgen::JsCast;

use crate::i18n::Locale;
use crate::session_ops::write_clipboard_text;

/// 若点击落在 `[data-md-copy-code]` 上则复制对应 `pre` 文本并消费事件。
pub fn try_copy_md_code_block(ev: &web_sys::MouseEvent, locale: Locale) -> bool {
    let Some(target) = ev.target() else {
        return false;
    };
    let Some(el) = target.dyn_ref::<web_sys::Element>().cloned().or_else(|| {
        target
            .dyn_ref::<web_sys::Node>()
            .and_then(|n| n.parent_element())
    }) else {
        return false;
    };
    let Ok(Some(btn)) = el.closest("[data-md-copy-code]") else {
        return false;
    };
    ev.prevent_default();
    ev.stop_propagation();
    let text = btn
        .closest(".md-code-block")
        .ok()
        .flatten()
        .and_then(|block| block.query_selector("pre").ok().flatten())
        .and_then(|pre| pre.text_content())
        .unwrap_or_default();
    write_clipboard_text(&text, locale);
    true
}
