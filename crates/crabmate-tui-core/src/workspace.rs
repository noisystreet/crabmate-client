//! `GET|POST /workspace`：查询 / 切换工作区根；`GET /workspace[?path=]` 目录列表。

use serde_json::json;

use crabmate_client_api::{parse_workspace_set_ok_body, percent_encode_unreserved};

use crate::client::ServeClient;
use crate::error::TermError;

pub use crabmate_client_api::{WorkspaceDirData, WorkspaceDirEntry, WorkspaceInfo};

/// 当前工作区根（`GET /workspace`）。
pub async fn fetch_workspace(client: &ServeClient) -> Result<WorkspaceInfo, TermError> {
    let info: WorkspaceInfo = client.get_json("/workspace").await?;
    if let Some(err) = info.error.as_deref().filter(|s| !s.is_empty()) {
        return Err(TermError::Message(err.to_string()));
    }
    Ok(info)
}

/// `GET /workspace[?path=]` 的请求路径（`rel` 为空取根；相对路径整体 percent-encode）。
#[must_use]
pub fn workspace_dir_path(rel: Option<&str>) -> String {
    match rel.map(str::trim).filter(|s| !s.is_empty()) {
        Some(r) => format!("/workspace?path={}", percent_encode_unreserved(r)),
        None => "/workspace".to_string(),
    }
}

/// 工作区目录列表（`GET /workspace[?path=<相对路径>]`）：`rel` 为空取根目录。
/// 与前端工作区侧栏同构——展开子目录即带路径再拉一次。
pub async fn fetch_workspace_dir(
    client: &ServeClient,
    rel: Option<&str>,
) -> Result<WorkspaceDirData, TermError> {
    let data: WorkspaceDirData = client.get_json(&workspace_dir_path(rel)).await?;
    if let Some(err) = data.error_text() {
        return Err(TermError::Message(err.to_string()));
    }
    Ok(data)
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

    use super::workspace_dir_path;

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

    #[test]
    fn dir_path_root_when_blank() {
        assert_eq!(workspace_dir_path(None), "/workspace");
        assert_eq!(workspace_dir_path(Some("   ")), "/workspace");
    }

    #[test]
    fn dir_path_encodes_rel_subdir() {
        assert_eq!(
            workspace_dir_path(Some("src/lib")),
            "/workspace?path=src%2Flib"
        );
        assert_eq!(
            workspace_dir_path(Some("笔记/说明.md")),
            "/workspace?path=%E7%AC%94%E8%AE%B0%2F%E8%AF%B4%E6%98%8E.md"
        );
    }
}
