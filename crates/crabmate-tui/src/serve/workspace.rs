//! `GET|POST /workspace`：查询 / 切换工作区根；`GET /workspace[?path=]` 目录列表；
//! `GET/POST /workspace/projects`：项目池查询与切换。

use serde::Deserialize;
use serde_json::{Value, json};

use crabmate_client_api::{
    WorkspaceSetErrorKind, classify_workspace_set_failure, parse_workspace_project_open_body,
    parse_workspace_set_ok_body, paths, percent_encode_unreserved,
};

use crate::serve::client::ServeClient;
use crate::serve::error::TermError;

pub use crabmate_client_api::{
    WorkspaceDirData, WorkspaceDirEntry, WorkspaceInfo, WorkspaceProjectsData, WorkspaceSetError,
};

/// 当前工作区根（`GET /workspace`）。
pub async fn fetch_workspace(client: &ServeClient) -> Result<WorkspaceInfo, TermError> {
    let info: WorkspaceInfo = client.get_json(paths::WORKSPACE).await?;
    if let Some(err) = info.error.as_deref().filter(|s| !s.is_empty()) {
        return Err(TermError::Message(err.to_string()));
    }
    Ok(info)
}

/// `GET /workspace[?path=]` 的请求路径（`rel` 为空取根；相对路径整体 percent-encode）。
#[must_use]
pub fn workspace_dir_path(rel: Option<&str>) -> String {
    match rel.map(str::trim).filter(|s| !s.is_empty()) {
        Some(r) => format!("{}?path={}", paths::WORKSPACE, percent_encode_unreserved(r)),
        None => paths::WORKSPACE.to_string(),
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

/// `GET /workspace/file?path=<相对路径>` 的端点（TUI 本地常量）。
///
/// 不进 `crabmate-client-api::paths`：那里的模块注释已写明 `/workspace/file*` 是单消费者
/// （Desktop 走自己的一套解析），共享一个只有 TUI 用的常量只是名义复用。
const WORKSPACE_FILE: &str = "/workspace/file";

/// 只读视图最多保留的行数。
///
/// 再多的内容终端里也不会有人滚到底，而整份文件（可能几十万行）留在内存里只是浪费；
/// 截断会在视图提示行里明说，不是静默丢弃。
pub const FILE_VIEW_MAX_LINES: usize = 20_000;

/// `GET /workspace/file` 的 UTF-8 JSON 体。
///
/// 字段与 `frontend/src/api/http.rs` 的 `WorkspaceFileReadData`（后端 `WorkspaceFileReadResponse`）
/// 对齐：`content` + 可选 `error`，**没有** `path` 回显 —— 标题就用请求时的相对路径。
/// `content` 在这里是 `Option`：二进制 / 超限时 serve 可能只给 `error`，缺失字段不该让整个
/// 反序列化失败（前端那侧是 `String`，因为它总带着 `error` 的语义走另一条分支）。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct WorkspaceFile {
    /// 文本内容；`None` = 服务端没给（通常同时给 `error`）。
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

impl WorkspaceFile {
    /// 200 体里给的错误（与 `WorkspaceDirData::error_text` 同一口径）。
    #[must_use]
    pub fn error_text(&self) -> Option<&str> {
        self.error.as_deref().filter(|s| !s.trim().is_empty())
    }
}

/// 文件读取请求路径（相对路径整体 percent-encode，与目录列表同一口径）。
#[must_use]
pub fn workspace_file_path(rel: &str) -> String {
    format!(
        "{}?path={}",
        WORKSPACE_FILE,
        percent_encode_unreserved(rel.trim())
    )
}

/// 读取工作区内的文本文件（`GET /workspace/file?path=`）。
///
/// 失败一律转成**面向用户**的一句话：二进制 / 超限 / 不存在都由 serve 在 `error` 里说明，
/// 客户端猜不出「为什么读不到」，所以不自己造诊断。
pub async fn fetch_workspace_file(
    client: &ServeClient,
    rel: &str,
) -> Result<WorkspaceFile, TermError> {
    let data: WorkspaceFile = client.get_json(&workspace_file_path(rel)).await?;
    if let Some(err) = data.error_text() {
        return Err(TermError::Message(err.to_string()));
    }
    if data.content.is_none() {
        return Err(TermError::Message(format!(
            "{rel}: 服务端没有返回内容（空响应，或该 serve 还没有 /workspace/file 路由）"
        )));
    }
    Ok(data)
}

/// 切换工作区根（`POST /workspace`，body.path）。空串表示恢复 serve 默认。
///
/// 返回 [`WorkspaceSetError`] 而不是 `TermError`：409 `WORKSPACE_ALREADY_SET`
/// （一实例一工作区）必须能被调用方分辨出来 —— 它不是网络错误，重试也不会成功。
pub async fn set_workspace(client: &ServeClient, path: &str) -> Result<String, WorkspaceSetError> {
    let body = json!({ "path": path });
    match client.post_json(paths::WORKSPACE, &body).await {
        Ok(val) => parse_workspace_set_ok_body(&val),
        // 非 2xx：体里可能是扁平 `code`（409 的那一支），也可能是纯文本/HTML（代理错误页）。
        // 解析失败就按 `Value::Null` 交给分类函数，文案回落成 `HTTP {status}`。
        Err(TermError::Http { status, body }) => {
            let val: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
            Err(classify_workspace_set_failure(status, &val))
        }
        Err(e) => Err(WorkspaceSetError {
            kind: WorkspaceSetErrorKind::Failed,
            message: e.to_string(),
        }),
    }
}

/// `GET /workspace/projects`：服务端项目池（enabled / pool_path / projects 名字）。
pub async fn fetch_workspace_projects(
    client: &ServeClient,
) -> Result<WorkspaceProjectsData, TermError> {
    client.get_json(paths::WORKSPACE_PROJECTS).await
}

/// `POST /workspace/projects`：切到项目池内已存在的项目（create=false）。
pub async fn switch_workspace_project(
    client: &ServeClient,
    name: &str,
) -> Result<String, TermError> {
    let body = json!({ "name": name, "create": false });
    let val = client.post_json(paths::WORKSPACE_PROJECTS, &body).await?;
    parse_workspace_project_open_body(&val).map_err(|e| TermError::Message(e.message))
}

#[cfg(test)]
mod tests {
    use crabmate_client_api::parse_workspace_set_ok_body;
    use serde_json::json;

    use super::{WorkspaceFile, workspace_dir_path};

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

    #[test]
    fn file_path_encodes_rel_like_the_dir_list() {
        use super::workspace_file_path;
        assert_eq!(
            workspace_file_path("src/lib.rs"),
            "/workspace/file?path=src%2Flib.rs"
        );
        // 相对路径整体编码；`..` 原样交给 serve 判，客户端不折叠
        assert_eq!(
            workspace_file_path("  ./a/../b.txt  "),
            "/workspace/file?path=.%2Fa%2F..%2Fb.txt"
        );
    }

    #[test]
    fn file_error_text_ignores_blank() {
        let blank = WorkspaceFile {
            error: Some("  ".to_string()),
            ..WorkspaceFile::default()
        };
        assert!(blank.error_text().is_none());
        let real = WorkspaceFile {
            error: Some("binary".to_string()),
            ..WorkspaceFile::default()
        };
        assert_eq!(real.error_text(), Some("binary"));
    }

    #[test]
    fn file_body_matches_the_frontend_shape() {
        // 与 frontend `WorkspaceFileReadData` 同一形状：只有 `content` + `error`，没有路径回显
        let ok: WorkspaceFile = serde_json::from_value(json!({
            "content": "fn main() {}\n",
            "error": null
        }))
        .expect("正常响应");
        assert_eq!(ok.content.as_deref(), Some("fn main() {}\n"));
        assert!(ok.error_text().is_none());

        // 二进制 / 超限：serve 只给 error，缺 content 也不能让整个反序列化失败
        let bad: WorkspaceFile = serde_json::from_value(json!({
            "content": null,
            "error": "二进制文件不支持预览"
        }))
        .expect("错误响应同样要能解析");
        assert_eq!(bad.error_text(), Some("二进制文件不支持预览"));
        assert!(bad.content.is_none());
    }
}
