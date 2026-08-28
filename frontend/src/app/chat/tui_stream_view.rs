//! TUI 风格聊天视图：每回合独立 wrap；操作经右键 / 长按菜单；live 按块局部更新。

use leptos::ev;
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

use super::find_highlight::{
    FindRestoreScope, apply_chat_find_highlights, find_restore_scope,
    reapply_chat_find_highlight_on_wrap,
};
use super::handles::ChatFindOverlaySignals;
use super::message_row_actions::MessageRowActionSignals;
use super::message_turn_menu::{
    MessageTurnContextMenuLayer, MessageTurnMenuAnchor, build_message_turn_press_handlers,
    try_open_message_turn_menu_from_keydown,
};
use super::scroll_follow::follow_after_content_paint;
use super::scroll_shell::ChatScrollShellSignals;
use super::session_hydrate::try_load_older_messages_for_active_session;
use super::stream_follow_up_gates::user_edit_save_blocked;
use super::tui_actions_bar::TuiTurnActionHandlers;
use super::tui_body_dom::{apply_incremental_tail, find_answer_body, reconcile_answer_hidden};
use super::tui_line_markdown::TuiBodyPatch;
use super::tui_transcript_sync::{PlanTuiSyncArgs, TuiMountState, TuiSyncPlan, plan_tui_sync};
use super::user_message_edit::{
    UserEditClick, mount_user_message_editor, try_handle_user_edit_click, try_sync_user_edit_draft,
};
use super::workspace_image_hydrate::{
    revoke_workspace_image_blobs, schedule_workspace_image_hydrate,
};
use crate::api::post_tool_job_cancel;
use crate::chat_session_state::ChatSessionSignals;
use crate::i18n::{self, Locale};
use crate::md_code_copy::try_copy_md_code_block;
use crate::session_ops::set_user_message_text;
use crate::session_search::normalize_search_query;
use crate::sse_dispatch::ToolJobState;
use crate::storage::ChatSession;
use crate::stream_text_overlay::StreamTextOverlay;
use std::collections::{HashMap, HashSet};

struct PlanActiveSessionArgs<'a> {
    sessions: &'a [ChatSession],
    active_id: &'a str,
    prev: Option<&'a TuiMountState>,
    overlay: Option<&'a StreamTextOverlay>,
    locale: Locale,
    apply_filters: bool,
    markdown_render: bool,
    show_turn_context_inject: bool,
    tool_chunks: &'a HashMap<String, String>,
    tool_jobs: &'a HashMap<String, ToolJobState>,
    think_open: &'a HashSet<String>,
}

fn plan_for_active_session(args: PlanActiveSessionArgs<'_>) -> TuiSyncPlan {
    let PlanActiveSessionArgs {
        sessions,
        active_id,
        prev,
        overlay,
        locale,
        apply_filters,
        markdown_render,
        show_turn_context_inject,
        tool_chunks,
        tool_jobs,
        think_open,
    } = args;
    match sessions.iter().find(|session| session.id == active_id) {
        None => plan_tui_sync(PlanTuiSyncArgs {
            prev,
            messages: &[],
            session_id: active_id,
            overlay: None,
            locale,
            apply_assistant_display_filters: apply_filters,
            markdown_render,
            show_turn_context_inject,
            tool_chunks,
            tool_jobs,
            think_open,
        }),
        Some(session) => plan_tui_sync(PlanTuiSyncArgs {
            prev,
            messages: &session.messages,
            session_id: &session.id,
            overlay,
            locale,
            apply_assistant_display_filters: apply_filters,
            markdown_render,
            show_turn_context_inject,
            tool_chunks,
            tool_jobs,
            think_open,
        }),
    }
}

fn apply_tool_row_patch(
    body: &web_sys::HtmlElement,
    status: &str,
    status_label: &str,
    one_line: &str,
    detail: Option<&str>,
) -> bool {
    let Some(status_el) = body
        .query_selector(".chat-tui-tool-status")
        .ok()
        .flatten()
        .and_then(|n| n.dyn_into::<web_sys::HtmlElement>().ok())
    else {
        return false;
    };
    let Some(one_el) = body
        .query_selector(".chat-tui-tool-one-line")
        .ok()
        .flatten()
        .and_then(|n| n.dyn_into::<web_sys::HtmlElement>().ok())
    else {
        return false;
    };
    status_el.set_text_content(Some(status));
    let _ = status_el.set_attribute("aria-label", status_label);
    let _ = status_el.set_attribute("title", status_label);
    one_el.set_text_content(Some(one_line));
    match detail {
        Some(text) => {
            let Some(pre) = body
                .query_selector(".chat-tui-tool-detail-body")
                .ok()
                .flatten()
                .and_then(|n| n.dyn_into::<web_sys::HtmlElement>().ok())
            else {
                // 需要 details 但 DOM 仍是无详情结构 → 交由全量重建
                return false;
            };
            pre.set_text_content(Some(text));
        }
        None => {
            if body
                .query_selector(".chat-tui-tool-details")
                .ok()
                .flatten()
                .is_some()
            {
                // DOM 仍有 details、计划已无 → 结构变化，重建
                return false;
            }
        }
    }
    true
}

fn apply_body_patch(
    wrap: &web_sys::HtmlElement,
    patch: TuiBodyPatch,
    markdown_render: bool,
) -> bool {
    match patch {
        TuiBodyPatch::ReplaceAll { chunks } => {
            // 思考段独立成段：先同步思考段（出现/消失/open 翻转），再整段替换正文段。
            if !super::tui_thinking_block::update_think_section(wrap, chunks.think.as_ref()) {
                return false;
            }
            let Some(body) = find_answer_body(wrap) else {
                return false;
            };
            let answer_html = chunks.answer_to_inner_html();
            // 内容相同时跳过整段重建（finalize 时正文与流式末帧一致），避免 WebKit 闪烁。
            if body.inner_html() != answer_html {
                revoke_workspace_image_blobs(&body);
                body.set_inner_html(&answer_html);
            }
            reconcile_answer_hidden(wrap, answer_html.trim().is_empty());
            true
        }
        TuiBodyPatch::Incremental {
            append_closed,
            open_plain,
        } => {
            if append_closed.is_empty() && open_plain.is_none() {
                // 无正文增量：维持正文段现状（含 hidden 状态）。
                return true;
            }
            let Some(body) = find_answer_body(wrap) else {
                return false;
            };
            let applied = apply_incremental_tail(
                &body,
                &append_closed,
                open_plain.as_deref(),
                markdown_render,
            );
            if applied {
                // 增量路径正文只增不减：有增量即非空，解除隐藏（避免逐帧序列化 innerHTML）。
                reconcile_answer_hidden(wrap, false);
            }
            applied
        }
        TuiBodyPatch::ThinkBody {
            body_html,
            append_closed,
            open_plain,
        } => {
            // 只更新思考段正文（open/summary 稳定，不重渲整个 details）。
            let Some(think_body) = wrap
                .query_selector(".chat-tui-body--think .chat-tui-think-body")
                .ok()
                .flatten()
                .and_then(|n| n.dyn_into::<web_sys::HtmlElement>().ok())
            else {
                // DOM 尚无思考段（结构已变）→ 交由全量重建
                return false;
            };
            think_body.set_inner_html(&body_html);
            let Some(body) = find_answer_body(wrap) else {
                return false;
            };
            if append_closed.is_empty() && open_plain.is_none() {
                // 纯思考流式：正文尚未开始，维持隐藏状态（若有）。
                return true;
            }
            let applied = apply_incremental_tail(
                &body,
                &append_closed,
                open_plain.as_deref(),
                markdown_render,
            );
            if applied {
                // 同 Incremental：有正文增量即非空，解除隐藏。
                reconcile_answer_hidden(wrap, false);
            }
            applied
        }
        TuiBodyPatch::ToolRow {
            status,
            status_label,
            one_line,
            detail,
        } => {
            let Some(body) = find_answer_body(wrap) else {
                return false;
            };
            apply_tool_row_patch(&body, &status, &status_label, &one_line, detail.as_deref())
        }
    }
}

fn find_turn_wrap(
    transcript: &web_sys::HtmlElement,
    message_id: &str,
) -> Option<web_sys::HtmlElement> {
    let selector = format!(".chat-tui-turn-wrap[data-tui-wrap-id=\"{message_id}\"]");
    transcript
        .query_selector(&selector)
        .ok()
        .flatten()
        .and_then(|n| n.dyn_into::<web_sys::HtmlElement>().ok())
}

fn apply_actions_html(transcript: &web_sys::HtmlElement, message_id: &str, html: &str) -> bool {
    let Some(wrap) = find_turn_wrap(transcript, message_id) else {
        return false;
    };
    if let Some(existing) = wrap.query_selector(".chat-tui-turn-actions").ok().flatten() {
        existing.remove();
    }
    if html.is_empty() {
        return true;
    }
    wrap.insert_adjacent_html("beforeend", html).is_ok()
}

fn apply_tui_promote_and_appends(transcript: &web_sys::HtmlElement, plan: &TuiSyncPlan) -> bool {
    if let Some(promote_id) = &plan.promote_id
        && let Some(wrap) = find_turn_wrap(transcript, promote_id)
        && let Ok(sections) = wrap.query_selector_all("section.chat-tui-turn")
    {
        // 思考段 + 正文段同属一个 wrap，需一并摘掉 live/loading 标记。
        for i in 0..sections.length() {
            let Some(node) = sections.get(i) else {
                continue;
            };
            let Ok(section) = node.dyn_into::<web_sys::Element>() else {
                continue;
            };
            let _ = section.remove_attribute("data-tui-live");
            let _ = section.class_list().remove_1("chat-tui-turn--live");
            let _ = section.class_list().remove_1("is-loading");
        }
    }

    if plan.append_sections.is_empty() {
        return true;
    }
    if let Some(empty) = transcript.query_selector(".chat-tui-empty").ok().flatten() {
        empty.remove();
    }
    for section_html in &plan.append_sections {
        if transcript
            .insert_adjacent_html("beforeend", section_html)
            .is_err()
        {
            return false;
        }
    }
    true
}

fn apply_tui_body_and_action_patches(
    transcript: &web_sys::HtmlElement,
    plan: &TuiSyncPlan,
) -> bool {
    if let Some(live) = &plan.live {
        let Some(wrap) = find_turn_wrap(transcript, &live.message_id) else {
            return false;
        };
        if !apply_body_patch(&wrap, live.patch.clone(), live.markdown_render) {
            return false;
        }
    }

    for refresh in &plan.refresh_bodies {
        let Some(wrap) = find_turn_wrap(transcript, &refresh.message_id) else {
            return false;
        };
        if !apply_body_patch(&wrap, refresh.patch.clone(), refresh.markdown_render) {
            return false;
        }
    }

    for actions in &plan.refresh_actions {
        if !apply_actions_html(transcript, &actions.message_id, &actions.html) {
            return false;
        }
    }
    true
}

fn apply_tui_sync_plan(transcript: &web_sys::HtmlElement, plan: &TuiSyncPlan) -> bool {
    if let Some(html) = &plan.full_html {
        revoke_workspace_image_blobs(transcript);
        transcript.set_inner_html(html);
        schedule_workspace_image_hydrate(transcript);
        return true;
    }
    if !apply_tui_promote_and_appends(transcript, plan) {
        return false;
    }
    if !apply_tui_body_and_action_patches(transcript, plan) {
        return false;
    }
    schedule_workspace_image_hydrate(transcript);
    true
}

fn find_restore_scope_from_plan(plan: &TuiSyncPlan) -> FindRestoreScope {
    find_restore_scope(
        plan.full_html.is_some()
            || !plan.append_sections.is_empty()
            || plan.promote_id.is_some()
            || !plan.refresh_bodies.is_empty(),
        plan.live.is_some(),
    )
}

fn apply_or_rebuild_tui_mount(
    el: &web_sys::HtmlElement,
    plan: TuiSyncPlan,
    mount_state: RwSignal<Option<TuiMountState>>,
    rebuild: impl FnOnce() -> TuiSyncPlan,
) -> FindRestoreScope {
    let scope = find_restore_scope_from_plan(&plan);
    if apply_tui_sync_plan(el, &plan) {
        mount_state.set(Some(plan.next));
        return scope;
    }
    let forced = rebuild();
    if apply_tui_sync_plan(el, &forced) {
        mount_state.set(Some(forced.next));
        return FindRestoreScope::Full;
    }
    web_sys::console::warn_1(
        &"chat-tui: forced rebuild failed; keeping previous mount_state".into(),
    );
    FindRestoreScope::None
}

struct TuiStreamDisplayOpts {
    locale: Locale,
    apply_filters: bool,
    markdown_render: bool,
    show_turn_context_inject: bool,
}

fn sync_chat_tui_stream_dom(
    chat: ChatSessionSignals,
    display: TuiStreamDisplayOpts,
    transcript_ref: NodeRef<leptos::html::Div>,
    mount_state: RwSignal<Option<TuiMountState>>,
    scroll_shell: ChatScrollShellSignals,
    think_open: &HashSet<String>,
) -> (FindRestoreScope, Option<String>) {
    let tool_chunks = chat.tool_output_chunks.get();
    let tool_jobs = chat.tool_job_states.get();
    let active_id = chat.active_id.get();
    let overlay = chat.stream_text_overlay.get();
    let live_id = overlay.as_ref().map(|o| o.message_id.clone());
    let prev = mount_state.get_untracked();
    let plan = chat.sessions.with(|sessions| {
        plan_for_active_session(PlanActiveSessionArgs {
            sessions,
            active_id: &active_id,
            prev: prev.as_ref(),
            overlay: overlay.as_ref(),
            locale: display.locale,
            apply_filters: display.apply_filters,
            markdown_render: display.markdown_render,
            show_turn_context_inject: display.show_turn_context_inject,
            tool_chunks: &tool_chunks,
            tool_jobs: &tool_jobs,
            think_open,
        })
    });

    let Some(node) = transcript_ref.get() else {
        return (FindRestoreScope::None, live_id);
    };
    let Some(el) = node.dyn_ref::<web_sys::HtmlElement>() else {
        return (FindRestoreScope::None, live_id);
    };

    let scope = apply_or_rebuild_tui_mount(el, plan, mount_state, || {
        chat.sessions.with(|sessions| {
            plan_for_active_session(PlanActiveSessionArgs {
                sessions,
                active_id: &active_id,
                prev: None,
                overlay: overlay.as_ref(),
                locale: display.locale,
                apply_filters: display.apply_filters,
                markdown_render: display.markdown_render,
                show_turn_context_inject: display.show_turn_context_inject,
                tool_chunks: &tool_chunks,
                tool_jobs: &tool_jobs,
                think_open,
            })
        })
    });
    follow_after_content_paint(scroll_shell);
    (scope, live_id)
}

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

fn transcript_html_el(transcript_ref: NodeRef<leptos::html::Div>) -> Option<web_sys::HtmlElement> {
    transcript_ref
        .get()
        .and_then(|node| node.dyn_into::<web_sys::HtmlElement>().ok())
}

struct FindOverlaySnapshot {
    query: String,
    match_ids: Vec<String>,
    current_id: Option<String>,
    open: bool,
}

fn find_overlay_snapshot_tracked(find: ChatFindOverlaySignals) -> FindOverlaySnapshot {
    let match_ids = find.match_ids.get();
    let current_id = match_ids.get(find.cursor.get()).cloned();
    FindOverlaySnapshot {
        query: normalize_search_query(&find.query.get()),
        match_ids,
        current_id,
        open: find.panel_open.get(),
    }
}

fn find_overlay_snapshot_untracked(find: ChatFindOverlaySignals) -> FindOverlaySnapshot {
    let match_ids = find.match_ids.get_untracked();
    let current_id = match_ids.get(find.cursor.get_untracked()).cloned();
    FindOverlaySnapshot {
        query: normalize_search_query(&find.query.get_untracked()),
        match_ids,
        current_id,
        open: find.panel_open.get_untracked(),
    }
}

fn apply_find_snapshot(el: &web_sys::HtmlElement, snap: &FindOverlaySnapshot) {
    if snap.open {
        apply_chat_find_highlights(
            el,
            snap.query.as_str(),
            &snap.match_ids,
            snap.current_id.as_deref(),
        );
    } else {
        apply_chat_find_highlights(el, "", &[], None);
    }
}

fn restore_overlays_after_tui_sync(
    transcript_ref: NodeRef<leptos::html::Div>,
    scope: FindRestoreScope,
    live_id: Option<&str>,
    editing: RwSignal<Option<super::user_message_edit::UserMessageEdit>>,
    locale: Locale,
    find: ChatFindOverlaySignals,
) {
    let Some(el) = transcript_html_el(transcript_ref) else {
        return;
    };
    let snap = find_overlay_snapshot_untracked(find);
    match scope {
        FindRestoreScope::None => {}
        FindRestoreScope::LiveWrap => {
            if snap.open
                && let Some(id) = live_id
            {
                reapply_chat_find_highlight_on_wrap(
                    &el,
                    id,
                    snap.query.as_str(),
                    &snap.match_ids,
                    snap.current_id.as_deref(),
                );
            }
        }
        FindRestoreScope::Full => {
            mount_user_message_editor(&el, editing.get_untracked().as_ref(), locale);
            apply_find_snapshot(&el, &snap);
        }
    }
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

    Effect::new(move |_| {
        let _ = chat.stream_overlay_revision.get();
        let loc = locale.get();
        let apply_filters = apply_assistant_display_filters.get();
        let md_on = markdown_render.get();
        let show_inject = show_turn_context_inject.get();
        // untracked 快照：toggle 时 DOM 已原生生效，无需因此重渲染；仅在后续 body 重建时读取。
        let think_open = think_manually_open.get_untracked();
        let (scope, live_id) = sync_chat_tui_stream_dom(
            chat,
            TuiStreamDisplayOpts {
                locale: loc,
                apply_filters,
                markdown_render: md_on,
                show_turn_context_inject: show_inject,
            },
            transcript_ref,
            mount_state,
            scroll_shell,
            &think_open,
        );
        restore_overlays_after_tui_sync(
            transcript_ref,
            scope,
            live_id.as_deref(),
            editing_user_message,
            loc,
            find,
        );
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
