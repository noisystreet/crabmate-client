//! 发送路径纯函数：澄清问卷折叠、用户行 + Loading 尾泡、流式壳忙态入口。

use leptos::prelude::*;

use crate::chat_session_state::ChatSessionSignals;
use crate::i18n;
use crate::i18n::Locale;
use crate::session_ops::{
    make_message_id, message_created_ms, patch_active_session, title_from_user_prompt,
};
use crate::storage::{StoredMessage, StoredMessageState};

use super::super::composer_stream::AttachChatStreamArc;
use super::super::handles::ComposerStreamShell;
use super::super::scroll_follow::engage_follow_and_scroll_bottom;
use super::super::scroll_shell::ChatScrollShellSignals;

pub(super) fn user_line_and_clarify_from_shell(
    shell: &ComposerStreamShell,
    trimmed_draft: &str,
    loc: Locale,
) -> Option<(String, Option<serde_json::Value>)> {
    if let Some(form) = shell.approval.pending_clarification.get() {
        // 必填校验与「按题目 id 对齐」都在共享层（`crabmate_client_api::clarify_form`）：
        // TUI 与 Web 各自写一遍的话，同一份问卷会在两端给出不同的「算不算填了」，
        // 而用户只会觉得表单坏了。文案仍留在这边（Web 用既有 i18n 串）。
        let cq = match crabmate_client_api::clarify_form::answers_json(
            &form.questionnaire_id,
            &form.fields,
            &form.values,
        ) {
            Ok(v) => v,
            Err(_) => {
                shell
                    .stream
                    .status_err
                    .set(Some(i18n::clarification_missing_required(loc).to_string()));
                return None;
            }
        };
        shell.approval.pending_clarification.set(None);
        Some((
            i18n::clarification_user_bubble_stub(loc).to_string(),
            Some(cq),
        ))
    } else {
        Some((trimmed_draft.to_string(), None))
    }
}

pub(super) fn push_user_and_loading_assistant(
    chat: ChatSessionSignals,
    user_line: String,
    imgs_send: Vec<String>,
    uid: String,
    asst_id: String,
) {
    patch_active_session(chat.sessions, &chat.active_id.get(), |s| {
        let now = message_created_ms();
        let is_first_user_turn = s.messages.iter().filter(|m| m.role == "user").count() == 0;
        s.messages.push(StoredMessage {
            id: uid.clone(),
            role: "user".to_string(),
            text: user_line.clone(),
            reasoning_text: String::new(),
            image_urls: imgs_send.clone(),
            state: None,
            is_tool: false,
            tool_call_id: None,
            tool_name: None,
            created_at: now,
        });
        s.messages.push(StoredMessage {
            id: asst_id.clone(),
            role: "assistant".to_string(),
            text: String::new(),
            reasoning_text: String::new(),
            image_urls: vec![],
            state: Some(StoredMessageState::Loading),
            is_tool: false,
            tool_call_id: None,
            tool_name: None,
            created_at: now,
        });
        if is_first_user_turn && i18n::is_default_session_title(&s.title) {
            s.title = title_from_user_prompt(&user_line);
        }
        s.draft.clear();
    });
}

pub(super) fn begin_stream_shell_turn(shell: &ComposerStreamShell) {
    shell.stream.clear_status_banners();
    shell.approval.clear_pending_user_interactions();
}

pub(super) fn start_user_stream_turn(
    chat: ChatSessionSignals,
    attach: &AttachChatStreamArc,
    scroll_shell: ChatScrollShellSignals,
    shell: &ComposerStreamShell,
    user_line: String,
    imgs_send: Vec<String>,
    clarify_json: Option<serde_json::Value>,
) {
    let uid = make_message_id();
    let asst_id = make_message_id();
    push_user_and_loading_assistant(
        chat,
        user_line.clone(),
        imgs_send.clone(),
        uid,
        asst_id.clone(),
    );
    engage_follow_and_scroll_bottom(scroll_shell);
    begin_stream_shell_turn(shell);
    attach(user_line, imgs_send, asst_id, clarify_json);
}
