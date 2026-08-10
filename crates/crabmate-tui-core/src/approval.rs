//! 命令审批：与 Web `POST /chat/approval` 决策字符串对齐。

use crate::error::TermError;

/// SSE `command_approval` 控制面请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandApprovalRequest {
    pub command: String,
    pub args: String,
    pub allowlist_key: Option<String>,
}

/// 投递给 `POST /chat/approval` 的决策。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    Deny,
    AllowOnce,
    AllowAlways,
}

impl ApprovalDecision {
    #[must_use]
    pub fn as_api_str(self) -> &'static str {
        match self {
            Self::Deny => "deny",
            Self::AllowOnce => "allow_once",
            Self::AllowAlways => "allow_always",
        }
    }
}

/// 同步审批闸门：在 SSE 消费循环中调用（服务端会阻塞等待 `POST /chat/approval`）。
pub trait ApprovalGate {
    fn decide(&mut self, req: &CommandApprovalRequest) -> Result<ApprovalDecision, TermError>;
}

/// `--yes`：非白名单命令一律 `allow_once`（执行仍在 serve）。
#[derive(Debug, Default, Clone, Copy)]
pub struct AutoAllowOnce;

impl ApprovalGate for AutoAllowOnce {
    fn decide(&mut self, _req: &CommandApprovalRequest) -> Result<ApprovalDecision, TermError> {
        Ok(ApprovalDecision::AllowOnce)
    }
}

/// 生成合法 `approval_session_id`（字母数字 / `-_.:`，≤128）。
#[must_use]
pub fn new_approval_session_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("tui_{ns}_{}_{seq}", std::process::id())
}

/// 从 AG-UI CUSTOM `command_approval` 的 `data` 对象解析请求。
pub fn parse_command_approval_data(data: &serde_json::Value) -> CommandApprovalRequest {
    CommandApprovalRequest {
        command: data
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        args: data
            .get("args")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        allowlist_key: data
            .get("allowlistKey")
            .and_then(|v| v.as_str())
            .map(str::to_string),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_approval_data() {
        let data = json!({"command":"rm","args":"-rf","allowlistKey":"rm"});
        let req = parse_command_approval_data(&data);
        assert_eq!(req.command, "rm");
        assert_eq!(req.args, "-rf");
        assert_eq!(req.allowlist_key.as_deref(), Some("rm"));
    }

    #[test]
    fn session_id_shape() {
        let id = new_approval_session_id();
        assert!(id.starts_with("tui_"));
        assert!(id.len() <= 128);
        assert!(
            id.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
        );
    }

    #[test]
    fn session_ids_differ_quickly() {
        let a = new_approval_session_id();
        let b = new_approval_session_id();
        assert_ne!(a, b);
    }
}
