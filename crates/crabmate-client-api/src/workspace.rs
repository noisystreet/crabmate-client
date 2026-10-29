//! `/workspace*` 响应视图与错误文案（client UI 投影；无 HTTP 客户端）。
//!
//! 镜像瘦身（0.5.2）例外：线形状权威是契约 `crabmate::cm_api_contract::workspace`，
//! 但 client 保留本地视图结构体——0.5.2 契约 DTO 仅 `Serialize + Deserialize`（无
//! `Debug`/`Default`，tui `UiState` / worker 事件枚举依赖），且 client 需要缺省容错
//! （`entries`/`name`/`path`/`error` 有 default，缺键不炸）。契约对齐由
//! `contract_roundtrip_*` 测试用契约类型钉住：契约字段改名/移位时测试即失败。
//! `parse_workspace_*_body`（client 错误分类）与 `messages::http_error_message`
//! （用户文案）不属于契约。

use serde::Deserialize;
use serde_json::Value;

use crate::messages::{http_error_message, top_level_error_code};

/// 服务端「一实例一工作区」冲突码：实例已绑定到别的路径。
///
/// 服务端把它放在错误体的**顶层** `code`（不是受保护 API 的嵌套 `error.code`），
/// 见 [`top_level_error_code`]。取值是契约，不要改写大小写。
pub const WORKSPACE_ALREADY_SET: &str = "WORKSPACE_ALREADY_SET";

/// `GET /workspace` 摘要子集。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct WorkspaceInfo {
    pub path: String,
    #[serde(default)]
    pub error: Option<String>,
}

/// `GET /workspace[?path=]` 目录列表中的单个条目。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct WorkspaceDirEntry {
    pub name: String,
    pub is_dir: bool,
}

/// `GET /workspace` / `GET /workspace?path=<相对路径>` 的目录列表响应子集
/// （与前端 `WorkspaceData` 同构：根或子目录均为 `{path, entries, error}`）。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct WorkspaceDirData {
    pub path: String,
    #[serde(default)]
    pub entries: Vec<WorkspaceDirEntry>,
    #[serde(default)]
    pub error: Option<String>,
}

impl WorkspaceDirData {
    /// HTTP 2xx 内服务端语义错误（`error` 非空）时返回其文案。
    #[must_use]
    pub fn error_text(&self) -> Option<&str> {
        self.error.as_deref().filter(|s| !s.trim().is_empty())
    }
}

/// `GET /workspace/projects`：服务端项目池是否启用与已有项目列表。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct WorkspaceProjectsData {
    pub enabled: bool,
    #[serde(default)]
    pub pool_path: Option<String>,
    #[serde(default)]
    pub projects: Vec<String>,
}

/// `POST /workspace/projects` 的切换响应子集。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct WorkspaceProjectOpenData {
    pub ok: bool,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub error: Option<String>,
}

/// 解析 **HTTP 已成功（2xx）** 的 `POST /workspace/projects` JSON：要求 `ok: true`，返回 `path`。
pub fn parse_workspace_project_open_body(val: &Value) -> Result<String, WorkspaceSetError> {
    if val.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        return match val
            .get("error")
            .and_then(|e| e.as_str())
            .filter(|s| !s.is_empty())
        {
            Some(msg) => Err(WorkspaceSetError {
                kind: WorkspaceSetErrorKind::RejectedWithDetail,
                message: msg.to_string(),
            }),
            None => Err(WorkspaceSetError {
                kind: WorkspaceSetErrorKind::RejectedWithoutDetail,
                message: "workspace project switch failed".into(),
            }),
        };
    }
    Ok(val
        .get("path")
        .and_then(|p| p.as_str())
        .unwrap_or("")
        .to_string())
}

/// `POST /workspace` 失败的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceSetErrorKind {
    /// `ok` 非 true，且 body 无可用 `error` 文案（调用方可换成本地化默认句）。
    RejectedWithoutDetail,
    /// 服务端提供了 `error` 字符串。
    RejectedWithDetail,
    /// HTTP 409 + 顶层 `WORKSPACE_ALREADY_SET`：实例已绑定别的路径（一实例一工作区）。
    ///
    /// 这一支**必须**与普通失败分开：它不是网络错误、重试也不会成功，
    /// 客户端该做的是提示「另起实例/另起 lane」而不是「再试一次」。
    AlreadySet,
    /// 其它 HTTP 非 2xx（403 越界 / 400 非法路径 / 传输失败）。
    Failed,
}

/// `POST /workspace` JSON 体解析失败。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceSetError {
    pub kind: WorkspaceSetErrorKind,
    pub message: String,
}

impl core::fmt::Display for WorkspaceSetError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for WorkspaceSetError {}

/// 解析 **HTTP 已成功（2xx）** 时的 `POST /workspace` JSON：要求 `ok: true`，返回 `path`。
pub fn parse_workspace_set_ok_body(val: &Value) -> Result<String, WorkspaceSetError> {
    if val.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        return match val
            .get("error")
            .and_then(|e| e.as_str())
            .filter(|s| !s.is_empty())
        {
            Some(msg) => Err(WorkspaceSetError {
                kind: WorkspaceSetErrorKind::RejectedWithDetail,
                message: msg.to_string(),
            }),
            None => Err(WorkspaceSetError {
                kind: WorkspaceSetErrorKind::RejectedWithoutDetail,
                message: "workspace set failed".into(),
            }),
        };
    }
    Ok(val
        .get("path")
        .and_then(|p| p.as_str())
        .unwrap_or("")
        .to_string())
}

/// 把**非 2xx** 的 `POST /workspace` 响应分类成可分支的失败。
///
/// `status` 是 HTTP 状态码，`val` 是响应体解析出的 JSON（体不是 JSON 时传 `Value::Null`）。
/// 只有「409 + 顶层 `WORKSPACE_ALREADY_SET`」落 [`WorkspaceSetErrorKind::AlreadySet`]；
/// 其余（403 越界 / 400 非法路径 / 传输失败 / 未来形状漂移）一律
/// [`WorkspaceSetErrorKind::Failed`]，文案取自 [`http_error_message`]，与服务端 `error`
/// 原文一致 —— 文案不是契约，这里只负责显示。
///
/// **不从文案里抠当前绑定路径**：要当前根就另外 `GET /workspace`。
#[must_use]
pub fn classify_workspace_set_failure(status: u16, val: &Value) -> WorkspaceSetError {
    let kind = if status == 409 && top_level_error_code(val) == Some(WORKSPACE_ALREADY_SET) {
        WorkspaceSetErrorKind::AlreadySet
    } else {
        WorkspaceSetErrorKind::Failed
    };
    WorkspaceSetError {
        kind,
        message: http_error_message(val, status),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn already_set_409_with_top_level_code_is_branchable() {
        let e = classify_workspace_set_failure(
            409,
            &json!({
                "ok": false,
                "code": WORKSPACE_ALREADY_SET,
                "error": "该实例已绑定工作区 /abs/p"
            }),
        );
        assert_eq!(e.kind, WorkspaceSetErrorKind::AlreadySet);
        // 文案原样透传：路径从服务端 `error` 直接展示，不做解析。
        assert_eq!(e.message, "该实例已绑定工作区 /abs/p");
    }

    /// 两个条件缺一不可：`409` 与顶层 `code` 都要对得上，否则退回普通失败（不猜）。
    #[test]
    fn already_set_requires_both_status_and_code() {
        // 409 但无 `code`：未来形状漂移时不假装认得出。
        let e = classify_workspace_set_failure(409, &json!({"ok": false, "error": "conflict"}));
        assert_eq!(e.kind, WorkspaceSetErrorKind::Failed);
        // 有 `code` 但状态码不是 409：不认。
        let e = classify_workspace_set_failure(
            400,
            &json!({"ok": false, "code": WORKSPACE_ALREADY_SET}),
        );
        assert_eq!(e.kind, WorkspaceSetErrorKind::Failed);
        // 嵌套形状（受保护 API）的 `code` 不是顶层 `code`：不认。
        let e =
            classify_workspace_set_failure(409, &json!({"error": {"code": WORKSPACE_ALREADY_SET}}));
        assert_eq!(e.kind, WorkspaceSetErrorKind::Failed);
    }

    /// 其它失败维持既有文案口径：服务端 `error` 原文，无文案时回落 `HTTP {status}`。
    #[test]
    fn other_failures_keep_the_existing_text() {
        let e = classify_workspace_set_failure(403, &json!({"ok": false, "error": "路径越界"}));
        assert_eq!(e.kind, WorkspaceSetErrorKind::Failed);
        assert_eq!(e.message, "路径越界");
        let e = classify_workspace_set_failure(502, &Value::Null);
        assert_eq!(e.kind, WorkspaceSetErrorKind::Failed);
        assert_eq!(e.message, "HTTP 502");
    }

    #[test]
    fn set_ok_returns_path() {
        let path = parse_workspace_set_ok_body(&json!({"ok":true,"path":"/tmp/ws"})).unwrap();
        assert_eq!(path, "/tmp/ws");
    }

    #[test]
    fn set_ok_false_uses_error() {
        let e = parse_workspace_set_ok_body(&json!({"ok":false,"error":"busy"})).unwrap_err();
        assert_eq!(e.kind, WorkspaceSetErrorKind::RejectedWithDetail);
        assert_eq!(e.message, "busy");
    }

    #[test]
    fn set_missing_ok_is_error() {
        let e = parse_workspace_set_ok_body(&json!({"path":"/tmp"})).unwrap_err();
        assert_eq!(e.kind, WorkspaceSetErrorKind::RejectedWithoutDetail);
    }

    #[test]
    fn dir_data_parses_entries_and_defaults() {
        let v = json!({
            "path": "/data/proj",
            "entries": [
                {"name": "src", "is_dir": true},
                {"name": "README.md", "is_dir": false}
            ]
        });
        let d: WorkspaceDirData = serde_json::from_value(v).unwrap();
        assert_eq!(d.path, "/data/proj");
        assert_eq!(d.entries.len(), 2);
        assert_eq!(d.entries[0].name, "src");
        assert!(d.entries[0].is_dir);
        assert!(!d.entries[1].is_dir);
        assert_eq!(d.error_text(), None);
    }

    #[test]
    fn dir_data_tolerates_missing_fields_and_surfaces_error() {
        let d: WorkspaceDirData = serde_json::from_value(json!({
            "path": "/x",
            "error": "   "
        }))
        .unwrap();
        assert!(d.entries.is_empty());
        assert_eq!(d.error_text(), None, "全空 error 视为无错误");

        let d: WorkspaceDirData =
            serde_json::from_value(json!({"path": "/x", "error": "busy"})).unwrap();
        assert_eq!(d.error_text(), Some("busy"));
    }

    #[test]
    fn projects_data_parses_pool() {
        let d: WorkspaceProjectsData = serde_json::from_value(json!({
            "enabled": true,
            "pool_path": "/data/pool",
            "projects": ["proj-a", "proj-b"]
        }))
        .unwrap();
        assert!(d.enabled);
        assert_eq!(d.pool_path.as_deref(), Some("/data/pool"));
        assert_eq!(d.projects, vec!["proj-a".to_string(), "proj-b".to_string()]);
        let d: WorkspaceProjectsData = serde_json::from_value(json!({"enabled": false})).unwrap();
        assert!(!d.enabled);
        assert!(d.projects.is_empty());
    }

    #[test]
    fn project_open_ok_returns_path() {
        let path = parse_workspace_project_open_body(&json!({"ok": true, "path": "/p/a"})).unwrap();
        assert_eq!(path, "/p/a");
    }

    #[test]
    fn project_open_false_uses_error() {
        let e = parse_workspace_project_open_body(&json!({"ok": false, "error": "no such"}))
            .unwrap_err();
        assert_eq!(e.kind, WorkspaceSetErrorKind::RejectedWithDetail);
        assert_eq!(e.message, "no such");
        let e = parse_workspace_project_open_body(&json!({"ok": false})).unwrap_err();
        assert_eq!(e.kind, WorkspaceSetErrorKind::RejectedWithoutDetail);
    }

    // —— 契约对齐钉（0.5.2 `cm_api_contract::workspace`）——
    // client 视图结构体与契约线形状 1:1；契约字段改名/移位时以下测试失败。

    #[test]
    fn contract_roundtrip_dir_data() {
        use crabmate::cm_api_contract::workspace as ws;
        let resp = ws::WorkspaceResponse {
            path: "/data/proj".into(),
            entries: vec![ws::WorkspaceEntry {
                name: "src".into(),
                is_dir: true,
            }],
            error: None,
        };
        let d: WorkspaceDirData = serde_json::from_value(serde_json::to_value(&resp).unwrap())
            .expect("contract dir response parses into client view");
        assert_eq!(d.path, "/data/proj");
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].name, "src");
        assert!(d.entries[0].is_dir);
        assert_eq!(d.error_text(), None);

        let resp = ws::WorkspaceResponse {
            path: "/x".into(),
            entries: vec![],
            error: Some("busy".into()),
        };
        let d: WorkspaceDirData = serde_json::from_value(serde_json::to_value(&resp).unwrap())
            .expect("contract dir response with error parses");
        assert_eq!(d.error_text(), Some("busy"));
    }

    #[test]
    fn contract_roundtrip_projects_data() {
        use crabmate::cm_api_contract::workspace as ws;
        let resp = ws::WorkspaceProjectsListResponse {
            enabled: true,
            pool_path: Some("/data/pool".into()),
            projects: vec!["proj-a".into(), "proj-b".into()],
        };
        let d: WorkspaceProjectsData = serde_json::from_value(serde_json::to_value(&resp).unwrap())
            .expect("contract projects response parses into client view");
        assert!(d.enabled);
        assert_eq!(d.pool_path.as_deref(), Some("/data/pool"));
        assert_eq!(d.projects, vec!["proj-a".to_string(), "proj-b".to_string()]);

        // 契约对空 projects / 无 pool_path 刻意 skip 出站：client 缺省容错补空。
        let resp = ws::WorkspaceProjectsListResponse {
            enabled: false,
            pool_path: None,
            projects: vec![],
        };
        let d: WorkspaceProjectsData = serde_json::from_value(serde_json::to_value(&resp).unwrap())
            .expect("contract projects response with skips parses");
        assert!(!d.enabled);
        assert!(d.pool_path.is_none());
        assert!(d.projects.is_empty());
    }

    #[test]
    fn contract_roundtrip_project_open_data() {
        use crabmate::cm_api_contract::workspace as ws;
        let resp = ws::WorkspaceProjectPostResponse {
            ok: true,
            name: "a".into(),
            path: "/p/a".into(),
            error: None,
        };
        let d: WorkspaceProjectOpenData =
            serde_json::from_value(serde_json::to_value(&resp).unwrap())
                .expect("contract project open response parses into client view");
        assert!(d.ok);
        assert_eq!(d.name, "a");
        assert_eq!(d.path, "/p/a");
        assert!(d.error.is_none());
    }
}
