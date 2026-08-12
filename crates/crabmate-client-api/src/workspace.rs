//! `POST /workspace` 响应解析（纯 JSON；无 HTTP 客户端）。

use serde::Deserialize;
use serde_json::Value;

/// `GET /workspace` 摘要子集。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct WorkspaceInfo {
    pub path: String,
    #[serde(default)]
    pub error: Option<String>,
}

/// `POST /workspace` 在 HTTP 2xx 下 JSON 语义失败的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceSetErrorKind {
    /// `ok` 非 true，且 body 无可用 `error` 文案（调用方可换成本地化默认句）。
    RejectedWithoutDetail,
    /// 服务端提供了 `error` 字符串。
    RejectedWithDetail,
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

/// HTTP 非 2xx：优先 body `error`，否则 `HTTP {status}`。
#[must_use]
pub fn workspace_set_http_error_message(val: &Value, status: u16) -> String {
    val.get("error")
        .and_then(|e| e.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| format!("HTTP {status}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
    fn http_error_prefers_body() {
        assert_eq!(
            workspace_set_http_error_message(&json!({"error":"forbidden"}), 403),
            "forbidden"
        );
        assert_eq!(
            workspace_set_http_error_message(&json!({}), 502),
            "HTTP 502"
        );
    }
}
