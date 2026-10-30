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

use super::LaneApp;
use super::spinner::Spinner;
use super::state::UiState;
use super::worker::{LaneId, UiEvent, WorkerJob};

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

/// 不连网的 `ServeClient`。
///
/// 端口 1 必然连不上：接线测试只关心「状态怎么变」，任何真发出去的请求都应该失败得
/// 明明白白，而不是悄悄打到某个本地 serve 上、让断言依赖外部环境。
pub(super) fn test_client() -> ServeClient {
    ServeClient::new(ConnectionConfig::new("http://127.0.0.1:1", "")).expect("client")
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
    rx: mpsc::Receiver<UiEvent>,
    tx: mpsc::Sender<UiEvent>,
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
    tx: mpsc::Sender<UiEvent>,
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
        // 测试默认不启用控制面：`/ctl` 会走「未启用」分支，除非用例自己设路径。
        ctl_socket: None,
    }
}

/// 单 lane 的常规测试入口（固定 `LaneId(0)`）。
pub(super) fn test_app<'a>(
    client: &'a ServeClient,
    prefs: &'a mut SessionPrefs,
    rx: mpsc::Receiver<UiEvent>,
    tx: mpsc::Sender<UiEvent>,
    job_tx: mpsc::Sender<WorkerJob>,
) -> LaneApp<'a> {
    test_lane(LaneId(0), client, prefs, rx, tx, job_tx)
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::super::TuiApp;
    use super::super::worker::{LaneId, LaneTx, UiEvent};
    use super::{lane_with_tx, test_app, test_client, test_prefs};

    /// 造 n 条 lane 的外壳：所有 lane 共用外壳这条通道（生产同构）。
    fn shell<'a>(
        client: &'a crate::serve::ServeClient,
        prefs: &'a mut [crate::SessionPrefs],
    ) -> TuiApp<'a> {
        let (shell_tx, rx) = mpsc::channel();
        let mut lanes = Vec::new();
        // 条数在借出去之前记下：`prefs` 会被逐条 `iter_mut` 借到 `'a`，循环后再
        // 读 `len()` 会与那份借用冲突。
        let count = prefs.len();
        for (i, p) in prefs.iter_mut().enumerate() {
            let (job_tx, _job_rx) = mpsc::channel();
            // 每条 lane 都用**外壳那条**通道的发送端：生产里 `tx` 就是这个（一个 `rx`），
            // 测试若给每条 lane 自成一条通道，`app.rx` 就永远收不到事件。
            lanes.push(lane_with_tx(
                LaneId(i as u32 + 1),
                client,
                p,
                shell_tx.clone(),
                job_tx,
            ));
        }
        let next_lane_id = count as u32 + 1;
        TuiApp {
            lanes,
            active: 0,
            rx,
            next_lane_id,
            template: super::super::LaneTemplate {
                overrides: test_prefs(),
                yes: true,
                no_keyring: true,
                bearer: String::new(),
                ctl_socket: None,
                tx: shell_tx,
            },
        }
    }

    #[test]
    fn switch_lane_is_a_noop_with_a_single_lane() {
        let client = test_client();
        let mut prefs = [test_prefs()];
        let mut app = shell(&client, &mut prefs);

        app.switch_lane(1);
        assert_eq!(app.active, 0, "单 lane 时切换必须无副作用");
        app.switch_lane(-1);
        assert_eq!(app.active, 0);
    }

    #[test]
    fn switch_lane_cycles_in_both_directions() {
        let client = test_client();
        let mut prefs = [test_prefs(), test_prefs(), test_prefs()];
        let mut app = shell(&client, &mut prefs);
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

    #[test]
    fn switching_does_not_move_state_between_lanes() {
        let client = test_client();
        let mut prefs = [test_prefs(), test_prefs()];
        let mut app = shell(&client, &mut prefs);

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
        let mut app = shell(&client, &mut prefs);
        let before = app.next_lane_id;

        app.attach_lane("proj-b", "http://127.0.0.1:8721/");

        assert_eq!(app.lanes.len(), 2, "应新增一条 lane");
        assert_eq!(app.active, 1, "新增后切到它");
        assert_eq!(app.lane().label, "proj-b");
        assert_eq!(app.next_lane_id, before + 1, "编号单调递增");
        assert_ne!(app.lanes[0].id, app.lanes[1].id, "LaneId 不能重复");
    }

    /// 同名连接已存在时只切过去：重复连会让左栏出现两条同名 lane。
    #[test]
    fn attaching_an_existing_name_only_switches() {
        let client = test_client();
        let mut prefs = [test_prefs()];
        let mut app = shell(&client, &mut prefs);
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
        let mut app = shell(&client, &mut prefs);
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
        let mut app = shell(&client, &mut prefs);

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
        let mut app = shell(&client, &mut prefs);
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
        let mut app = shell(&client, &mut prefs);

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
        let mut app = shell(&client, &mut prefs);

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
        let mut app = shell(&client, &mut prefs);
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
}
