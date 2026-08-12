//! Web 会话列表瘦模型与续聊 id（无本地 Web `id` 冒充）。

use serde::Deserialize;
use serde_json::Value;

/// 列表用的瘦会话行（忽略消息正文等大字段）。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SessionListItem {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub server_conversation_id: Option<String>,
}

/// 从 `sessions` 数组 JSON 解析瘦行；非数组返回空。
#[must_use]
pub fn parse_session_list_rows(sessions: &Value) -> Vec<SessionListItem> {
    let Some(arr) = sessions.as_array() else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|v| serde_json::from_value::<SessionListItem>(v.clone()).ok())
        .collect()
}

/// 续聊用的服务端 `conversation_id`；空 / 仅空白 → `None`（不可拿本地 Web `id` 冒充）。
#[must_use]
pub fn conversation_id_for_resume(server_conversation_id: Option<&str>) -> Option<&str> {
    server_conversation_id
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// [`SessionListItem`] 上的续聊 id。
#[must_use]
pub fn session_item_conversation_id_for_resume(item: &SessionListItem) -> Option<&str> {
    conversation_id_for_resume(item.server_conversation_id.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_rows_and_resume_id() {
        let v = json!([
            {"id":"s1","title":"hi","server_conversation_id":"c1"},
            {"id":"s2","title":"","extra":true},
            {"id":"s3","server_conversation_id":"  "}
        ]);
        let rows = parse_session_list_rows(&v);
        assert_eq!(rows.len(), 3);
        assert_eq!(
            session_item_conversation_id_for_resume(&rows[0]),
            Some("c1")
        );
        assert_eq!(session_item_conversation_id_for_resume(&rows[1]), None);
        assert_eq!(session_item_conversation_id_for_resume(&rows[2]), None);
        assert_eq!(conversation_id_for_resume(Some("  srv  ")), Some("srv"));
        assert_eq!(conversation_id_for_resume(None), None);
    }

    #[test]
    fn non_array_yields_empty() {
        assert!(parse_session_list_rows(&json!({"id":"x"})).is_empty());
    }
}
