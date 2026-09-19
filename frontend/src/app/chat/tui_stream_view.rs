//! TUI 风格聊天视图：每回合独立 wrap；操作经右键 / 长按菜单；live 按块局部更新。
//!
//! DOM 同步执行域（plan 应用、每 token 合帧 Effect）在 [`tui_stream_dom_sync`]。

use leptos::ev;
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

use super::handles::{ChatFindOverlaySignals, IdeOpenFileBridgeSignals};
use super::message_row_actions::MessageRowActionSignals;
use super::message_turn_menu::{
    MessageTurnContextMenuLayer, MessageTurnMenuAnchor, build_message_turn_press_handlers,
    try_open_message_turn_menu_from_keydown,
};
use super::scroll_shell::ChatScrollShellSignals;
use super::session_hydrate::try_load_older_messages_for_active_session;
use super::stream_follow_up_gates::user_edit_save_blocked;
use super::tui_actions_bar::TuiTurnActionHandlers;
use super::tui_stream_dom_sync::{
    TuiStreamSyncSignals, apply_find_snapshot, find_overlay_snapshot_tracked, transcript_html_el,
    wire_tui_stream_sync_effect,
};
use super::tui_transcript_sync::TuiMountState;
use super::user_message_edit::{
    UserEditClick, mount_user_message_editor, try_handle_user_edit_click, try_sync_user_edit_draft,
};
use crate::api::post_tool_job_cancel;
use crate::chat_session_state::ChatSessionSignals;
use crate::i18n::{self, Locale};
use crate::md_code_copy::try_copy_md_code_block;
use crate::session_ops::set_user_message_text;
use std::collections::HashSet;

/// 思维链折叠块 summary 点击：记录该 message 的**手动展开状态**（原生 `<details>` 已自行切换，
/// 此处仅持久化到 [`think_manually_open`]，供后续 body 重建时保持；流式自动展开/结束后收起不受影响）。
fn try_handle_think_toggle_click(
    ev: &web_sys::MouseEvent,
    think_manually_open: RwSignal<HashSet<String>>,
) -> bool {
    let Some(target) = ev
        .target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
    else {
        return false;
    };
    if target
        .closest(".chat-tui-think-summary")
        .ok()
        .flatten()
        .is_none()
    {
        return false;
    }
    let Some(section) = target.closest("[data-tui-msg-id]").ok().flatten() else {
        return false;
    };
    let Some(message_id) = section.get_attribute("data-tui-msg-id") else {
        return false;
    };
    // 点击已触发原生 toggle，此刻 `open` 属性反映**新**状态。
    let now_open = section
        .query_selector(".chat-tui-think")
        .ok()
        .flatten()
        .is_some_and(|el| el.has_attribute("open"));
    think_manually_open.update(|set| {
        if now_open {
            set.insert(message_id);
        } else {
            set.remove(&message_id);
        }
    });
    true
}

/// 后台任务取消按钮点击（`.chat-tui-tool-job-cancel`）：消费点击、禁用按钮并 POST 取消，
/// 成功后把任务状态落为返回值（`cancelled` 或当前终态）。返回是否已消费该点击。
fn try_handle_tool_job_cancel_click(
    ev: &web_sys::MouseEvent,
    chat: ChatSessionSignals,
    locale: Locale,
) -> bool {
    let Some(btn) = ev
        .target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
        .and_then(|el| el.closest(".chat-tui-tool-job-cancel").ok().flatten())
    else {
        return false;
    };
    let Some(job_id) = btn.get_attribute("data-tool-job-id") else {
        return false;
    };
    let job_id = job_id.trim().to_string();
    if job_id.is_empty() {
        return false;
    }
    ev.prevent_default();
    if let Ok(b) = btn.dyn_into::<web_sys::HtmlButtonElement>() {
        let _ = b.set_attribute("disabled", "");
    }
    let states = chat.tool_job_states;
    spawn_local(async move {
        if let Ok(status) = post_tool_job_cancel(&job_id, locale).await {
            states.update(|m| {
                if let Some(state) = m.values_mut().find(|s| s.id == job_id) {
                    state.status = status;
                }
            });
        }
        // 失败时下一次响应式渲染会重建按钮（不禁用态持久化）。
    });
    true
}

/// 工具卡「打开此文件」点击（`.chat-tui-tool-open-file`）：消费点击并把工作区相对路径
/// 经 nonce+path 桥接给 IDE 布局 Effect。返回是否已消费该点击。
fn try_handle_tool_open_file_click(
    ev: &web_sys::MouseEvent,
    bridge: IdeOpenFileBridgeSignals,
) -> bool {
    let Some(btn) = ev
        .target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
        .and_then(|el| el.closest(".chat-tui-tool-open-file").ok().flatten())
    else {
        return false;
    };
    let Some(path) = btn.get_attribute("data-file-path") else {
        return false;
    };
    let path = path.trim().to_string();
    if path.is_empty() {
        return false;
    }
    ev.prevent_default();
    bridge.path.set(Some(path));
    bridge.nonce.update(|n| *n = n.wrapping_add(1));
    true
}

fn save_edited_user_message(handlers: TuiTurnActionHandlers, message_id: String, text: String) {
    let trimmed = text.trim().to_string();
    if trimmed.is_empty() {
        return;
    }
    let follow_up = handlers.stream_follow_up.get_untracked();
    if user_edit_save_blocked(
        handlers.stream_turn_busy_ui.get_untracked(),
        follow_up.blocks_user_edit_save(),
    ) {
        handlers.status_err.set(Some(
            i18n::msg_edit_save_busy(handlers.locale.get_untracked()).to_string(),
        ));
        return;
    }
    let aid = handlers.chat.active_id.get_untracked();
    let mut idx = None;
    handlers.chat.update_sessions_message_row(|list| {
        let _ = set_user_message_text(list, &aid, &message_id, &trimmed);
        idx = list
            .iter()
            .find(|s| s.id == aid)
            .and_then(|s| s.messages.iter().position(|m| m.id == message_id));
    });
    let Some(msg_idx) = idx else {
        return;
    };
    MessageRowActionSignals {
        chat: handlers.chat,
        stream_follow_up: handlers.stream_follow_up,
        status_err: handlers.status_err,
        locale: handlers.locale,
    }
    .spawn_regenerate_from_user_line(msg_idx, message_id);
}

#[component]
fn ChatTuiHistoryLoadStrip(
    history_flags: Memo<(bool, bool)>,
    chat: ChatSessionSignals,
    locale: RwSignal<Locale>,
    scroll_shell: ChatScrollShellSignals,
    status_err: RwSignal<Option<String>>,
) -> impl IntoView {
    view! {
        <Show when=move || {
            let (has_older, loading_older) = history_flags.get();
            has_older || loading_older
        }>
            <div class="messages-history-load" role="status">
                <Show
                    when=move || history_flags.get().1
                    fallback=move || {
                        view! {
                            <button
                                type="button"
                                class="btn btn-ghost btn-sm"
                                data-testid="chat-load-older"
                                on:click=move |_| {
                                    try_load_older_messages_for_active_session(
                                        chat,
                                        locale.get_untracked(),
                                        scroll_shell,
                                        status_err,
                                    );
                                }
                            >
                                {move || i18n::chat_history_load_older(locale.get())}
                            </button>
                        }
                    }
                >
                    <span class="messages-history-load-busy">
                        {move || i18n::chat_history_loading_older(locale.get())}
                    </span>
                </Show>
            </div>
        </Show>
    }
}

#[component]
pub(crate) fn ChatTuiStreamView(
    chat: ChatSessionSignals,
    locale: RwSignal<Locale>,
    apply_assistant_display_filters: RwSignal<bool>,
    markdown_render: RwSignal<bool>,
    show_turn_context_inject: RwSignal<bool>,
    scroll_shell: ChatScrollShellSignals,
    action_handlers: TuiTurnActionHandlers,
    find: ChatFindOverlaySignals,
    ide_open_file: IdeOpenFileBridgeSignals,
) -> impl IntoView {
    let status_err = action_handlers.status_err;
    let editing_user_message = action_handlers.editing_user_message;
    let transcript_ref = NodeRef::<leptos::html::Div>::new();
    let mount_state = RwSignal::new(None::<TuiMountState>);
    // 用户手动展开的思维链折叠块（message id 集合）：原生 toggle 后记录，body 重建时保持展开。
    let think_manually_open = RwSignal::new(HashSet::<String>::new());
    let turn_menu = RwSignal::new(None::<MessageTurnMenuAnchor>);
    let press = build_message_turn_press_handlers(chat, turn_menu);
    let on_contextmenu = press.on_contextmenu.clone();
    let on_pointerdown = press.on_pointerdown.clone();
    let on_pointermove = press.on_pointermove.clone();
    let on_pointer_end = press.on_pointer_end.clone();
    let on_pointer_end_cancel = press.on_pointer_end.clone();
    let try_consume_suppress_click = press.try_consume_suppress_click.clone();

    window_event_listener(ev::keydown, move |ev| {
        if ev.key() != "Escape" {
            return;
        }
        if turn_menu.get_untracked().is_some() {
            turn_menu.set(None);
        }
        if editing_user_message.get_untracked().is_some() {
            editing_user_message.set(None);
        }
    });

    let editing_message_id =
        Memo::new(move |_| editing_user_message.with(|e| e.as_ref().map(|x| x.message_id.clone())));

    // 每 token 合帧 Effect（同帧多次触发合并为一次 rAF 内 DOM 同步）已封装在
    // wire_tui_stream_sync_effect，见 tui_stream_dom_sync。
    wire_tui_stream_sync_effect(TuiStreamSyncSignals {
        chat,
        locale,
        apply_assistant_display_filters,
        markdown_render,
        show_turn_context_inject,
        ide_narrow: ide_open_file.narrow,
        think_manually_open,
        transcript_ref,
        mount_state,
        scroll_shell,
        editing_user_message,
        find,
    });

    Effect::new(move |_| {
        let _ = editing_message_id.get();
        let loc = locale.get();
        let Some(el) = transcript_html_el(transcript_ref) else {
            return;
        };
        mount_user_message_editor(&el, editing_user_message.get_untracked().as_ref(), loc);
    });

    Effect::new(move |_| {
        let _ = chat.active_id.get();
        let snap = find_overlay_snapshot_tracked(find);
        let Some(el) = transcript_html_el(transcript_ref) else {
            return;
        };
        apply_find_snapshot(&el, &snap);
    });

    let history_flags = Memo::new(move |_| {
        let id = chat.active_id.get();
        chat.sessions.with(|list| {
            list.iter()
                .find(|s| s.id == id)
                .map(|s| (s.history_has_older_flag(), chat.history_loading_older.get()))
                .unwrap_or((false, false))
        })
    });

    view! {
        <div
            class="messages-inner chat-tui-inner"
            data-testid="chat-tui-stream-view"
        >
            <ChatTuiHistoryLoadStrip
                history_flags
                chat
                locale
                scroll_shell
                status_err
            />
            <div
                class="chat-tui-transcript"
                data-testid="chat-tui-transcript"
                node_ref=transcript_ref
                aria-live="polite"
                aria-atomic="false"
                on:contextmenu=move |ev| on_contextmenu(ev)
                on:keydown=move |ev: web_sys::KeyboardEvent| {
                    try_open_message_turn_menu_from_keydown(&ev, chat, turn_menu);
                }
                on:pointerdown=move |ev| on_pointerdown(ev)
                on:pointermove=move |ev| on_pointermove(ev)
                on:pointerup=move |_| on_pointer_end()
                on:pointercancel=move |_| on_pointer_end_cancel()
                on:input=move |ev: web_sys::Event| {
                    try_sync_user_edit_draft(&ev, editing_user_message);
                }
                on:click=move |ev| {
                    if try_handle_think_toggle_click(&ev, think_manually_open) {
                        return;
                    }
                    if try_handle_tool_job_cancel_click(&ev, chat, locale.get_untracked()) {
                        return;
                    }
                    if try_handle_tool_open_file_click(&ev, ide_open_file) {
                        return;
                    }
                    match try_handle_user_edit_click(&ev, editing_user_message) {
                        UserEditClick::None => {}
                        UserEditClick::Cancel => return,
                        UserEditClick::Save { message_id, text } => {
                            save_edited_user_message(action_handlers, message_id, text);
                            return;
                        }
                    }
                    if try_copy_md_code_block(&ev, locale.get_untracked()) {
                        return;
                    }
                    let _ = try_consume_suppress_click();
                }
            />
            <MessageTurnContextMenuLayer
                locale=locale
                menu=turn_menu
                action_handlers=action_handlers
            />
        </div>
    }
}
