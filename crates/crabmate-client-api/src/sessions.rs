//! Web 会话列表续聊 id（无本地 Web `id` 冒充）。
//!
//! 镜像瘦身（0.5.2）：列表瘦行直接使用契约 [`SessionListRow`]（含 `parse_rows` 宽容
//! 解析语义），client 仅保留防冒充的续聊 id 归一化逻辑。

pub use crabmate::cm_api_contract::SessionListRow;

/// 续聊用的服务端 `conversation_id`；空 / 仅空白 → `None`（不可拿本地 Web `id` 冒充）。
#[must_use]
pub fn conversation_id_for_resume(server_conversation_id: Option<&str>) -> Option<&str> {
    server_conversation_id
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// [`SessionListRow`] 上的续聊 id。
#[must_use]
pub fn session_row_conversation_id_for_resume(row: &SessionListRow) -> Option<&str> {
    conversation_id_for_resume(row.server_conversation_id.as_deref())
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
        let rows = SessionListRow::parse_rows(&v);
        assert_eq!(rows.len(), 3);
        assert_eq!(session_row_conversation_id_for_resume(&rows[0]), Some("c1"));
        assert_eq!(session_row_conversation_id_for_resume(&rows[1]), None);
        assert_eq!(session_row_conversation_id_for_resume(&rows[2]), None);
        assert_eq!(conversation_id_for_resume(Some("  srv  ")), Some("srv"));
        assert_eq!(conversation_id_for_resume(None), None);
    }

    #[test]
    fn non_array_yields_empty() {
        assert!(SessionListRow::parse_rows(&json!({"id":"x"})).is_empty());
    }
}
