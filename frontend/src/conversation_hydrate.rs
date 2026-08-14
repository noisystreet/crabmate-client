//! 将 `GET /conversation/messages` 返回的 OpenAI 兼容消息转为 [`crate::storage::StoredMessage`]。

use std::collections::HashSet;

use serde::Deserialize;
use serde_json::Value;

use crate::storage::StoredMessage;

use crate::conversation_hydrate_timeline::{
    append_assistant_tool_calls_timeline_card, append_crabmate_timeline_system_message,
    append_tool_role_timeline_row,
};

/// 与后端 `src/types.rs` 中 `CRABMATE_FIRST_TURN_WORKSPACE_CONTEXT_NAME` 一致。
const CRABMATE_FIRST_TURN_WORKSPACE_CONTEXT_NAME: &str = "crabmate_first_turn_workspace_context";

#[derive(Debug, Deserialize)]
pub struct ConversationMessagesResponse {
    #[allow(dead_code)]
    pub conversation_id: String,
    pub revision: u64,
    #[serde(default)]
    pub active_agent_role: Option<String>,
    #[serde(default)]
    pub active_session_mode: Option<String>,
    #[serde(default)]
    pub tiktoken_prompt_tokens: Option<TiktokenPromptTokensSnapshot>,
    pub messages: Vec<Value>,
    /// 过滤后可见消息总数。
    #[serde(default)]
    pub total_count: u32,
    /// 本页第一条在过滤后数组中的下标。
    #[serde(default)]
    pub window_start_index: u32,
    #[serde(default)]
    pub has_older: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TiktokenPromptTokensSnapshot {
    pub prompt_tokens: u32,
    pub tiktoken_model: String,
}

#[derive(Debug, Deserialize)]
struct ApiMessage {
    role: String,
    content: Option<Value>,
    #[serde(default, alias = "reasoning")]
    reasoning_content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Value>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    tool_call_id: Option<String>,
    #[serde(default)]
    display_content: Option<String>,
    #[serde(default)]
    display_reasoning_content: Option<String>,
}

/// 收集快照里已有结果的 `tool_call_id`，避免再为同一次调用水合「工具：name」短卡。
fn collect_tool_result_call_ids(msgs: &[Value]) -> HashSet<String> {
    let mut ids = HashSet::new();
    for raw in msgs {
        let Ok(parsed) = serde_json::from_value::<ApiMessage>(raw.clone()) else {
            continue;
        };
        if parsed.role.trim() != "tool" {
            continue;
        }
        if let Some(id) = parsed
            .tool_call_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            ids.insert(id.to_string());
        }
    }
    ids
}

/// 去掉已有 `role=tool` 结果的 tool_calls；若全部已覆盖则返回 `None`（不再造调用短卡）。
fn tool_calls_without_results(tool_calls: &Value, covered: &HashSet<String>) -> Option<Value> {
    let arr = tool_calls.as_array()?;
    let remaining: Vec<Value> = arr
        .iter()
        .filter(|tc| {
            let id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("").trim();
            id.is_empty() || !covered.contains(id)
        })
        .cloned()
        .collect();
    if remaining.is_empty() {
        None
    } else {
        Some(Value::Array(remaining))
    }
}

fn text_from_content(content: &Option<Value>) -> (String, Vec<String>) {
    let Some(c) = content else {
        return (String::new(), Vec::new());
    };
    match c {
        Value::String(s) => (s.clone(), Vec::new()),
        Value::Array(parts) => {
            let mut text = String::new();
            let mut urls = Vec::new();
            for p in parts {
                let Some(obj) = p.as_object() else {
                    continue;
                };
                let ty = obj.get("type").and_then(|t| t.as_str()).unwrap_or("");
                if ty == "text" {
                    if let Some(t) = obj.get("text").and_then(|x| x.as_str()) {
                        if !text.is_empty() {
                            text.push('\n');
                        }
                        text.push_str(t);
                    }
                } else if ty == "image_url" {
                    if let Some(u) = obj
                        .get("image_url")
                        .and_then(|iu| iu.get("url"))
                        .and_then(|x| x.as_str())
                    {
                        let u = u.trim();
                        if !u.is_empty() {
                            urls.push(u.to_string());
                        }
                    }
                }
            }
            (text, urls)
        }
        _ => (String::new(), Vec::new()),
    }
}

/// 返回 `true` 表示该条已由分支消费（外层应 `continue`）。
struct HydrateSpecialLine<'a> {
    parsed: &'a ApiMessage,
    role: &'a str,
    name: &'a str,
    text: &'a str,
    reasoning: &'a str,
    display_content: Option<&'a str>,
    display_reasoning: Option<&'a str>,
    /// 同页已有 `role=tool` 的 call id；用于抑制重复调用短卡。
    tool_result_call_ids: &'a HashSet<String>,
    base_ms: i64,
    out: &'a mut Vec<StoredMessage>,
    t: &'a mut i64,
}

fn hydrate_try_special_cases(line: HydrateSpecialLine<'_>) -> bool {
    if line.role == "system" && line.name == "crabmate_ui_sep" {
        return true;
    }
    if line.role == "user" && line.name == CRABMATE_FIRST_TURN_WORKSPACE_CONTEXT_NAME {
        return true;
    }
    if line.role == "system" && line.name == "crabmate_timeline" {
        append_crabmate_timeline_system_message(line.text, line.base_ms, line.out, line.t);
        return true;
    }
    if line.role == "assistant"
        && line.text.trim().is_empty()
        && line.reasoning.trim().is_empty()
        && let Some(ref tc) = line.parsed.tool_calls
    {
        // 有对应 tool 结果时只保留结果卡（见 chat_export_20260808 双「## 工具」）。
        if let Some(orphan_tc) = tool_calls_without_results(tc, line.tool_result_call_ids) {
            append_assistant_tool_calls_timeline_card(&orphan_tc, line.base_ms, line.out, line.t);
        }
        return true;
    }
    if line.role == "tool" {
        append_tool_role_timeline_row(
            line.name,
            line.text,
            line.display_content,
            line.display_reasoning,
            line.parsed.tool_call_id.as_deref(),
            line.base_ms,
            line.out,
            line.t,
        );
        return true;
    }
    false
}

/// 将会话快照转为 UI 消息列表（新 id；`created_at` 从 `base_ms` 递增以保证顺序）。
pub fn stored_messages_from_conversation_api_with_base(
    msgs: &[Value],
    base_ms: i64,
) -> Vec<StoredMessage> {
    let tool_result_call_ids = collect_tool_result_call_ids(msgs);
    let mut out: Vec<StoredMessage> = Vec::new();
    let mut t = base_ms;
    for raw in msgs {
        let parsed: ApiMessage = match serde_json::from_value(raw.clone()) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let role = parsed.role.trim().to_string();
        if role.is_empty() {
            continue;
        }
        let (text, image_urls) = text_from_content(&parsed.content);
        let reasoning = parsed.reasoning_content.clone().unwrap_or_default();
        let name = parsed.name.as_deref().unwrap_or("").trim();
        let display_content = parsed
            .display_content
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let display_reasoning = parsed
            .display_reasoning_content
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());

        if hydrate_try_special_cases(HydrateSpecialLine {
            parsed: &parsed,
            role: role.as_str(),
            name,
            text: text.as_str(),
            reasoning: reasoning.as_str(),
            display_content,
            display_reasoning,
            tool_result_call_ids: &tool_result_call_ids,
            base_ms,
            out: &mut out,
            t: &mut t,
        }) {
            continue;
        }

        let id = format!("h_{}_{}", base_ms, out.len());
        t = t.saturating_add(1);
        let is_user = role == "user";
        let display_text = display_content.unwrap_or(text.as_str()).to_string();
        // 助手 `display_content` 已在服务端合并 reasoning+正文并做展示层处理；勿再叠加 raw `reasoning_content`。
        let display_reasoning_text = if role == "assistant" && display_content.is_some() {
            display_reasoning.unwrap_or("").to_string()
        } else if !reasoning.trim().is_empty() {
            reasoning.clone()
        } else {
            display_reasoning.unwrap_or("").to_string()
        };
        out.push(StoredMessage {
            id,
            role,
            text: display_text,
            reasoning_text: display_reasoning_text,
            image_urls: if is_user { image_urls } else { vec![] },
            state: None,
            is_tool: false,
            tool_call_id: None,
            tool_name: None,
            created_at: t,
        });
    }
    out
}

/// WASM 入口：`base_ms` 为当前时间。
pub fn stored_messages_from_conversation_api(msgs: &[Value]) -> Vec<StoredMessage> {
    stored_messages_from_conversation_api_with_base(msgs, js_sys::Date::now() as i64)
}

/// 水合诊断：服务端 `messages` 非空但客户端解析结果为空（ParseFailed）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HydrationParseDiagnostics {
    pub raw_count: usize,
    pub revision: u64,
    pub conversation_id: String,
    pub sample_roles: Vec<String>,
}

impl HydrationParseDiagnostics {
    pub fn from_response(resp: &ConversationMessagesResponse) -> Self {
        Self {
            raw_count: resp.messages.len(),
            revision: resp.revision,
            conversation_id: resp.conversation_id.clone(),
            sample_roles: sample_roles_from_raw_messages(&resp.messages),
        }
    }
}

/// 从 raw 消息提取至多 3 个 `role` 样本（不含正文，供状态栏诊断）。
pub fn sample_roles_from_raw_messages(messages: &[Value]) -> Vec<String> {
    let mut out = Vec::new();
    for raw in messages {
        if out.len() >= 3 {
            break;
        }
        let role = raw
            .get("role")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        if let Some(r) = role {
            if !out.iter().any(|existing| existing == r) {
                out.push(r.to_string());
            }
        }
    }
    out
}

/// 解析水合消息；`ParseFailed` 时返回诊断（不覆盖本地时间线）。
pub fn stored_messages_for_hydration_or_parse_failed(
    resp: &ConversationMessagesResponse,
) -> Result<Vec<StoredMessage>, HydrationParseDiagnostics> {
    stored_messages_for_hydration_or_parse_failed_with_base(
        resp,
        js_sys::Date::now() as i64,
    )
}

pub fn stored_messages_for_hydration_or_parse_failed_with_base(
    resp: &ConversationMessagesResponse,
    base_ms: i64,
) -> Result<Vec<StoredMessage>, HydrationParseDiagnostics> {
    let msgs = stored_messages_from_conversation_api_with_base(&resp.messages, base_ms);
    if msgs.is_empty() && !resp.messages.is_empty() {
        Err(HydrationParseDiagnostics::from_response(resp))
    } else {
        Ok(msgs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn maps_user_text_and_skips_ui_sep() {
        let msgs = vec![
            json!({"role":"system","name":"crabmate_ui_sep","content":"x"}),
            json!({"role":"user","content":"hi"}),
        ];
        let out = stored_messages_from_conversation_api_with_base(&msgs, 0);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].role, "user");
        assert_eq!(out[0].text, "hi");
    }

    #[test]
    fn skips_first_turn_workspace_context_user() {
        let msgs = vec![
            json!({"role":"user","name":CRABMATE_FIRST_TURN_WORKSPACE_CONTEXT_NAME,"content":"profile"}),
            json!({"role":"user","content":"real"}),
        ];
        let out = stored_messages_from_conversation_api_with_base(&msgs, 0);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "real");
    }

    #[test]
    fn prefers_snapshot_display_fields_for_tool() {
        let msgs = vec![json!({
            "role":"tool",
            "name":"git_status",
            "content": r#"{"crabmate_tool":{"v":1,"name":"git_status","ok":true,"output":"x"}}"#,
            "display_content": "git_status · ok",
            "display_reasoning_content": "tool: git_status\nok"
        })];
        let out = stored_messages_from_conversation_api_with_base(&msgs, 0);
        assert_eq!(out.len(), 1);
        assert!(out[0].is_tool);
        assert_eq!(out[0].text, "git_status · ok");
        assert_eq!(out[0].reasoning_text, "tool: git_status\nok");
    }

    #[test]
    fn assistant_hydrate_ignores_raw_reasoning_when_display_content_present() {
        let plan_json = r#"{ "type": "agent_reply_plan", "version": 1, "steps": [ { "id": "x", "description": "d" } ] }"#;
        let msgs = vec![json!({
            "role": "assistant",
            "content": plan_json,
            "reasoning_content": plan_json,
            "display_content": "1. `x`: d",
        })];
        let out = stored_messages_from_conversation_api_with_base(&msgs, 0);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "1. `x`: d");
        assert!(
            out[0].reasoning_text.is_empty(),
            "raw reasoning_content must not hydrate when display_content is set: {:?}",
            out[0].reasoning_text
        );
    }

    /// 与 `GET /conversation/messages` 快照一致：助手同时下发 `display_content` 与 `display_reasoning_content`。
    #[test]
    fn assistant_hydrate_both_display_fields_prefers_display_reasoning_over_raw() {
        let plan_json = r#"{ "type": "agent_reply_plan", "version": 1, "steps": [ { "id": "x", "description": "d" } ] }"#;
        let msgs = vec![json!({
            "role": "assistant",
            "content": plan_json,
            "reasoning_content": plan_json,
            "display_content": "1. `x`: d",
            "display_reasoning_content": plan_json,
        })];
        let out = stored_messages_from_conversation_api_with_base(&msgs, 0);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "1. `x`: d");
        assert_eq!(
            out[0].reasoning_text, plan_json,
            "must hydrate display_reasoning_content, not fall back to raw reasoning_content"
        );
    }

    /// 对齐 chat_export_20260808：空 assistant.tool_calls + role=tool 不得拆成两张工具卡。
    #[test]
    fn skips_tool_call_stub_when_tool_result_present() {
        let msgs = vec![
            json!({"role":"user","content":"现在时间是什么"}),
            json!({
                "role":"assistant",
                "content":"",
                "tool_calls":[{
                    "id":"tc_time",
                    "type":"function",
                    "function":{"name":"get_current_time","arguments":"{}"}
                }]
            }),
            json!({
                "role":"tool",
                "name":"get_current_time",
                "tool_call_id":"tc_time",
                "content":"当前时间：2026-08-08 21:25:05"
            }),
            json!({"role":"assistant","content":"当前时间是 **2026-08-08 21:25:05**。"}),
        ];
        let out = stored_messages_from_conversation_api_with_base(&msgs, 0);
        let tools: Vec<_> = out.iter().filter(|m| m.is_tool).collect();
        assert_eq!(
            tools.len(),
            1,
            "roles={:?}",
            out.iter()
                .map(|m| (m.role.as_str(), m.is_tool, m.text.as_str()))
                .collect::<Vec<_>>()
        );
        assert!(!tools[0].text.starts_with("工具："));
        assert_eq!(tools[0].tool_call_id.as_deref(), Some("tc_time"));
    }

    #[test]
    fn keeps_orphan_tool_call_stub_without_result() {
        let msgs = vec![json!({
            "role":"assistant",
            "content":"",
            "tool_calls":[{
                "id":"tc_orphan",
                "type":"function",
                "function":{"name":"list_tree","arguments":"{}"}
            }]
        })];
        let out = stored_messages_from_conversation_api_with_base(&msgs, 0);
        assert_eq!(out.len(), 1);
        assert!(out[0].is_tool);
        assert!(out[0].text.contains("list_tree"));
        assert_eq!(out[0].tool_call_id.as_deref(), Some("tc_orphan"));
    }

    #[test]
    fn hydration_parse_failed_when_raw_non_empty_but_unmapped() {
        let resp = ConversationMessagesResponse {
            conversation_id: "c_parse_fail".into(),
            revision: 3,
            active_agent_role: None,
            active_session_mode: None,
            tiktoken_prompt_tokens: None,
            messages: vec![json!({"bad": true}), json!({"role": ""})],
            total_count: 2,
            window_start_index: 0,
            has_older: false,
        };
        let err =
            stored_messages_for_hydration_or_parse_failed_with_base(&resp, 0).unwrap_err();
        assert_eq!(err.raw_count, 2);
        assert_eq!(err.revision, 3);
        assert_eq!(err.conversation_id, "c_parse_fail");
        assert!(err.sample_roles.is_empty());
    }
}
