//! 全屏 TUI 的终端外壳：raw mode / 备用屏幕 / 鼠标与括号粘贴捕获的 RAII 复原，
//! 以及入口 [`run_tui`]（装配 lane 与事件通道）。
//!
//! 从 `mod.rs` 拆出：那边已顶在 `fn-nloc-ratchet` 的 920 行/文件上限，而这块只负责
//! 终端与接线，与「会话状态」无关，拆开后两边都留出余量。

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

use crate::SessionPrefs;
use crate::serve::ServeClient;

use super::lane_list::LaneEntry;
use super::state::UiState;
use super::worker::{LaneId, UiEvent, spawn_key_reader, spawn_worker};
use super::{LaneApp, TuiApp, body_cache, drafts, event_loop, session_index, spinner};

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

    let (tx, rx) = mpsc::channel::<UiEvent>();
    spawn_key_reader(tx.clone());

    // 逐条装配：每条 lane 一个 worker（各自的 SSE 流），共用这一个通道。
    let mut lanes = Vec::with_capacity(lanes_spec.len());
    for (i, (label, client)) in lanes_spec.iter().enumerate() {
        let lane_id = LaneId(i as u32);
        let job_tx = spawn_worker(lane_id, (*client).clone(), tx.clone());
        lanes.push(LaneApp {
            id: lane_id,
            label: label.clone(),
            client,
            overrides: overrides.clone(),
            yes,
            no_keyring,
            st: UiState::new(),
            drafts: drafts::Drafts::default(),
            spinner: spinner::Spinner::new(),
            tx: tx.clone(),
            job_tx,
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
            ctl_socket: ctl_socket.map(std::path::Path::to_path_buf),
        });
    }
    let mut app = TuiApp {
        lanes,
        active: 0,
        rx,
    };
    // 每条 lane 都拉一次会话 / 工作区根 / serve 默认状态 / user-data 设置快照：
    // 只刷当前 lane 的话，切过去会看到空的会话列表（切 lane 不该触发网络请求）。
    for lane in app.lanes.iter_mut() {
        lane.refresh_sessions();
        lane.st.ws_begin_root_fetch();
        lane.refresh_workspace(false);
        lane.refresh_status();
        lane.load_user_settings();
    }
    let res = app.run_loop(&mut terminal);
    // 显式删背景图：退出备用屏幕时终端本会清空，但别把图像配额留到下一次运行。
    app.lane_mut().clear_background();
    // 退出落盘（有界 best-effort）：`st.running` 仍为真 = 回合中被强退，不再等它落定。
    let force = app.lane().st.running;
    let warn = app.lane_mut().flush_before_exit(force).await;
    drop(guard);
    if let Some(warn) = warn {
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

/// 外壳层动作：切 lane、导出渲染快照、鼠标命中。
///
/// **放这里而不是 `mod.rs`**：`mod.rs` 贴着 920 行门禁，而这三件事都只跟「有几条
/// lane、当前是哪条」有关，与单条 lane 的行为无关 —— 正好是外壳该管的那一层。
impl<'a> super::TuiApp<'a> {
    /// 渲染用的 lane 快照：名字 + 各自当前绑定的工作区 + 谁是当前。
    ///
    /// 只导出渲染需要的三个字段，不导任何 lane 内部状态 —— 渲染层拿不到 `&mut`，
    /// 也就不可能在某条 lane 的绘制里改到别人。
    ///
    /// 带工作区是「一实例一工作区」的直接后果：光有连接名，用户看不出两条 lane
    /// 是不是同一个工作区（那正是切错目标的常见原因）。
    pub(super) fn lane_entries(&self) -> Vec<LaneEntry> {
        self.lanes
            .iter()
            .enumerate()
            .map(|(i, lane)| LaneEntry {
                label: lane.label.clone(),
                workspace: lane.st.workspace_path.clone(),
                active: i == self.active,
            })
            .collect()
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
