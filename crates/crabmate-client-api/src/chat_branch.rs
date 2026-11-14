//! `POST /chat/branch` 的纯逻辑（issue #422）：请求体、响应分类、「最后一轮的用户序号」。
//!
//! 为什么放在共享层：
//!
//! - **响应分类决定后续动作**（`404` = 会话不存在 → 清本地绑定；`409` = revision 过期 → 提示先刷新；
//!   其它 → 原样上抛）。Desktop `api/http.rs` 与 TUI 各写一份，迟早出现「同一次失败两端处置不同」。
//! - [`user_ordinal_of_last_turn`] 与 Desktop 的 `user_ordinal_for_message_index` 是同一条语义
//!   （会话级 0-based 用户序号，喂给 `before_user_ordinal`），口径应当只有一处。
//!
//! 序号是**会话级**、**含已归档回合**（Server `conversation_store/archive.rs` 的原文），而
//! `GET /conversation/messages` 默认只给 canonical。所以算不准时**必须报错**：用错的 ordinal
//! 会把会话截到别处，是不可逆的数据损失。

use serde::Serialize;
use serde_json::Value;

use crate::conversation_messages::{
    ConversationMessage, ConversationMessagesResponse, FIRST_TURN_WORKSPACE_CONTEXT_NAME,
};

/// `POST /chat/branch` 请求体。
///
/// 契约里的 `ChatBranchRequestBody` 只派生 `Deserialize`（服务端侧解析用），发请求这边自己带
/// `Serialize`——与 [`crate::approval`] 里 `ChatApprovalRequestBody` 的做法一致。
#[derive(Debug, Clone, Serialize)]
pub struct ChatBranchRequestBody {
    pub conversation_id: String,
    /// 从此序号对应的用户消息起（含）全部丢弃。
    pub before_user_ordinal: u64,
    /// 截断前客户端所知的 `revision`。
    pub expected_revision: u64,
}

/// branch 失败的三类：决定调用方接下来做什么，因此类型必须分开而不是压成一个字符串。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchFailure {
    /// `404`：会话在服务端不存在（serve 重启 / 过期 / 被别的端删了）→ 清掉本地 `conversation_id` 绑定。
    NotFound(String),
    /// `409`：客户端手里的 revision 已过期 → 提示先刷新，**不要**改本地 transcript。
    Conflict(String),
    /// 其它失败（含网络层）：原样上抛。
    Other(String),
}

impl BranchFailure {
    /// 给人看的一句话（三类都可直接进 transcript）。
    #[must_use]
    pub fn message(&self) -> &str {
        match self {
            Self::NotFound(m) | Self::Conflict(m) | Self::Other(m) => m,
        }
    }
}

/// 非 2xx 的 body → 一句话。
///
/// 服务端错误体是 `{"error": {"code": …, "message": …}}`，也见过 `{"error": "…"}` 与纯文本；
/// 三种都认，认不出就返回原文（截断到一屏内），不吞掉现场。
fn error_message(body: &str) -> String {
    let text = body.trim();
    if let Ok(v) = serde_json::from_str::<Value>(text) {
        let from_obj = v
            .get("error")
            .and_then(|e| e.get("message").or(Some(e)))
            .and_then(Value::as_str)
            .map(str::to_string);
        if let Some(m) = from_obj.filter(|m| !m.trim().is_empty()) {
            return m;
        }
    }
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.is_empty() {
        "（服务端没有返回错误正文）".to_string()
    } else {
        one_line.chars().take(300).collect()
    }
}

/// 状态码 + 响应正文 → 截断后的新 `revision`。
///
/// 2xx 但 body 解析不出 `revision` 也算失败（`Other`）：调用方拿不到新 revision 就继续用旧值，
/// 下一次 branch 必然 409，不如当场说清。
pub fn classify_branch_response(status: u16, body: &str) -> Result<u64, BranchFailure> {
    if (200..300).contains(&status) {
        let parsed = serde_json::from_str::<Value>(body)
            .ok()
            .and_then(|v| v.get("revision").and_then(Value::as_u64));
        return parsed.ok_or_else(|| {
            BranchFailure::Other(format!(
                "服务端回了 {status} 但读不出 revision：{}",
                error_message(body)
            ))
        });
    }
    let msg = error_message(body);
    match status {
        404 => Err(BranchFailure::NotFound(msg)),
        409 => Err(BranchFailure::Conflict(msg)),
        _ => Err(BranchFailure::Other(format!("{status}：{msg}"))),
    }
}

/// 算不出最后一轮的序号时**为什么**算不出来。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrdinalUnknown {
    /// 会话有归档回合（上下文压缩把旧回合移出了 canonical）：客户端只拿得到 canonical，
    /// 数出来的序号会偏小，用它截断会把会话截到别处。
    Archived,
    /// 这轮会话里没有任何（普通）用户消息 —— 没什么可再生的。
    NoUserMessage,
}

impl OrdinalUnknown {
    /// 给人看的一句话（直接进 transcript）。
    #[must_use]
    pub fn message(&self) -> &'static str {
        match self {
            Self::Archived => {
                "这段会话有已归档的历史（上下文压缩过）：客户端手里只有未归档部分，算不准用户消息序号，\
                 为避免把会话截到别处，这里不做再生；请改用 Desktop / Web 的「重新生成」"
            }
            Self::NoUserMessage => "当前会话里没有可再生的用户消息",
        }
    }
}

/// 是否一条「普通」用户消息。
///
/// 首轮工作区上下文注入条也是 `role=user`（靠 `name` 区分，见
/// [`FIRST_TURN_WORKSPACE_CONTEXT_NAME`]），回放时它会跳过、这里也必须跳过：漏掉就是永久偏一位。
fn is_plain_user(msg: &ConversationMessage) -> bool {
    msg.role.trim() == "user"
        && msg.name.as_deref().map(str::trim) != Some(FIRST_TURN_WORKSPACE_CONTEXT_NAME)
}

/// `fetch_all_pages` 的结果（**新 → 旧**）→ 最后一轮的用户序号（0-based）。
///
/// 只数「一共几条用户消息」再减一：序号是会话级的，与页序无关，因此不必关心页怎么切。
pub fn user_ordinal_of_last_turn(
    pages: &[ConversationMessagesResponse],
) -> Result<u64, OrdinalUnknown> {
    if pages.iter().any(|p| !p.archive.is_empty()) {
        return Err(OrdinalUnknown::Archived);
    }
    let total = pages
        .iter()
        .flat_map(|p| p.messages.iter())
        .filter(|m| is_plain_user(m))
        .count();
    if total == 0 {
        return Err(OrdinalUnknown::NoUserMessage);
    }
    Ok(total as u64 - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(
        messages: Vec<ConversationMessage>,
        archive: Vec<ConversationMessage>,
    ) -> ConversationMessagesResponse {
        ConversationMessagesResponse {
            conversation_id: "c1".into(),
            revision: 7,
            messages,
            total_count: 0,
            window_start_index: 0,
            has_older: false,
            tiktoken_prompt_tokens: None,
            archive,
        }
    }

    fn msg(role: &str) -> ConversationMessage {
        ConversationMessage {
            role: role.into(),
            content: None,
            reasoning_content: None,
            display_content: Some(format!("{role} 正文")),
            display_reasoning_content: None,
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    /// 首轮工作区上下文注入条虽然也是 `user`，但不算一条用户消息（否则序号永久偏一位）。
    #[test]
    fn workspace_context_injection_is_not_a_user_turn() {
        let mut injected = msg("user");
        injected.name = Some(FIRST_TURN_WORKSPACE_CONTEXT_NAME.into());
        // 一条真用户消息 + 一条注入条：真用户消息只有 1 条 → 序号 0。
        let pages = vec![page(
            vec![msg("user"), injected.clone(), msg("assistant")],
            vec![],
        )];
        assert_eq!(
            user_ordinal_of_last_turn(&pages),
            Ok(0),
            "注入条不该算成一条用户消息"
        );

        // 两条真用户消息中间夹一条注入条：最后一条的序号必须是 1；
        // 把注入条也数进去就会变成 2 —— 那会把会话截到别处。
        let pages = vec![page(
            vec![msg("user"), injected, msg("assistant"), msg("user")],
            vec![],
        )];
        assert_eq!(user_ordinal_of_last_turn(&pages), Ok(1));
    }

    /// 多页（新 → 旧）也按总数算：页怎么切不影响序号。
    #[test]
    fn ordinal_counts_across_pages() {
        let newest = page(vec![msg("user"), msg("assistant"), msg("user")], vec![]);
        let older = page(vec![msg("user"), msg("assistant")], vec![]);
        // 3 条用户消息 → 最后一轮序号 2。
        assert_eq!(
            user_ordinal_of_last_turn(&[newest, older]),
            Ok(2),
            "最后一轮的序号 = 用户消息总数 - 1"
        );
    }

    #[test]
    fn ordinal_refuses_archived_conversations() {
        let pages = vec![page(vec![msg("user")], vec![msg("user"), msg("assistant")])];
        assert_eq!(
            user_ordinal_of_last_turn(&pages),
            Err(OrdinalUnknown::Archived),
            "归档前缀数不到就必须拒绝，不能算个偏小的序号去截断"
        );
    }

    #[test]
    fn ordinal_needs_at_least_one_user_message() {
        assert_eq!(
            user_ordinal_of_last_turn(&[page(vec![msg("assistant")], vec![])]),
            Err(OrdinalUnknown::NoUserMessage)
        );
        assert_eq!(
            user_ordinal_of_last_turn(&[]),
            Err(OrdinalUnknown::NoUserMessage)
        );
    }

    #[test]
    fn classify_maps_status_to_the_three_failure_kinds() {
        assert_eq!(
            classify_branch_response(200, r#"{"ok":true,"revision":9}"#),
            Ok(9)
        );
        assert!(matches!(
            classify_branch_response(404, r#"{"error":{"code":"NOT_FOUND","message":"会话不存在"}}"#),
            Err(BranchFailure::NotFound(m)) if m == "会话不存在"
        ));
        assert!(matches!(
            classify_branch_response(409, r#"{"error":{"message":"revision 过期"}}"#),
            Err(BranchFailure::Conflict(m)) if m == "revision 过期"
        ));
        assert!(matches!(
            classify_branch_response(500, "boom"),
            Err(BranchFailure::Other(m)) if m.contains("boom")
        ));
    }

    /// 2xx 但读不出 revision：当场说清，别让调用方拿着旧 revision 去撞下一次 409。
    #[test]
    fn success_without_revision_is_a_failure() {
        assert!(matches!(
            classify_branch_response(200, r#"{"ok":true}"#),
            Err(BranchFailure::Other(_))
        ));
    }

    #[test]
    fn error_message_accepts_string_and_plain_text_forms() {
        assert_eq!(error_message(r#"{"error":"直接一句"}"#), "直接一句");
        assert_eq!(error_message("  纯文本\n第二行  "), "纯文本 第二行");
        assert!(!error_message("").is_empty(), "空正文也要给一句人话");
    }
}
