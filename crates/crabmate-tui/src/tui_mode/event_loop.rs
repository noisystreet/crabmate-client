//! 全屏 TUI 的事件循环：唤醒策略、按需重绘与终端尺寸同步。
//!
//! 从 `mod.rs` 拆出：那里顶着 920 行门禁，而这里正是「一轮循环该做什么」的全部
//! 逻辑（等多久 → 收消息 → 画一帧），放一处比散在 `mod.rs` 里好读。
//!
//! **两种模式**（`CM_TUI_LOOP`）：
//!
//! - `event`（默认）：阻塞等事件 + 动画 deadline + 1s 兜底；兜底醒来时若没有动画、
//!   尺寸也没变，这一帧整帧跳过 —— 静态空闲不产生终端写入。
//! - `interval`（`CM_TUI_LOOP=interval` 回退）：固定节流（有消息 2ms / 空闲 50ms），
//!   静止期也每轮 `draw`，留作一个版本的回退口。
//!
//! 尺寸真值**永远现查**（`read_terminal_size`）：`Resize` 事件只作唤醒源，
//! 绝不作为尺寸来源 —— 一旦 SIGWINCH 丢失，把事件当来源就会永久错排。

use std::io::{self, Write};
use std::sync::mpsc::RecvTimeoutError;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossterm::event::EnableBracketedPaste;
use crossterm::execute;
use crossterm::style::Print;
use ratatui::Terminal;
// `Backend` 只为 `size()` 这个 trait 方法进 scope（`CrosstermBackend` 自身没有它）。
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::layout::Rect;

use super::lane_list::spinner_turning;
use super::render::{self, SIDEBAR_MIN_WIDTH};
use super::shell::MOUSE_ON;
use super::spinner;
use super::state::Focus;
use super::{LaneApp, TuiApp};

/// 事件循环模式（`CM_TUI_LOOP`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LoopMode {
    /// 固定节流轮询（改造前的行为，`CM_TUI_LOOP=interval` 的回退口）。
    Interval,
    /// 事件唤醒 + 1s 兜底（默认）。
    Event,
}

/// 解析 `CM_TUI_LOOP`：只有 `interval` 显式回退到固定节流轮询，
/// 其余（含未设置、拼错）一律走事件驱动。
///
/// 宽松解析与 `CM_TUI_THEME` 一致：拼错的开关不该把壳启动搞崩，按「未设置」处理。
pub(super) fn loop_mode_from(raw: Option<&str>) -> LoopMode {
    match raw.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        Some("interval") => LoopMode::Interval,
        _ => LoopMode::Event,
    }
}

/// 读环境变量决定循环模式。
pub(super) fn loop_mode() -> LoopMode {
    let raw = std::env::var("CM_TUI_LOOP").ok();
    loop_mode_from(raw.as_deref())
}

/// 静态兜底：即使没有任何事件也要在这之后醒一次。
///
/// 防的是 `Resize` 漏转发 / SIGWINCH 丢失 —— 那会让画面永久停在旧尺寸。
/// 醒来后若尺寸没变、也没有动画，那一帧会被 [`idle_frame_is_noop`] 判成空帧跳过，
/// 所以静态空闲**不产生终端写入**；兜底的代价只是一次 ioctl 与一次比较。
const IDLE_FALLBACK: Duration = Duration::from_secs(1);

/// 动画帧最短间隔：`spinner_left` 已过期时不能算成 0，否则变成忙转。
const MIN_TICK: Duration = Duration::from_millis(16);

/// `event` 模式的帧率下限。
///
/// `interval` 路径有 `sleep(2ms)` 兜着（隐含 ≤500fps）；`event` 路径没有任何睡眠，
/// 逐 token 的 SSE 消息会一条一轮地整帧重绘，比改造前更频繁。这里补上同样的下限。
const MIN_FRAME_INTERVAL: Duration = Duration::from_millis(2);

/// 下一轮最多等多久。
///
/// `spinner_left`：`Some(剩余)` = 生成中（按动画帧唤醒）；`None` = 静止（用兜底）。
/// `refresh_left`：还有多久该补发一次「回合结束后的会话刷新」（`None` = 没有待发）。
///
/// 刷新 deadline 必须作为一个等待源参与：它若短于兜底，而回合结束后界面正处在
/// 静止态，下一轮就要等到 `IDLE_FALLBACK`（1s）才醒 —— 新会话会在左栏迟迟不出现
/// （issue #349）。
pub(super) fn next_wait(
    spinner_left: Option<Duration>,
    refresh_left: Option<Duration>,
) -> Duration {
    let idle = match spinner_left {
        Some(left) => left.max(MIN_TICK),
        None => IDLE_FALLBACK,
    };
    match refresh_left {
        // 与动画帧同一口径：不足一帧也等满一帧，别把 `recv_timeout(0)` 变成忙转。
        Some(left) => idle.min(left.max(MIN_TICK)),
        None => idle,
    }
}

/// 兜底醒来的那一帧是否可以整帧跳过。
///
/// 三个条件同时成立才是空帧：不是被事件叫醒、**任一条 lane 的**动画都不在跑、
/// 终端尺寸也没变。只要有一个不成立就必须画 —— 界面是即时模式，`draw` 是唯一的
/// 输出手段；判成空帧就等于那条后台 lane 的 spinner 停在原地。
pub(super) fn idle_frame_is_noop(skip_idle: bool, animating: bool, size_changed: bool) -> bool {
    skip_idle && !animating && !size_changed
}

/// 快照里有没有 lane 正在呼吸（左栏 `●` 的亮度动画）。
///
/// [`LaneApp::draw_frame`] 用它决定兜底空帧能不能整帧跳过。判**当前** lane 会让
/// 「当前空闲、后台在跑」被跳过 —— 相位由墙钟推导之后，那条 lane 的相位再新也画不
/// 出来，症状与原缺陷一模一样。
pub(super) fn lanes_animating(lanes: &[super::lane_list::LaneEntry]) -> bool {
    lanes.iter().any(|entry| entry.pulse.is_some())
}

/// 距上一次 `draw` 不足 [`MIN_FRAME_INTERVAL`] 就先补足睡眠。
fn throttle_frame(last_draw: Instant) {
    if let Some(rest) = MIN_FRAME_INTERVAL.checked_sub(last_draw.elapsed()) {
        thread::sleep(rest);
    }
}

/// 读终端尺寸。**只读**，不改任何派生态 —— 调用方先比尺寸、再决定要不要画。
fn read_terminal_size(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<(u16, u16)> {
    let size = terminal.backend().size().context("terminal size")?;
    Ok((size.width, size.height))
}

/// 鼠标捕获开关：默认开启；`CM_TUI_MOUSE=off|0|false|none` 关闭，
/// 回到纯键盘（保留终端原生拖选复制）。
pub(super) fn mouse_capture_enabled() -> bool {
    !env_flag_off("CM_TUI_MOUSE")
}

/// 括号粘贴（bracketed paste）开关：默认开启；`CM_TUI_PASTE=off|0|false|none` 关闭。
///
/// 关掉后终端不再把粘贴包装成 `Event::Paste`，而是当逐字符按键送进来：换行会变成
/// 提交、`Tab` 会变成焦点切换。只在粘贴会触发终端兼容问题时才该关。
pub(super) fn paste_capture_enabled() -> bool {
    !env_flag_off("CM_TUI_PASTE")
}

/// 环境变量是否是「显式关闭」的那几种写法。
fn env_flag_off(name: &str) -> bool {
    matches!(
        std::env::var(name)
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "off" | "0" | "false" | "none"
    )
}

/// 按开关向终端下发「捕获」类序列：鼠标点击/滚轮与括号粘贴。
///
/// 两条序列各自可关，语义不同：关鼠标是为了拿回终端原生拖选复制，
/// 关括号粘贴是为了绕开打包粘贴的终端兼容问题（见 [`paste_capture_enabled`]）。
pub(super) fn enable_captures(stdout: &mut io::Stdout, mouse: bool, paste: bool) -> Result<()> {
    if mouse {
        execute!(stdout, Print(MOUSE_ON)).context("enable mouse capture")?;
    }
    if paste {
        execute!(stdout, EnableBracketedPaste).context("enable bracketed paste")?;
    }
    Ok(())
}

impl TuiApp<'_> {
    /// 事件循环：收消息 → 画一帧 → 节流。
    pub(super) fn run_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> Result<()> {
        let mode = loop_mode();
        // 首轮不许长等：启动时的拉取请求已经发出去，第一条回包可能很久才来，
        // 而屏幕还是 `terminal.clear()` 之后的空白。
        let mut first = true;
        let mut last_draw = Instant::now();
        loop {
            // 退出判定在外壳层：任一条 lane 提的 `/quit` 都退出整个 TUI（issue #338）。
            if self.exit_requested() {
                return Ok(());
            }
            let had_event = self.collect_events(mode, first)?;
            first = false;
            if self.exit_requested() {
                return Ok(());
            }
            // 回合结束后的会话刷新在这里收敛：各 lane 只置意图位，按窗口合并再发
            // （issue #349）。先发请求、再画当前状态。
            self.flush_session_refresh(Instant::now());
            if mode == LoopMode::Event {
                // 帧率下限（`interval` 靠循环末尾那个 sleep）。
                throttle_frame(last_draw);
            }
            // 兜底醒来的空帧（无事件 / 无动画 / 尺寸未变）整帧跳过。
            // 先算 lane 快照再取 `lane_mut()`：两个借用会在同一条语句里打架。
            // 快照之前熄未读点：当前 lane 就在眼前，那个点没有存在意义。
            self.clear_active_unread();
            let lanes = self.lane_entries();
            self.lane_mut().draw_frame(terminal, !had_event, &lanes)?;
            last_draw = Instant::now();
            if mode == LoopMode::Interval {
                // 改造前的节奏原样保留：有消息 2ms、空闲 50ms。
                thread::sleep(Duration::from_millis(if had_event { 2 } else { 50 }));
            }
        }
    }

    /// 所有在转的 lane 里最近的一次换帧时刻；没有就是 `None`。
    ///
    /// 必须扫**全部** lane（[`spinner_turning`] 与左栏「帧该显示」同一个判定）：
    /// 只看当前 lane 的话，它一空闲唤醒源就退回兜底，后台 lane 的动画跟着停。
    fn next_spinner_tick(&self) -> Option<Duration> {
        self.lanes
            .iter()
            .filter(|lane| spinner_turning(lane))
            .map(|lane| spinner::next_tick(lane.turn_elapsed()))
            .min()
    }

    /// 收集本轮输入，返回「本轮是否收到过消息」。
    ///
    /// `interval`：非阻塞 drain（改造前语义），节流交给 [`Self::run_loop`] 的 sleep。
    /// `event`：先等一条（动画 deadline 或兜底），再 drain 积压 —— 一轮画完一次，
    /// 保住流式期「多块文本合并成一轮」的优先级。
    ///
    /// 取消息与分派拆成两句：`recv_timeout` 的临时值若留在 `match` 里，不可变借用
    /// 会覆盖整个 `match` 体，`lane_mut()` 就借不到了。
    fn collect_events(&mut self, mode: LoopMode, first: bool) -> Result<bool> {
        if mode == LoopMode::Interval || first {
            return Ok(self.drain_events());
        }
        // 唤醒按**全部**在跑 lane 里最近的一次换帧算：只看当前 lane 的话，
        // 它一空闲就退回 1s 兜底，后台 lane 的 spinner 退化成「1 秒一帧」。
        let left = self.next_spinner_tick();
        // 会话刷新的窗口剩余同样是唤醒源（见 `next_wait`）。
        let refresh_left = self.session_refresh.until_due(Instant::now());
        let got = self.rx.recv_timeout(next_wait(left, refresh_left));
        match got {
            Ok(msg) => self.on_msg(msg),
            // 超时醒来只为画动画帧 / 兜底查尺寸，本轮没有消息。
            Err(RecvTimeoutError::Timeout) => return Ok(false),
            // 结构上**不可达**：`LaneApp` 自持 `tx`，`run_loop` 期间它必然存活，
            // channel 不会断开。保留这一支只为 match 穷尽；真发生就退出，
            // 绝不让循环空转。排查「画面停住」时不要先看这里。
            Err(RecvTimeoutError::Disconnected) => {
                self.lane_mut().exit = true;
                return Ok(false);
            }
        }
        self.drain_events();
        Ok(true)
    }

    /// 非阻塞取完当前积压（与改造前的 `while let Ok(..)` 同语义）。
    fn drain_events(&mut self) -> bool {
        let mut any = false;
        loop {
            // 同上：先取再分派，避免 `try_recv` 的临时借用与 `lane_mut()` 冲突。
            let next = self.rx.try_recv();
            let Ok(msg) = next else { break };
            any = true;
            self.on_msg(msg);
            if self.exit_requested() {
                break;
            }
        }
        any
    }
}

impl LaneApp<'_> {
    /// 在途回合已过的时间；空闲时为零。
    ///
    /// spinner 帧号（[`spinner::frame_at`]）、左栏耗时与唤醒 deadline 都从这一处取：
    /// 从**发出**算起，首 token 之前的排队与建连同样是等。
    ///
    /// 定义在 `event_loop` 而不是 `mod.rs` / `shell.rs`：那两个文件都贴着 920 行门禁，
    /// 而它的三个调用点里有两个就在本文件。
    pub(super) fn turn_elapsed(&self) -> Duration {
        self.turn_started.map_or(Duration::ZERO, |at| at.elapsed())
    }

    /// 画一帧：拉尺寸 → 排版 → draw → 重放背景 → flush。
    ///
    /// 动画**不在这里推进**：帧号由墙钟推导（见 [`spinner`]），后台 lane 的帧
    /// 也不经过本方法 —— 谁都推不了别人，只能各自看时间。
    ///
    /// `skip_idle`：本轮是兜底超时醒来的（没有事件）。配合「无动画 + 尺寸未变」
    /// 就跳过整帧 —— 静止期不写终端。「有动画」取**快照里任一条 lane**：只判
    /// 当前 lane 会让「当前空闲、后台在跑」的帧被整帧跳过，那条 lane 的呼吸就永远
    /// 落不到屏幕上。
    fn draw_frame(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        skip_idle: bool,
        lanes: &[super::lane_list::LaneEntry],
    ) -> Result<()> {
        let dims = read_terminal_size(terminal)?;
        let animating = lanes_animating(lanes);
        if idle_frame_is_noop(skip_idle, animating, dims != self.last_size) {
            return Ok(());
        }
        self.apply_size(dims);
        let (w, h) = dims;
        // 内容/宽度/搜索未变时复用已构建的 body 物理行，避免每帧全量 tokenize+wrap。
        self.prepare_body(w);
        let mut info = self.status_info();
        // 连接计数取自外壳交来的渲染快照：左栏画的就是这批 entry，状态栏另数一份
        // 迟早出现「左栏两条、状态栏一条」。
        info.lanes = super::lane_list::LaneTally::from_entries(lanes);
        // 设置面板改成居中弹层后，内容必须按**弹层内宽**排版：用与绘制同源的
        // `settings_panel_content_width`，否则会出现「画 76 列、按 78 列生成」的错位。
        let screen = Rect::new(0, 0, w, h);
        let panel = self.settings_content(render::settings_panel_content_width(screen));
        terminal
            .draw(|f| {
                render::draw(
                    f,
                    &mut self.st,
                    &info,
                    self.body.as_slice(),
                    panel.as_ref(),
                    lanes,
                )
            })
            .context("tui draw failed")?;
        // 背景要在 draw 之后重放：kitty 让图像跟着文本一起滚动（规范明确），
        // 而 ratatui 的差分绘制只认字符网格，不会替我们维持图像层。
        self.paint_background(screen);
        let _ = io::stdout().flush();
        Ok(())
    }

    /// 尺寸派生态（纯计算）：窄屏隐藏左右栏；焦点滞留已隐藏的左栏则收回输入框。
    ///
    /// **每次 draw 前都要调**（[`Self::draw_frame`] 里紧跟尺寸现查）：
    /// `Resize` 事件丢了也不会错排，因为这里从不读事件里的值。
    pub(super) fn apply_size(&mut self, (width, height): (u16, u16)) {
        self.last_size = (width, height);
        self.st.sidebar_visible = width >= SIDEBAR_MIN_WIDTH;
        // PgUp/PgDn 页步 ≈ 聊天可视区行数（状态行 + composer 占几行）。
        self.st.page_rows = (height as usize).saturating_sub(4).max(1);
        if !self.st.sidebar_visible && matches!(self.st.focus, Focus::Sidebar | Focus::Workspace) {
            self.st.focus = Focus::Input;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use super::super::lane_list::{LaneEntry, LaneStatus};
    use super::super::state::Focus;
    use super::super::test_support::{test_app, test_client, test_prefs, test_shell};
    use super::super::worker::UiEvent;
    use super::{
        IDLE_FALLBACK, LoopMode, MIN_TICK, idle_frame_is_noop, lanes_animating, loop_mode_from,
        next_wait, spinner,
    };

    #[test]
    fn loop_mode_defaults_to_event() {
        assert_eq!(loop_mode_from(None), LoopMode::Event, "未设置走事件驱动");
        assert_eq!(loop_mode_from(Some("")), LoopMode::Event);
        // 未知值按「未设置」处理：拼错的开关不该改变行为，也不该让壳启动失败。
        // 样例刻意用非单词串 —— 若拿「event 的常见拼错」当真单词写进代码，typos 钩子
        // 会在提交时把它自动改回 event，断言就悄悄变成「event 也回落 event」。
        assert_eq!(loop_mode_from(Some("not-a-mode")), LoopMode::Event);
    }

    #[test]
    fn loop_mode_interval_is_the_explicit_opt_out() {
        assert_eq!(loop_mode_from(Some("interval")), LoopMode::Interval);
        assert_eq!(loop_mode_from(Some(" Interval ")), LoopMode::Interval);
    }

    #[test]
    fn loop_mode_accepts_event_with_any_case_and_padding() {
        assert_eq!(loop_mode_from(Some("event")), LoopMode::Event);
        assert_eq!(loop_mode_from(Some(" EVENT ")), LoopMode::Event);
    }

    #[test]
    fn wait_animates_while_generating() {
        let left = Duration::from_millis(80);
        assert_eq!(next_wait(Some(left), None), left, "生成中按动画帧唤醒");
    }

    #[test]
    fn wait_clamps_zero_to_min_tick() {
        // 已过期不能算成 0，否则 `recv_timeout(0)` 变成忙转。
        assert_eq!(next_wait(Some(Duration::ZERO), None), MIN_TICK);
    }

    #[test]
    fn wait_idle_falls_back() {
        assert_eq!(next_wait(None, None), IDLE_FALLBACK);
    }

    #[test]
    fn wait_is_capped_by_a_pending_session_refresh() {
        // 窗口剩余比兜底短就得按它醒，否则新会话要等到 1s 兜底才进左栏。
        assert_eq!(
            next_wait(None, Some(Duration::from_millis(30))),
            Duration::from_millis(30)
        );
        // 已经到点 / 比一帧还短：按最短帧醒，本轮就能发出去，但不忙转。
        assert_eq!(next_wait(None, Some(Duration::ZERO)), MIN_TICK);
        // 动画帧更急时听动画帧的。
        assert_eq!(
            next_wait(
                Some(Duration::from_millis(8)),
                Some(Duration::from_millis(90))
            ),
            MIN_TICK
        );
    }

    #[test]
    fn idle_frame_is_skipped_only_when_nothing_changed() {
        assert!(idle_frame_is_noop(true, false, false), "兜底空帧：跳过");
        // 任一条件不成立都必须画。
        assert!(!idle_frame_is_noop(false, false, false), "事件唤醒必须画");
        assert!(!idle_frame_is_noop(true, true, false), "动画帧必须画");
        assert!(!idle_frame_is_noop(true, false, true), "尺寸变了必须画");
    }

    /// 「当前 lane 空闲、后台 lane 在跑」的兜底帧不许跳过。
    ///
    /// 帧号改由墙钟推导之后，这是它唯一能落到屏幕上的出口：`lanes_animating` 若
    /// 仍判当前 lane（改之前的写法），后台那条的帧再新也会被整帧跳过 —— 症状与
    /// 原缺陷一模一样，只是成因从「没人推进」变成「没人画」。
    #[test]
    fn idle_frames_keep_painting_while_a_background_lane_runs() {
        let turning = |label: &str| LaneEntry {
            label: label.to_string(),
            status: LaneStatus::Running,
            pulse: Some(1),
            ..Default::default()
        };

        // 两条都在跑：有动画就得画。
        let both = [turning("lane-a"), turning("lane-b")];
        assert!(lanes_animating(&both));
        assert!(!idle_frame_is_noop(true, lanes_animating(&both), false));

        // 只有后台那条在跑（当前 lane 空闲、`pulse` 为 `None`）：仍要画。
        let background = [LaneEntry::default(), turning("lane-b")];
        assert!(lanes_animating(&background), "当前 lane 空闲不该连累后台");
        assert!(!idle_frame_is_noop(
            true,
            lanes_animating(&background),
            false
        ));

        // 全空闲才是空帧 —— 静态期仍然不写终端。
        let idle = [LaneEntry::default()];
        assert!(!lanes_animating(&idle));
        assert!(idle_frame_is_noop(true, lanes_animating(&idle), false));
    }

    /// 唤醒源要看**全部**在跑 lane。
    ///
    /// 只按当前 lane 算的话，它一空闲就退回 `IDLE_FALLBACK`(1s)：后台 lane 的帧号
    /// 虽然跟着墙钟走，却没人唤醒、没人画，肉眼看仍是「1 秒一格」。
    #[test]
    fn wake_source_covers_background_lanes() {
        let client = test_client();
        let mut prefs = [test_prefs(), test_prefs()];
        let mut app = test_shell(&client, &mut prefs);
        // 当前 lane 空闲，后台那条在跑。
        app.lanes[1].st.running = true;
        app.lanes[1].turn_started = Some(Instant::now());

        let left = app
            .next_spinner_tick()
            .expect("后台 lane 在跑就该有动画 deadline");
        assert!(
            left <= spinner::SPINNER_INTERVAL,
            "不能退到兜底：{left:?} 超过一个换帧间隔"
        );

        // 全空闲才回到「没有动画」，此时才轮到 1s 兜底。
        app.lanes[1].st.running = false;
        assert!(app.next_spinner_tick().is_none());
    }

    /// 窄屏隐藏左栏、焦点收回输入框；宽屏恢复。
    #[test]
    fn apply_size_hides_sidebar_and_reclaims_focus() {
        let client = test_client();
        let mut prefs = test_prefs();
        let (tx, rx) = mpsc::channel();
        let (job_tx, _job_rx) = mpsc::channel();
        let mut app = test_app(&client, &mut prefs, rx, tx, job_tx);

        app.st.focus = Focus::Workspace;
        app.apply_size((100, 30));
        assert!(!app.st.sidebar_visible, "窄屏隐藏左栏");
        assert!(
            matches!(app.st.focus, Focus::Input),
            "焦点不得滞留在已隐藏的左栏"
        );
        assert_eq!(app.st.page_rows, 26, "页步 = 高度 - 4");

        app.apply_size((140, 40));
        assert!(app.st.sidebar_visible, "宽屏恢复左栏");
    }

    /// `Resize` 只是唤醒源：不写尺寸、不改派生态（真值在 draw 前现查）。
    #[test]
    fn resize_event_is_wake_only() {
        let client = test_client();
        let mut prefs = test_prefs();
        let (tx, rx) = mpsc::channel();
        let (job_tx, _job_rx) = mpsc::channel();
        let mut app = test_app(&client, &mut prefs, rx, tx, job_tx);

        app.apply_size((140, 40));
        assert!(app.st.sidebar_visible);

        app.on_msg(UiEvent::Resize);

        // 若这里跟着变了，就说明有人把事件里的尺寸存下来了 —— 那正是「漏 resize」的源头。
        assert!(app.st.sidebar_visible, "Resize 不该改派生态");
        assert_eq!(app.last_size, (140, 40), "Resize 不该写 last_size");
    }
}
