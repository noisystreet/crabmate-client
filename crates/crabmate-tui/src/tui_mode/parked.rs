//! 会话级**停泊槽**：被切走的会话（连同它在途的回合）留在内存里继续收增量。
//!
//! 为什么需要：切会话就是换一份 transcript。若切走时把旧会话的 `UiState` 丢掉，正在跑的
//! 那一轮的增量就没有落点 —— 只能丢弃或（更糟）写进新会话的屏幕。Desktop 靠「每条会话各
//! 带自己的 `messages` + 流绑定会话 id」天然没有这个问题；这里用「整体换进换出」达到同一
//! 效果，代价是每个事件多两次 `mem::take`（`UiState` 里都是 `Vec` / 小结构，move 是 O(1)）。
//!
//! 三个动作：
//!
//! - [`LaneApp::park_current_session`]：切走时把当前会话整体挪进槽里；
//! - [`LaneApp::restore_parked_session`]：切回来时接上（**不**重发历史请求）；
//! - [`LaneApp::apply_to_parked`]：后台会话的增量借位应用 —— 换进去、走既有分派、换回来，
//!   于是 70 余处 `on_msg` 分支一行都不用改。
//!
//! 有意不随会话走的：`overrides`（`/model` `/mode` `/role` 是本进程表达）、`drafts`
//! （本来就按会话索引）、`index`（`/conv new` 的在途状态）、`palette`（外观是进程级）。
//!
//! 单独成模块的原因是行数门禁：`tui_mode/mod.rs` 在 `main` 上正好 920 行。

use std::collections::HashMap;
use std::time::Instant;

use crate::serve::StreamCancel;

use super::LaneApp;
use super::body_cache::BodyCache;
use super::resume::{ResumePoint, TurnEcho};
use super::state::{LineKind, UiState};
use super::worker::UiEvent;

/// `UiState::conversation_id` → 停泊槽（`None` = 还没绑定会话的那一份）。
pub(super) type ParkedSessionMap = HashMap<Option<String>, ParkedSession>;

/// 一个被切走的会话留下的那一份状态。
///
/// 字段与 `LaneApp` 上属于**会话**的那几个一一对应：换进换出时整体搬，语义才与「它就是
/// 当前会话」完全一致。凡是留在 `LaneApp` 上的字段，都是进程 / 连接级的东西。
pub(super) struct ParkedSession {
    st: UiState,
    body: BodyCache,
    cancel: Option<StreamCancel>,
    resume: Option<ResumePoint>,
    sent_turn: Option<TurnEcho>,
    turn_started: Option<Instant>,
}

impl LaneApp<'_> {
    /// 把当前会话的「会话级状态」整体取出，视图换成空白；视图级字段（输入框与光标、
    /// `palette`、会话列表与它的光标）留在原地。
    ///
    /// 抽成原语（不给 `parked` 建条目）是为了 [`Self::apply_to_parked`]：两条**未绑定**会话
    /// 的键都是 `None`，借位时把当前视图存回 `parked` 会覆盖掉正要处理的那一条。
    fn take_current_session(&mut self) -> ParkedSession {
        let mut st = std::mem::take(&mut self.st);
        // 视图保留输入框（含光标）：草稿本来就由 `drafts` 按会话保存 / 载入，进槽只会让
        // 切回来时看到别人的字，而 `set_input` 那条路还会把光标弹到行尾。
        self.st.swap_input_with(&mut st);
        self.st.palette = st.palette;
        // 会话列表是**连接**级事实（刷新按 `serve` 拉，同 serve 各 lane 共用一份）。
        self.st.sessions = std::mem::take(&mut st.sessions);
        self.st.selected = st.selected;
        ParkedSession {
            st,
            body: std::mem::take(&mut self.body),
            cancel: self.cancel.take(),
            resume: self.resume.take(),
            sent_turn: self.sent_turn.take(),
            turn_started: self.turn_started.take(),
        }
    }

    /// 把一份会话状态装进视图（会话级字段整体换入，视图级字段保持不动）。
    fn put_session_slot(&mut self, slot: ParkedSession) {
        let ParkedSession {
            mut st,
            body,
            cancel,
            resume,
            sent_turn,
            turn_started,
        } = slot;
        std::mem::swap(&mut self.st, &mut st);
        self.st.swap_input_with(&mut st);
        self.st.palette = st.palette;
        self.st.sessions = std::mem::take(&mut st.sessions);
        self.st.selected = st.selected;
        self.body = body;
        self.cancel = cancel;
        self.resume = resume;
        self.sent_turn = sent_turn;
        self.turn_started = turn_started;
    }

    /// 把**当前**会话整体挪进停泊槽，`st` 换成空白（随后由回放或接上填回）。
    pub(super) fn park_current_session(&mut self) {
        let slot = self.take_current_session();
        let key = slot.st.conversation_id.clone();
        self.parked.insert(key, slot);
    }

    /// 从停泊槽接回某一会话。返回 `false` = 它不在槽里（调用方要回放历史）。
    ///
    /// 接回**不**重发 `GET /conversation/messages`：槽里的 transcript 就是这条会话最新的
    /// 实时内容，重放一遍等于把正在流式的正文换成服务端的旧快照。
    pub(super) fn restore_parked_session(&mut self, key: &Option<String>) -> bool {
        let Some(slot) = self.parked.remove(key) else {
            return false;
        };
        self.put_session_slot(slot);
        true
    }

    /// 停掉**所有停泊会话**的在途回合，返回停掉的条数。
    ///
    /// 两个调用点，同一动作、不同措辞（`notice` 由调用方给）：
    ///
    /// - 发新回合前（`dispatch_turn`）：与 Desktop 同口径 —— 那边是
    ///   `reset_abort_state_for_new_attach` 里的 `ac.abort()` + 抬 `attach_generation`，
    ///   一个连接同时只跑一轮。TUI 的 worker 是串行的，不让位的话新回合只能排在后面，
    ///   而屏幕上看起来「已经在跑」。
    /// - 收尾时（`wind_down`，即 Ctrl+C / `/quit` / `/lane detach`）：不停它们就是
    ///   「屏幕上说停了、实际还在跑」—— serve 侧那一轮白跑，worker 也要等整轮跑完才回到
    ///   `recv`，退出因此变慢。
    ///
    /// 与 `wind_down` 的另一半差别在作用对象：那个还动连接级的 `JobCancel`（拉取也算它的
    /// 账），这里只按回合取消 —— 停泊会话没有自己的 job 队列，动 `JobCancel` 会误伤当前会话。
    pub(super) fn abandon_parked_turns(&mut self, notice: &str) -> usize {
        let keys: Vec<Option<String>> = self
            .parked
            .iter()
            .filter(|(_, slot)| slot.st.running)
            .map(|(key, _)| key.clone())
            .collect();
        for key in &keys {
            let Some(slot) = self.parked.get_mut(key) else {
                continue;
            };
            // 撤审批浮层同理：`drop(answer)` 让 gate 立刻按拒绝收尾，不必等满 `APPROVAL_WAIT`。
            slot.st.approval.take();
            if let Some(cancel) = slot.cancel.take() {
                cancel.cancel();
            }
            slot.st.running = false;
            slot.st.cancel_sent = true;
            slot.st.push_line(LineKind::System, notice);
        }
        keys.len()
    }

    /// 把一条事件应用到**停泊**的会话上：换进 → 既有分派 → 换回。
    ///
    /// 返回 `false` = 那条会话不在槽里（已被丢弃 / 从未停泊），事件直接丢弃 —— 静默写进
    /// 当前会话才是更坏的结果。
    pub(super) fn apply_to_parked(&mut self, key: &Option<String>, ev: UiEvent) -> bool {
        let Some(slot) = self.parked.remove(key) else {
            return false;
        };
        // 同一归属且**当前正在跑**时才该走直派；两条未绑定会话（键都是 `None`）里，
        // 正在跑的那条是「前一个视图」，没在跑的那条才是这里的合法目标。
        debug_assert!(
            self.st.conversation_id != *key || !self.st.running,
            "归属相同且当前在跑时不该走停泊路径（应当直接分派）"
        );
        // 当前视图整体取出而**不**入 `parked`：两条未绑定会话的键都是 `None`，存回去会覆盖
        // 掉正要处理的那一条（见 `take_current_session` 的注释）。
        let current = self.take_current_session();
        self.put_session_slot(slot);
        // 借位期间不许「回合结束顺手把排队消息发出去」：那会在用户看不见的会话里开一轮新
        // 回合，抢占串行 worker（见 `drafts::flush_queued_after_turn` 的守卫）。
        let outer = std::mem::replace(&mut self.in_borrow, true);
        self.on_msg(ev);
        self.in_borrow = outer;
        let updated = self.take_current_session();
        // 首轮可能刚刚绑定 `conversation_id`：用**当前** id 作键，下一个增量仍找得到它。
        let new_key = updated.st.conversation_id.clone();
        self.parked.insert(new_key, updated);
        self.put_session_slot(current);
        true
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;
    use crate::tui_mode::lane_route::TurnSession;
    use crate::tui_mode::test_support::{test_app, test_client, test_prefs};

    /// 停泊槽的三条不变量：增量不串台、在途标记跟着会话走、接回不重发历史。
    fn lane() -> LaneApp<'static> {
        let client = Box::leak(Box::new(test_client()));
        let prefs = Box::leak(Box::new(test_prefs()));
        let (job_tx, _job_rx) = mpsc::channel();
        let (tx, rx) = mpsc::channel();
        test_app(client, prefs, rx, tx, job_tx)
    }

    #[test]
    fn a_parked_session_keeps_its_deltas_out_of_the_current_transcript() {
        let mut app = lane();
        app.st.conversation_id = Some("A".into());
        app.st.running = true;
        app.on_msg(UiEvent::System("A 的上一段".into()));
        let before = app.st.lines.len();

        // 切走：A 整体进槽，视图换成空白（B）。
        app.park_current_session();
        app.st.conversation_id = Some("B".into());
        assert_eq!(app.st.lines.len(), 0, "停泊后视图应当是空白的");

        // A 的回合仍在跑，增量到达 —— 只能进它自己的槽。
        app.on_turn_event(
            TurnSession::Of(Some("A".into())),
            UiEvent::System("A 的后续".into()),
        );
        assert_eq!(app.st.lines.len(), 0, "A 的增量绝不能写进 B 的 transcript");
        let parked = app
            .parked
            .get(&Some("A".into()))
            .expect("A 应当还在停泊槽里");
        assert!(
            parked.st.lines.len() > before,
            "A 的槽要接着累积：{} > {}",
            parked.st.lines.len(),
            before
        );
        assert!(parked.st.running, "在途标记跟着会话进槽");
    }

    #[test]
    fn restoring_a_parked_session_rebinds_without_refetching_history() {
        let mut app = lane();
        app.st.conversation_id = Some("A".into());
        app.st.running = true;
        app.park_current_session();
        app.st.conversation_id = Some("B".into());

        assert!(app.restore_parked_session(&Some("A".into())));
        assert_eq!(app.st.conversation_id, Some("A".into()));
        assert!(app.st.running, "接回来的会话仍是「正在生成」");
        assert!(
            app.st.history_pending.is_none(),
            "槽里就是最新内容，不该再发 GET /conversation/messages"
        );
        assert!(app.parked.is_empty(), "接回后槽应当清空");
    }

    #[test]
    fn a_delta_for_an_unknown_session_is_dropped_not_rerouted() {
        let mut app = lane();
        app.st.conversation_id = Some("B".into());
        app.on_turn_event(
            TurnSession::Of(Some("X".into())),
            UiEvent::System("不属于任何在内存里的会话".into()),
        );
        assert_eq!(app.st.lines.len(), 0, "写进当前会话就是串台");
    }

    #[test]
    fn sending_a_new_turn_stops_the_parked_ones() {
        let mut app = lane();
        app.st.conversation_id = Some("A".into());
        app.st.running = true;
        app.park_current_session();
        app.st.conversation_id = Some("B".into());

        assert_eq!(app.abandon_parked_turns("让位"), 1, "后台那一轮应当让位");
        let parked = app.parked.get(&Some("A".into())).expect("A 仍在槽里");
        assert!(!parked.st.running, "让位之后不再是「正在生成」");
        assert!(
            parked.st.cancel_sent,
            "取消已经请求过（与 Ctrl+C 同一条路）"
        );
        assert_eq!(app.abandon_parked_turns("让位"), 0, "已经停了的不重复停");
    }

    #[test]
    fn a_parked_delta_leaves_the_composer_and_its_cursor_alone() {
        let mut app = lane();
        app.st.conversation_id = Some("A".into());
        app.st.running = true;
        app.st.set_input("ab");
        app.st.cursor_left();
        app.st.insert_char('X');
        assert_eq!(app.st.input_snapshot(), "aXb", "前置：光标停在 X 之后");

        // 切走 → A 的增量到达 → 走一遍换进换出。
        app.park_current_session();
        app.st.conversation_id = Some("B".into());
        app.on_turn_event(
            TurnSession::Of(Some("A".into())),
            UiEvent::System("A 的增量".into()),
        );

        assert_eq!(
            app.st.input_snapshot(),
            "aXb",
            "输入框内容不该被后台事件改动"
        );
        app.st.insert_char('Y');
        assert_eq!(
            app.st.input_snapshot(),
            "aXYb",
            "光标必须留在原处：被弹到行尾的话这里是 aXbY"
        );
    }

    #[test]
    fn an_unbound_turn_never_lands_in_a_freshly_created_session() {
        let mut app = lane();
        // A 的首轮：还没绑定 id。
        app.st.conversation_id = None;
        app.st.running = true;
        app.park_current_session();
        // `/conv new` 出来的空会话：同样没有 id，但没在跑。
        app.st.conversation_id = None;
        app.st.running = false;

        app.on_turn_event(TurnSession::Of(None), UiEvent::System("A 的增量".into()));

        assert_eq!(app.st.lines.len(), 0, "未绑定首轮的增量不能写进新空会话");
        let parked = app.parked.get(&None).expect("A 的槽还在");
        assert!(!parked.st.lines.is_empty(), "它应当进 A 自己的槽");
    }

    #[test]
    fn winding_down_also_stops_the_parked_turns() {
        let mut app = lane();
        app.st.conversation_id = Some("A".into());
        app.st.running = true;
        app.park_current_session();
        app.st.conversation_id = Some("B".into());
        assert!(!app.st.running, "当前视图本来就是空闲的");

        // Ctrl+C / `/quit` / `/lane detach` 共用这个入口。
        app.wind_down();

        let parked = app.parked.get(&Some("A".into())).expect("A 仍在槽里");
        assert!(
            !parked.st.running,
            "收尾要连停泊会话的回合一起停，否则 serve 侧白跑一轮、worker 也退不干净"
        );
    }

    #[test]
    fn a_parked_turn_end_does_not_fire_its_queued_message() {
        use crate::serve::ChatStreamOutcome;

        let mut app = lane();
        app.st.conversation_id = Some("A".into());
        app.st.running = true;
        app.st.queue_message("下一句");
        app.park_current_session();
        app.st.conversation_id = Some("B".into());

        app.on_turn_event(
            TurnSession::Of(Some("A".into())),
            UiEvent::TurnDone {
                outcome: ChatStreamOutcome::default(),
                error: None,
                interrupted: None,
            },
        );

        let parked = app.parked.get(&Some("A".into())).expect("A 仍在槽里");
        assert!(
            parked.st.has_queued(),
            "后台会话的排队消息不能自动发出去（那会抢占串行 worker）"
        );
    }

    #[test]
    fn the_session_list_stays_with_the_connection_not_the_conversation() {
        let mut app = lane();
        app.st.conversation_id = Some("A".into());
        app.park_current_session();
        // 侧栏是连接级事实：停泊不该把它带走（否则点一行切会话后列表就空了）。
        app.st.selected = 3;
        app.restore_parked_session(&Some("A".into()));
        assert_eq!(app.st.selected, 3, "光标属于侧栏，不随会话进槽");
    }
}
