//! 回合结束后的会话刷新收敛点（issue #349）。
//!
//! 原先是 `LaneApp::on_turn_done` 一结束就 `job_tx.send(WorkerJob::RefreshSessions)`，
//! 于是「每条 lane × 每一轮」各打一次 `GET /web_sessions`（`job_refresh_sessions` 还
//! 顺带打一次 `GET /workspace`）。连续多轮收尾、或多条 lane 集中收尾时，这些请求会在
//! 同一个瞬间一起打出去。
//!
//! 这里把「什么时候发」从 lane 内部搬到外壳：lane 只置一个意图位，外壳在帧循环里按
//! [`SESSION_REFRESH_WINDOW`] 合并后统一发。**刷新语义不变**（回合结束必然刷新），只是
//! 不再「一轮一发」。
//!
//! 本模块只收敛频次，**不按 serve 去重**：两条 lane 连同一台 serve 时仍各发一次（各自
//! worker、各自的连接池）。按 `api_base` 去重必须同时把结果 fan-out 回同组的其它 lane，
//! 否则那条 lane 的会话列表就再也不更新 —— 那是独立一步，见 issue #349。

use std::time::{Duration, Instant};

use super::worker::LaneId;

/// 合并窗口：这段时间内积累的刷新意图只发一次。
///
/// 200 ms 是「用户感知不到」与「别把刷新拖到下一轮之后」之间的取值：更短起不到合并
/// 作用（一次回合收尾的回包间隔就有几十毫秒），更长则新会话会在左栏迟迟不出现。
pub(super) const SESSION_REFRESH_WINDOW: Duration = Duration::from_millis(200);

/// 待发的会话刷新：按 lane **id** 记录，不按下标 —— `/lane detach` 会让下标整体前移。
#[derive(Debug, Default)]
pub(super) struct SessionRefresh {
    /// 已请求刷新、还没发出去的 lane。
    want: Vec<LaneId>,
    /// 上次发送时刻；`None` = 还没发过（第一批不必空等一个窗口）。
    last_sent: Option<Instant>,
}

impl SessionRefresh {
    /// 记一条意图（同一 lane 重复记只留一份）。
    pub(super) fn mark(&mut self, id: LaneId) {
        if !self.want.contains(&id) {
            self.want.push(id);
        }
    }

    /// 摘掉一条：lane 已关闭，再发只会换来「刷新会话失败：worker 已退出」。
    pub(super) fn forget(&mut self, id: LaneId) {
        self.want.retain(|w| *w != id);
    }

    /// 本轮该发给哪些 lane（返回后即清空）。
    ///
    /// 窗口未到就返回空 —— 意图留在 [`Self::want`] 里等下一轮，不会被丢弃。
    pub(super) fn take_due(&mut self, now: Instant) -> Vec<LaneId> {
        match self.until_due(now) {
            Some(left) if left.is_zero() => {
                self.last_sent = Some(now);
                std::mem::take(&mut self.want)
            }
            _ => Vec::new(),
        }
    }

    /// 距下一次可以发送还有多久；`None` = 没有待发的（不必为它唤醒循环）。
    pub(super) fn until_due(&self, now: Instant) -> Option<Duration> {
        if self.want.is_empty() {
            return None;
        }
        let Some(last) = self.last_sent else {
            return Some(Duration::ZERO);
        };
        Some(SESSION_REFRESH_WINDOW.saturating_sub(now.saturating_duration_since(last)))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use super::{SESSION_REFRESH_WINDOW, SessionRefresh};
    use crate::serve::ChatStreamOutcome;
    use crate::tui_mode::TuiApp;
    use crate::tui_mode::inbound::Inbound;
    use crate::tui_mode::test_support::{test_client, test_prefs, test_shell_with_jobs};
    use crate::tui_mode::worker::{LaneId, UiEvent, WorkerJob};

    fn lane(n: u32) -> LaneId {
        LaneId(n)
    }

    /// 基准时刻固定：这里的判定只比较相对间隔，不能让用例依赖真实挂钟。
    fn base() -> Instant {
        Instant::now()
    }

    fn later(base: Instant, ms: u64) -> Instant {
        base + Duration::from_millis(ms)
    }

    #[test]
    fn first_batch_is_sent_without_waiting() {
        let now = base();
        let mut r = SessionRefresh::default();
        r.mark(lane(1));
        assert_eq!(r.until_due(now), Some(Duration::ZERO), "首次不必等窗口");
        assert_eq!(r.take_due(now), vec![lane(1)]);
        assert!(r.take_due(now).is_empty(), "发过的不会再发一次");
    }

    #[test]
    fn later_batches_wait_for_the_window() {
        let now = base();
        let mut r = SessionRefresh::default();
        r.mark(lane(1));
        assert_eq!(r.take_due(now), vec![lane(1)]);

        r.mark(lane(2));
        assert_eq!(
            r.until_due(later(now, 50)),
            Some(Duration::from_millis(150)),
            "窗口还剩 150ms"
        );
        assert!(
            r.take_due(later(now, 50)).is_empty(),
            "窗口内不该再发：这才是「合并」"
        );
        assert_eq!(
            r.take_due(now + SESSION_REFRESH_WINDOW),
            vec![lane(2)],
            "窗口到点补发"
        );
    }

    #[test]
    fn repeated_marks_collapse_into_one() {
        let now = base();
        let mut r = SessionRefresh::default();
        r.mark(lane(1));
        r.mark(lane(1));
        assert_eq!(r.take_due(now), vec![lane(1)], "同一 lane 的排队只算一次");
    }

    #[test]
    fn lanes_are_independent_within_one_window() {
        let now = base();
        let mut r = SessionRefresh::default();
        r.mark(lane(1));
        r.mark(lane(2));
        // 当前口径：不同 lane 各自要刷新自己的会话列表，所以两条都发（#349 的
        // 按 serve 去重是后续一步，届时这里会变成一条 + fan-out）。
        assert_eq!(r.take_due(now), vec![lane(1), lane(2)]);
    }

    #[test]
    fn forgotten_lane_is_not_sent() {
        let now = base();
        let mut r = SessionRefresh::default();
        r.mark(lane(1));
        r.mark(lane(2));
        r.forget(lane(2));
        assert_eq!(r.take_due(now), vec![lane(1)]);
        assert_eq!(r.until_due(now), None, "没有待发时不该叫醒循环");
    }

    /// 取走这条 lane 已经发出的 job（断言「发了几次」用）。
    fn sent_jobs(rx: &mpsc::Receiver<WorkerJob>) -> usize {
        rx.try_iter().count()
    }

    /// 走真实事件路径结束一轮：`on_turn_done` 的接线也在被测范围内。
    fn finish_turn(app: &mut TuiApp<'_>, id: LaneId) {
        app.on_msg(Inbound::FromLane(
            id,
            UiEvent::TurnDone {
                outcome: ChatStreamOutcome::default(),
                error: None,
                interrupted: None,
            },
        ));
    }

    #[test]
    fn turn_done_marks_instead_of_sending_at_once() {
        let client = test_client();
        let mut prefs = [test_prefs()];
        let (mut app, jobs) = test_shell_with_jobs(&client, &mut prefs);
        let id = app.lanes[0].id;

        finish_turn(&mut app, id);
        assert_eq!(sent_jobs(&jobs[0]), 0, "回合结束不当场发请求，只留意图位");

        app.flush_session_refresh(Instant::now());
        assert_eq!(sent_jobs(&jobs[0]), 1, "外壳收敛后才发一次");
    }

    #[test]
    fn two_turns_inside_one_window_collapse_into_one_refresh() {
        let client = test_client();
        let mut prefs = [test_prefs()];
        let (mut app, jobs) = test_shell_with_jobs(&client, &mut prefs);
        let id = app.lanes[0].id;
        // 用注入的 `now` 推进窗口，不依赖真实挂钟。
        let t0 = Instant::now();

        finish_turn(&mut app, id);
        app.flush_session_refresh(t0);
        let mut sent = sent_jobs(&jobs[0]);
        assert_eq!(sent, 1);

        finish_turn(&mut app, id);
        app.flush_session_refresh(t0 + Duration::from_millis(50));
        sent += sent_jobs(&jobs[0]);
        assert_eq!(sent, 1, "窗口内不再发：这才是合并");

        app.flush_session_refresh(t0 + SESSION_REFRESH_WINDOW);
        sent += sent_jobs(&jobs[0]);
        assert_eq!(sent, 2, "窗口到点把攒下的那次补上");
    }

    #[test]
    fn each_lane_still_gets_its_own_refresh() {
        // 当前口径（#349 这一步只收敛频次）：不同 lane 各自刷新自己的会话列表，
        // 所以两条都发。按 serve 去重必须同时 fan-out 结果，是后续一步 —— 到那时
        // 这条断言应改成「1 条 + 同组两条 lane 都更新」。
        let client = test_client();
        let mut prefs = [test_prefs(), test_prefs()];
        let (mut app, jobs) = test_shell_with_jobs(&client, &mut prefs);
        let (first, second) = (app.lanes[0].id, app.lanes[1].id);

        finish_turn(&mut app, first);
        finish_turn(&mut app, second);
        app.flush_session_refresh(Instant::now());

        assert_eq!(sent_jobs(&jobs[0]), 1);
        assert_eq!(sent_jobs(&jobs[1]), 1);
    }
}
