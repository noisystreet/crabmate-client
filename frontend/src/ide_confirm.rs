//! IDE 内确认对话框（替代 `window.confirm`，桌面 WebView 与 E2E 更可靠）。
//!
//! 并发请求按 FIFO 排队：`pending` 是请求队列，UI 只展示队首；
//! 应答总是作用于队首，等待方按 `id` 匹配自己的结果。
//!
//! 等待是事件驱动的：`answer_pending_head` 直接唤醒对应等待者（每请求一个独立
//! 唤醒槽），不做轮询；等待方拿到的是自己那次应答的值，不受 result 单槽覆盖影响
//! （result 单槽仅供外部只读方使用）。
//!
//! **约束**：`pending` 队列中的请求只允许经 `answer_pending_head` 移除（它负责写
//! result 并唤醒等待者）。事件驱动改造已移除旧轮询的「id 不在 pending 即返回
//! false」兜底——任何绕过该函数的清空路径（如会话销毁清理）都会让对应等待者
//! **永久挂起**；新增此类路径时必须同时唤醒（或复用本函数）。

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use leptos::prelude::*;

thread_local! {
    static NEXT_CONFIRM_ID: Cell<u64> = const { Cell::new(1) };
    /// 等待应答的请求（id → 唤醒槽）；answer 时唤醒，替代轮询（WASM 单线程）。
    static CONFIRM_WAIT_SLOTS: RefCell<HashMap<u64, Rc<RefCell<ConfirmWaitSlot>>>> =
        RefCell::new(HashMap::new());
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
    ///
    /// 移除请求必须经 `answer_pending_head`（写 result + 唤醒等待者）；绕过它
    /// 清空队列会让等待者永久挂起（见模块文档约束）。
    pub pending: RwSignal<Vec<IdeConfirmPrompt>>,
    pub result: RwSignal<Option<IdeConfirmResult>>,
}

/// 等待槽：应答值 + 唤醒 Waker（事件驱动，无轮询）。
struct ConfirmWaitSlot {
    answered: Option<bool>,
    waker: Option<Waker>,
}

/// 等待某请求被应答的 Future；被丢弃时自动移除注册表条目。
struct ConfirmWait {
    slot: Rc<RefCell<ConfirmWaitSlot>>,
}

impl Future for ConfirmWait {
    type Output = bool;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<bool> {
        let mut slot = self.slot.borrow_mut();
        if let Some(ok) = slot.answered.take() {
            return Poll::Ready(ok);
        }
        slot.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

impl Drop for ConfirmWait {
    fn drop(&mut self) {
        // future 被丢弃（请求方取消）时移除条目，避免槽泄漏
        CONFIRM_WAIT_SLOTS.with(|w| {
            w.borrow_mut().retain(|_, s| !Rc::ptr_eq(s, &self.slot));
        });
    }
}

/// 注册等待槽并返回等待 Future。
fn register_confirm_wait(id: u64) -> ConfirmWait {
    let slot = Rc::new(RefCell::new(ConfirmWaitSlot {
        answered: None,
        waker: None,
    }));
    CONFIRM_WAIT_SLOTS.with(|w| {
        w.borrow_mut().insert(id, Rc::clone(&slot));
    });
    ConfirmWait { slot }
}

/// 唤醒该请求的等待者（answer 写 result 后调用）；无人等待时为 no-op。
fn wake_confirm_waiter(id: u64, ok: bool) {
    CONFIRM_WAIT_SLOTS.with(|w| {
        if let Some(slot) = w.borrow_mut().remove(&id) {
            let mut slot = slot.borrow_mut();
            slot.answered = Some(ok);
            if let Some(waker) = slot.waker.take() {
                waker.wake();
            }
        }
    });
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
    // 事件驱动等待：answer_pending_head 唤醒（原先每 16ms 轮询两个信号，挂起期间空转 CPU）
    register_confirm_wait(id).await
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
/// 随后按其 `id` 回写结果并唤醒等待者；队列空时为 no-op。
fn answer_pending_head(signals: IdeConfirmSignals, ok: bool) {
    let Some(id) = signals
        .pending
        .try_update(|q| (!q.is_empty()).then(|| q.remove(0).id))
        .flatten()
    else {
        return;
    };
    signals.result.set(Some(IdeConfirmResult { id, ok }));
    wake_confirm_waiter(id, ok);
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

    #[test]
    fn wait_future_resolves_when_answer_wakes() {
        let mut wait = register_confirm_wait(999);
        assert!(CONFIRM_WAIT_SLOTS.with(|w| w.borrow().contains_key(&999)));

        // 首次 poll：未应答 → Pending 并记录 waker；槽保持在注册表
        let mut cx = Context::from_waker(Waker::noop());
        assert_eq!(Pin::new(&mut wait).poll(&mut cx), Poll::Pending);
        assert!(CONFIRM_WAIT_SLOTS.with(|w| w.borrow().contains_key(&999)));

        // 应答唤醒：槽移除，等待者拿到自己的应答值
        wake_confirm_waiter(999, true);
        assert!(!CONFIRM_WAIT_SLOTS.with(|w| w.borrow().contains_key(&999)));
        assert_eq!(Pin::new(&mut wait).poll(&mut cx), Poll::Ready(true));

        // false（取消 / Escape dismiss）路径同样成立
        let mut wait2 = register_confirm_wait(1000);
        wake_confirm_waiter(1000, false);
        assert_eq!(Pin::new(&mut wait2).poll(&mut cx), Poll::Ready(false));
        assert!(!CONFIRM_WAIT_SLOTS.with(|w| w.borrow().contains_key(&1000)));
    }

    #[test]
    fn dropped_wait_future_removes_slot() {
        let wait = register_confirm_wait(1001);
        assert!(CONFIRM_WAIT_SLOTS.with(|w| w.borrow().contains_key(&1001)));
        // future 被丢弃（请求方取消）时清理注册表，不留悬空槽
        drop(wait);
        assert!(!CONFIRM_WAIT_SLOTS.with(|w| w.borrow().contains_key(&1001)));
    }
}
