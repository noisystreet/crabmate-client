//! `GET|POST /workspace`：查询 / 切换工作区根。

use serde::Deserialize;
use serde_json::json;

use crate::client::ServeClient;
use crate::error::TermError;

/// `GET /workspace` 摘要（P3 只需路径与错误）。
#[derive(Debug, Clone, Deserialize)]
pub struct WorkspaceInfo {
    pub path: String,
    #[serde(default)]
    pub error: Option<String>,
}

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
    parse_workspace_set_response(&val)
}

fn parse_workspace_set_response(val: &serde_json::Value) -> Result<String, TermError> {
    if val.get("ok").and_then(|v| v.as_bool()) == Some(false) {
        let err = val
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("workspace set failed");
        return Err(TermError::Message(err.to_string()));
    }
    Ok(val
        .get("path")
        .and_then(|p| p.as_str())
        .unwrap_or("")
        .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_set_ok() {
        let path = parse_workspace_set_response(&json!({"ok":true,"path":"/tmp/ws"})).unwrap();
        assert_eq!(path, "/tmp/ws");
    }

    #[test]
    fn parses_set_err() {
        let e = parse_workspace_set_response(&json!({"ok":false,"error":"busy"})).unwrap_err();
        assert!(matches!(e, TermError::Message(m) if m == "busy"));
    }
}
