//! 命令审批：闸门与会话 id 生成；类型/解析委托 [`crabmate_client_api::approval`]。

use crabmate_client_api::approval_session_id_is_valid;

use crate::error::TermError;

pub use crabmate_client_api::{
    ApprovalDecision, CommandApprovalRequest, parse_command_approval_data,
};

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

/// 生成合法 `approval_session_id`（字母数字 / `-_.:`，≤128；前缀 `tui_`）。
#[must_use]
pub fn new_approval_session_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let id = format!("tui_{ns}_{}_{seq}", std::process::id());
    debug_assert!(approval_session_id_is_valid(&id));
    id
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_via_shared() {
        let data = json!({"command":"rm","args":"-rf","allowlistKey":"rm"});
        let req = parse_command_approval_data(&data);
        assert_eq!(req.command, "rm");
        assert_eq!(req.allowlist_key.as_deref(), Some("rm"));
    }

    #[test]
    fn session_id_shape() {
        let id = new_approval_session_id();
        assert!(id.starts_with("tui_"));
        assert!(approval_session_id_is_valid(&id));
    }

    #[test]
    fn session_ids_differ_quickly() {
        let a = new_approval_session_id();
        let b = new_approval_session_id();
        assert_ne!(a, b);
    }

    #[test]
    fn decision_api_str_aligned() {
        assert_eq!(ApprovalDecision::Deny.as_api_str(), "deny");
        assert_eq!(ApprovalDecision::AllowOnce.as_api_str(), "allow_once");
        assert_eq!(ApprovalDecision::AllowAlways.as_api_str(), "allow_always");
    }
}
