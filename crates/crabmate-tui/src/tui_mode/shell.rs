//! 全屏 TUI 的终端外壳：raw mode / 备用屏幕 / 鼠标与括号粘贴捕获的 RAII 复原，
//! 以及入口 [`run_tui`]（装配 lane 与事件通道）。
//!
//! 从 `mod.rs` 拆出：那边已顶在 `fn-nloc-ratchet` 的 920 行/文件上限，而这块只负责
//! 终端与接线，与「会话状态」无关，拆开后两边都留出余量。

use std::borrow::Cow;
use std::io;
use std::sync::mpsc;

use anyhow::{Context, Result};
use crossterm::cursor::SetCursorStyle;
use crossterm::event::DisableBracketedPaste;
use crossterm::execute;
use crossterm::style::Print;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crabmate_client_api::ctl::{ToggleFailure, ToggleOutcome};

use crate::SessionPrefs;
use crate::serve::{ConnectionConfig, ServeClient};

use super::inbound::Inbound;
use super::state::{LineKind, UiState};
use super::worker::{JobCancel, LaneId, UiEvent, WorkerJob, spawn_key_reader, spawn_worker};
use super::{LaneApp, TuiApp, body_cache, drafts, event_loop, session_index, worker_ctl};

/// 鼠标上报：只要按下/释放（1000）、按住时的拖动（1002）与 SGR 编码（1006）。
///
/// 1002 目前没有消费者 —— `worker::is_actionable_mouse` 在读取线程就丢弃 `Drag`，
/// 保留它只是与 crossterm 的捕获集合保持一致（少开的是 any-event 1003，不是这一个）。
///
/// 刻意不用 crossterm 的 `EnableMouseCapture`：它会连带打开 any-event（1003），
/// 而「再把 1003 关掉」并不保证只关这一项 —— kitty 把 mouse tracking 存成**单值**，
/// `?1003l` 会把刚开的 1000/1002 一起清掉，整个鼠标就此静默（实测：kitty 里左栏、
/// 右栏与滚轮全部无响应，同一序列在 VTE 正常）。自己发最小集合后语义在所有终端一致；
/// 又因为从不开 any-event，本来就不会收到 `Moved`，退出时也只需对称关闭。
pub(super) const MOUSE_ON: &str = "\x1b[?1000h\x1b[?1002h\x1b[?1006h";
/// 与 [`MOUSE_ON`] 对称的关闭序列（`ScreenGuard` 的复原路径）。
const MOUSE_OFF: &str = "\x1b[?1000l\x1b[?1002l\x1b[?1006l";

/// raw mode + 备用屏幕 + 鼠标/括号粘贴捕获的 RAII 复原。
struct ScreenGuard {
    /// 是否启用过鼠标捕获（未启用时不补发 [`MOUSE_OFF`]，避免向终端写多余序列）。
    mouse: bool,
    /// 是否启用过括号粘贴（同 `mouse`：没开过就别补发关闭序列）。
    paste: bool,
}

impl Drop for ScreenGuard {
    fn drop(&mut self) {
        if self.mouse {
            let _ = execute!(io::stdout(), Print(MOUSE_OFF));
        }
        if self.paste {
            let _ = execute!(io::stdout(), DisableBracketedPaste);
        }
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

/// 全屏 TUI 入口（M2）：状态行 + 左栏会话 + 流式 transcript + 单行输入。
///
/// `lanes_spec` 为 `(名字, 客户端)` 列表，**第一条是初始激活的 lane**。每条 lane 各自
/// 一个 worker 与一份 `overrides`；共用同一个事件通道，按 `LaneId` 分发
/// （见 `docs/design/tui_multi_server.md` §4.2）。
pub async fn run_tui(
    lanes_spec: &[(String, &ServeClient)],
    overrides: &SessionPrefs,
    yes: bool,
    no_keyring: bool,
    ctl_socket: Option<&std::path::Path>,
) -> Result<()> {
    announce_ctl_socket(ctl_socket);
    enable_raw_mode().context("enable raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).context("enter alternate screen")?;
    // 鼠标捕获用于右栏工作区树的点击/滚轮。它会接管终端原生拖选复制
    // （多数终端 Shift+拖选仍可用）；`CM_TUI_MOUSE=off` 关闭。
    // 括号粘贴决定「粘贴是一个事件还是一串按键」（见 `event_loop`）；`CM_TUI_PASTE=off` 关闭。
    let mouse = event_loop::mouse_capture_enabled();
    let paste = event_loop::paste_capture_enabled();
    event_loop::enable_captures(&mut stdout, mouse, paste)?;
    // 输入行只显示文本，用闪烁竖条光标提示输入位置。
    execute!(stdout, SetCursorStyle::BlinkingBar).context("set cursor style")?;
    let guard = ScreenGuard { mouse, paste };
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout)).context("init terminal")?;
    // 必须在启动键盘读取线程前清屏：`clear()` 会查询光标位置（DSR），
    // 需要 crossterm 的全局事件锁；而键盘线程已阻塞在 `event::read()` 持锁，
    // 会导致清屏（及首帧绘制）一直等到用户第一次按键才完成。
    terminal.clear().context("clear screen")?;

    let (tx, rx) = mpsc::channel::<Inbound>();
    spawn_key_reader(tx.clone());

    // 控制面调用在外壳级执行（issue #350）：一个线程、一份 socket、天然串行。
    // 每条 lane 只拿到发送端，结果按发起它的 `LaneId` 回到那条 lane 的 transcript。
    let ctl = ctl_socket.map(|path| worker_ctl::spawn(path.to_path_buf(), tx.clone()));

    // 运行期新增 lane（`/lane attach|new`）的装配模板。`bearer` 取主连接那份：
    // 控制面里所有实例同机同用户，per-profile 槽会牵动 Desktop / Android 的槽名约定。
    let template = LaneTemplate {
        overrides: overrides.clone(),
        yes,
        no_keyring,
        bearer: lanes_spec
            .first()
            .map(|(_, c)| c.config().bearer_token.clone())
            .unwrap_or_default(),
        ctl,
        tx: tx.clone(),
    };

    // 逐条装配：每条 lane 一个 worker（各自的 SSE 流），共用这一个通道。
    let mut lanes = Vec::with_capacity(lanes_spec.len());
    for (i, (label, client)) in lanes_spec.iter().enumerate() {
        let lane_id = LaneId(i as u32);
        let (job_tx, job_cancel) = spawn_worker(lane_id, (*client).clone(), tx.clone());
        lanes.push(LaneApp::assemble(
            lane_id,
            label.clone(),
            Cow::Borrowed(*client),
            &template,
            job_tx,
            job_cancel,
        ));
    }
    let next_lane_id = lanes_spec.len() as u32;
    let mut app = TuiApp {
        lanes,
        active: 0,
        rx,
        template,
        next_lane_id,
        session_refresh: Default::default(),
    };
    // 每条 lane 都拉一次会话 / 工作区根 / serve 默认状态 / user-data 设置快照：
    // 只刷当前 lane 的话，切过去会看到空的会话列表（切 lane 不该触发网络请求）。
    for lane in app.lanes.iter_mut() {
        lane.bootstrap();
    }
    let res = app.run_loop(&mut terminal);
    // 退出收尾必须遍历**全部** lane（issue #338）：草稿是内存态、kitty 背景图配额是
    // 终端级的，只处理当前 lane 会让后台 lane 未发送的输入直接丢失、图像配额留给下一次运行。
    let warns = app.flush_all_before_exit().await;
    drop(guard);
    for warn in warns {
        eprintln!("{warn}");
    }
    res
}

/// 控制面启用时提示一次（`/ctl` 可查实例）；未启用则完全沉默。
///
/// 沉默是刻意：默认路径没有 socket 是**正常状态**（大多数人不用 ctl），每次启动都报
/// 「控制面连不上」只会变成噪音。单独成函数是为了让 `run_tui` 少一个分支
/// （`lizard` 对 CCN 是零容忍的 >10）。
fn announce_ctl_socket(ctl_socket: Option<&std::path::Path>) {
    let Some(path) = ctl_socket else {
        return;
    };
    eprintln!(
        "[crabmate-tui] ctl 控制面：{}（/ctl 查看实例）",
        path.display()
    );
}

/// 本进程最多同时维持多少条连接（lane）。
///
/// 只防手滑：`--connect` 可重复任意次、`/lane attach` 可反复敲，而每条 lane 的代价是
/// 一个 worker 线程 + 一个 `current_thread` runtime + 一份连接池，serve 那侧还各有自己的
/// 并发上限（`chat_queue_max_concurrent` 默认 2）。它**不是**性能目标 —— 真要 8 路并行
/// 得配 8 个 serve，见 `docs/design/tui_multi_server.md` §5.2。
pub(super) const MAX_LANES: usize = 8;

/// 还能不能再加一条 lane：超限时返回**可照抄**的提示。
///
/// 抽成纯函数只为可测：真攒 8 条 lane 要起 8 条线程与 8 份 runtime，单测里不值得。
fn capacity_notice(current: usize) -> Option<String> {
    (current >= MAX_LANES).then(|| {
        format!(
            "已达连接上限 {MAX_LANES} 条（现有 {current} 条）：先 `/lane detach <name>` 摘掉不用的再用 `/lane attach` / `/lane new`"
        )
    })
}

/// 启动装配前的连接数校验（`--connect` 可重复任意次）。
///
/// 这里**报错退出**而不是只连前 N 条：那样用户会对着另一台 serve 干活，屏幕上却没有任何
/// 迹象说明少了人。运行期的同一道闸在 `attach_lane`（那里只提示、不退出进程 —— 一条
/// 已经跑起来的 TUI 不该因为多敲一次 `/lane new` 就整个消失）。
///
/// 放 `shell` 而不是 `main`：`main.rs` 顶着 `fn-nloc-ratchet` 的 920 行/文件上限。
pub(crate) fn check_lane_capacity(total: usize) -> Result<()> {
    if total <= MAX_LANES {
        return Ok(());
    }
    anyhow::bail!(
        "连接数 {total} 超过上限 {MAX_LANES}（含主连接）：减少 `--connect`，或起来后用 `/lane detach <name>` 摘掉不用的"
    )
}

/// `disable` / `enable` 的回执 → 写给用户的行（纯函数，措辞钉在测试里）。
///
/// 三种情形必须分开说，因为下一步动作完全不同：真改了、本来就是这样（幂等命中），
/// 以及 `enable` 的「写成了但没清掉 `down` 标记」—— 最后一种只靠 Server 的 `warning`
/// 才能发现，不露出来就会变成「声明是启用的，实例却永远起不来」。
fn toggle_report_lines(name: &str, enabled: bool, out: &ToggleOutcome) -> Vec<String> {
    let mut lines = Vec::with_capacity(3);
    if out.changed {
        if enabled {
            lines.push(format!(
                "已启用声明 `{name}`（enabled=true）：ctl 下一轮收敛会拉起它，就绪后 `/lane attach {name}`"
            ));
        } else {
            lines.push(format!(
                "已关闭声明 `{name}`（enabled=false）：ctl 会停掉该实例，声明与手写配置都留着；恢复用 `/lane open {name}`"
            ));
        }
    } else {
        let state = if enabled { "已启用" } else { "已是关闭" };
        lines.push(format!("`{name}` 声明{state}（本次未改动，幂等命中）"));
    }
    if out.cleared_down {
        lines.push("顺带清掉了 `down` 标记（此前它会让收敛一直跳过该实例）".to_string());
    }
    if let Some(warn) = out.warning.as_deref() {
        lines.push(format!("注意：{warn}"));
    }
    if !out.changed
        && let Some(note) = out.note.as_deref()
    {
        lines.push(format!("Server：{note}"));
    }
    lines
}

/// 从 `active` 的**下一条**起环形找第一条在等审批的 lane 下标。
///
/// `waiting` 是各 lane「此刻有没有审批浮层」的派生快照 —— 审批槽本身住在那条 lane 的
/// `UiState` 里，这里只做下标选择（纯函数好测：真实 `TuiApp` 要起 N 条线程）。
///
/// 只有「唯一在等的就是它自己」时才会返回它自己（环形走满一圈）；否则「下一条」优先，
/// 免得这个热键停在原地、看起来像没生效。
fn next_approval_index(active: usize, waiting: &[bool]) -> Option<usize> {
    let n = waiting.len();
    if n == 0 {
        return None;
    }
    (1..=n)
        .map(|step| (active + step) % n)
        .find(|&i| waiting[i])
}

/// 运行期新增 lane 的装配模板（`/lane attach` / `/lane new`）。
///
/// 存**启动时的**表达层初值，而不是「当前 lane 的现值」：当前 lane 的 `/model`
/// 覆盖只属于它，复制给新 lane 等于把 A 的临时选择静默带进 B。
///
/// 定义在这里而不是 `mod.rs`：那边贴着 920 行门禁，而它只被外壳（装配 + 新增 lane）
/// 用到。
pub(super) struct LaneTemplate {
    /// 启动时的 `/model` `/mode` `/role` `/perm` 初值（CLI / env）。
    pub(super) overrides: SessionPrefs,
    /// `--yes`：审批自动放行。
    pub(super) yes: bool,
    /// `--no-keyring`：设置面板禁写密钥。
    pub(super) no_keyring: bool,
    /// Web API Bearer：控制面里所有实例同机同用户，第一版共用主连接那份
    /// （per-profile 槽需要牵动 Desktop / Android 的槽名约定，见设计稿 §4.5）。
    pub(super) bearer: String,
    /// 外壳级控制面通道（各 lane 相同：一个 socket 管所有实例）。
    /// 存通道句柄而不是 socket：socket 只由 `worker_ctl` 的线程持有，调用也在那里串行
    /// 发生（issue #350）。`None` = 没启用控制面（`/ctl` 会说明怎么启用）。
    pub(super) ctl: Option<worker_ctl::CtlHandle>,
    /// 所有 lane 共用的事件通道发送端。
    pub(super) tx: mpsc::Sender<Inbound>,
}

/// lane 的装配与首拉：启动装配与 `/lane attach|new` 共用。
///
/// 字段清单只此一份：两处各写一遍的话，加字段时必有一边漏掉，而漏的那条 lane
/// 会静默缺一项行为（例如没有 `ctl` 就连 `/lane` 都用不了）。
impl<'a> LaneApp<'a> {
    /// 按模板装配一条 lane（worker 由调用方先 spawn）。
    fn assemble(
        id: LaneId,
        label: String,
        client: Cow<'a, ServeClient>,
        template: &LaneTemplate,
        job_tx: mpsc::Sender<WorkerJob>,
        job_cancel: JobCancel,
    ) -> Self {
        Self {
            id,
            label,
            client,
            overrides: template.overrides.clone(),
            yes: template.yes,
            no_keyring: template.no_keyring,
            st: UiState::new(),
            drafts: drafts::Drafts::default(),
            tx: template.tx.clone(),
            job_tx,
            job_cancel,
            cancel: None,
            exit: false,
            quit_pending: false,
            body: body_cache::BodyCache::default(),
            panel: None,
            persisted: None,
            last_size: (0, 0),
            index: session_index::UiIndexState::default(),
            resume: None,
            sent_turn: None,
            bg: None,
            ctl: template.ctl.clone(),
            session_refresh_pending: false,
            unread: false,
            turn_started: None,
        }
    }

    /// 首次拉取：会话 / 工作区根 / serve 默认状态 / user-data 设置快照。
    ///
    /// 新增 lane 也要跑这一套：只刷当前 lane 的话，切过去会看到空的会话列表。
    fn bootstrap(&mut self) {
        self.refresh_sessions();
        self.st.ws_begin_root_fetch();
        self.refresh_workspace(false);
        self.refresh_status();
        self.load_user_settings();
    }
}

/// 外壳层动作：取当前 lane、切 lane、装配新 lane、导出渲染快照、鼠标命中。
///
/// **放这里而不是 `mod.rs`**：`mod.rs` 贴着 920 行门禁，而这些事都只跟「有几条
/// lane、当前是哪条」有关，与单条 lane 的行为无关 —— 正好是外壳该管的那一层。
impl<'a> super::TuiApp<'a> {
    /// 当前 lane（只读）。
    ///
    /// 放这里与 [`TuiApp::switch_lane`] 一起：`mod.rs` 贴着 920 行门禁，而这一对取值器
    /// 只跟「当前是哪条」有关。`active` 只由本模块的 `switch_lane` / `attach_lane`
    /// 与构造设置，越界即编程错误。
    pub(super) fn lane(&self) -> &LaneApp<'a> {
        &self.lanes[self.active]
    }

    /// 当前 lane（可变）。
    pub(super) fn lane_mut(&mut self) -> &mut LaneApp<'a> {
        &mut self.lanes[self.active]
    }

    /// 是否有 lane 请求退出进程。
    ///
    /// `/quit` 是**进程级**语义（所有 lane 一并退出），摘单条连接用 `/lane detach`，
    /// 所以判定必须扫全部 lane：只看 active 会让后台 lane 的 `/quit` 一直悬着 ——
    /// 它自己的回合结束时置了 `exit`，屏幕上却什么都没发生，等用户切过去才突然退出
    /// （issue #338 的症状 A）。`run_loop` / `drain_events` 都走这一个判据。
    pub(super) fn exit_requested(&self) -> bool {
        self.lanes.iter().any(|l| l.exit)
    }

    /// 在 lane 之间循环切换。
    ///
    /// 只换「看着谁」，不动任何 lane 的状态：在跑的回合、草稿、滚动位置都留在各自
    /// lane 里，切回来还是原样（与 `tmux` 切窗口同语义）。`len <= 1` 是 no-op ——
    /// 单 serve 时按 Alt+方向不报错也不留痕迹。
    pub(super) fn switch_lane(&mut self, delta: isize) {
        let n = self.lanes.len();
        if n <= 1 {
            return;
        }
        let n = n as isize;
        self.active = (self.active as isize + delta).rem_euclid(n) as usize;
    }

    /// Alt+1..8：直达第 `n` 条 lane（1-based，序号与左栏列表**自上而下**一致）。
    ///
    /// 与 [`Self::switch_lane`] 的分工是「绝对目标」对「相对移动」：这里直接写
    /// `active`，不做 `rem_euclid` 环回 —— 按 `Alt+7` 却因为只有 3 条而绕到第 1 条，
    /// 是比「什么都不发生」更容易让人迷路的行为。
    ///
    /// 越界**不切视图**，只留一句（与 `F4` 无人等待时同一个出口）：按了键却毫无反应，
    /// 用户会以为键没送达 —— 而 `Alt+数字` 恰好就是最容易被终端吞掉的那类组合键。
    /// 单 lane 时 `Alt+1` 落在自己身上，等价 no-op，不写提示。
    pub(super) fn goto_lane(&mut self, n: usize) {
        let total = self.lanes.len();
        if (1..=total).contains(&n) {
            self.active = n - 1;
            return;
        }
        self.lane_mut().st.push_line(
            LineKind::System,
            &format!("Alt+{n}：这里只有 {total} 条连接（Alt+1..{total} 直达）"),
        );
    }

    /// F4：切到下一条**在等审批**的 lane（环形，从当前的下一条找起）。
    ///
    /// 后台 lane 的浮层画不到屏幕上（`event_loop` 每帧只对 active 调 `draw_frame`），
    /// 左栏那一行的 `▲` 是唯一线索；三条以上时用 `Alt+→` 逐个试等于让用户自己找。
    ///
    /// 一条都没在等时**不切换**，只留一句：按了键却什么都没发生，比切错地方更让人懵。
    /// 单 lane（`n == 1`）时若自己就在等审批，命中的是它自己 —— 浮层本来就在眼前，
    /// 等价于 no-op，不另写提示。
    pub(super) fn focus_next_approval(&mut self) {
        let waiting: Vec<bool> = self.lanes.iter().map(|l| l.st.approval.is_some()).collect();
        match next_approval_index(self.active, &waiting) {
            Some(idx) => self.active = idx,
            None => self.lane_mut().st.push_line(
                LineKind::System,
                "当前没有在等待审批的连接（浮层只画当前连接，F4 才会跳过去）",
            ),
        }
    }

    /// 按 `LaneId` 找 lane；找不到（已被 `/lane detach` 摘掉）就退回当前 lane。
    ///
    /// 控制面回执的目标是**发起命令的那条**（issue #350：执行挪到了外壳级，归属仍按
    /// `LaneId`）。退回当前 lane 而不是丢弃：结果不该凭空消失，用户仍要能看到它。
    pub(super) fn lane_by_id_or_active(&mut self, id: LaneId) -> &mut LaneApp<'a> {
        let idx = self
            .lanes
            .iter()
            .position(|l| l.id == id)
            .unwrap_or(self.active);
        &mut self.lanes[idx]
    }

    /// `ShellEvent::Ready`：控制面回包后装配一条新 lane。
    ///
    /// 走外壳事件而不是 `UiEvent`：新 lane 还不存在，按 id 分发只会被当成「已关闭的
    /// lane」丢掉。`from` 只用于**失败**分支 —— 那条 lane 已经在了。
    pub(super) fn on_lane_ready(
        &mut self,
        from: LaneId,
        name: String,
        result: Result<String, String>,
    ) {
        match result {
            Ok(base_url) => self.attach_lane(&name, &base_url),
            // 失败写回**发起命令的那条** lane：`/lane new` 的 20 s 轮询期间用户很可能
            // 已经切走了，写进「当前」那条等于把别人的失败放到你的屏上。
            Err(why) => {
                self.lane_by_id_or_active(from)
                    .st
                    .push_line(LineKind::System, &format!("连接 `{name}` 失败：{why}"));
            }
        }
    }

    /// 装配并切到一条新 lane。
    ///
    /// 两重去重，缺一不可。**同名**只切过去：`--connect a=x --connect a=y` 这类手误若
    /// 静默再连一次，左栏会出现两条同名 lane，`/ctl` 里却只有一条实例 —— 之后所有判断
    /// 都会错。**同 URL** 也只切过去：名字不同不代表是两台 serve（启动 lane 的名字可能
    /// 来自 `--api-base` 反查，也可能是 `local`），重复连接会让同一台 serve 各开一条
    /// SSE、各读一份会话，左栏连接数还与 ctl 的进程数对不上。
    pub(super) fn attach_lane(&mut self, name: &str, base_url: &str) {
        if let Some(idx) = self.lanes.iter().position(|l| l.label == name) {
            self.active = idx;
            let lane = self.lane_mut();
            lane.st.push_line(
                LineKind::System,
                &format!("`{name}` 已经是本进程的连接（已切过去）"),
            );
            return;
        }
        if let Some(idx) = self
            .lanes
            .iter()
            .position(|l| crate::serve::same_api_base(&l.client.config().api_base, base_url))
        {
            self.active = idx;
            let existing = self.lanes[idx].label.clone();
            let lane = self.lane_mut();
            lane.st.push_line(
                LineKind::System,
                &format!("`{existing}` 已经连着 {base_url}（同一条 serve 不重复连，已切过去）"),
            );
            return;
        }
        // 名额检查刻意放在两道去重**之后**：已达上限时 `/lane attach <现有名字>`
        // 仍要能切过去 —— 那并不新增线程或连接，被上限挡住只会让人以为「连接坏了」。
        if let Some(notice) = capacity_notice(self.lanes.len()) {
            let lane = self.lane_mut();
            lane.st.push_line(LineKind::System, &notice);
            return;
        }
        let cfg = ConnectionConfig::new(base_url, self.template.bearer.clone());
        let client = match ServeClient::new(cfg) {
            Ok(c) => c,
            Err(e) => {
                let lane = self.lane_mut();
                lane.st
                    .push_line(LineKind::System, &format!("连接 `{name}` 失败：{e}"));
                return;
            }
        };
        let id = LaneId(self.next_lane_id);
        self.next_lane_id += 1;
        let (job_tx, job_cancel) = spawn_worker(id, client.clone(), self.template.tx.clone());
        let lane = LaneApp::assemble(
            id,
            name.to_string(),
            Cow::Owned(client),
            &self.template,
            job_tx,
            job_cancel,
        );
        self.lanes.push(lane);
        self.active = self.lanes.len() - 1;
        self.lane_mut().bootstrap();
        let lane = self.lane_mut();
        lane.st
            .push_line(LineKind::System, &format!("已连接 `{name}`（{base_url}）"));
    }

    /// 后台 lane 弹出审批时，在**当前 lane** 留一条提示。
    ///
    /// 浮层只画当前 lane（`event_loop` 每帧只对 active 调 `draw_frame`），后台那条的
    /// 审批在屏幕上**不可见**：不提示的话，用户看到的只是「lane-b 莫名不动了」，
    /// 而它的 worker 正阻塞在 gate 上，最长 `APPROVAL_WAIT`（600 s）才按拒绝收尾。
    pub(super) fn announce_background_approval(&mut self, from: LaneId, event: &UiEvent) {
        if !matches!(event, UiEvent::Approval { .. }) {
            return;
        }
        let Some(idx) = self.lanes.iter().position(|l| l.id == from) else {
            return;
        };
        if idx == self.active {
            return;
        }
        let label = self.lanes[idx].label.clone();
        let lane = self.lane_mut();
        lane.st.push_line(
            LineKind::System,
            &format!("`{label}` 正在等待审批：F4 跳到它，或 `/lane cancel {label}` 取消它那一轮"),
        );
    }

    /// `/lane cancel <name>`：取消指定 lane 正在跑的回合（不必先切过去）。
    ///
    /// 停下这一轮的两步收在 [`LaneApp::wind_down`]（`/lane detach`、Ctrl+C、`/quit`
    /// 共用同一入口）；这里只补一件只有「取消但保留 lane」才需要的事：**置
    /// `cancel_sent`** —— `on_turn_done` 据此把排队消息退回输入框，而不是当成正常完成
    /// 自动发出去（`drafts` 的保守边界：用户要的是「停下来」）。
    pub(super) fn cancel_lane(&mut self, name: &str) {
        let Some(idx) = self.lanes.iter().position(|l| l.label == name) else {
            let known = self.lane_labels().join(" / ");
            let lane = self.lane_mut();
            lane.st.push_line(
                LineKind::System,
                &format!("没有名为 `{name}` 的连接（现有：{known}）"),
            );
            return;
        };
        let label = self.lanes[idx].label.clone();
        let from = self.lane().label.clone();
        let stopped = self.lanes[idx].wind_down();
        let target = &mut self.lanes[idx];
        if !stopped.running {
            target
                .st
                .push_line(LineKind::System, &format!("`{label}` 当前没有在跑的回合"));
            if stopped.job {
                target
                    .st
                    .push_line(LineKind::System, "已请求中断它正在跑的拉取");
            }
            return;
        }
        // 与 Ctrl+C 同口径：`running` 留到 `TurnDone` 再清，这里只是「已请求取消」。
        target.st.cancel_sent = true;
        target.st.push_line(
            LineKind::System,
            &format!("已请求取消 `{label}` 的回合（来自 `{from}`）"),
        );
        if stopped.overlay {
            target
                .st
                .push_line(LineKind::System, "审批浮层已撤销（按拒绝处理）");
        }
    }

    /// 现有连接的展示名，顺序即左栏顺序。
    ///
    /// 报错时把这份名字列给用户：他刚打错的是名字（大小写、全角、`local` 还是实例名），
    /// 光说「没有这条」只会让人再猜一次。
    fn lane_labels(&self) -> Vec<String> {
        self.lanes.iter().map(|l| l.label.clone()).collect()
    }

    /// `ShellEvent::Detach`：摘掉本进程的一条连接（**只摘自己的账**）。
    ///
    /// 声明与 serve 进程都不动 —— `instances.toml` 归 ctl 写、进程归 ctl 管，`/lane` 的
    /// 语义是「本进程连不连」，与 `/lane attach` 对称；要真停实例请用 `crabmatectl down`
    /// 或 `/lane` 之后改声明。
    ///
    /// 两种情形明说而不是硬做：名字不存在（列出既有名字）与**最后一条**（TUI 至少要有
    /// 一条 lane 才画得出界面，退出客户端用 `/quit`）。
    pub(super) fn detach_lane(&mut self, name: &str) {
        let Some(idx) = self.lanes.iter().position(|l| l.label == name) else {
            let known = self.lane_labels().join(" / ");
            self.lane_mut().st.push_line(
                LineKind::System,
                &format!("没有叫 `{name}` 的连接（现有：{known}）"),
            );
            return;
        };
        if self.lanes.len() == 1 {
            self.lane_mut().st.push_line(
                LineKind::System,
                "这是唯一的连接，不能摘掉（要退出客户端用 `/quit`）",
            );
            return;
        }
        let mut removed = self.lanes.remove(idx);
        // 它可能正排着一次会话刷新：留着这个 id 只会让外壳每轮白等一个窗口。
        self.session_refresh.forget(removed.id);
        // 先清背景图：它为这条 lane 申请过终端图像配额，留着会算到下一个用户头上。
        removed.clear_background();
        let label = removed.label.clone();
        // 摘之前先停掉它正在跑的回合。只 drop `LaneApp` 不够：worker 在 `run_turn_job`
        // 的 `block_on` 里根本不看 job channel，要等**整个回合跑完**才回到 `recv` ——
        // 期间 serve 侧 job 白跑一轮、事件又全被丢弃，线程与 runtime 也白占着（issue #339）。
        let stopped = removed.wind_down();
        // drop 掉 `LaneApp` 即关闭它的 job channel：worker 线程的 `recv` 随即返回 `None`，
        // 线程与它那个 runtime（含在途请求、SSE 连接）自行收尾，不需要另发停止信号。
        drop(removed);
        // 指针修位：摘的是当前之前的，整体前移一格；摘的正好是当前，就停在原位置（= 原来
        // 它的下一条）；摘的是末尾那条当前，退回新的末尾。
        if idx < self.active {
            self.active -= 1;
        } else if self.active >= self.lanes.len() {
            self.active = self.lanes.len() - 1;
        }
        self.lane_mut().st.push_line(
            LineKind::System,
            &format!("已断开 `{label}`（声明与进程未动；要再连用 `/lane attach {label}`）"),
        );
        if stopped.running {
            // 说「已请求」而不是「已取消」：cancel 有没有送达只有 worker 知道
            // （`cancel_acknowledged`），而这条 lane 已经摘掉、回执没地方写。
            let overlay = if stopped.overlay {
                "，审批浮层已按拒绝撤销"
            } else {
                ""
            };
            self.lane_mut().st.push_line(
                LineKind::System,
                &format!(
                    "`{label}` 的在跑回合已请求取消{overlay}（serve 侧若未确认，该 job 可能仍会跑完）"
                ),
            );
        } else if stopped.job {
            self.lane_mut()
                .st
                .push_line(LineKind::System, &format!("`{label}` 在跑的拉取已请求中断"));
        }
    }

    /// `ShellEvent::Toggled`：写回执；关掉声明时顺手摘掉本进程那条连接。
    ///
    /// 先写回执再 detach：`detach_lane` 把「已断开…」写进**摘完之后**的当前 lane，
    /// 顺序反了用户就先看到「已断开」，再看到为什么要断。
    ///
    /// **唯一一条连接不摘**：声明一关 ctl 就会停掉那个实例、这条 lane 随后下线，但界面
    /// 至少要有一条 lane 才画得出来（`detach_lane` 同样拒绝最后一条）。所以这里只把后果
    /// 说清：退出用 `/quit`，反悔用 `/lane open`。
    pub(super) fn on_lane_toggled(
        &mut self,
        from: LaneId,
        result: Result<ToggleOutcome, ToggleFailure>,
    ) {
        let outcome = match result {
            Ok(o) => o,
            Err(f) => {
                self.lane_by_id_or_active(from).st.push_line(
                    LineKind::System,
                    &format!("`{}` 的声明未改动：{}", f.name, f.reason),
                );
                return;
            }
        };
        // Server 必回 `enabled`；缺字段（旧 ctl）按 `false` 处理 —— 报告行落在「已关闭 /
        // 已是关闭」两种措辞上，至少不会把方向说反。
        let enabled = outcome.enabled.unwrap_or(false);
        for line in toggle_report_lines(&outcome.name, enabled, &outcome) {
            self.lane_by_id_or_active(from)
                .st
                .push_line(LineKind::System, &line);
        }
        let name = outcome.name.clone();
        if enabled || !self.lanes.iter().any(|l| l.label == name) {
            return;
        }
        if self.lanes.len() > 1 {
            self.detach_lane(&name);
            return;
        }
        self.lane_by_id_or_active(from).st.push_line(
            LineKind::System,
            "这是唯一一条连接：ctl 会停掉该实例，这条 lane 随后下线；退出用 `/quit`，反悔用 `/lane open`",
        );
    }

    /// 鼠标落在左栏 lane 列表的某一行时，返回该 lane 的下标。
    ///
    /// 复用当前 lane 记下的终端尺寸与状态栈行数：一帧只有一个尺寸，各 lane 记的
    /// 是同一个值，没必要为命中检测再存一份。
    pub(super) fn lane_at_click(&self, m: &crossterm::event::MouseEvent) -> Option<usize> {
        let lane = self.lane();
        let (w, h) = lane.last_size;
        // 版面与 `draw` 共用 `layout::panes_with_lanes`：lane 区高度、可用条数、命中换算
        // 只有一份实现，两边各算一遍会点错行。
        let p = super::layout::panes_with_lanes(
            ratatui::layout::Rect::new(0, 0, w, h),
            lane.st.sidebar_visible,
            lane.status_stack_rows(),
            self.lanes.len(),
        );
        super::layout::lane_hit(&p, self.lanes.len(), m.column, m.row)
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_LANES, capacity_notice, next_approval_index, toggle_report_lines};
    use crate::tui_mode::test_support::{test_client, test_prefs, test_shell};
    use crabmate_client_api::ctl::ToggleOutcome;

    #[test]
    fn capacity_allows_up_to_the_cap_and_explains_the_way_out() {
        assert_eq!(capacity_notice(1), None, "1 条远未到上限");
        assert_eq!(capacity_notice(MAX_LANES - 1), None, "差一条时才允许再加");
        for current in [MAX_LANES, MAX_LANES + 1] {
            let notice = capacity_notice(current).expect("到上限后必须给提示");
            assert!(notice.contains(&MAX_LANES.to_string()), "{notice}");
            assert!(
                notice.contains("/lane detach"),
                "要给出可照抄的出路：{notice}"
            );
        }
    }

    /// `Alt+1..8` 是**绝对**目标，不是 `switch_lane` 那种环回：越界时视图留在原地，
    /// 只留一行说明。静默失败在这里尤其危险 —— `Alt+数字` 是最容易被 tmux / 终端吞掉的
    /// 组合键，用户分不清「终端没送达」和「送达了但没这条连接」（issue #361）。
    #[test]
    fn goto_lane_is_absolute_and_explains_out_of_range() {
        let client = test_client();
        let mut prefs = [test_prefs(), test_prefs(), test_prefs()];
        let mut app = test_shell(&client, &mut prefs);

        app.goto_lane(3);
        assert_eq!(app.active, 2, "Alt+3 落到第 3 条（与左栏自上而下同序号）");
        app.goto_lane(1);
        assert_eq!(app.active, 0, "Alt+1 回到第 1 条");

        let before = app.lanes[0].st.lines.len();
        app.goto_lane(7);
        assert_eq!(app.active, 0, "越界不切视图（更不能绕回第 1 条）");
        assert_eq!(app.lanes[0].st.lines.len(), before + 1, "要留一行说明");
        let last = app.lanes[0].st.lines.last().expect("说明行");
        assert!(last.text().contains("Alt+7"), "got {}", last.text());
        assert!(
            last.text().contains("3 条连接"),
            "要说清这里到底有几条：{}",
            last.text()
        );
    }

    #[test]
    fn next_approval_starts_right_after_the_active_lane() {
        // 0 空闲、1 在等 → 从 0 的下一步命中 1。
        assert_eq!(next_approval_index(0, &[false, true, false]), Some(1));
        // 从 2 出发：先回绕到 0（空闲），再到 1（在等）。
        assert_eq!(next_approval_index(2, &[false, true, true]), Some(1));
    }

    /// 自己也在等时优先跳**别的** lane：停在原地等于这个热键没用。
    #[test]
    fn next_approval_skips_the_active_lane_when_others_wait() {
        assert_eq!(next_approval_index(0, &[true, false, true]), Some(2));
        assert_eq!(next_approval_index(1, &[true, true]), Some(0));
    }

    /// 只有自己在等 → 回到自己（浮层本来就在眼前，切不切都一样）。
    #[test]
    fn next_approval_returns_to_self_when_only_it_waits() {
        assert_eq!(next_approval_index(0, &[true]), Some(0));
        assert_eq!(next_approval_index(1, &[false, true]), Some(1));
    }

    fn outcome(
        changed: bool,
        enabled: bool,
        cleared_down: bool,
        warning: Option<&str>,
    ) -> ToggleOutcome {
        ToggleOutcome {
            name: "proj-a".to_string(),
            enabled: Some(enabled),
            changed,
            cleared_down,
            note: None,
            warning: warning.map(str::to_string),
        }
    }

    /// 关闭要说清三件事：声明关了、进程会停、恢复走 `/lane open`。
    #[test]
    fn close_report_names_the_way_back() {
        let lines = toggle_report_lines("proj-a", false, &outcome(true, false, false, None));
        let text = lines.join("\n");
        assert!(text.contains("已关闭声明"), "{text}");
        assert!(text.contains("停掉该实例"), "{text}");
        assert!(text.contains("/lane open proj-a"), "要给恢复的路：{text}");
    }

    /// 幂等命中不是成功也不是失败：说「本来就是这样」，别写成「已关闭」。
    #[test]
    fn idempotent_hit_says_nothing_changed() {
        let mut out = outcome(false, false, false, None);
        out.note = Some("声明中不存在该实例".to_string());
        let text = toggle_report_lines("proj-a", false, &out).join("\n");
        assert!(text.contains("幂等命中"), "{text}");
        assert!(
            text.contains("声明中不存在该实例"),
            "要带 Server 的说明：{text}"
        );
        assert!(!text.contains("已关闭声明"), "{text}");
    }

    /// `enable` 的回执要指到 `/lane attach`，并把 `cleared_down` / `warning` 露出来。
    #[test]
    fn enable_report_points_at_attach_and_surfaces_warnings() {
        let text = toggle_report_lines(
            "proj-a",
            true,
            &outcome(
                true,
                true,
                true,
                Some("已写入 enabled = true，但清理 down 标记失败"),
            ),
        )
        .join("\n");
        assert!(text.contains("/lane attach proj-a"), "{text}");
        assert!(text.contains("down"), "{text}");
        assert!(text.contains("清理 down 标记失败"), "{text}");
    }

    #[test]
    fn next_approval_is_none_when_nobody_waits() {
        assert_eq!(next_approval_index(0, &[]), None, "空列表不能除零");
        assert_eq!(next_approval_index(0, &[false, false]), None);
    }
}
