//! 命令审批：决策串、`command_approval` data 解析、`POST /chat/approval` body 形状。

use serde::Serialize;
use serde_json::Value;

/// SSE `command_approval` 控制面请求（字段名与 AG-UI CUSTOM `data` 对齐）。
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
    /// 服务端契约字符串。
    #[must_use]
    pub fn as_api_str(self) -> &'static str {
        match self {
            Self::Deny => "deny",
            Self::AllowOnce => "allow_once",
            Self::AllowAlways => "allow_always",
        }
    }

    /// 解析决策串；未知值返回 `None`。
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "deny" => Some(Self::Deny),
            "allow_once" => Some(Self::AllowOnce),
            "allow_always" => Some(Self::AllowAlways),
            _ => None,
        }
    }
}

/// `POST /chat/approval` JSON body。
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ApprovalPostBody<'a> {
    pub approval_session_id: &'a str,
    pub decision: &'a str,
}

impl<'a> ApprovalPostBody<'a> {
    #[must_use]
    pub fn new(approval_session_id: &'a str, decision: ApprovalDecision) -> Self {
        Self {
            approval_session_id,
            decision: decision.as_api_str(),
        }
    }

    #[must_use]
    pub fn from_decision_str(approval_session_id: &'a str, decision: &'a str) -> Self {
        Self {
            approval_session_id,
            decision,
        }
    }
}

/// 序列化审批 POST body。
pub fn approval_post_body_json(
    approval_session_id: &str,
    decision: ApprovalDecision,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(&ApprovalPostBody::new(approval_session_id, decision))
}

/// `approval_session_id` 允许的字符（字母数字 / `-_.:`）。
#[must_use]
pub fn is_approval_session_id_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':')
}

/// 非空、≤128、且仅含合法字符。
#[must_use]
pub fn approval_session_id_is_valid(id: &str) -> bool {
    let t = id.trim();
    !t.is_empty() && t.len() <= 128 && t.chars().all(is_approval_session_id_char)
}

/// 从 AG-UI CUSTOM `command_approval` 的 `data` 对象解析请求。
///
/// 识别 camelCase `allowlistKey`（与 serve 下发一致）。
#[must_use]
pub fn parse_command_approval_data(data: &Value) -> CommandApprovalRequest {
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
    fn decision_roundtrip() {
        for d in [
            ApprovalDecision::Deny,
            ApprovalDecision::AllowOnce,
            ApprovalDecision::AllowAlways,
        ] {
            assert_eq!(ApprovalDecision::parse(d.as_api_str()), Some(d));
        }
        assert!(ApprovalDecision::parse("nope").is_none());
        assert_eq!(
            ApprovalDecision::parse(" allow_once "),
            Some(ApprovalDecision::AllowOnce)
        );
    }

    #[test]
    fn parses_approval_data_allowlist_key() {
        let data = json!({"command":"rm","args":"-rf","allowlistKey":"rm"});
        let req = parse_command_approval_data(&data);
        assert_eq!(req.command, "rm");
        assert_eq!(req.args, "-rf");
        assert_eq!(req.allowlist_key.as_deref(), Some("rm"));
        let empty = parse_command_approval_data(&json!({}));
        assert_eq!(empty.command, "");
        assert!(empty.allowlist_key.is_none());
    }

    #[test]
    fn post_body_shape() {
        let s = approval_post_body_json("approval_1", ApprovalDecision::Deny).unwrap();
        let v: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["approval_session_id"], "approval_1");
        assert_eq!(v["decision"], "deny");
    }

    #[test]
    fn session_id_charset() {
        assert!(approval_session_id_is_valid("tui_1_2:3.4-5"));
        assert!(approval_session_id_is_valid("approval_123"));
        assert!(!approval_session_id_is_valid(""));
        assert!(!approval_session_id_is_valid("bad id"));
        assert!(!approval_session_id_is_valid(&"a".repeat(129)));
    }
}
