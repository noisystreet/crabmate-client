//! 消息行上的**副作用动作**（分支 API、本地截断后再流式）：供 TUI 动作条调用。

use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::api::{ChatBranchError, fetch_conversation_messages, post_chat_branch};
use crate::chat_actions::apply_branch_success_revision;
use crate::chat_session_state::ChatSessionSignals;
use crate::conversation_messages_page::ConversationMessagesFetchParams;
use crate::i18n::Locale;
use crate::session_ops::{
    truncate_at_user_message_and_prepare_regenerate, truncate_at_user_message_branch_local,
    user_ordinal_for_message_index,
};

use super::composer_follow_up::ComposerStreamFollowUp;
use super::session_hydrate::apply_saved_revision_if_same_conversation;

fn dbg_log(msg: &str) {
    web_sys::console::log_1(&format!("[regen] {msg}").into());
}

fn dbg_log_with(msg: &str, val: &str) {
    web_sys::console::log_2(&format!("[regen] {}: {}", msg, val).into(), &val.into());
}

/// 重新生成分派所需快照：`(conversation_id, expected_revision, before_user_ordinal)`。
fn regenerate_dispatch_snapshot(
    chat: &ChatSessionSignals,
    msg_idx: usize,
) -> (Option<String>, Option<u64>, Option<u64>) {
    let (cid, rev) = chat.session_sync.with(|s| {
        let (a, b) = s.branch_id_and_expected_revision();
        (a.map(|x| x.to_string()), b)
    });
    let ord = chat.sessions.with(|list| {
        let aid = chat.active_id.get_untracked();
        list.iter()
            .find(|s| s.id == aid)
            .and_then(|s| user_ordinal_for_message_index(&s.messages, msg_idx))
    });
    (cid, rev, ord)
}

/// PATH B：仅本地截断当前用户行，并排队 [`ComposerStreamFollowUp::RegenerateAfterTruncate`]。
fn local_regenerate_then_stream(
    chat: ChatSessionSignals,
    stream_follow_up: RwSignal<ComposerStreamFollowUp>,
    user_message_id: String,
) {
    let mut prep: Option<(String, Vec<String>, String)> = None;
    chat.update_sessions_message_row(|list| {
        let aid = chat.active_id.get_untracked();
        prep = truncate_at_user_message_and_prepare_regenerate(list, &aid, &user_message_id);
    });
    if let Some((user_text, user_imgs, asst_id)) = prep {
        dbg_log_with("local regen prep set", &user_text);
        stream_follow_up.set(ComposerStreamFollowUp::RegenerateAfterTruncate {
            user_text,
            user_imgs,
            asst_id,
        });
    } else {
        dbg_log("local regen prep was None");
    }
}

/// PATH A（`POST /chat/branch`）请求要素。
struct BranchRegenerateRequest {
    conversation_id: String,
    before_user_ordinal: u64,
    expected_revision: u64,
    user_message_id: String,
}

/// PATH A 主体：服务端权威截断中断尾部；`404` 降级为本地截断，`409` / 其它钉状态栏。
async fn post_branch_for_regenerate(
    chat: ChatSessionSignals,
    stream_follow_up: RwSignal<ComposerStreamFollowUp>,
    status_err: RwSignal<Option<String>>,
    req: BranchRegenerateRequest,
    loc: Locale,
) {
    match post_chat_branch(
        &req.conversation_id,
        req.before_user_ordinal,
        req.expected_revision,
        loc,
    )
    .await
    {
        Ok(new_rev) => {
            dbg_log_with("post_chat_branch ok, new_rev", &new_rev.to_string());
            let aid = chat.active_id.get_untracked();
            apply_branch_success_revision(chat, &aid, new_rev);
            local_regenerate_then_stream(chat, stream_follow_up, req.user_message_id);
        }
        Err(ChatBranchError::NotFound(_)) => {
            // 会话在服务端不存在（server 重启/过期）：清除绑定，降级为本地截断。
            chat.session_sync.update(|s| s.invalidate_conversation_id());
            local_regenerate_then_stream(chat, stream_follow_up, req.user_message_id);
        }
        Err(e) => {
            dbg_log_with("post_chat_branch err", e.as_deref());
            chat.session_sync.update(|s| s.mark_branch_conflict());
            let msg = match e {
                ChatBranchError::Conflict(_) => crate::i18n::api_err_branch_failed(loc).to_string(),
                other => other.as_deref().to_string(),
            };
            status_err.set(Some(msg));
        }
    }
}

/// 停止生成后 revision 未知（`conversation_id` 有、`last_known_revision` 为 `None`）：
/// 先 `GET /conversation/messages` 补拉服务端 revision 并写回，再重试 PATH A
/// （由服务端权威截断中断尾部，避免本地截断后重发同一 user 造成重复落盘）；补拉失败则降级为本地截断。
async fn regenerate_after_revision_refresh(
    chat: ChatSessionSignals,
    stream_follow_up: RwSignal<ComposerStreamFollowUp>,
    status_err: RwSignal<Option<String>>,
    conversation_id: String,
    user_message_id: String,
    msg_idx: usize,
    loc: Locale,
) {
    if let Ok(resp) = fetch_conversation_messages(
        &conversation_id,
        ConversationMessagesFetchParams::tail_page(),
        loc,
    )
    .await
    {
        apply_saved_revision_if_same_conversation(&chat, &conversation_id, resp.revision);
    }
    let (cid, rev, ord) = regenerate_dispatch_snapshot(&chat, msg_idx);
    match (cid, rev, ord) {
        (Some(conv), Some(expected_revision), Some(before_user_ordinal)) => {
            dbg_log("PATH A after revision refresh");
            post_branch_for_regenerate(
                chat,
                stream_follow_up,
                status_err,
                BranchRegenerateRequest {
                    conversation_id: conv,
                    before_user_ordinal,
                    expected_revision,
                    user_message_id,
                },
                loc,
            )
            .await;
        }
        _ => {
            dbg_log("revision refresh fell back to local truncate");
            local_regenerate_then_stream(chat, stream_follow_up, user_message_id);
        }
    }
}

/// 用户消息上「再生 / 分支」按钮所需的信号子集（[`Copy`]，便于在闭包中捕获）。
#[derive(Clone, Copy)]
pub(crate) struct MessageRowActionSignals {
    pub chat: ChatSessionSignals,
    pub stream_follow_up: RwSignal<ComposerStreamFollowUp>,
    pub status_err: RwSignal<Option<String>>,
    pub locale: RwSignal<Locale>,
}

impl MessageRowActionSignals {
    /// 「在用户消息后重新生成」：
    /// - 会话有 revision → `POST /chat/branch`；
    /// - 停止生成后 revision 未知 → 先补拉 revision 再 `POST /chat/branch`；
    /// - 其余（无 `conversation_id` / 无 ordinal）→ 仅本地截断并排队
    ///   [`ComposerStreamFollowUp::RegenerateAfterTruncate`]。
    pub(crate) fn spawn_regenerate_from_user_line(self, msg_idx: usize, user_message_id: String) {
        let MessageRowActionSignals {
            chat,
            stream_follow_up,
            status_err,
            locale,
        } = self;
        let loc = locale.get_untracked();
        let (cid, rev, ord) = regenerate_dispatch_snapshot(&chat, msg_idx);
        dbg_log_with("cid", &cid.clone().unwrap_or_default());
        dbg_log_with("rev", &rev.map(|r| r.to_string()).unwrap_or_default());
        dbg_log_with("ord", &ord.map(|o| o.to_string()).unwrap_or_default());
        match (cid, rev, ord) {
            (Some(conversation_id), Some(expected_revision), Some(before_user_ordinal)) => {
                dbg_log("PATH A: post_chat_branch (cid + rev + ord all Some)");
                spawn_local(post_branch_for_regenerate(
                    chat,
                    stream_follow_up,
                    status_err,
                    BranchRegenerateRequest {
                        conversation_id,
                        before_user_ordinal,
                        expected_revision,
                        user_message_id,
                    },
                    loc,
                ));
            }
            (Some(conversation_id), None, Some(_)) => {
                dbg_log("PATH A': refresh revision then post_chat_branch");
                spawn_local(regenerate_after_revision_refresh(
                    chat,
                    stream_follow_up,
                    status_err,
                    conversation_id,
                    user_message_id,
                    msg_idx,
                    loc,
                ));
            }
            _ => {
                dbg_log("PATH B: local-only (cid or ord is None)");
                local_regenerate_then_stream(chat, stream_follow_up, user_message_id);
            }
        }
    }

    /// 「从用户消息分支」：服务端分支或仅本地截断视图。
    pub(crate) fn spawn_branch_at_user_line(self, msg_idx: usize, user_message_id: String) {
        let MessageRowActionSignals {
            chat,
            stream_follow_up: _,
            status_err,
            locale,
        } = self;

        let (cid, rev) = chat.session_sync.with(|s| {
            let (a, b) = s.branch_id_and_expected_revision();
            (a.map(|x| x.to_string()), b)
        });
        let ord = chat.sessions.with(|list| {
            let aid = chat.active_id.get_untracked();
            list.iter()
                .find(|s| s.id == aid)
                .and_then(|s| user_ordinal_for_message_index(&s.messages, msg_idx))
        });
        let uid = user_message_id;
        match (cid, rev, ord) {
            (Some(conv), Some(exp_rev), Some(before_ord)) => {
                let loc_b = locale.get_untracked();
                spawn_local(async move {
                    match post_chat_branch(&conv, before_ord, exp_rev, loc_b).await {
                        Ok(new_rev) => {
                            let aid = chat.active_id.get_untracked();
                            apply_branch_success_revision(chat, &aid, new_rev);
                            chat.update_sessions_message_row(|list| {
                                let _ = truncate_at_user_message_branch_local(list, &aid, &uid);
                            });
                        }
                        Err(e) => {
                            let err_display = match &e {
                                ChatBranchError::NotFound(_) => {
                                    chat.session_sync.update(|s| s.invalidate_conversation_id());
                                    crate::i18n::api_err_branch_failed(loc_b).to_string()
                                }
                                ChatBranchError::Conflict(_) | ChatBranchError::Other(_) => {
                                    chat.session_sync.update(|s| s.mark_branch_conflict());
                                    e.as_deref().to_string()
                                }
                            };
                            status_err.set(Some(err_display));
                        }
                    }
                });
            }
            _ => {
                chat.update_sessions_message_row(|list| {
                    let aid = chat.active_id.get_untracked();
                    let _ = truncate_at_user_message_branch_local(list, &aid, &uid);
                });
            }
        }
    }
}
