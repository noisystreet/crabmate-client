//! 助手气泡正文：`reasoning_text` / `text` 合并与规划围栏剥离。

use std::borrow::Cow;

use crate::i18n::Locale;
use crate::storage::{StoredMessage, StoredMessageState};

use super::super::plan_fence::{
    assistant_text_for_display, field_looks_like_agent_reply_plan_blob,
};
use super::super::thinking_strip::{
    assistant_thinking_body_and_answer_raw, filter_assistant_thinking_markers_for_display,
};
pub(super) fn assistant_message_text_for_display_ex(
    m: &StoredMessage,
    loc: Locale,
    apply_assistant_display_filters: bool,
) -> String {
    assistant_message_text_for_display_ex_with_body(
        m.text.as_str(),
        m.reasoning_text.as_str(),
        m.state.as_ref(),
        loc,
        apply_assistant_display_filters,
    )
}

/// 与 [`assistant_message_text_for_display_ex`] 相同，但返回拆分后的（思维链, 终答）。
pub(super) fn assistant_message_think_answer_for_display_ex(
    m: &StoredMessage,
    loc: Locale,
    apply_assistant_display_filters: bool,
) -> (String, String) {
    assistant_message_think_answer_for_display_ex_with_body(
        m.text.as_str(),
        m.reasoning_text.as_str(),
        m.state.as_ref(),
        loc,
        apply_assistant_display_filters,
    )
}

/// 与 [`assistant_message_text_for_display_ex`] 相同语义，但正文/思维链来自调用方字符串（例如 Web 流式 overlay 合并），避免为展示克隆整条 [`StoredMessage`]。
pub(super) fn assistant_message_text_for_display_ex_with_body(
    text: &str,
    reasoning_text: &str,
    state: Option<&StoredMessageState>,
    loc: Locale,
    apply_assistant_display_filters: bool,
) -> String {
    let (think, answer) = assistant_message_think_answer_for_display_ex_with_body(
        text,
        reasoning_text,
        state,
        loc,
        apply_assistant_display_filters,
    );
    join_think_answer_for_display(think, answer)
}

/// 与 [`assistant_message_text_for_display_ex`] 相同展示语义，但返回**拆分后的**（思维链， 终答）。
///
/// 供聊天气泡渲染折叠思考块：思维链不再与终答拼成同一段纯文本。
/// 对纯文本消费方（搜索/导出/复制等），用 [`assistant_message_text_for_display_ex_with_body`] 拿拼接串，
/// 二者由 [`join_think_answer_for_display`] 保证终态字符串完全一致。
pub(super) fn assistant_message_think_answer_for_display_ex_with_body(
    text: &str,
    reasoning_text: &str,
    state: Option<&StoredMessageState>,
    loc: Locale,
    apply_assistant_display_filters: bool,
) -> (String, String) {
    let is_streaming_last_assistant = state.as_ref().is_some_and(|s| s.is_loading());
    let reasoning_for_split: Cow<str> = if apply_assistant_display_filters {
        Cow::Owned(filter_assistant_thinking_markers_for_display(
            reasoning_text,
            is_streaming_last_assistant,
        ))
    } else {
        Cow::Borrowed(reasoning_text)
    };
    let text_for_split: Cow<str> = if apply_assistant_display_filters {
        Cow::Owned(filter_assistant_thinking_markers_for_display(
            text,
            is_streaming_last_assistant,
        ))
    } else {
        Cow::Borrowed(text)
    };
    let (r_body, t_body) = assistant_thinking_body_and_answer_raw(
        reasoning_for_split.as_ref(),
        text_for_split.as_ref(),
        apply_assistant_display_filters,
    );
    let answer = assistant_text_for_display(
        t_body,
        is_streaming_last_assistant,
        loc,
        apply_assistant_display_filters,
    );
    if apply_assistant_display_filters {
        assistant_body_with_filters(loc, is_streaming_last_assistant, r_body, t_body, answer)
    } else {
        assistant_body_without_filters(r_body, answer)
    }
}

/// 把拆分后的（思维链, 终答）按旧语义拼回单段字符串（`\n\n` 分隔），保证纯文本消费方终态一致。
fn join_think_answer_for_display(think: String, answer: String) -> String {
    if think.trim().is_empty() {
        return answer;
    }
    if answer.trim().is_empty() {
        return think;
    }
    format!("{think}\n\n{answer}")
}

fn assistant_body_with_filters(
    loc: Locale,
    is_streaming_last_assistant: bool,
    r_body: &str,
    t_body: &str,
    answer: String,
) -> (String, String) {
    let r = r_body.trim();
    let a = answer.trim();
    let t_trim = t_body.trim();
    let text_looks_like_plan_json = field_looks_like_agent_reply_plan_blob(t_trim);
    let reasoning_looks_like_plan_json = field_looks_like_agent_reply_plan_blob(r);

    // 任一侧含规划 JSON 时合并后再剥离，避免 reasoning 与 text 分轨写入时拼接泄漏原始 JSON
    // （如水合后 `display_content` 已可读化而 `reasoning_content` 仍为原文）。
    // 合并剥离后视作终答（思维链无法单独成块），与旧拼接行为一致。
    if reasoning_looks_like_plan_json || text_looks_like_plan_json {
        let merged = if r.is_empty() {
            t_body.trim().to_string()
        } else if t_trim.is_empty() {
            r_body.trim().to_string()
        } else {
            format!("{}\n\n{}", r_body.trim_end(), t_body.trim_start())
        };
        let merged_out =
            assistant_text_for_display(&merged, is_streaming_last_assistant, loc, true);
        let mv = merged_out.trim();
        if mv.is_empty() && !a.is_empty() {
            return (String::new(), answer);
        }
        return (String::new(), merged_out);
    }

    if r.is_empty() {
        (String::new(), answer)
    } else if a.is_empty() {
        (r.to_string(), String::new())
    } else {
        (r.to_string(), answer)
    }
}

fn assistant_body_without_filters(r_body: &str, answer: String) -> (String, String) {
    let r_empty = r_body.trim().is_empty();
    let a_empty = answer.trim().is_empty();
    if r_empty {
        (String::new(), answer)
    } else if a_empty {
        (r_body.to_string(), String::new())
    } else {
        (r_body.to_string(), answer)
    }
}
