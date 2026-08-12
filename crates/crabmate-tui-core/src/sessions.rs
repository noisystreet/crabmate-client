//! `GET /user-data/workspaces/current/sessions`：与 WASM 同源的 Web 会话列表。

use serde::Deserialize;
use serde_json::Value;

use crabmate_client_api::{parse_session_list_rows, session_item_conversation_id_for_resume};

use crate::client::ServeClient;
use crate::error::TermError;

pub use crabmate_client_api::SessionListItem;

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
        sessions: parse_session_list_rows(&dto.sessions),
        active_session_id: dto.active_session_id,
    })
}

/// 续聊用的服务端 `conversation_id`；仅本地 Web id 时返回 `None`（不可拿去 `/conv use`）。
#[must_use]
pub fn conversation_id_for_resume(item: &SessionListItem) -> Option<&str> {
    session_item_conversation_id_for_resume(item)
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
        let rows = parse_session_list_rows(&v);
        assert_eq!(rows.len(), 2);
        assert_eq!(conversation_id_for_resume(&rows[0]), Some("c1"));
        assert_eq!(conversation_id_for_resume(&rows[1]), None);
    }
}
