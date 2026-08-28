//! 助手气泡正文：`reasoning_text` / `text` 合并与规划围栏剥离。

use std::borrow::Cow;

use crate::i18n::Locale;
use crate::storage::{StoredMessage, StoredMessageState};

use super::super::plan_fence::{
    assistant_text_for_display, field_looks_like_agent_reply_plan_blob,
};
use super::super::thinking_strip::{
    assistant_thinking_body_and_answer_raw, filter_assistant_thinking_markers_for_display,
    filter_redacted_thinking_for_display,
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
    let is_streaming = state.as_ref().is_some_and(|s| s.is_loading());
    let (r, t) = filter_display_inputs(
        reasoning_text,
        text,
        is_streaming,
        apply_assistant_display_filters,
        false,
    );
    let (think, answer) = compute_think_answer_parts(
        r.as_ref(),
        t.as_ref(),
        is_streaming,
        loc,
        apply_assistant_display_filters,
    );
    join_think_answer_for_display(think, answer)
}

/// 与 [`assistant_message_text_for_display_ex`] 相同展示语义，但返回**拆分后的**（思维链, 终答）。
///
/// 供聊天气泡渲染折叠思考块：思维链不再与终答拼成同一段纯文本。**text 轨保留内联 `<think>`**
/// 供拆分提取（Qwen / vLLM 等把思考塞进 content 的网关也能进折叠块）；`redacted_thinking` 隐私块仍剥离。
/// 纯文本消费方（搜索/导出/复制等）用 [`assistant_message_text_for_display_ex_with_body`]，其**继续剥掉**
/// 内联 `<think>`（旧行为，输出不变）。
pub(super) fn assistant_message_think_answer_for_display_ex_with_body(
    text: &str,
    reasoning_text: &str,
    state: Option<&StoredMessageState>,
    loc: Locale,
    apply_assistant_display_filters: bool,
) -> (String, String) {
    let is_streaming = state.as_ref().is_some_and(|s| s.is_loading());
    let (r, t) = filter_display_inputs(
        reasoning_text,
        text,
        is_streaming,
        apply_assistant_display_filters,
        true,
    );
    compute_think_answer_parts(
        r.as_ref(),
        t.as_ref(),
        is_streaming,
        loc,
        apply_assistant_display_filters,
    )
}

/// 按 `apply_assistant_display_filters` 过滤两轨输入。
///
/// `keep_inline_think=true`（气泡拆分路径）：text 轨只剥 `redacted_thinking`，保留 `<think>` 供拆分；
/// `false`（纯文本拼接路径）：两轨都剥 think/redacted 标记，与旧输出完全一致。
fn filter_display_inputs<'a>(
    reasoning: &'a str,
    text: &'a str,
    streaming: bool,
    apply_filters: bool,
    keep_inline_think: bool,
) -> (Cow<'a, str>, Cow<'a, str>) {
    if !apply_filters {
        return (Cow::Borrowed(reasoning), Cow::Borrowed(text));
    }
    let r = filter_assistant_thinking_markers_for_display(reasoning, streaming);
    let t = if keep_inline_think {
        filter_redacted_thinking_for_display(text, streaming)
    } else {
        filter_assistant_thinking_markers_for_display(text, streaming)
    };
    (Cow::Owned(r), Cow::Owned(t))
}

/// 过滤后的两轨 → 拆分（思维链, 终答）并套用 filters/无 filters 分支。
fn compute_think_answer_parts(
    reasoning_for_split: &str,
    text_for_split: &str,
    is_streaming: bool,
    loc: Locale,
    apply_filters: bool,
) -> (String, String) {
    let (r_body, t_body) = assistant_thinking_body_and_answer_raw(
        reasoning_for_split,
        text_for_split,
        apply_filters,
        is_streaming,
    );
    let answer = assistant_text_for_display(t_body, is_streaming, loc, apply_filters);
    let (think, ans) = if apply_filters {
        assistant_body_with_filters(loc, is_streaming, r_body, t_body, answer)
    } else {
        assistant_body_without_filters(r_body, answer)
    };
    // 展示层去重：模型在正文回显的「### 思考过程」章节与 reasoning 重复时剥除（不动存储）。
    if apply_filters {
        let ans = super::super::thinking_strip::strip_echoed_thinking_section(&ans, &think);
        (think, ans)
    } else {
        (think, ans)
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
