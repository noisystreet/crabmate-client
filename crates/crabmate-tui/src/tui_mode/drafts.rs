//! 会话级输入草稿 + 「忙时排队发送」（`tui_usability.md` 的 W1.2 / W1.3）。
//!
//! 单独成模块而不是写进 `mod.rs`：那里已贴着 920 行门禁（同 `body_memo.rs` / `history.rs` 的理由）。
//!
//! 两件事共享同一个判断口径 —— **「这一份输入属于哪个会话」**：
//!
//! - **草稿**：切换会话时把输入框存回旧会话、取回目标会话那一份。内存态；
//!   退出时才由 [`super::exit_flush`] 把当前会话那一份写回服务端索引。
//! - **排队**：回合进行中按 Enter 不再丢弃，改为排进队列，回合结束后自动发出。
//!
//! 排队的**保守边界**：只有「正常完成」的回合才自动发出。被取消（Ctrl+C）或出错的回合
//! 一律把文本退回输入框 —— 用户按 Ctrl+C 的意图通常是「停下来」，此时替他再发一句
//! 意外消息，比多发一次询问更糟。

use std::collections::HashMap;

use super::TuiApp;
use super::state::LineKind;

/// 尚未绑定 `conversation_id` 的草稿所用的键。
///
/// 服务端 id 由服务端生成，空串不可能是合法 id，因此不会与真实会话撞键。
const UNBOUND: &str = "";

/// `conversation_id` → 输入框全文。
///
/// **全空白视为「没有草稿」**：与 `plan_exit_flush` 的判定保持一致，否则一个空格就能让
/// 空白会话钉在侧栏（`session_is_disposable_blank` 要求 `draft` 为空）。
#[derive(Debug, Default)]
pub(super) struct Drafts {
    by_conversation: HashMap<String, String>,
}

impl Drafts {
    fn key(conversation_id: Option<&str>) -> String {
        conversation_id.unwrap_or(UNBOUND).to_string()
    }

    /// 存回某个会话的草稿；全空白等同于删除。
    pub(super) fn save(&mut self, conversation_id: Option<&str>, text: &str) {
        let key = Self::key(conversation_id);
        if text.trim().is_empty() {
            self.by_conversation.remove(&key);
        } else {
            self.by_conversation.insert(key, text.to_string());
        }
    }

    /// 取回某个会话的草稿（**不清空**：同一会话可能被反复切回）。
    pub(super) fn load(&self, conversation_id: Option<&str>) -> Option<&str> {
        self.by_conversation
            .get(&Self::key(conversation_id))
            .map(String::as_str)
    }

    /// 当前存了几份非空草稿（测试用）。
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.by_conversation.len()
    }
}

/// 排队消息在回合结束后的去向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QueuedOutcome {
    /// 没有排队消息：什么都不做。
    Nothing,
    /// 回合正常完成：自动发出。
    Send,
    /// 回合被取消或出错：退回输入框，发不发由用户决定。
    ReturnToComposer,
}

/// 纯决策：避免把「回合怎么结束的」与「队列为空」两套条件散在调用点。
fn queued_outcome(has_queued: bool, failed: bool, cancelled: bool) -> QueuedOutcome {
    if !has_queued {
        QueuedOutcome::Nothing
    } else if failed || cancelled {
        QueuedOutcome::ReturnToComposer
    } else {
        QueuedOutcome::Send
    }
}

impl TuiApp<'_> {
    /// 按会话换入换出输入草稿：先把当前输入存回**正在离开**的会话，再取目标会话那一份。
    ///
    /// `next` 是即将进入的 `conversation_id`（`None` = 还没绑定的新会话）。调用方必须保证
    /// 此刻 `st.conversation_id` 仍是**旧**会话 —— 否则存错了行。
    pub(super) fn swap_draft_in(&mut self, next: Option<&str>) {
        let current = self.st.input_snapshot();
        self.drafts
            .save(self.st.conversation_id.as_deref(), &current);
        let restored = self.drafts.load(next).unwrap_or_default().to_string();
        self.st.set_input(&restored);
    }

    /// 忙时提交：文本进排队槽（覆盖前一条），输入框清空。
    pub(super) fn queue_submit(&mut self, text: &str) {
        self.st.queue_message(text);
        self.st.take_input();
        self.st
            .push_line(LineKind::System, "已排队：回合结束后自动发送（Esc 清队）");
    }

    /// `Esc`：清掉排队消息；返回是否消费了这次按键。
    pub(super) fn clear_queued_on_esc(&mut self) -> bool {
        if !self.st.clear_queued() {
            return false;
        }
        self.st.push_line(LineKind::System, "已取消排队消息");
        true
    }

    /// 回合结束：把排队消息发出去，或退回输入框。
    ///
    /// `failed` / `cancelled` 由 `on_turn_done` 在**复位标志前**算好传入（那里会清
    /// `cancel_sent`、并 `move` 走 `error`）。
    pub(super) fn flush_queued_after_turn(&mut self, failed: bool, cancelled: bool) {
        let has_queued = self.st.has_queued();
        match queued_outcome(has_queued, failed, cancelled) {
            QueuedOutcome::Nothing => {}
            QueuedOutcome::Send => {
                if let Some(text) = self.st.take_queued() {
                    self.start_turn(&text);
                }
            }
            QueuedOutcome::ReturnToComposer => {
                if let Some(text) = self.st.take_queued() {
                    self.st.set_input(&text);
                    self.st.push_line(
                        LineKind::System,
                        "排队消息未发送（回合未正常结束）：已放回输入框",
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::super::test_support::{test_app, test_client, test_prefs};
    use super::super::worker::WorkerJob;
    use super::*;
    use crate::serve::ChatStreamOutcome;

    /// 切走再切回：草稿跟着会话走，不会串到别人的输入框。
    #[test]
    fn draft_follows_its_conversation() {
        let mut d = Drafts::default();
        d.save(Some("c1"), "写给 c1 的");
        d.save(Some("c2"), "写给 c2 的");
        assert_eq!(d.load(Some("c1")), Some("写给 c1 的"));
        assert_eq!(d.load(Some("c2")), Some("写给 c2 的"));
        assert_eq!(d.load(Some("c3")), None);
    }

    /// 取回不清空：来回切换若干次都拿得到同一份。
    #[test]
    fn load_keeps_the_draft() {
        let mut d = Drafts::default();
        d.save(Some("c1"), "半句");
        assert_eq!(d.load(Some("c1")), Some("半句"));
        assert_eq!(d.load(Some("c1")), Some("半句"));
        assert_eq!(d.len(), 1);
    }

    /// 全空白当作没有草稿（否则空白会话会被钉在侧栏）。
    #[test]
    fn blank_draft_is_dropped() {
        let mut d = Drafts::default();
        d.save(Some("c1"), "有内容");
        d.save(Some("c1"), "  \n\t ");
        assert_eq!(d.load(Some("c1")), None);
        assert_eq!(d.len(), 0);
    }

    /// 未绑定的输入用独立键：既不会被绑到某个真实会话，也不会互相覆盖。
    #[test]
    fn unbound_draft_has_its_own_slot() {
        let mut d = Drafts::default();
        d.save(None, "还没绑定");
        d.save(Some("c1"), "已绑定");
        assert_eq!(d.load(None), Some("还没绑定"));
        assert_eq!(d.load(Some("c1")), Some("已绑定"));
        assert_eq!(d.len(), 2);
    }

    /// 正常结束才自动发；取消 / 出错退回输入框；无排队则什么都不做。
    #[test]
    fn queued_outcome_matrix() {
        assert_eq!(queued_outcome(false, false, false), QueuedOutcome::Nothing);
        assert_eq!(queued_outcome(false, true, true), QueuedOutcome::Nothing);
        assert_eq!(queued_outcome(true, false, false), QueuedOutcome::Send);
        assert_eq!(
            queued_outcome(true, true, false),
            QueuedOutcome::ReturnToComposer,
            "回合出错时不该替用户把排队消息发出去"
        );
        assert_eq!(
            queued_outcome(true, false, true),
            QueuedOutcome::ReturnToComposer,
            "Ctrl+C 取消后不该自动发送"
        );
    }

    /// 忙时提交不再丢弃：文本进队列、输入框清空，且**不**立刻发出新回合。
    #[test]
    fn busy_submit_is_queued_instead_of_dropped() {
        let client = test_client();
        let mut prefs = test_prefs();
        let (job_tx, job_rx) = mpsc::channel();
        let (ui_tx, ui_rx) = mpsc::channel();
        let mut app = test_app(&client, &mut prefs, ui_rx, ui_tx, job_tx);

        app.st.running = true;
        app.st.set_input("下一句");
        app.on_submit();

        assert_eq!(app.st.queued.as_deref(), Some("下一句"), "忙时提交应进队列");
        assert!(
            app.st.input_snapshot().is_empty(),
            "内容已进队列，输入框应清空（否则下次回车会重复发送）"
        );
        assert!(
            job_rx.try_iter().all(|j| !matches!(j, WorkerJob::Turn(_))),
            "回合还在跑，不该再发一轮"
        );
    }

    /// 回合正常结束 → 排队消息自动开成下一轮，并真的交给 worker。
    #[test]
    fn queued_message_is_sent_once_the_turn_finishes_cleanly() {
        let client = test_client();
        let mut prefs = test_prefs();
        let (job_tx, job_rx) = mpsc::channel();
        let (ui_tx, ui_rx) = mpsc::channel();
        let mut app = test_app(&client, &mut prefs, ui_rx, ui_tx, job_tx);

        app.st.running = true;
        app.st.set_input("下一句");
        app.on_submit();

        app.on_turn_done(ChatStreamOutcome::default(), None, None);

        assert!(app.st.queued.is_none(), "发出去之后队列应清空");
        assert!(app.st.running, "排队消息应开出一轮新回合");
        assert!(
            job_rx.try_iter().any(|j| matches!(j, WorkerJob::Turn(_))),
            "新回合必须真的交给 worker，否则界面显示在跑、实际没发"
        );
    }

    /// 回合被 Ctrl+C 取消 → 排队消息退回输入框，不替用户发出去。
    #[test]
    fn queued_message_returns_to_the_composer_when_cancelled() {
        let client = test_client();
        let mut prefs = test_prefs();
        let (job_tx, job_rx) = mpsc::channel();
        let (ui_tx, ui_rx) = mpsc::channel();
        let mut app = test_app(&client, &mut prefs, ui_rx, ui_tx, job_tx);

        app.st.running = true;
        app.st.set_input("下一句");
        app.on_submit();
        app.st.cancel_sent = true;

        app.on_turn_done(ChatStreamOutcome::default(), None, None);

        assert!(app.st.queued.is_none());
        assert_eq!(
            app.st.input_snapshot(),
            "下一句",
            "取消的回合不该替用户把排队消息发出去"
        );
        assert!(!app.st.running);
        assert!(
            job_rx.try_iter().all(|j| !matches!(j, WorkerJob::Turn(_))),
            "取消后不该自动开新回合"
        );
    }

    /// 回合出错 → 同样退回输入框（`error = Some` 即非正常结束）。
    #[test]
    fn queued_message_returns_to_the_composer_when_the_turn_failed() {
        let client = test_client();
        let mut prefs = test_prefs();
        let (job_tx, job_rx) = mpsc::channel();
        let (ui_tx, ui_rx) = mpsc::channel();
        let mut app = test_app(&client, &mut prefs, ui_rx, ui_tx, job_tx);

        app.st.running = true;
        app.st.set_input("下一句");
        app.on_submit();

        app.on_turn_done(ChatStreamOutcome::default(), Some("boom".into()), None);

        assert_eq!(app.st.input_snapshot(), "下一句", "出错的回合不该自动发送");
        assert!(!app.st.running);
        assert!(
            job_rx.try_iter().all(|j| !matches!(j, WorkerJob::Turn(_))),
            "出错后不该自动开新回合"
        );
    }
}
