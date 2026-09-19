//! TUI 会话 DOM 同步执行域：把 [`TuiSyncPlan`] 应用到 transcript DOM，
//! 并提供每 token 合帧的同步 Effect 调度（组件只负责挂载与事件）。

use leptos::prelude::*;
use leptos_dom::helpers::request_animation_frame;
use wasm_bindgen::JsCast;

use super::find_highlight::{
    FindRestoreScope, apply_chat_find_highlights, find_restore_scope,
    reapply_chat_find_highlight_on_wrap,
};
use super::handles::ChatFindOverlaySignals;
use super::scroll_follow::follow_after_content_paint;
use super::scroll_shell::ChatScrollShellSignals;
use super::tui_body_dom::{apply_incremental_tail, find_answer_body, reconcile_answer_hidden};
use super::tui_line_markdown::TuiBodyPatch;
use super::tui_transcript_sync::{PlanTuiSyncArgs, TuiMountState, TuiSyncPlan, plan_tui_sync};
use super::user_message_edit::{UserMessageEdit, mount_user_message_editor};
use super::workspace_image_hydrate::{
    revoke_workspace_image_blobs, schedule_workspace_image_hydrate,
};
use crate::chat_session_state::ChatSessionSignals;
use crate::i18n::Locale;
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
    tool_file_paths: &'a HashMap<String, String>,
    open_file_enabled: bool,
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
        tool_file_paths,
        open_file_enabled,
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
            tool_file_paths,
            open_file_enabled,
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
            tool_file_paths,
            open_file_enabled,
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

/// plan 是否会写入可能携带工作区图 / `/uploads/` 附图的新 DOM（需要 hydrate 扫描）。
///
/// 流式每 token 的纯文本增量（活跃块更新）与工具行文案不含新 img；全量重建、
/// 新回合追加、body 重建（ReplaceAll）、携带新闭合块的增量以及思维链正文
/// 流式重写（markdown 渲染下 `body_html` 可含工作区图）需要扫描。
fn plan_touches_images(plan: &TuiSyncPlan) -> bool {
    if plan.full_html.is_some()
        || !plan.append_sections.is_empty()
        || !plan.refresh_bodies.is_empty()
    {
        return true;
    }
    let Some(live) = plan.live.as_ref() else {
        return false;
    };
    match &live.patch {
        TuiBodyPatch::ReplaceAll { .. } => true,
        // 新闭合块由 markdown 渲染（可能含 img）；活跃块未闭合不成图
        TuiBodyPatch::Incremental { append_closed, .. } => !append_closed.is_empty(),
        // ThinkBody 每帧整块重写思维链正文 innerHTML，img 节点会被重建并丢失
        // hydrate 标记；body_html 含 img 标记时必须重新扫描（纯文本阶段跳过）。
        TuiBodyPatch::ThinkBody {
            body_html,
            append_closed,
            ..
        } => !append_closed.is_empty() || body_html.contains("<img"),
        // 工具行只 set_text_content，绝不引入 img
        TuiBodyPatch::ToolRow { .. } => false,
    }
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
    if plan_touches_images(plan) {
        schedule_workspace_image_hydrate(transcript);
    }
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
    /// 宽屏判定（窄屏 / 移动远端无 IDE 布局，不注入「打开此文件」按钮）。
    open_file_enabled: bool,
}

fn sync_chat_tui_stream_dom(
    chat: ChatSessionSignals,
    display: TuiStreamDisplayOpts,
    transcript_ref: NodeRef<leptos::html::Div>,
    mount_state: RwSignal<Option<TuiMountState>>,
    scroll_shell: ChatScrollShellSignals,
    think_open: &HashSet<String>,
) -> (FindRestoreScope, Option<String>) {
    let active_id = chat.active_id.get();
    let prev = mount_state.get_untracked();
    chat.sessions.with(|sessions| {
        chat.stream_text_overlay.with(|overlay| {
            chat.tool_output_chunks.with(|tool_chunks| {
                chat.tool_job_states.with(|tool_jobs| {
                    chat.tool_file_paths.with(|tool_file_paths| {
                        // 嵌套 with 零拷贝借用：避免每 token 把整份 overlay（含累计正文/思维链）
                        // 与各工具 HashMap `.get()` 深克隆出来（O(累计文本)）。
                        //
                        // 约束：此借用块内只能**读**这些信号并把副作用限制在 DOM/滚动上；
                        // 若未来有代码在同一信号上 `update()`（会重入），必须先把数据拷出借用块。
                        let overlay = overlay.as_ref();
                        let live_id = overlay.map(|o| o.message_id.clone());
                        // plan 在 with 内先算：即使 transcript DOM 暂缺，Effect 也先建立对这些信号的
                        // 依赖，之后再取节点；DOM 容器在组件挂载时即存在，节点缺失仅是很窄的窗口。
                        let plan = plan_for_active_session(PlanActiveSessionArgs {
                            sessions,
                            active_id: &active_id,
                            prev: prev.as_ref(),
                            overlay,
                            locale: display.locale,
                            apply_filters: display.apply_filters,
                            markdown_render: display.markdown_render,
                            show_turn_context_inject: display.show_turn_context_inject,
                            tool_chunks,
                            tool_jobs,
                            tool_file_paths,
                            open_file_enabled: display.open_file_enabled,
                            think_open,
                        });
                        let Some(node) = transcript_ref.get() else {
                            return (FindRestoreScope::None, live_id);
                        };
                        let Some(el) = node.dyn_ref::<web_sys::HtmlElement>() else {
                            return (FindRestoreScope::None, live_id);
                        };
                        let scope = apply_or_rebuild_tui_mount(el, plan, mount_state, || {
                            plan_for_active_session(PlanActiveSessionArgs {
                                sessions,
                                active_id: &active_id,
                                prev: None,
                                overlay,
                                locale: display.locale,
                                apply_filters: display.apply_filters,
                                markdown_render: display.markdown_render,
                                show_turn_context_inject: display.show_turn_context_inject,
                                tool_chunks,
                                tool_jobs,
                                tool_file_paths,
                                open_file_enabled: display.open_file_enabled,
                                think_open,
                            })
                        });
                        follow_after_content_paint(scroll_shell);
                        (scope, live_id)
                    })
                })
            })
        })
    })
}

pub(super) fn transcript_html_el(
    transcript_ref: NodeRef<leptos::html::Div>,
) -> Option<web_sys::HtmlElement> {
    transcript_ref
        .get()
        .and_then(|node| node.dyn_into::<web_sys::HtmlElement>().ok())
}

pub(super) struct FindOverlaySnapshot {
    query: String,
    match_ids: Vec<String>,
    current_id: Option<String>,
    open: bool,
}

pub(super) fn find_overlay_snapshot_tracked(find: ChatFindOverlaySignals) -> FindOverlaySnapshot {
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

pub(super) fn apply_find_snapshot(el: &web_sys::HtmlElement, snap: &FindOverlaySnapshot) {
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
    editing: RwSignal<Option<UserMessageEdit>>,
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

/// 主同步 Effect 的信号与句柄集合（打包传参以守 fn-param 上限）。
pub(super) struct TuiStreamSyncSignals {
    pub(super) chat: ChatSessionSignals,
    pub(super) locale: RwSignal<Locale>,
    pub(super) apply_assistant_display_filters: RwSignal<bool>,
    pub(super) markdown_render: RwSignal<bool>,
    pub(super) show_turn_context_inject: RwSignal<bool>,
    pub(super) ide_narrow: RwSignal<bool>,
    pub(super) think_manually_open: RwSignal<HashSet<String>>,
    pub(super) transcript_ref: NodeRef<leptos::html::Div>,
    pub(super) mount_state: RwSignal<Option<TuiMountState>>,
    pub(super) scroll_shell: ChatScrollShellSignals,
    pub(super) editing_user_message: RwSignal<Option<UserMessageEdit>>,
    pub(super) find: ChatFindOverlaySignals,
}

/// 主同步 Effect：tracked 读建立依赖，同帧多次触发合并到一次 rAF 内做 DOM 同步。
/// 独立函数以隔离闭包复杂度（lizard CCN≤10），并保持组件函数轻量。
pub(super) fn wire_tui_stream_sync_effect(s: TuiStreamSyncSignals) {
    let TuiStreamSyncSignals {
        chat,
        locale,
        apply_assistant_display_filters,
        markdown_render,
        show_turn_context_inject,
        ide_narrow,
        think_manually_open,
        transcript_ref,
        mount_state,
        scroll_shell,
        editing_user_message,
        find,
    } = s;
    // 每 token 合帧：同帧多次触发只在 rAF 回调中执行最后一次 DOM 同步，
    // 避免每个 SSE token 各跑一次全量 plan + DOM patch + 贴底。
    let sync_scheduled = StoredValue::new(false);
    Effect::new(move |_| {
        // tracked 读建立依赖；值在 rAF 回调中 untracked 重读，保证用同帧最新状态。
        let _ = chat.stream_overlay_revision.get();
        let _ = locale.get();
        let _ = apply_assistant_display_filters.get();
        let _ = markdown_render.get();
        let _ = show_turn_context_inject.get();
        // 宽屏才注入「打开此文件」；窄屏 / 移动远端无 IDE 布局。tracked 读取（有意）：
        // resize 跨阈值时按钮即时出现/消失，无需等下一个 token。
        let _ = ide_narrow.get();
        // rAF 合帧前必须显式 tracked 读全量依赖：sync_chat_tui_stream_dom 内部对这些信号的
        // 读取已随 DOM 同步移入 rAF 回调（非响应式上下文），若不在此建立依赖，
        // sessions / overlay / tool_* 更新将不再触发同步（transcript 停留在旧内容）。
        let _ = chat.active_id.get();
        chat.sessions.with(|_| ());
        chat.stream_text_overlay.with(|_| ());
        chat.tool_output_chunks.with(|_| ());
        chat.tool_job_states.with(|_| ());
        chat.tool_file_paths.with(|_| ());
        let _ = transcript_ref.get();
        if sync_scheduled.get_value() {
            return;
        }
        sync_scheduled.set_value(true);
        request_animation_frame(move || {
            sync_scheduled.set_value(false);
            // rAF 回调不在响应式上下文中：untracked 取当下最新值即可。
            let loc = locale.get_untracked();
            let opts = TuiStreamDisplayOpts {
                locale: loc,
                apply_filters: apply_assistant_display_filters.get_untracked(),
                markdown_render: markdown_render.get_untracked(),
                show_turn_context_inject: show_turn_context_inject.get_untracked(),
                open_file_enabled: !ide_narrow.get_untracked()
                    && !crate::mobile_remote::mobile_remote_client(),
            };
            // untracked 快照：toggle 时 DOM 已原生生效，无需因此重渲染；仅在 body 重建时读取。
            let think_open = think_manually_open.get_untracked();
            let (scope, live_id) = sync_chat_tui_stream_dom(
                chat,
                opts,
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
    });
}

#[cfg(test)]
mod tests {
    use super::super::tui_line_markdown::TuiBodyChunks;
    use super::super::tui_transcript_sync::LiveBodyPlan;
    use super::*;

    fn bare_plan() -> TuiSyncPlan {
        TuiSyncPlan {
            next: TuiMountState::default(),
            full_html: None,
            promote_id: None,
            append_sections: Vec::new(),
            refresh_bodies: Vec::new(),
            refresh_actions: Vec::new(),
            live: None,
        }
    }

    fn with_live(patch: TuiBodyPatch) -> TuiSyncPlan {
        let mut plan = bare_plan();
        plan.live = Some(LiveBodyPlan {
            message_id: "m1".to_string(),
            patch,
            markdown_render: true,
        });
        plan
    }

    fn replace_all_patch() -> TuiBodyPatch {
        TuiBodyPatch::ReplaceAll {
            chunks: TuiBodyChunks {
                think: None,
                closed: vec![],
                open_plain: None,
                markdown_render: true,
            },
        }
    }

    #[test]
    fn image_scan_skips_text_only_live_patches() {
        // 纯文本增量（活跃块更新、无新闭合块）与工具行文案：不触发 img hydrate 扫描。
        for plan in [
            bare_plan(),
            with_live(TuiBodyPatch::Incremental {
                append_closed: vec![],
                open_plain: Some("abc".to_string()),
            }),
            with_live(TuiBodyPatch::Incremental {
                append_closed: vec![],
                open_plain: None,
            }),
            with_live(TuiBodyPatch::ThinkBody {
                body_html: "<p>t</p>".to_string(),
                append_closed: vec![],
                open_plain: Some("t".to_string()),
            }),
            with_live(TuiBodyPatch::ToolRow {
                status: "running".to_string(),
                status_label: "…".to_string(),
                one_line: "ls".to_string(),
                detail: None,
            }),
        ] {
            assert!(!plan_touches_images(&plan), "should skip scan");
        }
    }

    #[test]
    fn image_scan_covers_rebuilds_and_new_closed_blocks() {
        for plan in [
            TuiSyncPlan {
                full_html: Some("<div/>".to_string()),
                ..bare_plan()
            },
            TuiSyncPlan {
                append_sections: vec!["<section/>".to_string()],
                ..bare_plan()
            },
            TuiSyncPlan {
                refresh_bodies: vec![LiveBodyPlan {
                    message_id: "m1".to_string(),
                    patch: replace_all_patch(),
                    markdown_render: true,
                }],
                ..bare_plan()
            },
            with_live(replace_all_patch()),
            with_live(TuiBodyPatch::Incremental {
                append_closed: vec!["<pre><code>x</code></pre>".to_string()],
                open_plain: Some("y".to_string()),
            }),
            with_live(TuiBodyPatch::ThinkBody {
                body_html: "<p>t</p>".to_string(),
                append_closed: vec!["<pre><code>t</code></pre>".to_string()],
                open_plain: None,
            }),
            // 回归用例：纯思维链流式阶段（append_closed 为空）但 body_html 含图，
            // 必须扫描——ThinkBody 每帧重写 innerHTML 会丢掉已有 hydrate 标记。
            with_live(TuiBodyPatch::ThinkBody {
                body_html: "<p><img src=\"/workspace/file/raw?path=a.png\"></p>".to_string(),
                append_closed: vec![],
                open_plain: None,
            }),
        ] {
            assert!(plan_touches_images(&plan), "should scan");
        }
    }
}
