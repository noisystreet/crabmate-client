//! `on_done` 中对会话 **`messages`** 的尾泡收尾（读列表 → [`super::done_bubble::decide_done_bubble_action`] → 写回），
//! 与 **`ChatStreamCallbackCtx::update_bound_session`**（`stream_session_access`）解耦以便单测与降 [`super::builders::chat_stream_on_done_builder`] nloc。

use crate::i18n::Locale;
use crate::message_loading::is_loading_plain_assistant;
use crate::storage::StoredMessage;
use crate::timeline_scan::is_ephemeral_timeline_assistant_for_chat_ui;
use crate::visible_messages::is_empty_assistant_body;

use super::super::per_stream_accum::PerStreamTurnSummary;
use super::done_bubble::{DoneBubbleAction, DoneBubbleDecisionInputs, decide_done_bubble_action};
use super::helpers::build_empty_reply_with_diagnostic;

fn push_missing_assistant_diagnostic(
    messages: &mut Vec<StoredMessage>,
    turn: &PerStreamTurnSummary,
    in_answer_body_lane: bool,
    locale: Locale,
) {
    let text = build_empty_reply_with_diagnostic(
        locale,
        in_answer_body_lane,
        turn.answer_delta_chars,
        turn.stream_end_reason.as_deref(),
    );
    messages.push(StoredMessage {
        id: format!("asst_diag_{}", messages.len()),
        role: "assistant".to_string(),
        text,
        reasoning_text: String::new(),
        image_urls: Vec::new(),
        state: None,
        is_tool: false,
        tool_call_id: None,
        tool_name: None,
        created_at: 0,
    });
}

/// 流式整轮结束后仍残留的助手 `Loading` 占位（轮换/id 漂移等）须在此清除，避免 UI 与 lifecycle 长期不一致。
pub(super) fn clear_residual_assistant_loading_placeholders(messages: &mut Vec<StoredMessage>) {
    messages.retain(|m| !is_loading_plain_assistant(m));
}

/// 本回合（最后一条 `user` 之后）是否已有面向用户的可见输出。
///
/// 与 Server `current_turn_has_visible_assistant_output` 对齐：`StreamEndReason::Completed`
/// 只在**本轮确有可见输出**（`Answer` / `ToolCalls`）时给出，因此客户端不得再补「无回复」
/// 诊断与之矛盾。可见口径取主列读路径：工具行，或未被旁注过滤且正文/思维链非空的助手行。
fn current_turn_has_visible_output(messages: &[StoredMessage]) -> bool {
    let start = messages
        .iter()
        .rposition(|m| m.role == "user")
        .map_or(0, |idx| idx.saturating_add(1));
    messages[start..].iter().any(|m| {
        m.is_tool
            || (m.role == "assistant"
                && !is_empty_assistant_body(m)
                && !is_ephemeral_timeline_assistant_for_chat_ui(m, messages))
    })
}

/// `assistant_message_id` 已不在列表（轮换 / dedupe 移除了空尾泡）时的收尾。
///
/// 仅当本回合确无可见输出时才补「无回复」诊断；否则本轮已有工具卡或正文，补诊断会与其矛盾。
fn finalize_missing_loading_assistant(
    messages: &mut Vec<StoredMessage>,
    turn: &PerStreamTurnSummary,
    in_answer_body_lane: bool,
    locale: Locale,
) {
    clear_residual_assistant_loading_placeholders(messages);
    clear_residual_empty_assistant_rows(messages);
    if turn.answer_delta_chars == 0
        && !turn.saw_final_response_timeline
        && !current_turn_has_visible_output(messages)
    {
        push_missing_assistant_diagnostic(messages, turn, in_answer_body_lane, locale);
    }
}

/// 在会话消息列表上对 `assistant_message_id` 指向的 **loading** 尾泡应用 `on_done` 决策。
pub(super) fn apply_stream_done_to_loading_assistant(
    messages: &mut Vec<StoredMessage>,
    assistant_message_id: &str,
    turn: &PerStreamTurnSummary,
    in_answer_body_lane: bool,
    locale: Locale,
) {
    let has_tool = messages.iter().any(|x| x.is_tool);
    let Some(idx) = messages.iter().position(|m| m.id == assistant_message_id) else {
        finalize_missing_loading_assistant(messages, turn, in_answer_body_lane, locale);
        return;
    };
    if !is_loading_plain_assistant(&messages[idx]) {
        clear_residual_assistant_loading_placeholders(messages);
        clear_residual_empty_assistant_rows(messages);
        return;
    }
    messages[idx].state = None;
    let body_chars =
        messages[idx].text.chars().count() + messages[idx].reasoning_text.chars().count();
    let diag_chars = body_chars.max(turn.answer_delta_chars);
    let body_and_reasoning_empty =
        messages[idx].text.trim().is_empty() && messages[idx].reasoning_text.trim().is_empty();
    let end_reason = turn.stream_end_reason.as_deref();
    match decide_done_bubble_action(DoneBubbleDecisionInputs {
        body_and_reasoning_empty,
        end_reason_raw: end_reason,
        in_answer_body_lane,
        diag_chars,
        has_tool,
        saw_final_response_timeline: turn.saw_final_response_timeline,
    }) {
        DoneBubbleAction::Keep => {}
        DoneBubbleAction::RemoveBubble => {
            messages.remove(idx);
        }
        DoneBubbleAction::FillMissingFinalHint => {
            messages[idx].text = format!(
                "{}\n\n{}",
                crate::i18n::stream_completed_missing_final_summary_hint(locale),
                crate::i18n::stream_empty_reply_diag_line(
                    locale,
                    end_reason,
                    in_answer_body_lane,
                    diag_chars,
                )
            );
        }
        DoneBubbleAction::FillDiagnostic => {
            messages[idx].text = build_empty_reply_with_diagnostic(
                locale,
                in_answer_body_lane,
                diag_chars,
                end_reason,
            );
        }
    }
    clear_residual_assistant_loading_placeholders(messages);
    // 安全网：清理残留的空非 loading assistant 行（由 detach/finalize 等路径产生）
    clear_residual_empty_assistant_rows(messages);
}

/// 清理空的非 loading assistant 行（state=None，text 和 reasoning_text 均空）。
/// 这些行由 detach_final_answer_projection / finalize_loading_row_at 等路径残留，
/// 在读路径被 `is_empty_assistant_body` 过滤，但会污染 stored_messages 数组。
fn clear_residual_empty_assistant_rows(messages: &mut Vec<StoredMessage>) {
    messages.retain(|m| {
        if m.role != "assistant" || m.is_tool {
            return true;
        }
        if m.state.is_some() {
            return true;
        }
        !(m.text.trim().is_empty() && m.reasoning_text.trim().is_empty())
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StoredMessageState;

    fn loading_asst(id: &str) -> StoredMessage {
        StoredMessage {
            id: id.into(),
            role: "assistant".into(),
            text: String::new(),
            reasoning_text: String::new(),
            image_urls: vec![],
            state: Some(StoredMessageState::Loading),
            is_tool: false,
            tool_call_id: None,
            tool_name: None,
            created_at: 0,
        }
    }

    fn plain(id: &str, role: &str, text: &str, is_tool: bool) -> StoredMessage {
        StoredMessage {
            id: id.into(),
            role: role.into(),
            text: text.into(),
            reasoning_text: String::new(),
            image_urls: vec![],
            state: None,
            is_tool,
            tool_call_id: None,
            tool_name: None,
            created_at: 0,
        }
    }

    #[test]
    fn missing_primary_id_keeps_tool_turn_without_diagnostic() {
        let mut msgs = vec![
            plain("u", "user", "q", false),
            plain("t", "tool", "tool result", true),
        ];
        apply_stream_done_to_loading_assistant(
            &mut msgs,
            "missing",
            &PerStreamTurnSummary {
                answer_delta_chars: 0,
                stream_end_reason: Some("completed".into()),
                saw_final_response_timeline: false,
            },
            false,
            crate::i18n::Locale::ZhHans,
        );
        assert_eq!(msgs.len(), 2);
        assert!(msgs.iter().all(|m| !m.text.contains("无回复")));
    }

    #[test]
    fn missing_primary_id_keeps_assistant_text_without_diagnostic() {
        let mut msgs = vec![
            plain("u", "user", "q", false),
            plain("a", "assistant", "终答正文", false),
        ];
        apply_stream_done_to_loading_assistant(
            &mut msgs,
            "missing",
            &PerStreamTurnSummary {
                answer_delta_chars: 0,
                stream_end_reason: Some("completed".into()),
                saw_final_response_timeline: false,
            },
            false,
            crate::i18n::Locale::ZhHans,
        );
        assert_eq!(msgs.len(), 2);
        assert!(msgs.iter().all(|m| !m.text.contains("无回复")));
    }

    #[test]
    fn clears_orphan_loading_when_primary_id_missing() {
        let mut msgs = vec![loading_asst("orphan")];
        apply_stream_done_to_loading_assistant(
            &mut msgs,
            "missing",
            &PerStreamTurnSummary {
                answer_delta_chars: 0,
                stream_end_reason: None,
                saw_final_response_timeline: false,
            },
            false,
            crate::i18n::Locale::ZhHans,
        );
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].text.contains("未进入正文阶段"));
    }

    #[test]
    fn clears_extra_loading_after_primary_done() {
        let mut msgs = vec![loading_asst("orphan"), loading_asst("primary")];
        apply_stream_done_to_loading_assistant(
            &mut msgs,
            "primary",
            &PerStreamTurnSummary {
                answer_delta_chars: 0,
                stream_end_reason: Some("completed".into()),
                saw_final_response_timeline: false,
            },
            true,
            crate::i18n::Locale::ZhHans,
        );
        assert!(msgs.is_empty());
    }

    #[test]
    fn clears_empty_finalized_assistant_rows() {
        let mut msgs = vec![
            StoredMessage {
                id: "u".into(),
                role: "user".into(),
                text: "q".into(),
                reasoning_text: String::new(),
                image_urls: vec![],
                state: None,
                is_tool: false,
                tool_call_id: None,
                tool_name: None,
                created_at: 0,
            },
            // 由 detach/finalize 产生的空 assistant 行
            StoredMessage {
                id: "detached_empty".into(),
                role: "assistant".into(),
                text: String::new(),
                reasoning_text: String::new(),
                image_urls: vec![],
                state: None,
                is_tool: false,
                tool_call_id: None,
                tool_name: None,
                created_at: 0,
            },
            StoredMessage {
                id: "answer".into(),
                role: "assistant".into(),
                text: "终答正文".into(),
                reasoning_text: String::new(),
                image_urls: vec![],
                state: None,
                is_tool: false,
                tool_call_id: None,
                tool_name: None,
                created_at: 0,
            },
        ];
        clear_residual_empty_assistant_rows(&mut msgs);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].id, "u");
        assert_eq!(msgs[1].text, "终答正文");
    }
}
