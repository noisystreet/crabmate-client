//! `GET|POST /workspace`：查询 / 切换工作区根。

use serde_json::json;

use crabmate_client_api::parse_workspace_set_ok_body;

use crate::client::ServeClient;
use crate::error::TermError;

pub use crabmate_client_api::WorkspaceInfo;

/// 当前工作区根（`GET /workspace`）。
pub async fn fetch_workspace(client: &ServeClient) -> Result<WorkspaceInfo, TermError> {
    let info: WorkspaceInfo = client.get_json("/workspace").await?;
    if let Some(err) = info.error.as_deref().filter(|s| !s.is_empty()) {
        return Err(TermError::Message(err.to_string()));
    }
    Ok(info)
}

/// 切换工作区根（`POST /workspace`，body.path）。空串表示恢复 serve 默认。
pub async fn set_workspace(client: &ServeClient, path: &str) -> Result<String, TermError> {
    let body = json!({ "path": path });
    let val = client.post_json("/workspace", &body).await?;
    parse_workspace_set_ok_body(&val).map_err(|e| TermError::Message(e.message))
}

#[cfg(test)]
mod tests {
    use crabmate_client_api::parse_workspace_set_ok_body;
    use serde_json::json;

    #[test]
    fn parses_set_ok() {
        let path = parse_workspace_set_ok_body(&json!({"ok":true,"path":"/tmp/ws"})).unwrap();
        assert_eq!(path, "/tmp/ws");
    }

    #[test]
    fn parses_set_err() {
        let e = parse_workspace_set_ok_body(&json!({"ok":false,"error":"busy"})).unwrap_err();
        assert_eq!(e.message, "busy");
    }
}
