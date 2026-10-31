//! `LaneApp` 的测试构造辅助（仅 `#[cfg(test)]`）。
//!
//! 事件循环里有几处「接线」只能在 `LaneApp` 层验证 —— 草稿换入换出、排队消息的去向、
//! 回放失败后的回滚 —— 它们都不是纯函数，单测得先造出一个 `LaneApp`。
//!
//! 这份构造器原本是 `workspace_cmd` 测试里的私有辅助。多个模块各写一份必然漂移，
//! 而漂移的表现是「某个测试的初始状态与真实启动不一致」这种**假通过**，所以提到这里共用。

use std::borrow::Cow;
use std::sync::mpsc;

use crate::SessionPrefs;
use crate::serve::{ConnectionConfig, ServeClient};

use super::inbound::Inbound;
use super::spinner::Spinner;
use super::state::UiState;
use super::worker::{LaneId, WorkerJob};
use super::{LaneApp, LaneTemplate, TuiApp};

/// 空的会话级 override（`*` 覆盖），让状态行回落到 serve 默认。
pub(super) fn test_prefs() -> SessionPrefs {
    SessionPrefs {
        api_key: None,
        model: None,
        api_base: None,
        agent_role: None,
        session_mode: None,
        permission_level: None,
    }
}

/// 不连网的 `ServeClient`（端口 1 必然连不上）。
pub(super) fn test_client() -> ServeClient {
    test_client_at(1)
}

/// 指定端口的「不连网」client。
///
/// 接线测试只关心「状态怎么变」，任何真发出去的请求都应该失败得明明白白，而不是悄悄
/// 打到某个本地 serve 上、让断言依赖外部环境。多 serve 场景要能造**不同**的
/// `api_base`，所以端口可指定。
pub(super) fn test_client_at(port: u16) -> ServeClient {
    ServeClient::new(ConnectionConfig::new(
        format!("http://127.0.0.1:{port}"),
        "",
    ))
    .expect("client")
}

/// 构造一条 lane：`yes` / `no_keyring` 都打开，其余为启动初始态。
///
/// `id` 由调用方给 —— 外壳层测试要造**多条** lane，必须能区分事件来源。
/// 单 lane 的常规测试用 [`test_app`] 即可。
///
/// `rx` / `tx` 必须是同一对：单测没有 tokio runtime，读请求会退回队列；但若将来某个
/// 测试跑在 `#[tokio::test]` 下，读请求就真的会 `spawn`，那时发到一个没人读的通道里
/// 会让断言假通过。`job_tx` 同理 —— 要断言「发了哪些 worker 任务」就把接收端留在
/// 调用方。
pub(super) fn test_lane<'a>(
    id: LaneId,
    client: &'a ServeClient,
    prefs: &'a mut SessionPrefs,
    rx: mpsc::Receiver<Inbound>,
    tx: mpsc::Sender<Inbound>,
    job_tx: mpsc::Sender<WorkerJob>,
) -> LaneApp<'a> {
    // `rx` 不再交给 lane：它由外层 `TuiApp` 持有（见 `mod.rs`）。这里只把接收端
    // 留在内存里让通道保持连通 —— 提前 drop 会让 lane 的 `tx.send` 全部失败，
    // 而测试恰恰靠「发了什么事件」断言。
    std::mem::forget(rx);
    lane_with_tx(id, client, prefs, tx, job_tx)
}

/// lane 的构造主体：**发送端由调用方决定**。
///
/// 多 lane 的外壳测试必须让每条 lane 都拿外壳那条通道的发送端（与生产同构：
/// 全进程一个 `rx`，见 `mod.rs`），否则 `TuiApp::rx` 收不到任何事件；单 lane 的
/// 常规测试走 [`test_app`]，自带一条没人读的通道。
pub(super) fn lane_with_tx<'a>(
    id: LaneId,
    client: &'a ServeClient,
    prefs: &'a mut SessionPrefs,
    tx: mpsc::Sender<Inbound>,
    job_tx: mpsc::Sender<WorkerJob>,
) -> LaneApp<'a> {
    LaneApp {
        id,
        label: format!("lane-{}", id.0),
        client: Cow::Borrowed(client),
        overrides: prefs.clone(),
        yes: true,
        no_keyring: true,
        st: UiState::new(),
        drafts: Default::default(),
        spinner: Spinner::new(),
        tx,
        job_tx,
        cancel: None,
        exit: false,
        quit_pending: false,
        body: Default::default(),
        panel: None,
        persisted: None,
        last_size: (0, 0),
        index: Default::default(),
        resume: None,
        sent_turn: None,
        bg: None,
        // 测试默认不启用控制面：`/ctl` 会走「未启用」分支，除非用例自己塞一个发送端。
        ctl: None,
        session_refresh_pending: false,
        unread: false,
        turn_started: None,
    }
}

/// 单 lane 的常规测试入口（固定 `LaneId(0)`）。
pub(super) fn test_app<'a>(
    client: &'a ServeClient,
    prefs: &'a mut SessionPrefs,
    rx: mpsc::Receiver<Inbound>,
    tx: mpsc::Sender<Inbound>,
    job_tx: mpsc::Sender<WorkerJob>,
) -> LaneApp<'a> {
    test_lane(LaneId(0), client, prefs, rx, tx, job_tx)
}

/// 造 n 条 lane 的外壳（`prefs.len()` 即 lane 数，第一条是激活的）。
///
/// 所有 lane 共用外壳这条通道 —— 与生产同构（全进程一个 `rx`，见 `mod.rs`）；
/// 给每条 lane 自成一条通道的话，`app.rx` 就永远收不到事件。
///
/// 放在模块顶层而不是某个用例的私有辅助里：**外壳级**判定与收尾
/// （`exit_requested`、退出落盘的逐 lane 遍历）都必须有多 lane 才测得出来。
pub(super) fn test_shell<'a>(client: &'a ServeClient, prefs: &'a mut [SessionPrefs]) -> TuiApp<'a> {
    test_shell_with_jobs(client, prefs).0
}

/// 同 [`test_shell`]，另把各 lane 的 `job_rx` 交出来。
///
/// 回合结束后的会话刷新收敛到外壳（issue #349）之后，「发没发、发了几次」只能靠
/// `job_rx` 断言 —— 事件通道上看不到这条边界。
pub(super) fn test_shell_with_jobs<'a>(
    client: &'a ServeClient,
    prefs: &'a mut [SessionPrefs],
) -> (TuiApp<'a>, Vec<mpsc::Receiver<WorkerJob>>) {
    let clients: Vec<&'a ServeClient> = prefs.iter().map(|_| client).collect();
    test_shell_with_clients(&clients, prefs)
}

/// 同 [`test_shell_with_jobs`]，但每条 lane 用**各自的** client（不同 `api_base`）。
///
/// 「同一台 serve 只问一次」与「不同 serve 各问各的」这两条规则只有在这里才测得出来。
pub(super) fn test_shell_with_clients<'a>(
    clients: &[&'a ServeClient],
    prefs: &'a mut [SessionPrefs],
) -> (TuiApp<'a>, Vec<mpsc::Receiver<WorkerJob>>) {
    assert_eq!(clients.len(), prefs.len(), "每条 lane 一个 client");
    let (shell_tx, rx) = mpsc::channel();
    let mut lanes = Vec::new();
    let mut job_rxs = Vec::new();
    // 条数在借出去之前记下：`prefs` 会被逐条 `iter_mut` 借到 `'a`，循环后再
    // 读 `len()` 会与那份借用冲突。
    let count = prefs.len();
    for (i, (p, client)) in prefs.iter_mut().zip(clients.iter()).enumerate() {
        let (job_tx, job_rx) = mpsc::channel();
        job_rxs.push(job_rx);
        lanes.push(lane_with_tx(
            LaneId(i as u32 + 1),
            client,
            p,
            shell_tx.clone(),
            job_tx,
        ));
    }
    let next_lane_id = count as u32 + 1;
    let app = TuiApp {
        lanes,
        active: 0,
        rx,
        next_lane_id,
        template: LaneTemplate {
            overrides: test_prefs(),
            yes: true,
            no_keyring: true,
            bearer: String::new(),
            ctl: None,
            tx: shell_tx,
        },
        session_refresh: Default::default(),
    };
    (app, job_rxs)
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use crate::serve::StreamCancel;

    use super::super::approve::ApprovalPrompt;
    use super::super::controls::Control;
    use super::super::inbound::{Inbound, ShellEvent};
    use super::super::lane_list::LaneStatus;
    use super::super::worker::{LaneId, LaneTx, UiEvent};
    use super::{test_app, test_client, test_prefs, test_shell};

    #[test]
    fn switch_lane_is_a_noop_with_a_single_lane() {
        let client = test_client();
        let mut prefs = [test_prefs()];
        let mut app = test_shell(&client, &mut prefs);

        app.switch_lane(1);
        assert_eq!(app.active, 0, "单 lane 时切换必须无副作用");
        app.switch_lane(-1);
        assert_eq!(app.active, 0);
    }

    #[test]
    fn switch_lane_cycles_in_both_directions() {
        let client = test_client();
        let mut prefs = [test_prefs(), test_prefs(), test_prefs()];
        let mut app = test_shell(&client, &mut prefs);
        assert_eq!(app.lane().id, LaneId(1));

        app.switch_lane(1);
        assert_eq!(app.lane().id, LaneId(2));
        app.switch_lane(1);
        assert_eq!(app.lane().id, LaneId(3));
        // 到尾部后循环回第一条，而不是卡住或越界。
        app.switch_lane(1);
        assert_eq!(app.lane().id, LaneId(1));
        // 反向：从第 1 条回到最后一条。
        app.switch_lane(-1);
        assert_eq!(app.lane().id, LaneId(3));
    }

    /// 后台 lane 的 `/quit` 必须被外壳看见（issue #338 的症状 A）。
    ///
    /// 旧判据是 `self.lane().exit`：后台 lane 在回合结束时置了自己的 `exit`，屏幕上却
    /// 毫无反应，等用户切过去才突然退出整个 TUI。判据改成扫全部 lane 之后，「是谁提的」
    /// 不再影响「要不要退」，`exit` 也不再随聚焦焦点漂移。
    #[test]
    fn exit_requested_sees_a_background_lane() {
        let client = test_client();
        let mut prefs = [test_prefs(), test_prefs()];
        let mut app = test_shell(&client, &mut prefs);
        assert!(!app.exit_requested(), "没请求时不退出");

        // lane-2 在后台（active 仍是 lane-1）提了退出。
        app.lanes[1].exit = true;
        assert!(!app.lane().exit, "当前 lane 自己并没有请求");
        assert!(
            app.exit_requested(),
            "任何一条 lane 请求都要退出整个 TUI，而不是等用户切过去"
        );

        // 切走不影响结论：退出语义不随焦点漂移。
        app.switch_lane(1);
        assert!(app.exit_requested());
    }

    #[test]
    fn switching_does_not_move_state_between_lanes() {
        let client = test_client();
        let mut prefs = [test_prefs(), test_prefs()];
        let mut app = test_shell(&client, &mut prefs);

        // 在 lane 1 里设一个 `/model` 覆盖，切走再切回来必须是同一份，
        // 且 lane 2 完全不受影响（这正是 overrides 从 `&mut` 改自有值的理由）。
        app.lane_mut().overrides.model = Some("lane-1-model".to_string());
        app.switch_lane(1);
        assert_eq!(app.lane().overrides.model, None, "lane 2 有自己的表达层");
        app.switch_lane(-1);
        assert_eq!(app.lane().overrides.model.as_deref(), Some("lane-1-model"));
    }

    #[test]
    fn test_app_still_builds_a_single_default_lane() {
        let client = test_client();
        let mut prefs = test_prefs();
        let (tx, rx) = mpsc::channel();
        let (job_tx, _job_rx) = mpsc::channel();
        let app = test_app(&client, &mut prefs, rx, tx, job_tx);
        assert_eq!(app.id, LaneId(0), "常规测试默认 lane 编号保持不变");
    }

    /// `/lane attach` 的落地动作：加一条 lane 并切过去，编号不复用。
    #[test]
    fn attach_lane_appends_a_lane_and_switches_to_it() {
        let client = test_client();
        let mut prefs = [test_prefs()];
        let mut app = test_shell(&client, &mut prefs);
        let before = app.next_lane_id;

        app.attach_lane("proj-b", "http://127.0.0.1:8721/");

        assert_eq!(app.lanes.len(), 2, "应新增一条 lane");
        assert_eq!(app.active, 1, "新增后切到它");
        assert_eq!(app.lane().label, "proj-b");
        assert_eq!(app.next_lane_id, before + 1, "编号单调递增");
        assert_ne!(app.lanes[0].id, app.lanes[1].id, "LaneId 不能重复");
    }

    /// 控制面回执里的失败写回**发起命令的那条** lane（issue #350）。
    ///
    /// 控制面调用已由外壳级线程执行，归属仍按 `LaneId`：`/lane new` 的 20 s 轮询期间
    /// 用户很可能已经切走了，写进「当前」那条等于把别人的失败放到你的屏上。
    #[test]
    fn control_plane_failure_lands_in_the_lane_that_asked() {
        let client = test_client();
        let mut prefs = [test_prefs(), test_prefs()];
        let mut app = test_shell(&client, &mut prefs);

        // lane-2 发起的 `/lane new` 失败了，而当前看的是 lane-1。
        app.on_lane_ready(
            LaneId(2),
            "proj-c".to_string(),
            Err("工作区不在白名单".to_string()),
        );

        assert!(
            app.lanes[1]
                .st
                .lines
                .iter()
                .any(|l| l.text().contains("连接 `proj-c` 失败：工作区不在白名单")),
            "失败要写进发起命令的那条 lane"
        );
        assert!(
            !app.lanes[0]
                .st
                .lines
                .iter()
                .any(|l| l.text().contains("proj-c")),
            "另一条 lane 不该看到别人的失败"
        );
    }

    /// 同名连接已存在时只切过去：重复连会让左栏出现两条同名 lane。
    #[test]
    fn attaching_an_existing_name_only_switches() {
        let client = test_client();
        let mut prefs = [test_prefs()];
        let mut app = test_shell(&client, &mut prefs);
        let label = app.lanes[0].label.clone();

        app.attach_lane(&label, "http://127.0.0.1:1/");

        assert_eq!(app.lanes.len(), 1, "同名不重复连");
        assert_eq!(app.active, 0, "切回已有的那条");
    }

    /// 名字不同但 URL 相同也是「同一条 serve」：不重复连（否则各开一条 SSE、各读一份会话）。
    #[test]
    fn attaching_a_different_name_on_the_same_url_only_switches() {
        let client = test_client();
        let mut prefs = [test_prefs()];
        let mut app = test_shell(&client, &mut prefs);
        // 尾斜杠与大小写不参与比较 —— 它们本来就是同一条 serve。
        let url = format!("{}/", client.config().api_base);

        app.attach_lane("另一个名字", &url);

        assert_eq!(app.lanes.len(), 1, "同 URL 不重复连");
        assert_eq!(app.active, 0, "切回已连着的那条");
        let last = app.lanes[0].st.lines.last().expect("应有提示").text();
        assert!(last.contains("不重复连"), "{last}");
    }

    /// `/lane detach`：只摘命名的那条，其余 lane 与声明都不动。
    #[test]
    fn detach_lane_removes_only_the_named_lane() {
        let client = test_client();
        let mut prefs = [test_prefs(), test_prefs(), test_prefs()];
        let mut app = test_shell(&client, &mut prefs);

        app.detach_lane("lane-2");
        let labels: Vec<&str> = app.lanes.iter().map(|l| l.label.as_str()).collect();
        assert_eq!(labels, ["lane-1", "lane-3"]);
        assert_eq!(app.active, 0, "摘的不是当前，指针不动");

        // 摘掉末尾那条：指针必须仍在合法范围内。
        app.detach_lane("lane-3");
        assert_eq!(app.lanes.len(), 1);
        assert_eq!(app.active, 0);
        let last = app.lanes[0].st.lines.last().expect("应有提示").text();
        assert!(last.contains("已断开 `lane-3`"), "{last}");
    }

    /// 摘当前且它不在末尾：停在原位置（= 原来的下一条），不跳到别处。
    #[test]
    fn detaching_the_active_middle_lane_stays_in_place() {
        let client = test_client();
        let mut prefs = [test_prefs(), test_prefs(), test_prefs()];
        let mut app = test_shell(&client, &mut prefs);
        app.active = 1;

        app.detach_lane("lane-2");

        assert_eq!(app.lanes.len(), 2);
        assert_eq!(app.active, 1, "原位现在是原来的下一条");
        assert_eq!(app.lane().label, "lane-3");
    }

    /// 最后一条不许摘：界面至少要有一条 lane（退出客户端用 `/quit`）。
    #[test]
    fn detaching_the_last_lane_is_refused() {
        let client = test_client();
        let mut prefs = [test_prefs()];
        let mut app = test_shell(&client, &mut prefs);

        app.detach_lane("lane-1");

        assert_eq!(app.lanes.len(), 1, "唯一连接不许摘");
        let last = app.lanes[0].st.lines.last().expect("应有提示").text();
        assert!(last.contains("唯一的连接"), "{last}");
    }

    /// 名字不存在：列出既有名字，省得用户再猜一次。
    #[test]
    fn detaching_an_unknown_name_lists_the_known_ones() {
        let client = test_client();
        let mut prefs = [test_prefs(), test_prefs()];
        let mut app = test_shell(&client, &mut prefs);

        app.detach_lane("lane-x");

        assert_eq!(app.lanes.len(), 2, "不认识的名字不动任何 lane");
        let last = app.lanes[0].st.lines.last().expect("应有提示").text();
        assert!(last.contains("lane-1 / lane-2"), "{last}");
    }

    /// **一条 lane 自己的回包只影响它自己**（多 lane 的事件归属，P0）。
    ///
    /// 多 lane 共用**一条**事件通道，而外层 `TuiApp::on_msg` 对不带标签的事件一律交给
    /// **当前 lane**（那是键盘 / 鼠标这类全局输入的落点）。所以每条 lane 的出站事件必须
    /// 自带来源：否则 lane-2 的回合结束会清掉 lane-1 的 `running`（结果是**真正在跑的**
    /// 那条 lane 永远停在「运行中」，`on_submit` 只会进排队，再也发不出下一轮），
    /// 会话列表 / 工作区路径 / 审批浮层同理互相覆盖。
    ///
    /// 生产侧由 [`LaneTx`] 从类型上强制（拿不到裸通道就发不出裸事件），这里验证
    /// 入口 `TuiApp::on_msg` 按标签分发 —— 事件经**真实出口**发出，而不是手写
    /// `UiEvent::Lane` 猜一个形状。
    #[test]
    fn a_lanes_own_event_only_touches_that_lane() {
        let client = test_client();
        let mut prefs = [test_prefs(), test_prefs()];
        let mut app = test_shell(&client, &mut prefs);
        app.lanes[0].st.running = true;
        app.lanes[1].st.running = true;

        // lane-2 的 worker 报回合结束：走的是它自己的出口。
        let out = LaneTx::new(app.lanes[1].id, app.lanes[1].tx.clone());
        let _ = out.send(UiEvent::TurnDone {
            outcome: Default::default(),
            error: None,
            interrupted: None,
        });
        // 带超时：接线若再退回「发到没人读的通道」，这里 5 秒内就该失败，
        // 而不是让整个 `make test` 挂在一次阻塞的 `recv()` 上。
        let ev = app
            .rx
            .recv_timeout(Duration::from_secs(5))
            .expect("外壳通道里应有一条事件");
        app.on_msg(ev);

        assert!(app.lanes[0].st.running, "lane-1 的回合还在跑，不能被动结束");
        assert!(
            !app.lanes[1].st.running,
            "回合结束只清自己那条 lane 的 running"
        );
    }

    /// 审批请求的最小样本。
    fn approval_req(command: &str) -> crate::serve::CommandApprovalData {
        crate::serve::CommandApprovalData {
            command: command.to_string(),
            args: String::new(),
            allowlist_key: None,
            permission_level: None,
        }
    }

    /// 左栏快照要把「在跑」与「等审批」分开：后台 lane 的浮层画不出来，
    /// 状态列是唯一线索（issue #337）。
    #[test]
    fn lane_entries_carry_the_render_status() {
        let client = test_client();
        let mut prefs = [test_prefs(), test_prefs(), test_prefs(), test_prefs()];
        let mut app = test_shell(&client, &mut prefs);
        app.lanes[0].st.running = true;
        app.lanes[1].st.running = true;
        let (answer, _answer_rx) = mpsc::channel();
        app.lanes[1].st.begin_approval(&approval_req("rm"), answer);
        // lane-4 的 `/quit` 还在等它这一轮结束：回合仍在跑，但状态必须已经显示「要退出」。
        app.lanes[3].st.running = true;
        app.lanes[3].quit_pending = true;

        let entries = app.lane_entries();
        assert_eq!(entries[0].status, LaneStatus::Running);
        assert_eq!(
            entries[1].status,
            LaneStatus::WaitingApproval,
            "等审批优先于运行中：用户要知道它在等人"
        );
        assert_eq!(entries[2].status, LaneStatus::Idle);
        assert_eq!(
            entries[3].status,
            LaneStatus::Quitting,
            "退出请求压过「运行中」：否则退出前的等待窗口在别的 lane 上完全不可见"
        );
    }

    /// 后台 lane 弹审批：当前 lane 留提示，但浮层仍建在**它自己**那条 lane 上。
    #[test]
    fn background_approval_is_announced_but_stays_on_its_own_lane() {
        let client = test_client();
        let mut prefs = [test_prefs(), test_prefs()];
        let mut app = test_shell(&client, &mut prefs);
        let (answer, _answer_rx) = mpsc::channel();

        let out = LaneTx::new(app.lanes[1].id, app.lanes[1].tx.clone());
        let _ = out.send(UiEvent::Approval {
            prompt: ApprovalPrompt {
                req: approval_req("rm"),
                answer,
            },
        });
        let ev = app.rx.recv_timeout(Duration::from_secs(5)).expect("事件");
        app.on_msg(ev);

        assert!(
            app.lanes[1].st.approval.is_some(),
            "浮层属于它自己那条 lane"
        );
        assert!(app.lanes[0].st.approval.is_none(), "不能串到当前 lane");
        let last = app.lanes[0].st.lines.last().expect("应有提示").text();
        assert!(last.contains("等待审批"), "{last}");
    }

    /// `/lane cancel`：撤浮层（`drop(answer)` 让 gate 立刻按拒绝收尾）+ 标记「取消」。
    #[test]
    fn cancelling_a_background_lane_clears_its_overlay_and_marks_the_cancel() {
        let client = test_client();
        let mut prefs = [test_prefs(), test_prefs()];
        let mut app = test_shell(&client, &mut prefs);
        let (answer, answer_rx) = mpsc::channel();
        app.lanes[1].st.running = true;
        app.lanes[1].st.begin_approval(&approval_req("rm"), answer);

        app.cancel_lane("lane-2");

        assert!(app.lanes[1].st.approval.is_none(), "浮层要撤掉");
        assert!(
            answer_rx.recv().is_err(),
            "answer 必须被 drop：gate 靠 Disconnected 立刻收尾，不必等 600s"
        );
        assert!(
            app.lanes[1].st.cancel_sent,
            "要标成「取消」，否则排队消息会被当成正常完成自动发出去"
        );
        assert!(
            app.lanes[1].st.running,
            "running 留到 TurnDone 再清（与 Ctrl+C 同口径）"
        );
    }

    /// Ctrl+C 卡在审批浮层上：浮层必须撤掉，gate 立刻按拒绝收尾（issue #346）。
    ///
    /// 少这一步，SSE 消费仍阻塞在 gate 的 `recv` 上 —— 取消计数 +1 也白搭，要等满
    /// `APPROVAL_WAIT`（600 s）才收尾，而屏幕上早已写着「正在取消回合…」。
    #[test]
    fn ctrl_c_with_an_approval_overlay_drops_the_answer() {
        let client = test_client();
        let mut prefs = [test_prefs()];
        let mut app = test_shell(&client, &mut prefs);
        let (answer, answer_rx) = mpsc::channel();
        app.lanes[0].st.running = true;
        app.lanes[0].st.begin_approval(&approval_req("rm"), answer);
        let cancel = StreamCancel::new();
        app.lanes[0].cancel = Some(cancel.clone());

        app.lanes[0].on_ctrl_c();

        assert!(app.lanes[0].st.approval.is_none(), "浮层要撤掉");
        assert!(
            answer_rx.recv().is_err(),
            "answer 必须被 drop：gate 靠 Disconnected 立刻收尾，不必等 600s"
        );
        assert_eq!(cancel.requests(), 1, "取消只请求一次（2 才会被当成强退）");
        assert!(
            app.lanes[0].st.cancel_sent,
            "要标成「取消」，排队消息才会退回输入框"
        );
        assert!(!app.lanes[0].exit, "第一次 Ctrl+C 不是退出");
    }

    /// 第一次 Ctrl+C 取消、第二次强退 —— 收尾收敛后这条既有语义不能被带偏。
    #[test]
    fn second_ctrl_c_while_cancelling_exits() {
        let client = test_client();
        let mut prefs = [test_prefs()];
        let mut app = test_shell(&client, &mut prefs);
        app.lanes[0].st.running = true;
        let cancel = StreamCancel::new();
        app.lanes[0].cancel = Some(cancel.clone());

        app.lanes[0].on_ctrl_c();
        assert_eq!(cancel.requests(), 1, "第一次只请求取消");
        assert!(!app.lanes[0].exit);

        app.lanes[0].on_ctrl_c();
        assert!(app.lanes[0].exit, "第二次直接强退");
        assert_eq!(
            cancel.requests(),
            1,
            "强退不该再追加一次取消（那会被误当成 `>= 2` 的强退信号）"
        );
    }

    /// 空闲 lane 上 Ctrl+C 就是退出：没有回合可取消时不该留下任何取消痕迹。
    #[test]
    fn ctrl_c_on_an_idle_lane_exits_immediately() {
        let client = test_client();
        let mut prefs = [test_prefs()];
        let mut app = test_shell(&client, &mut prefs);

        app.lanes[0].on_ctrl_c();

        assert!(app.lanes[0].exit, "空闲时 Ctrl+C 就是退出");
        assert!(
            !app.lanes[0].st.cancel_sent,
            "没有回合可取消，不该留下取消标记（否则排队消息会被误当成被取消）"
        );
    }

    /// `/quit` 卡在审批浮层上：同样必须撤浮层，否则退出要白等 600 s（issue #346）。
    #[test]
    fn quitting_while_waiting_for_approval_drops_the_answer() {
        let client = test_client();
        let mut prefs = [test_prefs()];
        let mut app = test_shell(&client, &mut prefs);
        let (answer, answer_rx) = mpsc::channel();
        app.lanes[0].st.running = true;
        app.lanes[0].st.begin_approval(&approval_req("rm"), answer);

        app.lanes[0].apply_control(Control::Quit);

        assert!(app.lanes[0].st.approval.is_none(), "浮层要撤掉");
        assert!(
            answer_rx.recv().is_err(),
            "answer 必须被 drop：gate 靠 Disconnected 立刻收尾"
        );
        assert!(app.lanes[0].quit_pending, "取消完再退，不是立刻退");
        assert!(!app.lanes[0].exit, "还在取消中，本轮还没走");
    }

    /// 目标 lane 已摘除时，`FromLane` 事件必须被丢弃，**不能**落到当前 lane（issue #347）。
    ///
    /// 这正是 `Inbound` 存在的理由：改动前漏一个 `Lane` 标签，事件就静默落到当前聚焦的
    /// lane 上。类型层面已经发不出「没标签的 lane 事件」，分发端还得保证「找不到就丢」。
    #[test]
    fn from_lane_event_without_a_target_lane_is_dropped() {
        let client = test_client();
        let mut prefs = [test_prefs()];
        let mut app = test_shell(&client, &mut prefs);
        // 一个**从未装配**的 LaneId：等价于那条 lane 已被 `/lane detach` 摘掉。
        let gone = LaneId(app.next_lane_id);
        app.lanes[0].st.running = true;

        app.on_msg(Inbound::FromLane(
            gone,
            UiEvent::TurnDone {
                outcome: Default::default(),
                error: Some("某个已摘除 lane 的回合结束".to_string()),
                interrupted: None,
            },
        ));

        assert!(
            app.lanes[0].st.running,
            "没有归属的回合结束不能清掉当前 lane 的 running（那正是串台）"
        );
    }

    /// `Inbound::Shell` 必须真的走到外壳：`ShellEvent::Detach` 摘掉一条 lane（issue #347）。
    ///
    /// 覆盖的是**分发层**：`lane_cmd` 只负责把事件发出去，`on_shell` 才是唯一能改
    /// `lanes` 的地方。旧代码由 `on_msg` 的内联 match 直调，这一层一直没有测试。
    #[test]
    fn shell_event_detach_reaches_the_shell() {
        let client = test_client();
        let mut prefs = [test_prefs(), test_prefs()];
        let mut app = test_shell(&client, &mut prefs);
        assert_eq!(app.lanes.len(), 2, "前提：两条 lane");

        app.on_msg(Inbound::Shell(ShellEvent::Detach {
            name: "lane-2".to_string(),
        }));

        assert_eq!(app.lanes.len(), 1, "摘连接是外壳动作，必须真生效");
        assert_eq!(
            app.lanes[0].label, "lane-1",
            "摘的必须是指名的那条，而不是当前 lane"
        );
    }

    /// `/lane detach` 摘掉一条**正在跑**的 lane：必须先停掉它那一轮（issue #339）。
    ///
    /// 只 drop `LaneApp` 不够 —— worker 在 `run_turn_job` 的 `block_on` 里不看 job
    /// channel，会等整个 SSE 回合跑完才回到 `recv`：serve 侧 job 白跑一轮、事件又全被
    /// 丢弃，线程与 runtime 也白占着。
    #[test]
    fn detaching_a_running_lane_requests_cancel() {
        let client = test_client();
        let mut prefs = [test_prefs(), test_prefs()];
        let mut app = test_shell(&client, &mut prefs);
        app.lanes[1].st.running = true;
        let cancel = StreamCancel::new();
        app.lanes[1].cancel = Some(cancel.clone());

        app.detach_lane("lane-2");

        assert_eq!(
            cancel.requests(),
            1,
            "摘掉前要请求取消（1 = 取消当前回合，2 才会被当成强退）"
        );
        assert_eq!(app.lanes.len(), 1, "lane 本身照旧摘掉");
        let last = app.lanes[0].st.lines.last().expect("应有提示").text();
        assert!(
            last.contains("lane-2") && last.contains("已请求取消"),
            "{last}"
        );
        assert!(
            last.contains("仍会跑完"),
            "取消是否送达只有 worker 知道，措辞不能承诺 serve 侧一定停了：{last}"
        );
    }

    /// 摘一条卡在审批上的 lane：`answer` 必须被 drop，否则 gate 要干等到 600 s。
    #[test]
    fn detaching_a_lane_waiting_for_approval_drops_the_answer() {
        let client = test_client();
        let mut prefs = [test_prefs(), test_prefs()];
        let mut app = test_shell(&client, &mut prefs);
        let (answer, answer_rx) = mpsc::channel();
        app.lanes[1].st.running = true;
        app.lanes[1].st.begin_approval(&approval_req("rm"), answer);

        app.detach_lane("lane-2");

        assert!(
            answer_rx.recv().is_err(),
            "answer 必须被 drop：gate 靠 Disconnected 立刻收尾，不必等 600s"
        );
        let last = app.lanes[0].st.lines.last().expect("应有提示").text();
        assert!(last.contains("审批浮层已按拒绝撤销"), "{last}");
    }

    /// 空闲 lane 被摘：不要凭空说「已请求取消」，否则用户会以为白白杀掉了一轮。
    #[test]
    fn detaching_an_idle_lane_says_nothing_about_cancel() {
        let client = test_client();
        let mut prefs = [test_prefs(), test_prefs()];
        let mut app = test_shell(&client, &mut prefs);

        app.detach_lane("lane-2");

        let last = app.lanes[0].st.lines.last().expect("应有提示").text();
        assert!(last.contains("已断开 `lane-2`"), "{last}");
        assert!(!last.contains("取消"), "空闲 lane 没有回合可取消：{last}");
    }

    /// 远程取消之后，排队消息要退回输入框 —— 不能因为「回合结束了」就替用户发出去。
    #[test]
    fn a_remotely_cancelled_turn_returns_the_queued_message_to_the_composer() {
        let client = test_client();
        let mut prefs = [test_prefs(), test_prefs()];
        let mut app = test_shell(&client, &mut prefs);
        app.lanes[1].st.running = true;
        app.lanes[1].st.queue_message("排队的那句");
        app.cancel_lane("lane-2");

        let out = LaneTx::new(app.lanes[1].id, app.lanes[1].tx.clone());
        let _ = out.send(UiEvent::TurnDone {
            outcome: Default::default(),
            error: None,
            interrupted: None,
        });
        let ev = app.rx.recv_timeout(Duration::from_secs(5)).expect("事件");
        app.on_msg(ev);

        assert_eq!(app.lanes[1].st.input_snapshot(), "排队的那句");
        assert!(app.lanes[1].st.queued.is_none());
    }

    /// 没有在跑回合的 lane：说清楚「没得取消」，不要造一个假的取消。
    #[test]
    fn cancelling_an_idle_lane_says_so() {
        let client = test_client();
        let mut prefs = [test_prefs(), test_prefs()];
        let mut app = test_shell(&client, &mut prefs);

        app.cancel_lane("lane-2");

        assert!(!app.lanes[1].st.cancel_sent);
        let last = app.lanes[1].st.lines.last().expect("应有提示").text();
        assert!(last.contains("没有在跑的回合"), "{last}");
    }

    /// 名字打错：列出既有连接，别让人再猜一次。
    #[test]
    fn cancelling_an_unknown_lane_lists_the_known_ones() {
        let client = test_client();
        let mut prefs = [test_prefs(), test_prefs()];
        let mut app = test_shell(&client, &mut prefs);

        app.cancel_lane("lane-9");

        let last = app.lanes[0].st.lines.last().expect("应有提示").text();
        assert!(last.contains("lane-9"), "{last}");
        assert!(last.contains("lane-1 / lane-2"), "{last}");
    }
}
