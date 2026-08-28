//! 回合正文段（`.chat-tui-body--answer`）的 DOM 增量应用与隐藏状态管理。
//!
//! 从 `tui_stream_view` 拆出，控制该文件行数与函数形参数目（fn-param / fn-nloc ratchet）。

use wasm_bindgen::JsCast;

use super::tui_line_markdown::{
    open_active_block_class, open_block_is_fence_buffer, render_open_active_html,
};

/// 定位/创建正文段内的活跃行容器（`.chat-tui-line--active`）。
pub(super) fn ensure_open_block(body: &web_sys::HtmlElement) -> Option<web_sys::HtmlElement> {
    if let Some(existing) = body
        .query_selector(".chat-tui-line--plain, .chat-tui-line--active")
        .ok()
        .flatten()
        .and_then(|n| n.dyn_into::<web_sys::HtmlElement>().ok())
    {
        return Some(existing);
    }
    let document = body.owner_document()?;
    let block = document
        .create_element("div")
        .ok()?
        .dyn_into::<web_sys::HtmlElement>()
        .ok()?;
    block.set_class_name("chat-tui-line chat-tui-line--active");
    let _ = body.append_child(&block);
    Some(block)
}

/// 移除正文段内的活跃行（`.chat-tui-line--active` / `.chat-tui-line--plain`）。
pub(super) fn remove_open_block(body: &web_sys::HtmlElement) {
    if let Some(block) = body
        .query_selector(".chat-tui-line--plain, .chat-tui-line--active")
        .ok()
        .flatten()
    {
        block.remove();
    }
}

/// 将活跃行容器更新为给定源文本的实时渲染。
pub(super) fn apply_open_active_block(
    body: &web_sys::HtmlElement,
    text: &str,
    markdown_render: bool,
) -> bool {
    let Some(block) = ensure_open_block(body) else {
        return false;
    };
    block.set_class_name(open_active_block_class(text, markdown_render));
    // 未闭合围栏：textContent 避免半截 HTML；关 MD / 行内增强走统一入口。
    if markdown_render && open_block_is_fence_buffer(text) {
        block.set_text_content(Some(text));
    } else {
        block.set_inner_html(&render_open_active_html(text, markdown_render));
    }
    true
}

/// Incremental / ThinkBody 共用的「追加闭合块 + 更新活跃块」尾部应用。
pub(super) fn apply_incremental_tail(
    body: &web_sys::HtmlElement,
    append_closed: &[String],
    open_plain: Option<&str>,
    markdown_render: bool,
) -> bool {
    if !append_closed.is_empty() {
        remove_open_block(body);
        for chunk in append_closed {
            if body.insert_adjacent_html("beforeend", chunk).is_err() {
                return false;
            }
        }
    }
    match open_plain {
        Some(text) => apply_open_active_block(body, text, markdown_render),
        None => {
            remove_open_block(body);
            true
        }
    }
}

/// 回合 wrap 内的**正文段** body（思考段独立成段后，增量/工具 patch 只作用于正文段）。
pub(super) fn find_answer_body(wrap: &web_sys::HtmlElement) -> Option<web_sys::HtmlElement> {
    wrap.query_selector(".chat-tui-body--answer")
        .ok()
        .flatten()
        .and_then(|n| n.dyn_into::<web_sys::HtmlElement>().ok())
}

/// 正文被展示层去重剥空时整段隐藏（保留 DOM 供流式定位）；正文开始出现后解除。
pub(super) fn reconcile_answer_hidden(wrap: &web_sys::HtmlElement, hidden: bool) {
    let Some(section) = wrap
        .query_selector(".chat-tui-body--answer")
        .ok()
        .flatten()
        .and_then(|n| n.parent_element())
    else {
        return;
    };
    if hidden {
        let _ = section.class_list().add_1("chat-tui-turn--hidden");
    } else {
        let _ = section.class_list().remove_1("chat-tui-turn--hidden");
    }
}
