//! 未连接界面：进程起来了、但一条 lane 都还没有时的那块屏。
//!
//! 为什么要有它：`--api-base` 曾经是**启动的硬前提**，而「先起来、再决定连哪台」是
//! 多 serve 用户的常态（控制面里通常已经跑着几条实例）。这里给那块过渡期一块能用的
//! 屏：一句引言 + 已输出的提示 + 一行输入，只认与「连上一条连接」有关的命令。
//!
//! 为什么自成一类而不是「空 `lanes` 的普通界面」：普通界的输入行、焦点、审批浮层、
//! 状态栏全都住在某条 `LaneApp` 的 `UiState` 里 —— 没有 lane 就没有输入框，用户要敲的
//! `/lane attach` 无处可敲。硬把 `UiState` 塞进外壳则要给每条 lane 的代码都加一个
//! 「这条其实没连上」的分支（与 `pending_lane` 不进 `lanes` 同一个理由）。
//!
//! 边界（写在这里，免得以后被当成遗漏）：
//!
//! - 只认 `/lane`（`list` / `attach` / `attach-url`）、`/ctl`、`/help`、`/quit`；
//!   其余命令一律答「先连上一条连接」。`/lane new|status|restart|close|open` 同理 ——
//!   它们要么需要一个已连上的 lane 当上下文（`new` 的工作区兜底），要么本来就是
//!   「管理一条已在用的连接」，空态下做了也只会多一条无处可看的报告。
//! - 一旦第一条 lane 连上（`attach_lane` 成功），`lanes` 不再为空，本界面立刻让位给
//!   正常界面，之后也不会再回来：最后一条 lane 不允许摘（`detach_lane`）。
//! - **本文件里不要写 `KeyCode::Char('c')` 字面量**：它会让 `lizard` 的 Rust 词法在这里
//!   失去同步，把 `on_empty_key` 之后的整段代码并成一个函数，`fn-nloc` 门禁于是误报
//!   「266 行」。需要 Ctrl+C / Ctrl+D 时走 [`is_quit_key`] 的码位比较；改回字面量之前
//!   先跑 `bash scripts/fn-nloc-ratchet.sh`。

use std::io;
use std::io::Write;

use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use super::TuiApp;
use super::ctl::{CTL_DISABLED_HINT, listing_lines};
use super::event_loop::idle_frame_is_noop;
use super::inbound::{Inbound, ShellEvent};
use super::lane_cmd::{self, LaneCommand};
use super::worker::{LaneId, UiEvent};
use super::worker_ctl::CtlJob;

/// 空态事件的「归属 id」。
///
/// 还没有 lane，就没有 `LaneId` 可分派 —— 但控制面回包（[`UiEvent::CtlList`]）仍要落到
/// 某个地方。取一个**不可能**被真实 lane 用到的号：`next_lane_id` 从
/// `lanes_spec.len()` 起单调递增（空态时是 0），永远到不了 `u32::MAX`。
pub(super) const EMPTY_LANE: LaneId = LaneId(u32::MAX);

/// 空态保留的历史行数上限。
///
/// `/lane` 一次可能打出十几行（每条实例一行），无界增长只会把引言与输入行挤出屏幕。
/// 保留**末尾**若干行，与终端自己的滚屏方向一致（新内容在最下面）。
const MAX_EMPTY_LINES: usize = 200;

/// 未连接界面的状态：几行输出 + 一行输入 + 退出意图。
#[derive(Debug, Default)]
pub(super) struct EmptyState {
    /// 已输出的提示与命令结果。
    pub(super) lines: Vec<String>,
    /// 正在输入的一行。
    pub(super) input: String,
    /// 已请求退出（`/quit`、Ctrl+C、Ctrl+D）。
    ///
    /// 不走任何 lane 的 `exit`：空态下没有 lane 可以承载这个意图，而 `exit_requested`
    /// 是外壳判据（它同时读 `empty.exit`）。
    pub(super) exit: bool,
    /// 上一次**画过**的终端尺寸；`None` = 还没画过（首帧必须画）。
    ///
    /// 与普通界面的 `LaneApp::last_size` 同一个用途：兜底醒来的那一帧要能整帧跳过，
    /// 而「尺寸变没变」只能跟上次画过的值比。空态不处理 `Resize` 事件
    /// （`on_empty_msg` 丢鼠标 / 尺寸），尺寸**只靠**这里每帧现查兜住 ——
    /// 所以它不能被优化成「不读尺寸就直接跳过」。
    pub(super) last_size: Option<(u16, u16)>,
}

impl EmptyState {
    /// 写一行（多行文本按行拆开，与 transcript 的 `push_line` 同口径）。
    pub(super) fn push_line(&mut self, text: &str) {
        for line in text.lines() {
            if self.lines.len() >= MAX_EMPTY_LINES {
                self.lines.remove(0);
            }
            self.lines.push(line.to_string());
        }
    }

    /// 末尾 `room` 行（不足则全给）。
    fn tail(&self, room: usize) -> &[String] {
        let from = self.lines.len().saturating_sub(room);
        &self.lines[from..]
    }
    /// 删掉输入行的最后一个字符（Backspace）。
    ///
    /// 用 `pop()` 而不是按字节：UTF-8 多字节字符（中文注释、emoji）按字节删会留下
    /// 半个字符，下一次 `push` 就写出一个非法串。
    fn pop_char(&mut self) {
        self.input.pop();
    }
}

/// 一行输入的解析结果。
///
/// 抽成纯函数：这块屏唯一的「智能」就在这里（认哪几条命令、其余怎么答），而它不需要
/// 终端、通道或控制面就能验证。
#[derive(Debug, PartialEq, Eq)]
pub(super) enum EmptyCommand {
    /// 空行：什么都不做。
    Blank,
    /// `/quit` / `/exit` / `/q`。
    Quit,
    /// `/help` / `/?`。
    Help,
    /// `/lane` / `/lane list` / `/ctl`：列控制面实例。
    List,
    /// `/lane attach <name>`。
    Attach(String),
    /// `/lane attach-url <name>=<url>`（`/connect` / `/url` 同义）。
    AttachUrl {
        /// 新 lane 的 `label`。
        name: String,
        /// serve 根 URL。
        url: String,
    },
    /// 认得出是命令，但在未连接界面上做不了（或语法有错），理由是给人看的。
    NotHere(String),
    /// 普通文本：还没连上，发不出去。
    NotConnected,
}

/// 解析空态输入行。
pub(super) fn parse_empty_command(text: &str) -> EmptyCommand {
    let text = text.trim();
    if text.is_empty() {
        return EmptyCommand::Blank;
    }
    let Some(rest) = text.strip_prefix('/') else {
        return EmptyCommand::NotConnected;
    };
    let (head, tail) = match rest.split_once(char::is_whitespace) {
        Some((head, tail)) => (head, tail.trim()),
        None => (rest, ""),
    };
    match head.to_ascii_lowercase().as_str() {
        "quit" | "exit" | "q" => EmptyCommand::Quit,
        "help" | "?" => EmptyCommand::Help,
        "ctl" if tail.is_empty() => EmptyCommand::List,
        "lane" => parse_empty_lane(tail),
        // `/connect <name>=<url>`：`--connect` 的写法，整段就是 `<name>=<url>`。
        "connect" | "url" => attach_url(tail),
        _ => EmptyCommand::NotHere(format!(
            "`/{head}` 需要先连上一条连接：`/lane` 看控制面实例，再 `/lane attach <name>`（未声明的 serve 用 `/lane attach-url name=url`）"
        )),
    }
}

/// `/lane ...` 在空态下只认三条（见模块头注释的边界）。
fn parse_empty_lane(tail: &str) -> EmptyCommand {
    if tail.is_empty() {
        return EmptyCommand::List;
    }
    match lane_cmd::parse(Some(tail)) {
        Ok(LaneCommand::List) => EmptyCommand::List,
        Ok(LaneCommand::Attach(name)) => EmptyCommand::Attach(name),
        Ok(LaneCommand::AttachUrl { name, url }) => EmptyCommand::AttachUrl { name, url },
        Ok(_) => EmptyCommand::NotHere(
            "这条 `/lane` 子命令需要一个已连上的连接（`new` 的工作区兜底、`status` / `restart` / \
             `close` / `open` 管理的都是已在用的实例）：先 `/lane attach <name>`"
                .to_string(),
        ),
        Err(usage) => EmptyCommand::NotHere(usage),
    }
}

/// `/connect <name>=<url>`：借 `/lane attach-url` 的解析，报错也指回这条写法。
fn attach_url(tail: &str) -> EmptyCommand {
    match lane_cmd::parse(Some(&format!("attach-url {tail}"))) {
        Ok(LaneCommand::AttachUrl { name, url }) => EmptyCommand::AttachUrl { name, url },
        Ok(_) => EmptyCommand::NotHere("用法：`/connect <name>=<url>`".to_string()),
        Err(why) => EmptyCommand::NotHere(why),
    }
}

/// `Ctrl+C` / `Ctrl+D` 认的是这两个键（码位比较，原因见模块头注释的 `lizard` 一条）。
fn is_quit_key(c: char) -> bool {
    c == char::from(99) || c == char::from(100)
}

/// 空态这一帧能不能整帧跳过（纯函数，便于单测）。
///
/// 与普通界面的 [`idle_frame_is_noop`] **同一口径**，只是空态没有任何动画
/// （没有 spinner、没有 `●` 呼吸），所以 `animating` 恒为 `false`；`last_size`
/// 是上一次画过的尺寸，`None` 表示还没画过 —— 首帧必须画，否则屏幕上什么都没有。
///
/// 为什么需要它：早先空态分支在 `run_loop` 里无条件 `draw_empty` + `continue`，
/// 于是 `IDLE_FALLBACK` 每醒一次就白写一整屏（`CM_TUI_LOOP=interval` 下是 20 fps），
/// 与同文件 `IDLE_FALLBACK` 注释承诺的「静态空闲不产生终端写入」相反 —— SSH /
/// 慢终端上表现为持续闪烁。
pub(super) fn empty_frame_is_noop(
    skip_idle: bool,
    dims: (u16, u16),
    last_size: Option<(u16, u16)>,
) -> bool {
    idle_frame_is_noop(skip_idle, false, last_size != Some(dims))
}

/// 引言：这块屏能敲什么，一次说完（空态下没有 `/help` 之外的第二次机会）。
fn head_lines() -> Vec<String> {
    vec![
        "还没有连接 —— 这是未连接界面".to_string(),
        "  /lane                        列控制面实例（同 /ctl）".to_string(),
        "  /lane attach <name>          连上已就绪的实例".to_string(),
        "  /lane attach-url name=url    直连一个地址（未声明过的 serve；/connect 同义）"
            .to_string(),
        "  /help · /quit（Ctrl+C 同义）".to_string(),
        String::new(),
    ]
}

impl TuiApp<'_> {
    /// 空态的事件入口（`on_msg` 在 `lanes` 为空时转到这里）。
    ///
    /// 只有三类事件有意义：键盘输入、控制面回包、外壳动作（`attach` 的成败）。
    /// 其余（鼠标 / 尺寸 / 粘贴 / 某条已关闭 lane 的迟到事件）一律丢弃 —— 空态没有
    /// 可画可点的东西，把它们塞进历史行只会噪声化那块屏。
    pub(super) fn on_empty_msg(&mut self, msg: Inbound) {
        match msg {
            Inbound::Input(UiEvent::Key(key)) => self.on_empty_key(key),
            Inbound::FromLane(id, UiEvent::CtlList(result)) if id == EMPTY_LANE => match result {
                Ok(items) => {
                    for line in listing_lines(&items) {
                        self.empty.push_line(&line);
                    }
                }
                Err(why) => self.empty.push_line(&why),
            },
            Inbound::Shell(ev) => self.on_shell(ev),
            _ => {}
        }
    }

    fn on_empty_key(&mut self, key: KeyEvent) {
        // 两个「走人」键先判：`Ctrl+C` 与普通界面同义（这里没有回合可取消），
        // `Ctrl+D` 是终端习惯里的 EOF。
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && matches!(key.code, KeyCode::Char(c) if is_quit_key(c)) {
            self.empty.exit = true;
        } else if key.code == KeyCode::Enter {
            self.submit_empty();
        } else if key.code == KeyCode::Esc {
            // Esc 只清输入行：空态没有浮层可关，误清一行比误退出便宜得多。
            self.empty.input.clear();
        } else if key.code == KeyCode::Backspace {
            self.empty.pop_char();
        } else if let KeyCode::Char(c) = key.code {
            self.empty.input.push(c);
        }
    }

    /// 回车：解析 → 执行（只做「连上一条」范围内的几件事）。
    fn submit_empty(&mut self) {
        let text = self.empty.input.trim().to_string();
        self.empty.input.clear();
        match parse_empty_command(&text) {
            EmptyCommand::Blank => {}
            EmptyCommand::Quit => self.empty.exit = true,
            EmptyCommand::Help => {
                for line in head_lines() {
                    self.empty.push_line(&line);
                }
            }
            EmptyCommand::NotConnected => self.empty.push_line(
                "还没有连接，发不出消息：`/lane` 看实例，`/lane attach <name>` 或 `/lane attach-url name=url` 连上一条",
            ),
            EmptyCommand::NotHere(why) => self.empty.push_line(&why),
            EmptyCommand::List => self.empty_ctl_list(),
            EmptyCommand::Attach(name) => self.empty_ctl_attach(&name),
            EmptyCommand::AttachUrl { name, url } => {
                self.empty
                    .push_line(&format!("正在直连 `{name}`（{url}）…"));
                if self
                    .template
                    .tx
                    .send(Inbound::Shell(ShellEvent::AttachUrl { name, url }))
                    .is_err()
                {
                    self.empty.push_line("连接未发出：事件通道已关闭");
                }
            }
        }
    }

    /// `/lane` / `/ctl`：拉一次控制面列表，回包按 [`EMPTY_LANE`] 落回本屏。
    fn empty_ctl_list(&mut self) {
        let Some(ctl) = self.template.ctl.clone() else {
            self.empty.push_line(CTL_DISABLED_HINT);
            return;
        };
        self.empty.push_line("正在查询 ctl 实例…");
        if ctl.enqueue(CtlJob::List { from: EMPTY_LANE }).is_err() {
            self.empty.push_line("ctl 查询未发出：控制面线程已退出");
        }
    }

    /// `/lane attach <name>`：地址由控制面给，装配仍由外壳的 `attach_lane` 完成。
    fn empty_ctl_attach(&mut self, name: &str) {
        let Some(ctl) = self.template.ctl.clone() else {
            self.empty.push_line(CTL_DISABLED_HINT);
            return;
        };
        self.empty
            .push_line(&format!("正在连接 ctl 实例 `{name}`…"));
        let job = CtlJob::Attach {
            from: EMPTY_LANE,
            name: name.to_string(),
        };
        if ctl.enqueue(job).is_err() {
            self.empty.push_line("连接未发出：控制面线程已退出");
        }
    }

    /// 画一帧：引言 + 历史（取末尾）+ 底部输入行。
    ///
    /// 不用 `render::draw`：那块是 lane 的排版（左栏 / 右栏 / 状态栈 / 审批浮层），空态
    /// 一个都没有。这里就三行布局，自己排比给那边加一个「没 lane 时怎么画」的分支清楚。
    ///
    /// `skip_idle`：本轮是兜底超时醒来的（没有事件）。配合「尺寸未变」整帧跳过
    /// （[`empty_frame_is_noop`]）—— 尺寸仍然每次都查，所以 `Resize` 事件丢了也不会
    /// 让画面停在旧尺寸上；跳过的只是「写终端」这一步。
    pub(super) fn draw_empty(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        skip_idle: bool,
    ) -> Result<()> {
        let size = terminal.backend().size().context("terminal size")?;
        let dims = (size.width, size.height);
        if empty_frame_is_noop(skip_idle, dims, self.empty.last_size) {
            return Ok(());
        }
        self.empty.last_size = Some(dims);
        let area = Rect::new(0, 0, size.width, size.height);
        let head = head_lines();
        // 历史区可用行数：总高减去引言与底部两行（空行 + 输入行）。
        let room = (size.height as usize).saturating_sub(head.len() + 2);
        let history: Vec<Line> = self
            .empty
            .tail(room)
            .iter()
            .map(|l| Line::from(l.clone()))
            .collect();
        let mut body: Vec<Line> = head.into_iter().map(Line::from).collect();
        body.extend(history);
        let prompt = format!("> {}", self.empty.input);
        // 光标落在输入末尾：这块屏只有一行、不支持左右移动，所以 x = 前缀 + 内容显示宽。
        // 不设它的话打字时终端上没有插入点（普通界面的 composer 有 `BlinkingBar`）。
        let prompt_width = 2 + UnicodeWidthStr::width(self.empty.input.as_str());
        terminal
            .draw(|f| {
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(1), Constraint::Length(1)])
                    .split(area);
                f.render_widget(Paragraph::new(body).wrap(Wrap { trim: false }), chunks[0]);
                f.render_widget(Paragraph::new(Line::from(prompt)), chunks[1]);
                // 夹在行内：超长输入不该把光标推到屏幕外。
                let max_x = chunks[1].width.saturating_sub(1) as usize;
                let x = chunks[1].x + prompt_width.min(max_x) as u16;
                f.set_cursor_position((x, chunks[1].y));
            })
            .context("empty draw failed")?;
        let _ = io::stdout().flush();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{EmptyCommand, parse_empty_command};

    /// 空闲帧不写终端；首帧与尺寸变化必须画。
    ///
    /// `Resize` 事件在空态是被丢弃的（`on_empty_msg` 只认键盘 / ctl 回包 / 外壳事件），
    /// 所以「尺寸变了」是那块屏跟得上终端窗口的**唯一**途径 —— 这条判据必须与
    /// 「别重绘」一起测，否则修掉闪烁的同时会把窗口缩放弄坏。
    #[test]
    fn idle_frames_do_not_repaint_the_unconnected_screen() {
        use super::empty_frame_is_noop;

        // 首帧（还没画过）：必须画，否则整块屏是空的。
        assert!(!empty_frame_is_noop(true, (80, 24), None));
        // 兜底醒来、尺寸没变：跳过。
        assert!(empty_frame_is_noop(true, (80, 24), Some((80, 24))));
        // 尺寸变了：必须画。
        assert!(!empty_frame_is_noop(true, (100, 30), Some((80, 24))));
        // 有事件（按键 / ctl 回包 / attach 成败）：必须画。
        assert!(!empty_frame_is_noop(false, (80, 24), Some((80, 24))));
    }

    #[test]
    fn blank_and_plain_text_are_told_apart() {
        assert_eq!(parse_empty_command("   "), EmptyCommand::Blank);
        // 普通文本不是命令：空态发不出消息，要说清而不是静默吞掉。
        assert_eq!(parse_empty_command("hello"), EmptyCommand::NotConnected);
    }

    #[test]
    fn quit_aliases_and_help() {
        for text in ["/quit", "/exit", "/q", " /Q "] {
            assert_eq!(parse_empty_command(text), EmptyCommand::Quit, "{text}");
        }
        assert_eq!(parse_empty_command("/help"), EmptyCommand::Help);
    }

    #[test]
    fn list_has_three_spellings() {
        assert_eq!(parse_empty_command("/lane"), EmptyCommand::List);
        assert_eq!(parse_empty_command("/lane list"), EmptyCommand::List);
        assert_eq!(parse_empty_command("/ctl"), EmptyCommand::List);
        // `list` 后面跟东西是手误：解析层报用法，这里原样透出。
        assert!(matches!(
            parse_empty_command("/lane list x"),
            EmptyCommand::NotHere(_)
        ));
    }

    #[test]
    fn attach_takes_a_name_and_attach_url_takes_an_address() {
        assert_eq!(
            parse_empty_command("/lane attach proj-a"),
            EmptyCommand::Attach("proj-a".to_string())
        );
        let cmd = parse_empty_command("/lane attach-url proj-a=http://127.0.0.1:8080");
        assert_eq!(
            cmd,
            EmptyCommand::AttachUrl {
                name: "proj-a".to_string(),
                url: "http://127.0.0.1:8080".to_string(),
            }
        );
        // `/connect` 与 `/lane attach-url` 是同一种写法（`--connect` 的肌肉记忆）。
        assert_eq!(
            parse_empty_command("/connect vps=https://cm.example"),
            EmptyCommand::AttachUrl {
                name: "vps".to_string(),
                url: "https://cm.example".to_string(),
            }
        );
    }

    /// 需要已连上的子命令要**说清**为什么不行，而不是落到一句泛泛的用法上。
    #[test]
    fn lane_subcommands_that_need_a_connection_explain_themselves() {
        for text in [
            "/lane new proj-b /srv/b",
            "/lane status proj-a",
            "/lane restart proj-a",
            "/lane close proj-a",
            "/lane detach",
        ] {
            match parse_empty_command(text) {
                EmptyCommand::NotHere(why) => {
                    assert!(why.contains("/lane attach"), "{text} → {why}");
                }
                other => panic!("{text} 应被拒绝：{other:?}"),
            }
        }
    }

    /// 其余命令同样要被拒绝并指路，而不是静默无效。
    #[test]
    fn other_commands_point_at_attach() {
        match parse_empty_command("/model gpt-4") {
            EmptyCommand::NotHere(why) => assert!(why.contains("/lane"), "{why}"),
            other => panic!("应拒绝：{other:?}"),
        }
    }

    /// 控制面回包与 attach 失败都要落在这块屏上：空态没有 lane，`lane_mut()` 会 panic，
    /// 所以这两条走的是外壳的 `shell_line*` 与 [`EMPTY_LANE`] 分流。
    #[test]
    fn ctl_list_and_attach_failures_land_on_the_empty_screen() {
        use std::sync::mpsc;

        use crabmate_client_api::ctl::CtlInstance;

        use super::super::test_support::test_shell_empty;
        use super::super::worker::UiEvent;
        use super::super::worker_ctl::CtlHandle;
        use super::{EMPTY_LANE, Inbound, ShellEvent};

        let (ctl_tx, _ctl_rx) = mpsc::channel();
        let mut app = test_shell_empty(Some(CtlHandle::detached(ctl_tx)));
        assert!(app.lanes.is_empty(), "这条用例的前提就是一条 lane 都没有");

        let items = vec![CtlInstance {
            name: "proj-a".to_string(),
            observed: Some("ready".to_string()),
            ..CtlInstance::default()
        }];
        app.on_msg(Inbound::FromLane(EMPTY_LANE, UiEvent::CtlList(Ok(items))));
        assert!(
            app.empty.lines.iter().any(|l| l.contains("ctl 实例 1 条")),
            "{:?}",
            app.empty.lines
        );
        assert!(
            app.empty.lines.iter().any(|l| l.contains("proj-a")),
            "每条实例各一行：{:?}",
            app.empty.lines
        );

        // attach 失败：发起者的 id 就是 `EMPTY_LANE`（还没有真 lane）。
        app.on_msg(Inbound::Shell(ShellEvent::Ready {
            from: EMPTY_LANE,
            name: "proj-b".to_string(),
            result: Err("声明已关闭（enabled = false）".to_string()),
        }));
        assert!(
            app.empty
                .lines
                .iter()
                .any(|l| l.contains("proj-b") && l.contains("enabled = false")),
            "{:?}",
            app.empty.lines
        );
        assert!(app.lanes.is_empty(), "连接失败不该凭空造出一条 lane");
    }

    /// 空态的退出：Ctrl+C / `/quit` 都置外壳的 `empty.exit`（`exit_requested` 读它）。
    #[test]
    fn quitting_the_empty_screen_sets_the_shell_exit_flag() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        use super::super::test_support::test_shell_empty;
        use super::super::worker::UiEvent;
        use super::Inbound;

        let mut app = test_shell_empty(None);
        assert!(!app.exit_requested());

        // 这里**不能**写 `KeyCode::Char('c')`：那个字符字面量会让 `lizard` 的 Rust
        // 词法在本文件里失去同步，把 `on_empty_key` 之后的整段代码并成一个函数，
        // `fn-nloc` 门禁就会误报「267 行」。`char::from(99)` 是同一个键、同样的语义，
        // 但不会踩那个坑（改回字面量之前先跑 `bash scripts/fn-nloc-ratchet.sh`）。
        let ctrl_c = KeyEvent::new(KeyCode::Char(char::from(99)), KeyModifiers::CONTROL);
        app.on_msg(Inbound::Input(UiEvent::Key(ctrl_c)));
        assert!(app.exit_requested(), "Ctrl+C 要能退出未连接界面");
    }

    /// 空态 → 第一条 lane 连上：`lanes` 不再为空，未连接界面就此让位给正常界面。
    ///
    /// 这条路径（`ShellEvent::Ready` 成功 → `attach_lane`）是空态唯一的**出口**，
    /// 失败分支有单独的用例，成功分支此前没有覆盖。
    #[test]
    fn a_successful_attach_leaves_the_unconnected_screen() {
        use super::super::test_support::test_shell_empty;

        let mut app = test_shell_empty(None);
        // 端口 1 必然连不上：装配会成功（地址合法），首拉的失败由 worker 自己收尾。
        app.attach_lane("proj-a", "http://127.0.0.1:1");
        assert_eq!(app.lanes.len(), 1, "装配出一条真 lane");
        assert_eq!(app.lane().label, "proj-a");
        assert!(
            app.lane()
                .st
                .lines
                .iter()
                .any(|l| l.text().contains("已连接")),
            "要留下「已连接」这一行"
        );
    }

    /// 没有 scheme 的地址不许造出「已连接」的假象：`ServeClient::new` 不校验 `api_base`，
    /// 放过去就是一条每个请求都失败的死 lane（错误信息还与「地址写错了」无关）。
    #[test]
    fn a_scheme_less_address_is_refused() {
        use super::super::test_support::test_shell_empty;

        let mut app = test_shell_empty(None);
        app.attach_lane("typo", "127.0.0.1:8080");
        assert!(app.lanes.is_empty(), "不许凭空造出一条死 lane");
        assert!(
            app.empty.lines.iter().any(|l| l.contains("http://")),
            "要说清地址该怎么写：{:?}",
            app.empty.lines
        );
    }
}
