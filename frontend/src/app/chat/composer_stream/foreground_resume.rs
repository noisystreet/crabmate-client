//! 回前台：软续传 `/chat/stream` 或清绑定后水合。

use std::rc::Rc;

use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::api::{SendChatStreamParams, send_chat_stream};
use crate::app::chat::session_hydrate::bump_session_hydrate_nonce;
use crate::app::chat::turn_lifecycle::TurnLifecycleEvent;
use crate::chat_session_state::ChatSessionSignals;
use crate::i18n::Locale;

use super::super::handles::ComposerStreamShell;
use super::callbacks::build_chat_stream_callbacks;
use super::shell_abort::{reset_abort_state_for_new_attach, user_cancelled_flag};
use super::stream_attach_lifecycle::prepare_stream_resume_attach;

/// [`spawn_foreground_stream_resume`] 入参（满足 clippy 形参上限）。
pub(crate) struct ForegroundStreamResumeArgs {
    pub chat: ChatSessionSignals,
    pub shell: ComposerStreamShell,
    pub locale: RwSignal<Locale>,
    pub selected_agent_role: RwSignal<Option<String>>,
    pub selected_session_mode: RwSignal<String>,
    pub job_id: u64,
    pub after_seq: u64,
    pub session_id: String,
}

fn resume_assistant_message_id(chat: ChatSessionSignals) -> Option<String> {
    chat.stream_text_overlay
        .get_untracked()
        .map(|o| o.message_id)
        .or_else(|| chat.stream_overlay_display_mid.get_untracked())
        .filter(|s| !s.trim().is_empty())
}

pub(crate) fn hydrate_after_background(chat: ChatSessionSignals, shell: &ComposerStreamShell) {
    if chat.stream_bound_resume_handles_untracked().is_some() {
        let attach_gen = chat.stream_attach_generation_untracked();
        reset_abort_state_for_new_attach(shell);
        chat.clear_stream_resume_handles();
        chat.clear_stream_text_overlay();
        shell.stream.apply_release_turn_and_stream_run(attach_gen);
    }
    bump_session_hydrate_nonce(chat);
    crate::mobile_stream_keepalive::on_stream_attach_finished();
}

pub(crate) fn spawn_foreground_stream_resume(args: ForegroundStreamResumeArgs) {
    let ForegroundStreamResumeArgs {
        chat,
        shell,
        locale,
        selected_agent_role,
        selected_session_mode,
        job_id,
        after_seq,
        session_id,
    } = args;
    let Some(asst_id) = resume_assistant_message_id(chat) else {
        hydrate_after_background(chat, &shell);
        return;
    };
    let prepared = prepare_stream_resume_attach(chat, &shell, locale, asst_id, session_id, job_id);
    // 后台流期间用户可能已切到其它会话：resume 的 conversation_id 必须取**绑定会话记录**，
    // 而非全局 session_sync 槽（后者只反映当前正在查看的会话），否则会把绑定会话的流续到错误会话。
    let conv = prepared
        .stream_ctx
        .bound_session_server_conversation_id()
        .or_else(|| {
            chat.session_sync
                .with_untracked(|s| s.stream_conversation_id())
        });
    let agent_role = selected_agent_role.get_untracked();
    let session_mode = {
        let m = selected_session_mode
            .get_untracked()
            .trim()
            .to_ascii_lowercase();
        if matches!(m.as_str(), "ask" | "plan" | "act") {
            Some(m)
        } else {
            None
        }
    };
    let cbs = build_chat_stream_callbacks(Rc::clone(&prepared.stream_ctx));
    let gen_snapshot = prepared.attach_generation;
    let shell_for_stream = shell.clone();
    let on_error_spawn = cbs.on_error.clone();
    let appr = prepared.approval_session_id.clone();
    spawn_local(async move {
        shell_for_stream
            .stream
            .dispatch_turn_lifecycle(TurnLifecycleEvent::HttpStreamOpened {
                attach_generation: gen_snapshot,
            });
        let stream_result = send_chat_stream(SendChatStreamParams {
            message: String::new(),
            image_urls: Vec::new(),
            conversation_id: conv,
            agent_role,
            session_mode,
            approval_session_id: Some(appr),
            stream_resume_job_id: Some(job_id),
            stream_resume_after_seq: Some(after_seq),
            signal: &prepared.abort_signal,
            cbs: cbs.clone(),
            loc: locale.get_untracked(),
            clarify_questionnaire_answers: None,
        })
        .await;
        if chat.stream_attach_generation_untracked() != gen_snapshot {
            return;
        }
        shell_for_stream
            .stream
            .apply_release_turn_and_stream_run(gen_snapshot);
        match stream_result {
            Ok(()) => {}
            Err(e)
                if user_cancelled_flag(&shell_for_stream)
                    || crate::i18n::is_stream_stopped_error(&e) => {}
            Err(e) => {
                shell_for_stream.stream.status_err.set(Some(e.clone()));
                on_error_spawn(e);
                bump_session_hydrate_nonce(chat);
            }
        }
        crate::mobile_stream_keepalive::on_stream_attach_finished();
    });
}
