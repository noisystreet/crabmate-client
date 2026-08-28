//! 思维链折叠块：展示拆分（思维链, 终答）与 `<details>` 折叠块数据组装。
//!
//! 从 `tui_transcript_sync` 拆出，控制该文件行数与函数形参数目（fn-param / fn-nloc ratchet）。

use crate::i18n::Locale;
use crate::markdown::{plaintext_to_safe_html, to_safe_html};
use crate::storage::StoredMessage;
use crate::stream_text_overlay::{
    StreamTextOverlay, message_think_answer_for_display_including_stream_overlay,
};

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
