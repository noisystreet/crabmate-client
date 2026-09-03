//! `agent_reply_plan` JSON、``` 围栏剥离与助手正文「规划轮」展示。

use serde_json::Value;

use crate::i18n::Locale;
use crate::storage::StoredMessage;

use super::thinking_strip::filter_assistant_thinking_markers_for_display;

fn format_agent_reply_plan_json_for_display(
    json_text: &str,
    goal: &str,
    loc: Locale,
) -> Option<String> {
    let v: Value = serde_json::from_str(json_text).ok()?;
    let obj = v.as_object()?;
    if obj.get("type").and_then(|x| x.as_str()) != Some("agent_reply_plan") {
        return None;
    }
    let steps = obj.get("steps").and_then(|x| x.as_array())?;

    let mut lines = Vec::with_capacity(steps.len().saturating_add(1));
    let goal = goal.trim();
    if !goal.is_empty() {
        lines.push(goal.to_string());
    }
    if steps.is_empty() {
        if !goal.is_empty() {
            return Some(goal.to_string());
        }
        return Some(crate::i18n::plan_generated(loc).to_string());
    }
    if !goal.is_empty() {
        lines.push(String::new());
    }
    for (idx, s) in steps.iter().enumerate() {
        let id = s
            .get("id")
            .and_then(|x| x.as_str())
            .filter(|x| !x.trim().is_empty())
            .unwrap_or(crate::i18n::plan_step_placeholder_id());
        let desc = s
            .get("description")
            .and_then(|x| x.as_str())
            .filter(|x| !x.trim().is_empty())
            .unwrap_or(crate::i18n::plan_step_no_desc(loc));
        lines.push(crate::i18n::plan_step_line(
            loc,
            idx,
            id.trim(),
            desc.trim(),
        ));
    }
    Some(lines.join("\n"))
}

fn plan_steps_are_all_strings(steps: &[Value]) -> bool {
    steps.iter().all(|step| step.is_string())
}

fn append_numbered_plan_steps(lines: &mut Vec<String>, steps: &[Value]) {
    if steps.is_empty() {
        return;
    }
    lines.push(String::new());
    for (idx, step) in steps.iter().enumerate() {
        let text = step.as_str().unwrap_or("").trim();
        if text.is_empty() {
            continue;
        }
        let n = idx + 1;
        lines.push(format!("{n}. {text}"));
    }
}

/// 执行器输出的紧凑规划：`plan_summary` + `steps`（**字符串**数组）+ 可选 `no_new_tool_calls`。
fn format_plan_summary_steps_json_for_display(json_text: &str, loc: Locale) -> Option<String> {
    let v: Value = serde_json::from_str(json_text).ok()?;
    let obj = v.as_object()?;
    let summary = obj.get("plan_summary")?.as_str()?.trim();
    if summary.is_empty() {
        return None;
    }
    let steps = obj.get("steps")?.as_array()?;
    if !plan_steps_are_all_strings(steps) {
        return None;
    }
    let mut lines: Vec<String> = vec![summary.to_string()];
    append_numbered_plan_steps(&mut lines, steps);
    if obj.get("no_new_tool_calls").and_then(|x| x.as_bool()) == Some(true) {
        lines.push(String::new());
        lines.push(crate::i18n::plan_no_new_tool_calls_note(loc).to_string());
    }
    Some(lines.join("\n"))
}

fn formatted_structured_plan_from_json_body(body: &str, loc: Locale) -> Option<String> {
    let b = body.trim();
    format_plan_summary_steps_json_for_display(b, loc)
        .or_else(|| format_agent_reply_plan_json_for_display(b, "", loc))
}

/// 语言标签后紧跟分隔符或 JSON 开括号（`json\n` / `json{` / `json` 行尾等形态）。
fn jsonish_lang_label_tail(next: Option<char>) -> bool {
    matches!(next, None | Some('\n' | '\r' | ' ' | '\t' | '{' | '['))
}

fn fenced_body_after_optional_jsonish_lang_label(inner: &str) -> Option<&str> {
    let s = inner.trim_start_matches(['\n', '\r', ' ', '\t']);
    if s.is_empty() {
        return Some("");
    }
    for label in ["json", "markdown", "md"] {
        if let Some(rest) = s.strip_prefix(label)
            && jsonish_lang_label_tail(rest.chars().next())
        {
            return Some(rest.trim_start_matches(['\n', '\r', ' ', '\t']));
        }
    }
    None
}

fn triple_backtick_fence_count(s: &str) -> usize {
    s.match_indices("```").count()
}

fn first_fence_inner_looks_like_json_object(s: &str) -> bool {
    let mut it = s.split("```");
    let _ = it.next();
    let Some(inner) = it.next() else {
        return false;
    };
    let Some(body) = fenced_body_after_optional_jsonish_lang_label(inner) else {
        return false;
    };
    let b = body.trim();
    b.is_empty() || b.starts_with('{')
}

/// 单字段（`text` / `reasoning_text` 或 overlay 片段）是否像 `agent_reply_plan` v1 或其在途不完整 JSON。
pub(crate) fn field_looks_like_agent_reply_plan_blob(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() {
        return false;
    }
    t.contains("\"agent_reply_plan\"")
        || t.contains("\"type\":\"agent_reply_plan\"")
        || looks_like_incomplete_agent_reply_plan_whole_json(t)
}

fn looks_like_incomplete_agent_reply_plan_whole_json(t: &str) -> bool {
    let t = t.trim();
    if !t.starts_with('{') {
        return false;
    }
    if t.contains("\"agent_reply_plan\"") {
        return true;
    }
    t.contains("\"type\"") && t.contains("\"version\"") && t.contains("\"steps\"")
}

/// Web 气泡眉「规划轮」标记：分阶段规划模型产出在 `text` / `reasoning_text` 中含 `agent_reply_plan`（含流式不完整前缀）。
pub(crate) fn stored_message_is_staged_planner_round(m: &StoredMessage) -> bool {
    if m.role != "assistant" || m.is_tool {
        return false;
    }
    let raw = format!("{}{}", m.reasoning_text, m.text);
    if raw.contains("\"agent_reply_plan\"") || raw.contains("\"type\":\"agent_reply_plan\"") {
        return true;
    }
    looks_like_incomplete_agent_reply_plan_whole_json(raw.trim())
}

fn should_buffer_agent_reply_plan_stream(stripped: &str) -> bool {
    if triple_backtick_fence_count(stripped) % 2 == 1
        && first_fence_inner_looks_like_json_object(stripped)
    {
        return true;
    }
    let t = stripped.trim();
    if !t.starts_with('{') {
        return false;
    }
    if format_agent_reply_plan_json_for_display(t, "", Locale::ZhHans).is_some() {
        return false;
    }
    serde_json::from_str::<Value>(t).is_err()
        && looks_like_incomplete_agent_reply_plan_whole_json(t)
}

fn prose_before_first_fence(s: &str) -> String {
    s.split("```").next().unwrap_or("").trim().to_string()
}

/// `{"type":"agent_reply_plan",...}` 独占行尾（前有换行、后无其它正文）时返回去掉该 JSON 后的前缀。
///
/// 模型常见形态：先输出 `1. \`id\`: 描述` 列表，再单独输出一行裸 JSON；整段不以 `{` 开头时
/// 须在此剥离，避免气泡回显原始 JSON。
fn strip_trailing_standalone_agent_reply_plan_blob(s: &str) -> Option<String> {
    let trimmed = s.trim_end();
    let bytes = trimmed.as_bytes();
    let mut starts: Vec<usize> = Vec::new();
    for (i, _) in trimmed.match_indices('{') {
        if i == 0 {
            starts.push(0);
        } else if matches!(bytes.get(i - 1), Some(b'\n') | Some(b'\r')) {
            starts.push(i);
        }
    }
    for &start in starts.iter().rev() {
        let tail = &trimmed[start..];
        let mut de = serde_json::Deserializer::from_str(tail).into_iter::<Value>();
        let Some(Ok(v)) = de.next() else {
            continue;
        };
        if v.as_object()
            .and_then(|o| o.get("type"))
            .and_then(|x| x.as_str())
            != Some("agent_reply_plan")
        {
            continue;
        }
        let consumed = de.byte_offset();
        let after = trimmed.get(start + consumed..).unwrap_or("").trim();
        if !after.is_empty() {
            continue;
        }
        let prefix = trimmed[..start].trim_end();
        if prefix.is_empty() {
            continue;
        }
        return Some(prefix.to_string());
    }
    None
}

/// 围栏内为可展示的「结构化规划」JSON（`agent_reply_plan` 或 `plan_summary` 紧凑块）时返回格式化正文。
fn try_fence_inner_structured_plan_display(inner: &str, loc: Locale) -> Option<String> {
    let raw = inner.trim();
    let body = fenced_body_after_optional_jsonish_lang_label(raw)
        .unwrap_or(raw)
        .trim();
    if !body.starts_with('{') {
        return None;
    }
    formatted_structured_plan_from_json_body(body, loc)
}

/// 首段围栏前文本是否可视为「仅规划说明」从而省略，避免误删「大段正文 + 文末 plan 围栏」。
fn drop_first_segment_before_hidden_agent_reply_plan_fence(segment: &str) -> bool {
    let t = segment.trim();
    if t.is_empty() {
        return true;
    }
    if t.contains("\n## ") || t.contains("\n### ") || t.starts_with("## ") || t.starts_with("### ")
    {
        return false;
    }
    true
}

/// 首个围栏段前的正文是否可省略（无标题/分区标记且非空才保留场景由上层保证）。
fn first_fence_prose_is_omittable(is_first_code_fence: bool, segment: &str) -> bool {
    is_first_code_fence && drop_first_segment_before_hidden_agent_reply_plan_fence(segment)
}

fn out_needs_separating_newline(out: &str) -> bool {
    !out.is_empty() && !out.ends_with('\n')
}

/// 末尾未闭合围栏且其内容为空白（多行连续围栏边界）时停止追加。
fn should_stop_on_blank_trailing_fence(unclosed: bool, at_end: bool, inner: &str) -> bool {
    unclosed && at_end && inner.trim().is_empty()
}

fn strip_agent_reply_plan_fence_blocks_for_display(content: &str, loc: Locale) -> String {
    let parts: Vec<&str> = content.split("```").collect();
    let unclosed_trailing_fence = parts.len().is_multiple_of(2);
    let mut out = String::new();
    let mut i = 0usize;
    let mut is_first_code_fence = true;
    while i < parts.len() {
        let segment = parts[i];
        i += 1;
        if i >= parts.len() {
            out.push_str(segment);
            break;
        }
        let inner = parts[i];
        i += 1;
        if let Some(disp) = try_fence_inner_structured_plan_display(inner, loc) {
            let skip_segment = first_fence_prose_is_omittable(is_first_code_fence, segment);
            if !skip_segment {
                out.push_str(segment);
            }
            if !disp.trim().is_empty() {
                if out_needs_separating_newline(&out) {
                    out.push('\n');
                }
                out.push_str(disp.trim());
            }
            is_first_code_fence = false;
            continue;
        }
        is_first_code_fence = false;
        out.push_str(segment);
        if should_stop_on_blank_trailing_fence(unclosed_trailing_fence, i >= parts.len(), inner) {
            break;
        }
        out.push_str("```");
        out.push_str(inner);
        out.push_str("```");
    }
    out
}

fn strip_leading_agent_reply_plan_json_tail(s: &str) -> Option<String> {
    let t = s.trim_start();
    if !t.starts_with('{') || !t.contains("\"agent_reply_plan\"") {
        return None;
    }
    let mut de = serde_json::Deserializer::from_str(t).into_iter::<Value>();
    let v = de.next()?.ok()?;
    if v.as_object()
        .and_then(|o| o.get("type"))
        .and_then(|x| x.as_str())
        != Some("agent_reply_plan")
    {
        return None;
    }
    let offset = de.byte_offset();
    if offset >= t.len() {
        return None;
    }
    let tail = t[offset..].trim();
    if tail.is_empty() {
        None
    } else {
        Some(tail.to_string())
    }
}

/// 从展示文本中剥离 `<tool_call>...</tool_call>` XML 块（内部工具调用标记）。
fn strip_tool_call_xml_from_display(s: &str) -> String {
    let mut out = s.to_string();
    while let Some(start) = out.find("<tool_call") {
        let after_open = &out[start..];
        let Some(open_end) = after_open.find('>') else {
            out.replace_range(start.., "");
            break;
        };
        let content_start = start + open_end + 1;
        let Some(close_rel) = out[content_start..].find("</tool_call>") else {
            out.replace_range(start.., "");
            break;
        };
        let end = content_start + close_rel + "</tool_call>".len();
        out.replace_range(start..end, "");
    }
    out
}

pub(crate) fn assistant_text_for_display(
    raw: &str,
    is_streaming_last_assistant: bool,
    loc: Locale,
    apply_filters: bool,
) -> String {
    if !apply_filters {
        return raw.to_string();
    }
    let inner = assistant_text_for_display_inner(raw, is_streaming_last_assistant, loc);
    filter_assistant_thinking_markers_for_display(&inner, is_streaming_last_assistant)
}

/// 流式末条 + 尚无完整 JSON 时先显示围栏前的说明文本。
fn should_buffer_streaming_plan_prose(is_streaming_last_assistant: bool, trimmed: &str) -> bool {
    is_streaming_last_assistant && should_buffer_agent_reply_plan_stream(trimmed)
}

fn contains_plan_blob_marker(s: &str) -> bool {
    s.contains("\"agent_reply_plan\"") || s.contains("\"plan_summary\"")
}

/// 不完整且解析失败的裸 `agent_reply_plan` 片段（流式残留），不应回显。
fn is_incomplete_unparsed_plan(t: &str) -> bool {
    looks_like_incomplete_agent_reply_plan_whole_json(t)
        && serde_json::from_str::<Value>(t).is_err()
}

fn assistant_text_for_display_inner(
    raw: &str,
    is_streaming_last_assistant: bool,
    loc: Locale,
) -> String {
    let content = strip_tool_call_xml_from_display(raw);
    let content = strip_trailing_standalone_agent_reply_plan_blob(&content).unwrap_or(content);
    let trimmed = content.trim();

    if let Some(tail) = strip_leading_agent_reply_plan_json_tail(&content) {
        return filter_assistant_thinking_markers_for_display(&tail, is_streaming_last_assistant);
    }

    if should_buffer_streaming_plan_prose(is_streaming_last_assistant, trimmed) {
        // 须与 `assistant_text_for_display` 外套的 `filter_assistant_thinking_markers_for_display` 一致。
        return filter_assistant_thinking_markers_for_display(
            &prose_before_first_fence(trimmed),
            true,
        );
    }

    if let Some(display) =
        formatted_structured_plan_from_json_body(trimmed, loc).filter(|d| !d.trim().is_empty())
    {
        return filter_assistant_thinking_markers_for_display(
            &display,
            is_streaming_last_assistant,
        );
    }

    // 再做一次全量围栏剥离兜底：无论 `agent_reply_plan` / `plan_summary` 围栏出现在第几个代码块，都不回显原始 JSON。
    let stripped_fences = strip_agent_reply_plan_fence_blocks_for_display(&content, loc);
    let stripped_trim = stripped_fences.trim();
    if stripped_trim != trimmed {
        if stripped_trim.is_empty() && contains_plan_blob_marker(&content) {
            return crate::i18n::plan_generated(loc).to_string();
        }
        return stripped_trim.to_string();
    }

    if is_incomplete_unparsed_plan(trimmed) {
        return String::new();
    }

    content
}
