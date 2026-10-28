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
/// **W1 阶段只有一条 lane**：`client` / `overrides` 直接构成那条 lane。W2 起这里改为
/// 遍历 profile 列表逐条装配（见 `docs/design/tui_multi_server.md` §6）。
pub async fn run_tui(
    client: &ServeClient,
    overrides: &mut SessionPrefs,
    yes: bool,
    no_keyring: bool,
) -> Result<()> {
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
    // W1：唯一的 lane 编号为 0；W2 起按 profile 逐个分配。
    let lane_id = LaneId(0);
    let job_tx = spawn_worker(lane_id, client.clone(), tx.clone());

    // W1：单 lane。W2 起按 profile 列表逐条建 lane（见 `docs/design/tui_multi_server.md` §6）。
    let lane = LaneApp {
        id: lane_id,
        client,
        overrides,
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
    };
    let mut app = TuiApp {
        lanes: vec![lane],
        active: 0,
        rx,
    };
    // 启动即拉一次会话 / 工作区根 / serve 默认状态 / user-data 设置快照（失败以系统行提示）。
    app.lane_mut().refresh_sessions();
    app.lane_mut().st.ws_begin_root_fetch();
    app.lane_mut().refresh_workspace(false);
    app.lane_mut().refresh_status();
    app.lane_mut().load_user_settings();
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
