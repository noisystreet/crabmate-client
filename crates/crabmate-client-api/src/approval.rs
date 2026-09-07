//! 命令审批：契约类型 re-export、决策串映射扩展、`POST /chat/approval` body 形状、
//! `approval_session_id` 校验。
//!
//! 镜像瘦身（0.5.2）：`CommandApprovalData` / `CommandApprovalDecision` /
//! `ChatApprovalRequestBody` 均为 Server 契约类型 re-export，client 不再持有本地镜像；
//! 仅保留 0.5.2 契约面没有的产品逻辑（决策串映射、session id 字符集校验）。

pub use crabmate::cm_api_contract::chat::ChatApprovalRequestBody;
pub use crabmate::cm_sse_protocol::CommandApprovalData;
pub use crabmate::cm_types::CommandApprovalDecision as ApprovalDecision;

/// [`ApprovalDecision`]（契约 `CommandApprovalDecision`）的 client 扩展：0.5.2 枚举
/// 无方法无 serde，这里补 API 决策串映射，保持消费端 `as_api_str()` / `parse()` 语法不变。
pub trait ApprovalDecisionApi: Copy {
    /// 服务端契约字符串（`POST /chat/approval` body 的 `decision` 字段）。
    #[must_use]
    fn as_api_str(self) -> &'static str;

    /// 解析决策串；未知值返回 `None`（容忍首尾空白）。
    #[must_use]
    fn parse(raw: &str) -> Option<Self>
    where
        Self: Sized;
}

impl ApprovalDecisionApi for ApprovalDecision {
    fn as_api_str(self) -> &'static str {
        match self {
            Self::Deny => "deny",
            Self::AllowOnce => "allow_once",
            Self::AllowAlways => "allow_always",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "deny" => Some(Self::Deny),
            "allow_once" => Some(Self::AllowOnce),
            "allow_always" => Some(Self::AllowAlways),
            _ => None,
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

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
    fn parses_approval_data_camel_case() {
        let data = json!({"command":"rm","args":"-rf","allowlistKey":"rm"});
        let req: CommandApprovalData = serde_json::from_value(data).unwrap();
        assert_eq!(req.command, "rm");
        assert_eq!(req.args, "-rf");
        assert_eq!(req.allowlist_key.as_deref(), Some("rm"));
        let no_key: CommandApprovalData =
            serde_json::from_value(json!({"command":"ls","args":"-l"})).unwrap();
        assert!(no_key.allowlist_key.is_none());
        // 契约 command/args 必填：缺键解析失败（0.5.2 类型无宽容 default；SSE 线上恒有）。
        assert!(serde_json::from_value::<CommandApprovalData>(json!({})).is_err());
    }

    #[test]
    fn post_body_shape() {
        let body = ChatApprovalRequestBody {
            approval_session_id: "approval_1".to_string(),
            decision: ApprovalDecision::Deny.as_api_str().to_string(),
        };
        let s = serde_json::to_string(&body).unwrap();
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
