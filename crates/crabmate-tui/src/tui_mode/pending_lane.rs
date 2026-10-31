//! 「已请求、尚未就绪」的 lane：还不是连接，所以不进 `lanes`。
//!
//! `/lane new` 的 20 s 就绪轮询跑在**外壳级的 ctl 线程**上（issue #350），但它的结果
//! 只在线程返回时才有 —— 期间用户敲完命令，屏幕上没有任何变化，读起来就是「命令没生效」。
//! 这里给外壳一份显式三态，由左栏画成 `◌` / `✗` 一行（见 `lane_list::pending_row`）。
//!
//! 为什么自成一类而不是塞进 `Vec<LaneApp>`：它没有客户端、没有 URL、没有事件通道，塞进
//! 去之后每条与 lane 相关的代码都要多一个「这条其实还没连上」的分支。而 `LaneEntry` /
//! `layout::lane_index_at` 的点击映射只覆盖前 `lanes.len()` 行，pending 行排在它们之后，
//! 点不到，也就不会误切到某条真 lane。

use super::shell::MAX_LANES;
use super::state::LineKind;
use super::worker::LaneId;
use super::worker_ctl;

/// 一条待就绪 lane 的当前阶段。
///
/// 用枚举而不是「有没有失败 + 有没有超时」两个布尔：后者允许「既失败又超时」这种
/// 非法组合，画哪一行就要靠 if 的先后顺序决定（那是最难在测试里说清的一种实现）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PendingState {
    /// 声明已写、ctl 正在起。
    Starting,
    /// 超过 `LANE_READY_TIMEOUT` 仍未就绪。
    ///
    /// **不是**失败结论：实例可能仍在起，把它报成失败会让人去重试一件正在发生的事
    /// （与 ctl worker 侧同一口径）。
    TimedOut,
    /// ctl 明确报 `failed`，原因已拿到（Server 的 `last_error` 原文）。
    Failed(String),
}

impl PendingState {
    /// 单列状态符号。
    ///
    /// 与 `LaneStatus::mark` 同一约束：宽度必须确定是 1 —— `◌` 与 `✗` 被
    /// `every_row_fills_the_width` 一起钉住，任一被算成双列就会把整行撑破。
    pub(super) fn mark(&self) -> &'static str {
        match self {
            // 超时仍画 `◌`：实例还在起，只差一句「已经等过了」的尾注。
            Self::Starting | Self::TimedOut => "◌",
            Self::Failed(_) => "✗",
        }
    }

    /// 行尾的一句话补充；没有就是空串。
    ///
    /// 失败原因原样透传（Server 的 `last_error`），不在这里改写成统一文案：真正知道
    /// 「为什么起不来」的只有 serve。
    pub(super) fn note(&self) -> &str {
        match self {
            Self::Starting => "",
            Self::TimedOut => "超时",
            Self::Failed(why) => why,
        }
    }
}

/// 一条待就绪的 lane。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PendingLane {
    /// 实例名（将来那条 lane 的 `label`）。
    pub(super) name: String,
    /// 当前阶段。
    pub(super) state: PendingState,
}

/// 待就绪 lane 的集合：外壳持有，`/lane new` 的两条事件往里写。
#[derive(Debug, Default)]
pub(super) struct PendingLanes {
    items: Vec<PendingLane>,
}

impl PendingLanes {
    /// 声明写成功：该名字出现（或重置）为「启动中」。
    ///
    /// 同名重试会清掉上一次的 `TimedOut` / `Failed` —— 否则重试成功之后，左栏还挂着
    /// 一行旧错误，用户无法判断哪一行是这次的。
    pub(super) fn upsert(&mut self, name: &str) {
        if self.set_state(name, PendingState::Starting) {
            return;
        }
        // 上限与真连接同一个数：超了丢**最旧**的一条。它排在最上面，也最可能已经被
        // 忘掉；无界增长只会把真连接挤出可见区。
        if self.items.len() >= MAX_LANES {
            self.items.remove(0);
        }
        self.items.push(PendingLane {
            name: name.to_string(),
            state: PendingState::Starting,
        });
    }

    /// 20 s 内没就绪：仍留在列表里（`◌` + 「超时」），因为它可能还在起。
    pub(super) fn mark_timed_out(&mut self, name: &str) {
        if !self.set_state(name, PendingState::TimedOut) {
            // 没收到过 `Pending`（理论上不会：轮询之前一定先发过）也补一条，
            // 免得结果无处可落、用户什么也看不到。
            self.upsert(name);
            self.set_state(name, PendingState::TimedOut);
        }
    }

    /// ctl 报 `failed`：留在列表里，画成 `✗` + 原因原文。
    pub(super) fn mark_failed(&mut self, name: &str, why: &str) {
        let state = PendingState::Failed(why.to_string());
        if !self.set_state(name, state.clone()) {
            self.items.push(PendingLane {
                name: name.to_string(),
                state,
            });
        }
    }

    /// 就绪（紧接着会被 `attach_lane` 装配成真 lane）。
    pub(super) fn remove(&mut self, name: &str) {
        self.items.retain(|p| p.name != name);
    }

    /// 按出现顺序遍历（左栏就按这个顺序画）。
    pub(super) fn iter(&self) -> impl Iterator<Item = &PendingLane> {
        self.items.iter()
    }

    /// 写状态；该名字不在集合里时返回 `false`（由调用方决定要不要新建）。
    fn set_state(&mut self, name: &str, state: PendingState) -> bool {
        match self.items.iter_mut().find(|p| p.name == name) {
            Some(item) => {
                item.state = state;
                true
            }
            None => false,
        }
    }
}

impl<'a> super::TuiApp<'a> {
    /// `ShellEvent::NewSettled`：`/lane new` 的三种结局。
    ///
    /// 就绪 → 去掉 `◌` 行并装配真 lane；失败 / 超时 → **留在**待就绪列表里（`✗` 或
    /// `◌` + 尾注），而面向用户的那句话写回**发起命令**的 lane —— 20 s 轮询期间用户
    /// 很可能已经切走了，写进「当前」那条等于把别人的结果放到你的屏上。
    ///
    /// 超时单独一档而不是同一个 `Err`：实例可能仍在起，文案是「稍后 `/ctl` 复查」而
    /// 不是「失败」，两者的后续动作不同（重试 vs 等待）。
    pub(super) fn on_lane_new_settled(
        &mut self,
        from: LaneId,
        name: String,
        outcome: worker_ctl::NewOutcome,
    ) {
        match outcome {
            worker_ctl::NewOutcome::Ready(base_url) => {
                self.pending.remove(&name);
                self.attach_lane(&name, &base_url);
            }
            worker_ctl::NewOutcome::Failed(why) => {
                self.pending.mark_failed(&name, &why);
                self.lane_by_id_or_active(from)
                    .st
                    .push_line(LineKind::System, &format!("连接 `{name}` 失败：{why}"));
            }
            worker_ctl::NewOutcome::TimedOut => {
                self.pending.mark_timed_out(&name);
                self.lane_by_id_or_active(from).st.push_line(
                    LineKind::System,
                    &format!(
                        "实例 {name} 在 {} s 内未就绪（可能仍在启动）：`/ctl` 复查，就绪后 `/lane attach {name}`",
                        worker_ctl::LANE_READY_TIMEOUT.as_secs()
                    ),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(p: &PendingLanes) -> Vec<&str> {
        p.iter().map(|p| p.name.as_str()).collect()
    }

    #[test]
    fn upsert_keeps_one_row_per_name_and_resets_the_state() {
        let mut p = PendingLanes::default();
        p.upsert("a");
        p.mark_failed("a", "工作区不在白名单");
        assert_eq!(p.iter().next().unwrap().state.mark(), "✗");

        // 同名重试：回到「启动中」，不新增一行（否则左栏会有两条 `a`）。
        p.upsert("a");
        assert_eq!(names(&p), ["a"]);
        assert_eq!(p.iter().next().unwrap().state, PendingState::Starting);
    }

    #[test]
    fn timeout_is_not_a_failure() {
        let mut p = PendingLanes::default();
        p.upsert("a");
        p.mark_timed_out("a");
        let row = p.iter().next().unwrap();
        assert_eq!(row.state.mark(), "◌", "超时仍画启动中，不画失败");
        assert_eq!(row.state.note(), "超时");
    }

    #[test]
    fn failed_carries_the_server_reason_verbatim() {
        let mut p = PendingLanes::default();
        // 没收到过 `Pending` 也要能落一条：add 失败走的就是这条路。
        p.mark_failed("a", "工作区不在白名单");
        let row = p.iter().next().unwrap();
        assert_eq!(row.state.mark(), "✗");
        assert_eq!(row.state.note(), "工作区不在白名单");
    }

    #[test]
    fn ready_removes_the_row() {
        let mut p = PendingLanes::default();
        p.upsert("a");
        p.upsert("b");
        p.remove("a");
        assert_eq!(names(&p), ["b"]);
    }

    /// 无界增长会把真连接挤出可见区：满 `MAX_LANES` 后丢最旧的。
    #[test]
    fn capacity_drops_the_oldest() {
        let mut p = PendingLanes::default();
        for i in 0..=MAX_LANES {
            p.upsert(&format!("n{i}"));
        }
        let names = names(&p);
        assert_eq!(names.len(), MAX_LANES);
        assert!(!names.contains(&"n0"), "最旧的一条应被丢掉：{names:?}");
        assert!(names.contains(&"n8"), "最新的一条必须在：{names:?}");
    }

    /// `/lane new` 的两条事件在外壳上的落点：`Pending` 摆一行，失败把它留在列表里并
    /// 写回**发起命令**那条 lane；同名重试只重置状态，不新增行。
    ///
    /// 就绪分支要真的装配一条 lane（`attach_lane`），那条路径由 attach 自己的测试与
    /// `on_lane_ready` 覆盖；这里钉的是「`◌` 行什么时候出现、什么时候该还在」。
    #[test]
    fn pending_lane_tracks_the_new_events() {
        use crate::tui_mode::inbound::{Inbound, ShellEvent};
        use crate::tui_mode::pending_lane::PendingState;
        use crate::tui_mode::test_support::{test_client, test_prefs, test_shell_with_jobs};
        use crate::tui_mode::worker::LaneId;
        use crate::tui_mode::worker_ctl::NewOutcome;

        let client = test_client();
        let mut prefs = [test_prefs()];
        let (mut app, _job_rxs) = test_shell_with_jobs(&client, &mut prefs);
        app.on_msg(Inbound::Shell(ShellEvent::Pending {
            name: "proj-c".into(),
        }));
        assert_eq!(app.pending.iter().count(), 1, "写声明之后要有一行 ◌");

        app.on_msg(Inbound::Shell(ShellEvent::NewSettled {
            from: LaneId(1),
            name: "proj-c".into(),
            outcome: NewOutcome::Failed("工作区不在白名单".into()),
        }));
        let row = app.pending.iter().next().expect("失败要留在列表里");
        assert_eq!(row.state.mark(), "✗");
        assert_eq!(row.state.note(), "工作区不在白名单");
        assert!(
            app.lanes.iter().any(|l| l
                .st
                .lines
                .iter()
                .any(|line| line.text().contains("工作区不在白名单"))),
            "失败原因也要写进发起命令那条 lane 的 transcript"
        );

        // 同名重试：回到「启动中」，并且仍然只有一行。
        app.on_msg(Inbound::Shell(ShellEvent::Pending {
            name: "proj-c".into(),
        }));
        assert_eq!(
            app.pending.iter().next().unwrap().state,
            PendingState::Starting
        );
        assert_eq!(app.pending.iter().count(), 1, "同名重试不该多一行");
    }
}
