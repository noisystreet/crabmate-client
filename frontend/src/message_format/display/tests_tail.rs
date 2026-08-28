//! display 管道补充单测（自 [`super`] 拆出，避免 lizard 误解析超长 `mod tests`）。

use super::message_text_for_display_ex;
use crate::i18n::Locale;
use crate::storage::StoredMessage;

#[test]
fn no_task_plan_split_across_reasoning_and_text_hides_planner_prose() {
    let reasoning = concat!(
        "用户问\"你是谁\"。这是一个简单的自我介绍问题，不需要调用任何工具。\n\n",
        "根据规则，用户没有提出需要分步执行的具体任务，所以应该设置 `\"no_task\": true`，并且 `\"steps\"` 为空数组。\n\n",
        "让我构建 JSON 对象：\n",
        "- type: \"agent_reply_plan\"\n",
        "- version: 1\n",
        "- no_task: true\n",
        "- steps: []\n\n\n\n",
    );
    let text = concat!(
        "```json\n",
        "{\n",
        "  \"type\": \"agent_reply_plan\",\n",
        "  \"version\": 1,\n",
        "  \"no_task\": true,\n",
        "  \"steps\": []\n",
        "}\n",
        "```\n",
    );
    let m = StoredMessage {
        id: "x".into(),
        role: "assistant".into(),
        text: text.into(),
        reasoning_text: reasoning.into(),
        image_urls: vec![],
        state: None,
        is_tool: false,
        tool_call_id: None,
        tool_name: None,
        created_at: 0,
    };
    let out = message_text_for_display_ex(&m, Locale::ZhHans, true);
    assert!(
        !out.contains("用户问"),
        "planner preamble in reasoning_text should not leak: {out}"
    );
    assert!(
        !out.contains("agent_reply_plan"),
        "plan json should be stripped: {out}"
    );
}

/// 整段规划（含围栏）均在 `reasoning_text`、`text` 为空（常见于未下发 `assistant_answer_phase` 的流式收尾）。
#[test]
fn no_task_plan_whole_in_reasoning_text_still_hidden() {
    let body = concat!(
        "用户问\"你是谁\"。\n\n",
        "```json\n",
        "{\"type\":\"agent_reply_plan\",\"version\":1,\"no_task\":true,\"steps\":[]}\n",
        "```\n",
    );
    let m = StoredMessage {
        id: "x".into(),
        role: "assistant".into(),
        text: String::new(),
        reasoning_text: body.into(),
        image_urls: vec![],
        state: None,
        is_tool: false,
        tool_call_id: None,
        tool_name: None,
        created_at: 0,
    };
    let out = message_text_for_display_ex(&m, Locale::ZhHans, true);
    assert!(!out.contains("用户问"), "preamble should not leak: {out}");
    assert!(
        !out.contains("agent_reply_plan"),
        "json should not leak: {out}"
    );
}

#[test]
fn pretty_bare_plan_json_in_text_only() {
    use super::message_text_for_display_ex;
    let text = r#"{ "type": "agent_reply_plan", "version": 1, "steps": [ { "id": "modify-main-py-cpu-info", "description": "修改 main.py", "executor_kind": "patch_write", "acceptance": { "expect_exit_code": 0, "expect_stdout_contains": "CPU" } } ] }"#;
    let m = StoredMessage {
        id: "x".into(),
        role: "assistant".into(),
        text: text.into(),
        reasoning_text: String::new(),
        image_urls: vec![],
        state: None,
        is_tool: false,
        tool_call_id: None,
        tool_name: None,
        created_at: 0,
    };
    let out = message_text_for_display_ex(&m, Locale::ZhHans, true);
    assert!(
        !out.contains("agent_reply_plan"),
        "raw plan json should be hidden: {out}"
    );
    assert!(
        out.contains("modify-main-py-cpu-info"),
        "step id should remain: {out}"
    );
}

#[test]
fn duplicate_raw_plan_json_in_reasoning_and_text() {
    use super::message_text_for_display_ex;
    let json = r#"{ "type": "agent_reply_plan", "version": 1, "steps": [ { "id": "x", "description": "d" } ] }"#;
    let m = StoredMessage {
        id: "x".into(),
        role: "assistant".into(),
        text: json.into(),
        reasoning_text: json.into(),
        image_urls: vec![],
        state: None,
        is_tool: false,
        tool_call_id: None,
        tool_name: None,
        created_at: 0,
    };
    let out = message_text_for_display_ex(&m, Locale::ZhHans, true);
    assert!(
        !out.contains("agent_reply_plan"),
        "duplicate plan json should not leak: {out}"
    );
}

#[test]
fn plan_json_in_reasoning_with_prose_in_text() {
    use super::message_text_for_display_ex;
    let reasoning = r#"{ "type": "agent_reply_plan", "version": 1, "steps": [ { "id": "a", "description": "步一" } ] }"#;
    let m = StoredMessage {
        id: "x".into(),
        role: "assistant".into(),
        text: "补充说明".into(),
        reasoning_text: reasoning.into(),
        image_urls: vec![],
        state: None,
        is_tool: false,
        tool_call_id: None,
        tool_name: None,
        created_at: 0,
    };
    let out = message_text_for_display_ex(&m, Locale::ZhHans, true);
    assert!(
        !out.contains("agent_reply_plan"),
        "plan json in reasoning should not leak: {out}"
    );
    assert!(
        out.contains("补充说明"),
        "non-plan tail should remain: {out}"
    );
}

#[test]
fn plan_json_in_reasoning_formatted_steps_in_text() {
    use super::message_text_for_display_ex;
    let reasoning = r#"{ "type": "agent_reply_plan", "version": 1, "steps": [ { "id": "x", "description": "d" } ] }"#;
    let text = "1. `x`: d\n";
    let m = StoredMessage {
        id: "x".into(),
        role: "assistant".into(),
        text: text.into(),
        reasoning_text: reasoning.into(),
        image_urls: vec![],
        state: None,
        is_tool: false,
        tool_call_id: None,
        tool_name: None,
        created_at: 0,
    };
    let out = message_text_for_display_ex(&m, Locale::ZhHans, true);
    assert!(
        !out.contains("agent_reply_plan"),
        "hydration-like split should not leak raw json: {out}"
    );
}

/// `display_content` + `display_reasoning_content` 水合后的助手气泡形态（见 `conversation_hydrate`）。
#[test]
fn snapshot_both_display_fields_hydrated_shape_hides_plan_json() {
    use super::message_text_for_display_ex;
    let plan_json = r#"{ "type": "agent_reply_plan", "version": 1, "steps": [ { "id": "x", "description": "d" } ] }"#;
    let m = StoredMessage {
        id: "x".into(),
        role: "assistant".into(),
        text: "1. `x`: d".into(),
        reasoning_text: plan_json.into(),
        image_urls: vec![],
        state: None,
        is_tool: false,
        tool_call_id: None,
        tool_name: None,
        created_at: 0,
    };
    let out = message_text_for_display_ex(&m, Locale::ZhHans, true);
    assert!(
        !out.contains("agent_reply_plan"),
        "hydrated snapshot must not leak raw plan json in bubble: {out}"
    );
    assert!(
        out.contains("x") && out.contains('d'),
        "formatted step should remain readable: {out}"
    );
}

#[test]
fn plan_json_reasoning_only_while_loading() {
    use super::message_text_for_display_ex;
    use crate::storage::StoredMessageState;
    let reasoning = r#"{ "type": "agent_reply_plan", "version": 1, "steps": [ { "id": "x", "description": "d" } ] }"#;
    let m = StoredMessage {
        id: "x".into(),
        role: "assistant".into(),
        text: String::new(),
        reasoning_text: reasoning.into(),
        image_urls: vec![],
        state: Some(StoredMessageState::Loading),
        is_tool: false,
        tool_call_id: None,
        tool_name: None,
        created_at: 0,
    };
    let out = message_text_for_display_ex(&m, Locale::ZhHans, true);
    assert!(
        !out.contains("agent_reply_plan"),
        "streaming planner json should be readable: {out}"
    );
}

#[test]
fn incomplete_plan_json_not_streaming_is_hidden() {
    use super::message_text_for_display_ex;
    let text =
        r#"{ "type": "agent_reply_plan", "version": 1, "steps": [ { "id": "x", "description": "d""#;
    let m = StoredMessage {
        id: "x".into(),
        role: "assistant".into(),
        text: text.into(),
        reasoning_text: String::new(),
        image_urls: vec![],
        state: None,
        is_tool: false,
        tool_call_id: None,
        tool_name: None,
        created_at: 0,
    };
    let out = message_text_for_display_ex(&m, Locale::ZhHans, true);
    assert!(
        !out.contains("agent_reply_plan"),
        "incomplete plan json should not leak when not streaming: {out}"
    );
}

#[test]
fn think_answer_split_matches_joined_for_reasoning_and_answer() {
    use super::{
        assistant_message_text_for_display_ex_with_body_strings,
        assistant_message_think_answer_for_display_ex_with_body_strings,
    };
    let (think, ans) = assistant_message_think_answer_for_display_ex_with_body_strings(
        "这是回答\n第二段",
        "先思考一下",
        None,
        Locale::ZhHans,
        true,
    );
    assert_eq!(think, "先思考一下");
    assert_eq!(ans, "这是回答\n第二段");
    let joined = assistant_message_text_for_display_ex_with_body_strings(
        "这是回答\n第二段",
        "先思考一下",
        None,
        Locale::ZhHans,
        true,
    );
    assert_eq!(joined, "先思考一下\n\n这是回答\n第二段");
}

#[test]
fn think_answer_split_answers_only_when_no_reasoning() {
    use super::{
        assistant_message_text_for_display_ex_with_body_strings,
        assistant_message_think_answer_for_display_ex_with_body_strings,
    };
    let (think, ans) = assistant_message_think_answer_for_display_ex_with_body_strings(
        "纯回答",
        "",
        None,
        Locale::ZhHans,
        true,
    );
    assert_eq!(think, "");
    assert_eq!(ans, "纯回答");
    assert_eq!(
        assistant_message_text_for_display_ex_with_body_strings(
            "纯回答",
            "",
            None,
            Locale::ZhHans,
            true
        ),
        "纯回答",
    );
}

#[test]
fn think_answer_split_never_leaks_plan_json_into_thinking_block() {
    use super::assistant_message_think_answer_for_display_ex_with_body_strings;
    let reasoning = r#"{ "type": "agent_reply_plan", "version": 1, "steps": [ { "id": "a", "description": "步一" } ] }"#;
    let (think, ans) = assistant_message_think_answer_for_display_ex_with_body_strings(
        "补充说明",
        reasoning,
        None,
        Locale::ZhHans,
        true,
    );
    assert!(
        think.is_empty(),
        "plan json must not become a thinking block: {think:?}"
    );
    assert!(
        !ans.contains("agent_reply_plan"),
        "raw json must not leak into answer: {ans:?}"
    );
    assert!(ans.contains("补充说明"), "{ans:?}");
}

#[test]
fn inline_think_extracted_by_split_but_stripped_by_joined_when_filters_on() {
    use super::{
        assistant_message_text_for_display_ex_with_body_strings,
        assistant_message_think_answer_for_display_ex_with_body_strings,
    };
    let inline = concat!(
        "<",
        "think",
        ">",
        "计划步骤",
        "</",
        "think",
        ">",
        "\n\n答案是 A"
    );
    let (think, ans) = assistant_message_think_answer_for_display_ex_with_body_strings(
        inline,
        "",
        None,
        Locale::ZhHans,
        true,
    );
    assert_eq!(think, "计划步骤");
    assert_eq!(ans, "答案是 A");
    // 纯文本消费方（搜索/复制/导出）保持旧行为：内联 think 仍被剥离。
    let joined = assistant_message_text_for_display_ex_with_body_strings(
        inline,
        "",
        None,
        Locale::ZhHans,
        true,
    );
    assert_eq!(joined, "答案是 A");
}

#[test]
fn inline_think_unclosed_streaming_goes_to_thinking_not_answer() {
    use super::assistant_message_think_answer_for_display_ex_with_body_strings;
    use crate::storage::StoredMessageState;
    let state = Some(StoredMessageState::Loading);
    let (think, ans) = assistant_message_think_answer_for_display_ex_with_body_strings(
        concat!("<", "think", ">", "正在推理片段"),
        "",
        state.as_ref(),
        Locale::ZhHans,
        true,
    );
    assert_eq!(think, "正在推理片段");
    assert!(ans.is_empty());
}

#[test]
fn no_think_tag_goes_to_answer() {
    use super::assistant_message_think_answer_for_display_ex_with_body_strings;
    // 无内联思考标签的普通文本：全部进终答，思维链为空。
    let (think, ans) = assistant_message_think_answer_for_display_ex_with_body_strings(
        "前置文本",
        "",
        None,
        Locale::ZhHans,
        true,
    );
    assert!(think.is_empty());
    assert_eq!(ans, "前置文本");
}

#[test]
fn inline_think_unclosed_finalized_keeps_whole_text_in_answer() {
    use super::assistant_message_think_answer_for_display_ex_with_body_strings;
    // 终态（非流式）畸形输入：未闭合 `<think>` 不吞正文（开标签前文本不得丢失），
    // 与旧 joined 行为完全一致（残片随 answer 侧剥除，前缀保留）；仅流式残片才归入思维链。
    let (think, ans) = assistant_message_think_answer_for_display_ex_with_body_strings(
        "前置<think>残片",
        "",
        None,
        Locale::ZhHans,
        true,
    );
    assert!(think.is_empty());
    assert_eq!(ans, "前置", "prefix before open tag must survive");
    assert!(!ans.contains("残片"), "{ans:?}");
}
