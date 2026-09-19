//! StickToBottom **内容侧**：Pinned 时内容变高则 snap 到底。
//!
//! 状态图与用户意图（wheel / pointer / 近底 re-pin）见 [`super::scroll_shell`]。
//!
//! | API | 作用 |
//! |-----|------|
//! | [`engage_follow_and_scroll_bottom`] | 发送 / End → pin + 滚底（即 engage_on_user_send） |
//! | [`on_content_resize_if_pinned`] / [`follow_after_content_paint`] | ResizeObserver / DOM paint 后若仍 pin 则贴底 |
//! | [`wire_content_follow_scroll`] | 会话尾指纹 / overlay revision 变化时若 pin 则 rAF 贴底 |
//! | [`disengage_follow_and_scroll_top`] | Home → unpin + 滚顶 |

use gloo_timers::callback::Timeout;
use leptos::prelude::*;
use leptos_dom::helpers::request_animation_frame;
use std::cell::Cell;

use crate::app::chat::scroll_shell::{ChatScrollShellSignals, stick_pin, stick_unpin};
use crate::chat_session_state::ChatSessionSignals;
use crate::session_ops::messages_scroller_has_non_collapsed_selection;
use crate::storage::ChatSession;

thread_local! {
    /// 每帧延迟贴底去重标志：同帧多个触发源（DOM paint 回调 / 内容信号 Effect）
    /// 只保留最早调度的一次 rAF + 失焦兜底，避免每 token 多次强制布局读。
    static FOLLOW_SNAP_SCHEDULED: Cell<bool> = const { Cell::new(false) };
}

fn snap_to_bottom(shell: ChatScrollShellSignals) {
    let Some(el) = shell.messages_scroller.get_untracked() else {
        return;
    };
    if messages_scroller_has_non_collapsed_selection(&el) {
        return;
    }
    el.set_scroll_top(el.scroll_height());
}

/// 跟底仍开启时 snap 到底；延迟任务执行前必须重新检查用户意图。
fn snap_to_bottom_if_following(shell: ChatScrollShellSignals) {
    if shell.auto_scroll_chat.get_untracked() {
        snap_to_bottom(shell);
    }
}

/// 滚底：rAF 等布局完成；setTimeout 作 Tauri 失焦兜底。
fn scroll_to_bottom(shell: ChatScrollShellSignals) {
    request_animation_frame(move || {
        snap_to_bottom_if_following(shell);
        // Tauri/WebKitGTK 失焦时 rAF 可能不触发，setTimeout 兜底
        Timeout::new(100, move || {
            snap_to_bottom_if_following(shell);
        })
        .forget();
    });
}

/// 延迟贴底：rAF 等布局完成，setTimeout 作 Tauri 失焦兜底；同帧多次调用合并为一次。
///
/// 失焦（Tauri/WebKitGTK rAF 停摆）时挂起的 rAF 不执行，标志会卡在 `true` 且
/// rAF 内安排的兜底 Timeout 永不运行；因此同步路径再安排一个自愈 Timeout：
/// 到期时若 rAF 尚未跑完则复位标志并直接兜底贴底（setTimeout 失焦仍会触发）。
fn schedule_follow_snap(shell: ChatScrollShellSignals) {
    if FOLLOW_SNAP_SCHEDULED.with(Cell::get) {
        return;
    }
    FOLLOW_SNAP_SCHEDULED.with(|p| p.set(true));
    // 自愈：rAF 停摆时复位去重标志并兜底贴底；rAF 正常执行时此分支为 no-op。
    Timeout::new(200, move || {
        if FOLLOW_SNAP_SCHEDULED.with(Cell::get) {
            FOLLOW_SNAP_SCHEDULED.with(|p| p.set(false));
            snap_to_bottom_if_following(shell);
        }
    })
    .forget();
    request_animation_frame(move || {
        FOLLOW_SNAP_SCHEDULED.with(|p| p.set(false));
        snap_to_bottom_if_following(shell);
        // setTimeout 只作 Tauri/WebKitGTK 失焦兜底；执行时重新检查用户是否已上滚。
        Timeout::new(100, move || {
            snap_to_bottom_if_following(shell);
        })
        .forget();
    });
}

fn scroll_to_top(shell: ChatScrollShellSignals) {
    request_animation_frame(move || {
        if let Some(el) = shell.messages_scroller.get() {
            el.set_scroll_top(0);
        }
        Timeout::new(50, move || {
            if let Some(el) = shell.messages_scroller.get() {
                el.set_scroll_top(0);
            }
        })
        .forget();
    });
}

/// 用户发送 / End：进入 Pinned 并滚到底。
///
/// setTimeout(0) + rAF + setTimeout(100) 三重确保：
/// - setTimeout(0)：等 Leptos DOM 批处理完成后再 snap，避免提前读到旧 scrollHeight
/// - rAF：布局完成后最终位置
/// - setTimeout(100)：Tauri 失焦兜底
pub(crate) fn engage_follow_and_scroll_bottom(shell: ChatScrollShellSignals) {
    stick_pin(shell);
    // setTimeout(0) 等 Leptos DOM 批处理完成后再 snap，
    // 避免同步 snap 读到旧 scrollHeight 导致的"向上跳动"。
    // 首次 snap 是明确的发送/End 意图，不能被内容增长产生的 Observer 中间态取消。
    Timeout::new(0, move || {
        snap_to_bottom(shell);
        stick_pin(shell);
    })
    .forget();
    // rAF + setTimeout 兜底
    scroll_to_bottom(shell);
}

/// Home 键：关闭跟底并滚到顶。
pub(crate) fn disengage_follow_and_scroll_top(shell: ChatScrollShellSignals) {
    stick_unpin(shell);
    scroll_to_top(shell);
}

/// Markdown/纯文本已实际写入 DOM 后跟底，避免先读旧 `scrollHeight` 再发生内容增高。
///
/// 同步 snap 一次（尽快贴底）；后续 rAF + 兜底经每帧去重，与内容信号 Effect 合并。
pub(crate) fn follow_after_content_paint(shell: ChatScrollShellSignals) {
    snap_to_bottom_if_following(shell);
    schedule_follow_snap(shell);
}

/// 内容根尺寸变化且仍 Pinned 时贴底（ResizeObserver）。
#[inline]
pub(crate) fn on_content_resize_if_pinned(shell: ChatScrollShellSignals) {
    follow_after_content_paint(shell);
}

fn active_session_tail_scroll_fingerprint(list: &[ChatSession], aid: &str) -> u64 {
    let Some(session) = list.iter().find(|s| s.id == aid) else {
        return 0;
    };
    let mut fingerprint = session.messages.len() as u64;
    for msg in session.messages.iter().rev().take(6) {
        fingerprint = fingerprint.wrapping_mul(41);
        fingerprint = fingerprint.wrapping_add(msg.id.len() as u64);
        fingerprint = fingerprint.wrapping_add(msg.text.len() as u64);
        fingerprint = fingerprint.wrapping_add(msg.reasoning_text.len() as u64);
        if let Some(state) = &msg.state {
            fingerprint = fingerprint.wrapping_add(state.to_wire().len() as u64);
        }
        fingerprint = fingerprint.wrapping_add(u64::from(msg.is_tool));
    }
    fingerprint
}

fn tool_chunks_scroll_fingerprint(chat: ChatSessionSignals) -> u64 {
    chat.tool_output_chunks.with(|m| {
        let mut fp = m.len() as u64;
        for (k, v) in m.iter() {
            fp = fp.wrapping_mul(31).wrapping_add(k.len() as u64);
            fp = fp.wrapping_mul(31).wrapping_add(v.len() as u64);
        }
        fp
    })
}

/// 内容信号变化时若仍 Pinned 则 snap 到底。
///
/// stored 尾消息 / overlay / 工具输出 chunk 变化兜底；流式正文另有 DOM paint 回调。
pub(crate) fn wire_content_follow_scroll(chat: ChatSessionSignals, shell: ChatScrollShellSignals) {
    let version = Memo::new(move |_| {
        let aid = chat.active_id.get();
        let fingerprint = chat
            .sessions
            .with(|list| active_session_tail_scroll_fingerprint(list, &aid));
        let rev = chat.stream_overlay_revision.get();
        let tools = tool_chunks_scroll_fingerprint(chat);
        (fingerprint, rev, tools)
    });
    Effect::new(move |_| {
        let _ = version.get();
        if !shell.auto_scroll_chat.get() {
            return;
        }
        // 与 DOM paint 回调共用每帧去重的延迟贴底（rAF 等布局完成再读 scrollHeight）
        schedule_follow_snap(shell);
    });
}
