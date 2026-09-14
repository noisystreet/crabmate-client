//! IDE 内确认对话框（替代 `window.confirm`，桌面 WebView 与 E2E 更可靠）。
//!
//! 并发请求按 FIFO 排队：`pending` 是请求队列，UI 只展示队首；
//! 应答总是作用于队首，等待方按 `id` 匹配自己的结果。
//!
//! 已知限制：result 是单槽。极端情况下（队首 A 被应答后的 16ms 轮询窗口内，
//! 用户又对新队首 B 应答），A 的结果会被 B 覆盖，A 将经「id 已不在队列」
//! 兜底返回 `false`；需要第二个人为点击落在一帧渲染内，实际不可触发。

use std::cell::Cell;

use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;

thread_local! {
    static NEXT_CONFIRM_ID: Cell<u64> = const { Cell::new(1) };
}

fn next_confirm_id() -> u64 {
    NEXT_CONFIRM_ID.with(|c| {
        let id = c.get();
        c.set(id.saturating_add(1));
        id
    })
}

/// 待展示的确认请求（按钮文案由调用方按场景传入，保持短词）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdeConfirmPrompt {
    pub id: u64,
    pub message: String,
    pub ok_label: String,
    pub cancel_label: String,
}

/// 用户对某次请求的应答。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdeConfirmResult {
    pub id: u64,
    pub ok: bool,
}

/// 确认框信号（挂于 [`crate::app::app_signals::IdeChromeSignals`]）。
#[derive(Clone, Copy)]
pub struct IdeConfirmSignals {
    /// FIFO 请求队列；UI 只消费队首。
    pub pending: RwSignal<Vec<IdeConfirmPrompt>>,
    pub result: RwSignal<Option<IdeConfirmResult>>,
}

/// 展示确认框并异步等待用户选择；取消或关闭返回 `false`。
pub async fn ide_confirm_user(
    signals: IdeConfirmSignals,
    message: String,
    ok_label: String,
    cancel_label: String,
) -> bool {
    let id = next_confirm_id();
    signals.pending.update(|q| {
        q.push(IdeConfirmPrompt {
            id,
            message,
            ok_label,
            cancel_label,
        })
    });
    loop {
        TimeoutFuture::new(16).await;
        if let Some(r) = signals.result.get_untracked()
            && r.id == id
        {
            signals.result.set(None);
            return r.ok;
        }
        if !signals.pending.get_untracked().iter().any(|p| p.id == id) {
            return false;
        }
    }
}

/// 由确认框 UI 调用：应答队首请求并弹出。
pub fn resolve_ide_confirm(signals: IdeConfirmSignals, ok: bool) {
    answer_pending_head(signals, ok);
}

/// 无待处理请求时由 Escape 等调用（语义同 [`resolve_ide_confirm`]，应答 `false`）。
pub fn dismiss_ide_confirm(signals: IdeConfirmSignals) {
    answer_pending_head(signals, false);
}

/// 在一次同步 `try_update` 内取队首并弹出（不存在空队列 panic 窗口），
/// 随后按其 `id` 回写结果；队列空时为 no-op。
fn answer_pending_head(signals: IdeConfirmSignals, ok: bool) {
    let Some(id) = signals
        .pending
        .try_update(|q| (!q.is_empty()).then(|| q.remove(0).id))
        .flatten()
    else {
        return;
    };
    signals.result.set(Some(IdeConfirmResult { id, ok }));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_signals() -> IdeConfirmSignals {
        IdeConfirmSignals {
            pending: RwSignal::new(Vec::new()),
            result: RwSignal::new(None),
        }
    }

    fn push(signals: IdeConfirmSignals, message: &str) -> u64 {
        let id = next_confirm_id();
        signals.pending.update(|q| {
            q.push(IdeConfirmPrompt {
                id,
                message: message.to_string(),
                ok_label: "ok".into(),
                cancel_label: "cancel".into(),
            })
        });
        id
    }

    #[test]
    fn resolve_answers_head_and_keeps_followups_queued() {
        let owner = leptos::reactive::owner::Owner::new();
        owner.with(|| {
            let signals = test_signals();
            let first = push(signals, "first");
            let second = push(signals, "second");
            assert_ne!(first, second);

            resolve_ide_confirm(signals, true);
            let r = signals.result.get_untracked().expect("result written");
            assert_eq!(r.id, first);
            assert!(r.ok);
            let q = signals.pending.get_untracked();
            assert_eq!(q.len(), 1);
            assert_eq!(q[0].id, second);

            dismiss_ide_confirm(signals);
            let r = signals.result.get_untracked().expect("result written");
            assert_eq!(r.id, second);
            assert!(!r.ok);
            assert!(signals.pending.get_untracked().is_empty());
        });
    }

    #[test]
    fn confirm_ops_are_noop_on_empty_queue() {
        let owner = leptos::reactive::owner::Owner::new();
        owner.with(|| {
            let signals = test_signals();
            resolve_ide_confirm(signals, true);
            dismiss_ide_confirm(signals);
            assert!(signals.pending.get_untracked().is_empty());
            assert!(signals.result.get_untracked().is_none());
        });
    }
}
