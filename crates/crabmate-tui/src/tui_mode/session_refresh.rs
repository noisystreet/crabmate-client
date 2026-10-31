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
//! 本模块只管「什么时候发」。按 `api_base` 去重与回包 fan-out 在外壳
//! （`TuiApp::flush_session_refresh` / `TuiApp::fanout_shared_refresh`）：那两件事要
//! 看得到全部 lane 的连接信息，而这里只认 `LaneId`。

use std::time::{Duration, Instant};

use crate::serve::same_api_base;

use super::worker::{LaneId, UiEvent};

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

/// 外壳侧的收敛点与广播：这两件事要看得到全部 lane 的连接信息，所以放在这里
/// （`TuiApp` 的字段与 `LaneApp` 的意图位都在 `tui_mode` 模块内可见），而不是挤进
/// 已经贴着 920 行上限的 `shell.rs`。
impl super::TuiApp<'_> {
    /// 把各 lane 的「回合结束待刷新」意图收敛成一次发送（issue #349）。
    ///
    /// 每轮帧循环调一次（`event_loop::run_loop`）：先收集意图，再按窗口合并发出去。
    /// 窗口未到就什么都不发 —— 意图留在 [`SessionRefresh`] 里，下一轮的等待时长会把
    /// 窗口剩余算进去，不会拖到 1s 兜底。
    ///
    /// 同一台 serve 只让**第一条** lane 去问：它的回包由 [`Self::fanout_shared_refresh`]
    /// 复制给同组，所以被跳过的那条不是「不刷新」，只是不当第二个请求方。
    pub(super) fn flush_session_refresh(&mut self, now: Instant) {
        let wanted: Vec<LaneId> = self
            .lanes
            .iter()
            .filter(|l| l.session_refresh_pending)
            .map(|l| l.id)
            .collect();
        for id in wanted {
            self.session_refresh.mark(id);
        }
        let mut claimed: Vec<String> = Vec::new();
        for id in self.session_refresh.take_due(now) {
            // 已 detach 的 lane：`detach_lane` 应该已经 `forget` 掉了，这里只是兜底。
            let Some(lane) = self.lanes.iter_mut().find(|l| l.id == id) else {
                continue;
            };
            lane.session_refresh_pending = false;
            let base = lane.client.config().api_base.clone();
            if claimed.iter().any(|c| same_api_base(c, base.as_str())) {
                continue;
            }
            claimed.push(base);
            lane.refresh_sessions();
        }
    }

    /// 同一台 serve 的 lane 共用一份会话列表：代劳发请求的那条把回包复制给同组（issue #349）。
    ///
    /// 无状态广播，所以手动刷新（侧栏 `r` / `/conv refresh`）也会让同组一起更新 ——
    /// 同一台 serve 本来就只有一份事实。只复制 [`UiEvent::Sessions`] 与
    /// [`UiEvent::WorkspacePath`]：正文增量、审批、历史分页都属于**某条 lane 的会话**，
    /// 复制过去就是串台。
    ///
    /// 启动期的 `bootstrap` 不走这里：那时每条 lane 各自首拉一次，属于启动瞬态。
    pub(super) fn fanout_shared_refresh(&mut self, from: LaneId, ev: &UiEvent) {
        if !matches!(ev, UiEvent::Sessions(_) | UiEvent::WorkspacePath(_)) {
            return;
        }
        for id in self.shared_refresh_targets(from) {
            let Some(lane) = self.lanes.iter_mut().find(|l| l.id == id) else {
                continue;
            };
            match ev {
                UiEvent::Sessions(result) => lane.on_msg(UiEvent::Sessions(result.clone())),
                UiEvent::WorkspacePath(path) => lane.on_msg(UiEvent::WorkspacePath(path.clone())),
                // 上面 `matches!` 已经挡掉其它变体。
                _ => {}
            }
        }
    }

    /// 与 `from` 连同一台 serve 的其它 lane（`api_base` 按 [`same_api_base`] 比较，
    /// 尾斜杠与大小写不算差异）。
    fn shared_refresh_targets(&self, from: LaneId) -> Vec<LaneId> {
        let Some(base) = self
            .lanes
            .iter()
            .find(|l| l.id == from)
            .map(|l| l.client.config().api_base.clone())
        else {
            return Vec::new();
        };
        self.lanes
            .iter()
            .filter(|l| {
                l.id != from && same_api_base(base.as_str(), l.client.config().api_base.as_str())
            })
            .map(|l| l.id)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use super::{SESSION_REFRESH_WINDOW, SessionRefresh};
    use crate::serve::ChatStreamOutcome;
    use crate::serve::WebSessionsList;
    use crate::tui_mode::TuiApp;
    use crate::tui_mode::inbound::Inbound;
    use crate::tui_mode::test_support::{
        test_client, test_client_at, test_prefs, test_shell_with_clients, test_shell_with_jobs,
    };
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
    /// 采集层不按 serve 去重：它只认 `LaneId`，不知道谁和谁连同一台。去重在外壳的
    /// `flush_session_refresh`（按 `api_base`），所以这里两条都留下是正确的。
    fn lanes_are_independent_within_one_window() {
        let now = base();
        let mut r = SessionRefresh::default();
        r.mark(lane(1));
        r.mark(lane(2));
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
    fn lanes_on_the_same_serve_share_one_refresh() {
        // 两条 lane 连同一台 serve（`test_shell_with_jobs` 给的是同一个 client）：
        // 只让第一条去问，第二条等回包复制过去（issue #349 的去重面）。
        let client = test_client();
        let mut prefs = [test_prefs(), test_prefs()];
        let (mut app, jobs) = test_shell_with_jobs(&client, &mut prefs);
        let (first, second) = (app.lanes[0].id, app.lanes[1].id);

        finish_turn(&mut app, first);
        finish_turn(&mut app, second);
        app.flush_session_refresh(Instant::now());

        assert_eq!(sent_jobs(&jobs[0]), 1);
        assert_eq!(sent_jobs(&jobs[1]), 0, "同一台 serve 不当第二个请求方");
    }

    #[test]
    fn lanes_on_different_serves_refresh_separately() {
        let first_client = test_client();
        let second_client = test_client_at(2);
        let mut prefs = [test_prefs(), test_prefs()];
        let (mut app, jobs) = test_shell_with_clients(&[&first_client, &second_client], &mut prefs);
        let (first, second) = (app.lanes[0].id, app.lanes[1].id);

        finish_turn(&mut app, first);
        finish_turn(&mut app, second);
        app.flush_session_refresh(Instant::now());

        assert_eq!(sent_jobs(&jobs[0]), 1);
        assert_eq!(sent_jobs(&jobs[1]), 1, "不同 serve 各刷各的");
    }

    #[test]
    fn sessions_result_reaches_every_lane_on_the_same_serve() {
        let client = test_client();
        let mut prefs = [test_prefs(), test_prefs()];
        let (mut app, _jobs) = test_shell_with_jobs(&client, &mut prefs);
        let first = app.lanes[0].id;

        app.on_msg(Inbound::FromLane(
            first,
            UiEvent::Sessions(Ok(WebSessionsList {
                sessions: Vec::new(),
                active_session_id: Some("conv-x".into()),
                raw_sessions: serde_json::Value::Null,
            })),
        ));

        assert_eq!(app.lanes[0].st.active_session_id.as_deref(), Some("conv-x"));
        assert_eq!(
            app.lanes[1].st.active_session_id.as_deref(),
            Some("conv-x"),
            "同 serve 的第二条也要拿到这份列表"
        );
    }

    #[test]
    fn workspace_path_is_shared_between_lanes_on_the_same_serve() {
        let client = test_client();
        let mut prefs = [test_prefs(), test_prefs()];
        let (mut app, _jobs) = test_shell_with_jobs(&client, &mut prefs);
        let first = app.lanes[0].id;

        app.on_msg(Inbound::FromLane(
            first,
            UiEvent::WorkspacePath(Some("/srv/a".into())),
        ));

        assert_eq!(app.lanes[1].st.workspace_path.as_deref(), Some("/srv/a"));
    }

    #[test]
    fn workspace_path_is_not_shared_across_serves() {
        let first_client = test_client();
        let second_client = test_client_at(2);
        let mut prefs = [test_prefs(), test_prefs()];
        let (mut app, _jobs) =
            test_shell_with_clients(&[&first_client, &second_client], &mut prefs);
        let first = app.lanes[0].id;

        app.on_msg(Inbound::FromLane(
            first,
            UiEvent::WorkspacePath(Some("/srv/a".into())),
        ));

        assert_eq!(app.lanes[0].st.workspace_path.as_deref(), Some("/srv/a"));
        assert_eq!(app.lanes[1].st.workspace_path, None, "不同 serve 不许串台");
    }
}
