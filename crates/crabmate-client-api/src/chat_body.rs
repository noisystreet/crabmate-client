//! `POST /chat/stream` **核心**字段（message / `client_sse_protocol` / conversation_id / approval_session_id）
//! 与**可选块取值规则**（`client_llm` 子字段、顶层 `temperature` / `readonly_tool_ttl_cache_secs`）。
//!
//! 图像、`stream_resume`、`llm_thinking_mode`、`executor_llm` 等仍由各端自行追加
//! （取值来源是本端存储 / 本端语义，见 `docs/design/client_shared_logic.md` §4.5）。
//! 核心键集以契约
//! `crabmate::cm_api_contract::chat_keys::CHAT_REQUEST_BODY_ALLOWED_KEYS` 为单一来源（测试钉住）；
//! 出站仍保留薄 `json!` builder（0.5.2 `ChatRequestBodyWire` 是入站线型，
//! client 出站无现成 builder 可复用）。
//! `client_sse_protocol` 取值由调用方传入（通常为 `crabmate::cm_sse_protocol::SSE_PROTOCOL_VERSION`）。

use serde_json::{Value, json};

/// 核心字段入参。
#[derive(Debug, Clone, Copy)]
pub struct ChatStreamCoreFields<'a> {
    pub message: &'a str,
    /// 与 serve / sse-protocol 钉死的协议版本（如 `SSE_PROTOCOL_VERSION`）。
    pub client_sse_protocol: u8,
    pub approval_session_id: Option<&'a str>,
    pub conversation_id: Option<&'a str>,
}

/// 仅含核心字段的 JSON 对象；空的 conversation / approval id 省略（不写 `null`）。
#[must_use]
pub fn build_chat_stream_core_body(fields: ChatStreamCoreFields<'_>) -> Value {
    let mut body = json!({
        "message": fields.message,
        "client_sse_protocol": fields.client_sse_protocol,
    });
    apply_optional_id(&mut body, "approval_session_id", fields.approval_session_id);
    apply_optional_id(&mut body, "conversation_id", fields.conversation_id);
    body
}

/// 将核心字段写入已有 `body`（覆盖同名键；空 id 删除键以免残留 `null`）。
pub fn merge_chat_stream_core_fields(body: &mut Value, fields: ChatStreamCoreFields<'_>) {
    let obj = body.as_object_mut();
    let Some(map) = obj else {
        *body = build_chat_stream_core_body(fields);
        return;
    };
    map.insert("message".into(), json!(fields.message));
    map.insert(
        "client_sse_protocol".into(),
        json!(fields.client_sse_protocol),
    );
    apply_optional_id(body, "approval_session_id", fields.approval_session_id);
    apply_optional_id(body, "conversation_id", fields.conversation_id);
}

fn apply_optional_id(body: &mut Value, key: &str, raw: Option<&str>) {
    let Some(map) = body.as_object_mut() else {
        return;
    };
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        Some(id) => {
            map.insert(key.into(), json!(id));
        }
        None => {
            map.remove(key);
        }
    }
}

/// 可选字符串键的取值规则：`trim` 后非空才写入（写入 `trim` 后的值）。
///
/// 用于 `client_llm` / `executor_llm` 子字段与 `agent_role` / `session_mode` 等可省略键；
/// 空值不写键（而非写 `null`），避免 serve 侧把空串当成显式覆盖。
pub fn insert_trimmed_str(
    map: &mut serde_json::Map<String, Value>,
    key: &str,
    value: Option<&str>,
) {
    if let Some(v) = value.map(str::trim).filter(|s| !s.is_empty()) {
        map.insert(key.to_string(), Value::String(v.to_string()));
    }
}

/// `client_llm.llm_context_tokens`：仅 `trim` 后为非空数字且 `> 0` 时发送。
pub fn llm_context_tokens_for_chat_body(raw: Option<&str>) -> Option<u64> {
    raw.map(str::trim)
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|n| *n > 0)
}

/// 顶层 `temperature`：仅有限且落在 `0.0..=2.0` 时才发送（越界 / `NaN` / `inf` 省略）。
pub fn temperature_for_chat_body(raw: Option<f64>) -> Option<f64> {
    raw.filter(|t| t.is_finite() && (0.0..=2.0).contains(t))
}

/// 顶层 `readonly_tool_ttl_cache_secs`：关闭缓存时发 `0`；跟随 serve 时省略键。
#[must_use]
pub fn readonly_tool_ttl_cache_secs_for_chat_body(follow_server: bool) -> Option<u64> {
    (!follow_server).then_some(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_body_pins_protocol_and_omits_empty_ids() {
        let v = build_chat_stream_core_body(ChatStreamCoreFields {
            message: "hi",
            client_sse_protocol: 2,
            approval_session_id: Some("appr_1"),
            conversation_id: Some("  "),
        });
        assert_eq!(v["message"], "hi");
        assert_eq!(v["client_sse_protocol"], 2);
        assert_eq!(v["approval_session_id"], "appr_1");
        assert!(v.get("conversation_id").is_none());
    }

    #[test]
    fn merge_overwrites_and_clears_nullish() {
        let mut body = json!({
            "message": "old",
            "conversation_id": null,
            "agent_role": "default",
        });
        merge_chat_stream_core_fields(
            &mut body,
            ChatStreamCoreFields {
                message: "new",
                client_sse_protocol: 2,
                approval_session_id: None,
                conversation_id: Some("c1"),
            },
        );
        assert_eq!(body["message"], "new");
        assert_eq!(body["client_sse_protocol"], 2);
        assert_eq!(body["conversation_id"], "c1");
        assert!(body.get("approval_session_id").is_none());
        assert_eq!(body["agent_role"], "default");
    }

    #[test]
    fn core_keys_are_within_contract_allowed_keys() {
        // 0.5.2：核心键集必须是契约 `CHAT_REQUEST_BODY_ALLOWED_KEYS` 的子集；
        // 契约改名 / 收窄白名单时此测试失败。
        let v = build_chat_stream_core_body(ChatStreamCoreFields {
            message: "hi",
            client_sse_protocol: 2,
            approval_session_id: Some("appr_1"),
            conversation_id: Some("c1"),
        });
        let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(keys.len(), 4);
        for k in keys {
            assert!(
                crabmate::cm_api_contract::chat_keys::CHAT_REQUEST_BODY_ALLOWED_KEYS.contains(&k),
                "key `{k}` not in contract CHAT_REQUEST_BODY_ALLOWED_KEYS"
            );
        }
    }

    #[test]
    fn insert_trimmed_str_skips_blank_and_writes_trimmed() {
        let mut map = serde_json::Map::new();
        insert_trimmed_str(&mut map, "agent_role", Some("  default  "));
        insert_trimmed_str(&mut map, "session_mode", Some("   "));
        insert_trimmed_str(&mut map, "client_llm", None);
        assert_eq!(map["agent_role"], "default");
        assert!(map.get("session_mode").is_none());
        assert!(map.get("client_llm").is_none());
    }

    #[test]
    fn llm_context_tokens_only_positive_number() {
        assert_eq!(llm_context_tokens_for_chat_body(Some(" 8192 ")), Some(8192));
        assert_eq!(llm_context_tokens_for_chat_body(Some("0")), None);
        assert_eq!(llm_context_tokens_for_chat_body(Some("-1")), None);
        assert_eq!(llm_context_tokens_for_chat_body(Some("abc")), None);
        assert_eq!(llm_context_tokens_for_chat_body(Some("")), None);
        assert_eq!(llm_context_tokens_for_chat_body(None), None);
    }

    #[test]
    fn temperature_only_finite_in_range() {
        assert_eq!(temperature_for_chat_body(Some(0.0)), Some(0.0));
        assert_eq!(temperature_for_chat_body(Some(2.0)), Some(2.0));
        assert_eq!(temperature_for_chat_body(Some(1.25)), Some(1.25));
        assert_eq!(temperature_for_chat_body(Some(-0.1)), None);
        assert_eq!(temperature_for_chat_body(Some(2.1)), None);
        assert_eq!(temperature_for_chat_body(Some(f64::NAN)), None);
        assert_eq!(temperature_for_chat_body(Some(f64::INFINITY)), None);
        assert_eq!(temperature_for_chat_body(None), None);
    }

    #[test]
    fn readonly_ttl_sends_zero_only_when_not_following_server() {
        assert_eq!(readonly_tool_ttl_cache_secs_for_chat_body(false), Some(0));
        assert_eq!(readonly_tool_ttl_cache_secs_for_chat_body(true), None);
    }

    #[test]
    fn optional_block_keys_are_within_contract_allowed_keys() {
        // 0.5.2：可选块产出的键也必须是契约白名单子集；契约改名 / 收窄时此测试失败。
        let mut map = serde_json::Map::new();
        insert_trimmed_str(&mut map, "agent_role", Some("default"));
        insert_trimmed_str(&mut map, "session_mode", Some("agent"));
        insert_trimmed_str(&mut map, "temperature", Some("0.7"));
        insert_trimmed_str(&mut map, "readonly_tool_ttl_cache_secs", Some("0"));
        for k in map.keys() {
            assert!(
                crabmate::cm_api_contract::chat_keys::CHAT_REQUEST_BODY_ALLOWED_KEYS
                    .contains(&k.as_str()),
                "key `{k}` not in contract CHAT_REQUEST_BODY_ALLOWED_KEYS"
            );
        }
    }
}
