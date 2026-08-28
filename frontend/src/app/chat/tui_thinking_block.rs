//! 思维链折叠块：展示拆分（思维链, 终答）与 `<details>` HTML 组装。
//!
//! 从 `tui_transcript_sync` 拆出，控制该文件行数与函数形参数目（fn-param / fn-nloc ratchet）。

use crate::i18n::Locale;
use crate::markdown::{plaintext_to_safe_html, to_safe_html};
use crate::storage::StoredMessage;
use crate::stream_text_overlay::{
    StreamTextOverlay, message_think_answer_for_display_including_stream_overlay,
};

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

/// 思维链折叠块 HTML：`open=true` 时带 `open` 属性（流式自动展开 / 用户手动展开），否则收起。
pub(super) fn thinking_details_html(
    thinking: &str,
    open: bool,
    markdown_render: bool,
    locale: Locale,
) -> String {
    let open_attr = if open { " open" } else { "" };
    let label = plaintext_to_safe_html(crate::i18n::chat_thinking_label(locale));
    let body = if markdown_render {
        to_safe_html(thinking)
    } else {
        plaintext_to_safe_html(thinking)
    };
    format!(
        "<details class=\"chat-tui-think\"{open_attr}>\
         <summary class=\"chat-tui-think-summary\">{label}</summary>\
         <div class=\"chat-tui-think-body msg-md-prose\">{body}</div>\
         </details>"
    )
}
