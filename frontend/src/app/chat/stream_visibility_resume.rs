//! 页面从后台回前台：有 `job_id` 则 `stream_resume` 软续传，否则水合会话快照。

use std::cell::Cell;
use std::rc::Rc;

use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use crate::app::chat::composer_stream::foreground_resume::{
    ForegroundStreamResumeArgs, hydrate_after_background, spawn_foreground_stream_resume,
};
use crate::app::chat::foreground_stream_action::{
    ForegroundStreamAction, foreground_stream_action_after_hidden,
};
use crate::app::chat::handles::ComposerStreamShell;
use crate::chat_session_state::ChatSessionSignals;
use crate::i18n::Locale;

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

/// 订阅 `visibilitychange`：回前台时续传或水合（Android 切后台 P0）。
pub(crate) fn wire_stream_visibility_resume(
    initialized: RwSignal<bool>,
    chat: ChatSessionSignals,
    locale: RwSignal<Locale>,
    selected_agent_role: RwSignal<Option<String>>,
    selected_session_mode: RwSignal<String>,
    stream_shell: ComposerStreamShell,
) {
    Effect::new(move |_| {
        if !initialized.get() {
            return;
        }
        let Some(window) = web_sys::window() else {
            return;
        };
        let Some(doc) = window.document() else {
            return;
        };

        let was_hidden = Rc::new(Cell::new(false));
        let stream_shell = stream_shell.clone();

        let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
            let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
                return;
            };
            if doc.hidden() {
                was_hidden.set(true);
                return;
            }
            let action = foreground_stream_action_after_hidden(
                was_hidden.get(),
                chat.stream_bound_resume_handles_untracked(),
                chat.stream_last_sse_event_seq.get_untracked(),
            );
            was_hidden.set(false);
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
