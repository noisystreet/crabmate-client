//! [`super::handles::WireComposerStreamsArgs`] → [`super::handles::ChatComposerWires`] 的接线实现，降低 `composer` 单文件圈复杂度。
//!
//! 子模块拆分：**`helpers`** 为发送路径纯函数；**`follow_up`** 单独承载待发流式队列的 `Effect`，便于审阅「单数据流」边界。

mod follow_up;
mod helpers;

use std::rc::Rc;
use std::sync::Arc;

use leptos::prelude::*;

use self::follow_up::{StreamFollowUpWiring, wire_stream_follow_up_effect};
use self::helpers::{start_user_stream_turn, user_line_and_clarify_from_shell};
use super::composer_follow_up::ComposerStreamFollowUp;
use super::composer_slash_control::{WebSlashControlCtx, try_handle_web_control_slash};
use super::composer_stream::{ComposerStreamHandles, make_attach_chat_stream};
use super::handles::{
    ChatComposerWires, ComposerStreamShell, WireComposerStreamsArgs,
    WireComposerStreamsSessionSlice, WireComposerStreamsStreamSlice,
};
use super::stream_follow_up_gates::{ComposerSendDecision, decide_composer_send};
use super::stream_user_abort::apply_user_abort_of_inflight_stream;
use crate::chat_session_state::ChatSessionSignals;
use crate::session_ops::{flush_active_composer_draft, flush_composer_draft_to_session};
use crate::session_sync::SessionSyncState;
use crate::storage::{ChatSession, DEFAULT_CHAT_SESSION_TITLE, make_session_id};

/// 发送路径所需的会话草稿上下文（Copy；供斜杠命令拦截判定与闭包捕获共用）。
#[derive(Clone, Copy)]
struct ComposerSendCtx {
    chat: ChatSessionSignals,
    locale: RwSignal<crate::i18n::Locale>,
    draft: RwSignal<String>,
    selected_agent_role: RwSignal<Option<String>>,
    agent_role_user_override: RwSignal<bool>,
    apply_assistant_display_filters: RwSignal<bool>,
}

/// 流正在忙且有待澄清问题时不允许发送。
fn send_blocked_by_busy_clarification(busy: bool, shell: &ComposerStreamShell) -> bool {
    busy && shell.approval.pending_clarification.get().is_some()
}

/// Web 控制斜杠命令是否消费了本次发送（仅无附图时）。
fn web_slash_intercepts_send(
    imgs_empty: bool,
    text: &str,
    shell: &ComposerStreamShell,
    ctx: ComposerSendCtx,
) -> bool {
    let ComposerSendCtx {
        chat,
        locale,
        draft,
        selected_agent_role,
        agent_role_user_override,
        apply_assistant_display_filters,
    } = ctx;
    imgs_empty
        && try_handle_web_control_slash(
            text,
            shell,
            WebSlashControlCtx {
                chat,
                locale,
                draft,
                selected_agent_role,
                agent_role_user_override,
                apply_assistant_display_filters,
            },
        )
}

pub(crate) fn wire_chat_composer_streams(args: WireComposerStreamsArgs) -> ChatComposerWires {
    let WireComposerStreamsArgs { session, stream } = args;
    let WireComposerStreamsSessionSlice {
        initialized,
        chat,
        locale,
        draft,
        selected_agent_role,
        agent_role_user_override,
        selected_session_mode,
        session_mode_user_override,
        apply_assistant_display_filters,
    } = session;
    let WireComposerStreamsStreamSlice {
        stream_shell,
        stream_turn_busy_ui,
        tool_timeline_busy_ui,
        scroll_shell,
        pending_images,
    } = stream;

    let send_ctx = ComposerSendCtx {
        chat,
        locale,
        draft,
        selected_agent_role,
        agent_role_user_override,
        apply_assistant_display_filters,
    };

    let stream_shell_for_attach = stream_shell.clone();
    let attach_chat_stream = make_attach_chat_stream(ComposerStreamHandles {
        chat,
        locale,
        selected_agent_role,
        agent_role_user_override,
        selected_session_mode,
        session_mode_user_override,
        shell: stream_shell_for_attach,
    });

    let stream_follow_up = RwSignal::new(ComposerStreamFollowUp::Idle);

    let run_send_message: Arc<dyn Fn() + Send + Sync> = Arc::new({
        let chat = chat;
        let attach = Arc::clone(&attach_chat_stream);
        let scroll_shell = scroll_shell;
        let shell = stream_shell.clone();
        let locale_sig = locale;
        let send_ctx = send_ctx;
        let stream_follow_up = stream_follow_up;
        move || {
            let text = draft.get_untracked().trim().to_string();
            let imgs = pending_images.get();
            let loc = locale_sig.get();
            if web_slash_intercepts_send(imgs.is_empty(), &text, &shell, send_ctx) {
                return;
            }
            let busy = stream_turn_busy_ui.get() || tool_timeline_busy_ui.get();
            if send_blocked_by_busy_clarification(busy, &shell) {
                return;
            }
            let Some((user_line, clarify_json)) =
                user_line_and_clarify_from_shell(&shell, &text, loc)
            else {
                return;
            };
            match decide_composer_send(
                initialized.get(),
                busy,
                user_line.is_empty(),
                imgs.is_empty(),
                clarify_json.is_none(),
            ) {
                ComposerSendDecision::Ignore => {}
                ComposerSendDecision::QueueWhileBusy => {
                    if stream_follow_up.get_untracked().blocks_user_queue() {
                        return;
                    }
                    stream_follow_up.set(ComposerStreamFollowUp::QueuedUserMessage {
                        session_id: chat.active_id.get_untracked(),
                        user_text: user_line,
                        user_imgs: imgs,
                    });
                    draft.set(String::new());
                    pending_images.set(Vec::new());
                }
                ComposerSendDecision::SendNow => {
                    start_user_stream_turn(
                        chat,
                        &attach,
                        scroll_shell,
                        &shell,
                        user_line,
                        imgs,
                        clarify_json,
                    );
                    draft.set(String::new());
                    pending_images.set(Vec::new());
                }
            }
        }
    });

    wire_stream_follow_up_effect(StreamFollowUpWiring {
        initialized,
        chat,
        attach_chat_stream: Arc::clone(&attach_chat_stream),
        scroll_shell,
        shell: stream_shell.clone(),
        stream_follow_up,
        stream_turn_busy_ui,
        tool_timeline_busy_ui,
    });

    super::stream_visibility_resume::wire_stream_visibility_resume(
        initialized,
        chat,
        locale,
        selected_agent_role,
        selected_session_mode,
        stream_shell.clone(),
    );

    let cancel_stream: Arc<dyn Fn() + Send + Sync> = Arc::new({
        let chat = chat;
        let shell = stream_shell.clone();
        let locale = locale;
        move || {
            let loc = locale.get_untracked();
            let _ = apply_user_abort_of_inflight_stream(chat, &shell, loc);
        }
    });

    let new_session: Rc<dyn Fn()> = Rc::new({
        let chat = chat;
        let stream_follow_up = stream_follow_up;
        move || {
            let parked = stream_follow_up
                .get_untracked()
                .queued_draft_to_park()
                .map(|(id, text)| (id.to_string(), text.to_string()));
            stream_follow_up.set(ComposerStreamFollowUp::Idle);
            flush_active_composer_draft(chat.sessions, chat.active_id, draft);
            if let Some((sid, text)) = parked {
                flush_composer_draft_to_session(chat.sessions, &sid, &text);
            }
            let prev_id = chat.active_id.get_untracked();
            let inherited_ws = chat.sessions.with_untracked(|list| {
                list.iter()
                    .find(|s| s.id == prev_id)
                    .and_then(|s| s.workspace_root.clone())
            });
            let now = js_sys::Date::now() as i64;
            let s = ChatSession {
                id: make_session_id(),
                layout_schema_version: crate::storage::CURRENT_LAYOUT_SCHEMA_VERSION,
                title: DEFAULT_CHAT_SESSION_TITLE.to_string(),
                draft: String::new(),
                messages: vec![],
                updated_at: now,
                pinned: false,
                starred: false,
                server_conversation_id: None,
                server_revision: None,
                workspace_root: inherited_ws,
                history_total: None,
                history_window_start: None,
                history_has_older: None,
            };
            let id = s.id.clone();
            chat.update_sessions_composer(|list| {
                list.insert(0, s);
            });
            chat.active_id.set(id);
            draft.set(String::new());
            chat.session_sync.set(SessionSyncState::local_only());
        }
    });

    ChatComposerWires {
        run_send_message,
        cancel_stream,
        new_session,
        stream_follow_up,
    }
}
