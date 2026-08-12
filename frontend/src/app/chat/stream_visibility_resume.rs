//! 页面从后台回前台：连接已断才 `stream_resume`；否则按需水合。

use std::cell::Cell;
use std::rc::Rc;

use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use crate::app::chat::composer_stream::foreground_resume::{
    ForegroundStreamResumeArgs, hydrate_after_background, spawn_foreground_stream_resume,
};
use crate::app::chat::foreground_stream_action::{
    ForegroundStreamAction, ForegroundStreamDecisionInput, foreground_stream_action_after_hidden,
};
use crate::app::chat::handles::ComposerStreamShell;
use crate::chat_session_state::ChatSessionSignals;
use crate::i18n::Locale;
use crate::message_loading::messages_have_loading_tool;
use crate::storage::StoredMessageState;

fn stream_abort_present(shell: &ComposerStreamShell) -> bool {
    shell.stream.abort_cell.lock().unwrap().is_some()
}

fn looks_stream_busy(chat: ChatSessionSignals) -> bool {
    if chat.stream_bound_resume_handles_untracked().is_some() {
        return true;
    }
    if chat.stream_text_overlay.get_untracked().is_some() {
        return true;
    }
    let sid = chat
        .stream_transport
        .get_untracked()
        .bound_session_id()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| chat.active_id.get_untracked());
    chat.sessions.with_untracked(|sessions| {
        sessions.iter().any(|s| {
            s.id == sid
                && (messages_have_loading_tool(&s.messages)
                    || s.messages.iter().any(|m| {
                        !m.is_tool && matches!(m.state, Some(StoredMessageState::Loading))
                    }))
        })
    })
}

fn apply_foreground_stream_action(
    chat: ChatSessionSignals,
    shell: &ComposerStreamShell,
    locale: RwSignal<Locale>,
    selected_agent_role: RwSignal<Option<String>>,
    selected_session_mode: RwSignal<String>,
    action: ForegroundStreamAction,
) {
    match action {
        ForegroundStreamAction::None => {}
        ForegroundStreamAction::Hydrate => hydrate_after_background(chat, shell),
        ForegroundStreamAction::Resume {
            job_id,
            after_seq,
            session_id,
        } => {
            spawn_foreground_stream_resume(ForegroundStreamResumeArgs {
                chat,
                shell: shell.clone(),
                locale,
                selected_agent_role,
                selected_session_mode,
                job_id,
                after_seq,
                session_id,
            });
        }
    }
}

/// 订阅 `visibilitychange`：回前台时按需续传或水合（Android 切后台 P0）。
pub(crate) fn wire_stream_visibility_resume(
    initialized: RwSignal<bool>,
    chat: ChatSessionSignals,
    locale: RwSignal<Locale>,
    selected_agent_role: RwSignal<Option<String>>,
    selected_session_mode: RwSignal<String>,
    stream_shell: ComposerStreamShell,
) {
    let listener_registered = StoredValue::new(false);
    Effect::new(move |_| {
        if !initialized.get() {
            return;
        }
        if listener_registered.get_value() {
            return;
        }
        let Some(window) = web_sys::window() else {
            return;
        };
        let Some(doc) = window.document() else {
            return;
        };
        listener_registered.set_value(true);

        let was_hidden = Rc::new(Cell::new(false));
        let was_busy_when_hidden = Rc::new(Cell::new(false));
        let stream_shell = stream_shell.clone();

        let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
            let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
                return;
            };
            if doc.hidden() {
                was_hidden.set(true);
                was_busy_when_hidden.set(looks_stream_busy(chat));
                return;
            }
            let action = foreground_stream_action_after_hidden(ForegroundStreamDecisionInput {
                was_hidden: was_hidden.get(),
                bound: chat.stream_bound_resume_handles_untracked(),
                after_seq: chat.stream_last_sse_event_seq.get_untracked(),
                abort_present: stream_abort_present(&stream_shell),
                was_busy_when_hidden: was_busy_when_hidden.get(),
            });
            was_hidden.set(false);
            was_busy_when_hidden.set(false);
            apply_foreground_stream_action(
                chat,
                &stream_shell,
                locale,
                selected_agent_role,
                selected_session_mode,
                action,
            );
        });
        let _ =
            doc.add_event_listener_with_callback("visibilitychange", cb.as_ref().unchecked_ref());
        cb.forget();
    });
}
