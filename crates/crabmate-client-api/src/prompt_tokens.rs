//! tiktoken 用量快照（SSE 控制面 / 会话水合共用 DTO；无 IO）。
//!
//! 与 Server `conversation_saved` / `stream_ended` / `GET /conversation/messages`
//! 的 `tiktokenPromptTokens` 子对象同形；字段名为 snake_case。

use serde::Deserialize;
use serde_json::Value;

/// `tiktokenPromptTokens` 快照。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TiktokenPromptTokensSnapshot {
    pub prompt_tokens: u32,
    pub tiktoken_model: String,
    #[serde(default)]
    pub used_input_tokens: Option<u32>,
    #[serde(default)]
    pub max_input_tokens: Option<u32>,
    #[serde(default)]
    pub reserved_output_tokens: Option<u32>,
    #[serde(default)]
    pub message_tokens: Option<u32>,
    #[serde(default)]
    pub tool_schema_tokens: Option<u32>,
    #[serde(default)]
    pub attachment_tokens: Option<u32>,
    #[serde(default)]
    pub counting_source: Option<String>,
    #[serde(default)]
    pub provider_input_tokens: Option<u64>,
}

/// 从 `conversation_saved` / `stream_ended` 等控制面 JSON 子对象解析。
pub fn parse_tiktoken_prompt_tokens_value(v: &Value) -> Option<TiktokenPromptTokensSnapshot> {
    serde_json::from_value(v.clone()).ok()
}

/// 从 AG-UI 事件根或 `CUSTOM conversation_saved` 的 `data` 中读取可选 tiktoken。
///
/// 同时接受 camelCase（`tiktokenPromptTokens`）与 snake_case（`tiktoken_prompt_tokens`）。
#[must_use]
pub fn tiktoken_from_ag_ui_object(obj: &Value) -> Option<TiktokenPromptTokensSnapshot> {
    obj.get("tiktokenPromptTokens")
        .or_else(|| obj.get("tiktoken_prompt_tokens"))
        .and_then(parse_tiktoken_prompt_tokens_value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_camel_case_and_snake_case_keys() {
        let camel = json!({
            "tiktokenPromptTokens": {
                "prompt_tokens": 1200,
                "tiktoken_model": "gpt-4o",
                "used_input_tokens": 1500,
                "max_input_tokens": 9000,
                "counting_source": "provider_usage",
                "provider_input_tokens": 1500
            }
        });
        let snake = json!({
            "tiktoken_prompt_tokens": {
                "prompt_tokens": 99,
                "tiktoken_model": "gpt-4"
            }
        });
        let a = tiktoken_from_ag_ui_object(&camel).expect("camel");
        assert_eq!(a.prompt_tokens, 1200);
        assert_eq!(a.tiktoken_model, "gpt-4o");
        assert_eq!(a.used_input_tokens, Some(1500));
        assert_eq!(a.max_input_tokens, Some(9000));
        assert_eq!(a.counting_source.as_deref(), Some("provider_usage"));
        let b = tiktoken_from_ag_ui_object(&snake).expect("snake");
        assert_eq!(b.prompt_tokens, 99);
        assert!(tiktoken_from_ag_ui_object(&json!({"revision": 1})).is_none());
        assert!(tiktoken_from_ag_ui_object(&json!({"tiktokenPromptTokens": null})).is_none());
    }

    /// 快照本身即 `tiktokenPromptTokens` 子对象形状（与 `GET /conversation/messages` 同源）。
    #[test]
    fn parses_snapshot_directly() {
        let v = json!({"prompt_tokens": 7, "tiktoken_model": "gpt-4o-mini"});
        let snap = parse_tiktoken_prompt_tokens_value(&v).expect("snapshot");
        assert_eq!(snap.prompt_tokens, 7);
        assert_eq!(snap.tiktoken_model, "gpt-4o-mini");
        assert!(parse_tiktoken_prompt_tokens_value(&json!(null)).is_none());
    }
}
