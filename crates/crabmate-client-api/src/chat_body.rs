//! `POST /chat/stream` **核心**字段（message / `client_sse_protocol` / conversation_id / approval_session_id）。
//!
//! 图像、`stream_resume`、`client_llm`、温度等仍由各端自行追加。
//! `client_sse_protocol` 取值由调用方传入（通常为 `crabmate::cm_sse_protocol::SSE_PROTOCOL_VERSION`），
//! 本 crate **不**依赖 sse-protocol，以免拖进契约 git 依赖。

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
}
