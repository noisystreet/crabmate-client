//! 思维链折叠块：展示拆分（思维链, 终答）与 `<details>` 折叠块数据组装。
//!
//! 从 `tui_transcript_sync` 拆出，控制该文件行数与函数形参数目（fn-param / fn-nloc ratchet）。

use crate::i18n::Locale;
use crate::markdown::{plaintext_to_safe_html, to_safe_html};
use crate::storage::StoredMessage;
use crate::stream_text_overlay::{
    StreamTextOverlay, message_think_answer_for_display_including_stream_overlay,
};
use wasm_bindgen::JsCast;

use super::tui_line_markdown::ThinkBlock;

/// 与 [`crate::stream_text_overlay::message_text_for_display_including_stream_overlay`] 相同的
/// 合并规则，但返回拆分后的（思维链, 终答），供折叠思考块渲染。
pub(super) fn message_think_answer_display_text(
    message: &StoredMessage,
    session_id: &str,
    overlay: Option<&StreamTextOverlay>,
    locale: Locale,
    apply_assistant_display_filters: bool,
) -> (String, String) {
    message_think_answer_for_display_including_stream_overlay(
        message,
        overlay,
        session_id,
        locale,
        apply_assistant_display_filters,
    )
}

/// 组装思维链折叠块渲染数据：`open` 默认展开，`summary_html` 带 i18n 标签，`body_html` 为已转义正文。
///
/// 流式增量只更新 `body_html`（[`crate::app::chat::tui_line_markdown::TuiBodyPatch::ThinkBody`]）；
/// `summary_html` 稳定且不参与增量比较，避免每帧重渲整个 `<details>`。
pub(super) fn build_think_block(
    thinking: &str,
    open: bool,
    markdown_render: bool,
    locale: Locale,
) -> ThinkBlock {
    let label = plaintext_to_safe_html(crate::i18n::chat_thinking_label(locale));
    let body_html = if markdown_render {
        to_safe_html(thinking)
    } else {
        plaintext_to_safe_html(thinking)
    };
    ThinkBlock {
        open,
        summary_html: format!("<summary class=\"chat-tui-think-summary\">{label}</summary>"),
        body_html,
    }
}

/// 思考段（独立气泡）section 的 class 标识。
pub(super) const THINK_SECTION_CLASS: &str = "chat-tui-turn--think";

/// 更新/创建回合 wrap 内的思考段；`None` 时移除思考段。
///
/// 仅被 `TuiBodyPatch::ReplaceAll`（结构变化：think 出现/消失、open 翻转、刷新）调用；
/// 流式正文走定向 `ThinkBody`（不经过此处）。返回是否成功。
pub(super) fn update_think_section(
    wrap: &web_sys::HtmlElement,
    block: Option<&ThinkBlock>,
) -> bool {
    let Some(block) = block else {
        if let Some(existing) = wrap.query_selector(".chat-tui-turn--think").ok().flatten() {
            existing.remove();
        }
        return true;
    };
    if let Some(existing) = wrap.query_selector(".chat-tui-turn--think").ok().flatten() {
        // 定向更新而非整段重建：思考完成收起 / 手动展开时只翻转 open 属性，
        // 避免 WebKit 下 set_inner_html 重建 <details> 造成的闪烁（消息块先消失再出现）。
        let Some(details) = existing
            .query_selector(".chat-tui-think")
            .ok()
            .flatten()
            .and_then(|n| n.dyn_into::<web_sys::HtmlElement>().ok())
        else {
            // DOM 结构异常（缺 details）→ 整段重建兜底。
            existing.set_inner_html(&format!(
                "<div class=\"chat-tui-body chat-tui-body--think\">{}</div>",
                block.to_details_html()
            ));
            return true;
        };
        if block.open {
            let _ = details.set_attribute("open", "");
        } else {
            let _ = details.remove_attribute("open");
        }
        if let Some(summary) = details
            .query_selector(".chat-tui-think-summary")
            .ok()
            .flatten()
            .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
            && summary.outer_html() != block.summary_html
        {
            // summary 变化（如 i18n 切换）时按需同步，通常不发生。
            summary.set_outer_html(&block.summary_html);
        }
        if let Some(body) = details
            .query_selector(".chat-tui-think-body")
            .ok()
            .flatten()
            .and_then(|n| n.dyn_into::<web_sys::HtmlElement>().ok())
            && body.inner_html() != block.body_html
        {
            // 思考正文变化（刷新/重读）时按需更新；finalize 收起时通常与流式一致。
            body.set_inner_html(&block.body_html);
        }
        return true;
    }
    // 首次插入（think 流式中途出现）：带上 data-tui-msg-id，保持点击/菜单定位一致。
    let id_attr = wrap
        .get_attribute("data-tui-wrap-id")
        .filter(|id| !id.is_empty())
        .map(|id| format!(" data-tui-msg-id=\"{}\"", plaintext_to_safe_html(&id)))
        .unwrap_or_default();
    let section_html = format!(
        "<section class=\"chat-tui-turn chat-tui-turn--assistant {THINK_SECTION_CLASS}\"{id_attr}>\
         <div class=\"chat-tui-body chat-tui-body--think\">{}</div>\
         </section>",
        block.to_details_html()
    );
    // 插到 role 块之后（正文段之前）。
    match wrap.query_selector(".chat-tui-role").ok().flatten() {
        Some(role) => role.insert_adjacent_html("afterend", &section_html).is_ok(),
        None => wrap
            .insert_adjacent_html("afterbegin", &section_html)
            .is_ok(),
    }
}
