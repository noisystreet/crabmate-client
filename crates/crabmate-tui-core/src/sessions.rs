//! `GET /user-data/workspaces/current/sessions`：与 WASM 同源的 Web 会话列表。

use serde::Deserialize;
use serde_json::Value;

use crate::client::ServeClient;
use crate::error::TermError;

/// 列表用的瘦会话行（忽略消息正文等大字段）。
#[derive(Debug, Clone, Deserialize)]
pub struct SessionListItem {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub server_conversation_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WebSessionsList {
    pub sessions: Vec<SessionListItem>,
    pub active_session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WebSessionsDto {
    #[serde(default)]
    sessions: Value,
    #[serde(default)]
    active_session_id: Option<String>,
}

/// 拉取当前工作区绑定的 Web 会话快照。
pub async fn fetch_web_sessions(client: &ServeClient) -> Result<WebSessionsList, TermError> {
    let dto: WebSessionsDto = client
        .get_json("/user-data/workspaces/current/sessions")
        .await?;
    Ok(WebSessionsList {
        sessions: parse_session_rows(&dto.sessions),
        active_session_id: dto.active_session_id,
    })
}

fn parse_session_rows(sessions: &Value) -> Vec<SessionListItem> {
    let Some(arr) = sessions.as_array() else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|v| serde_json::from_value::<SessionListItem>(v.clone()).ok())
        .collect()
}

/// 续聊用的服务端 `conversation_id`；仅本地 Web id 时返回 `None`（不可拿去 `/conv use`）。
#[must_use]
pub fn conversation_id_for_resume(item: &SessionListItem) -> Option<&str> {
    item.server_conversation_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_rows() {
        let v = json!([
            {"id":"s1","title":"hi","server_conversation_id":"c1"},
            {"id":"s2","title":"","extra":true}
        ]);
        let rows = parse_session_rows(&v);
        assert_eq!(rows.len(), 2);
        assert_eq!(conversation_id_for_resume(&rows[0]), Some("c1"));
        assert_eq!(conversation_id_for_resume(&rows[1]), None);
    }
}
