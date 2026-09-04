//! `crabmate-tui tui`：全屏模式（M2）。
//!
//! 布局：状态行 + 左栏会话 + 主区流式 transcript + 底栏输入。
//! 回合/拉取在持久 worker 线程的 current-thread tokio runtime 中串行执行
//! （`run_chat_stream_sink` / `fetch_web_sessions` / `/status`），避免全屏事件循环
//! 与 IO 互相阻塞。raw mode 下 Ctrl+C 是按键事件，取消经 [`StreamCancel`] 走
//! "外部取消"通道，不复用文本模式的 `ctrl_c` 信号路径。

mod render;
mod state;

use std::io::{self, Write};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::{Backend, CrosstermBackend};

use crabmate_tui_core::{
    ApprovalDecision, ApprovalGate, AutoAllowOnce, ChatStreamOptions, ChatStreamOutcome,
    ClientLlmFields, CommandApprovalRequest, ServeClient, StreamCancel, StreamSink, TermError,
    WebSessionsList, fetch_web_sessions, new_approval_session_id, run_chat_stream_sink,
};

use self::render::{SIDEBAR_MIN_WIDTH, StatusInfo};
use super::SessionPrefs;
use state::{Focus, LineKind, ServeDefaults, UiState};

/// UI 事件（键盘线程与回合/拉取线程共同的生产者）。
enum UiEvent {
    Key(KeyEvent),
    Text(String),
    Thinking(String),
    System(String),
    Denied {
        command: String,
        args: String,
    },
    TurnDone {
        outcome: ChatStreamOutcome,
        error: Option<String>,
    },
    Sessions(Result<WebSessionsList, String>),
    Status(Result<ServeDefaults, String>),
}

/// 回合后台任务：由持久 worker 线程串行消费。
struct TurnRequest {
    opts: ChatStreamOptions,
    cancel: StreamCancel,
    yes: bool,
}

/// worker 任务：回合 / 会话刷新 / 状态刷新（复用同一 runtime 与连接池）。
enum WorkerJob {
    Turn(Box<TurnRequest>),
    RefreshSessions,
    RefreshStatus,
}

/// 全屏模式 sink：把流事件发往 UI 事件通道。
struct UiSink {
    tx: Sender<UiEvent>,
}

impl StreamSink for UiSink {
    fn on_text(&mut self, delta: &str) -> Result<(), TermError> {
        if !delta.is_empty() {
            self.tx
                .send(UiEvent::Text(delta.to_string()))
                .map_err(|_| TermError::Message("ui channel closed".into()))?;
        }
        Ok(())
    }

    fn on_reasoning(&mut self, delta: &str) -> Result<(), TermError> {
        if !delta.is_empty() {
            self.tx
                .send(UiEvent::Thinking(delta.to_string()))
                .map_err(|_| TermError::Message("ui channel closed".into()))?;
        }
        Ok(())
    }

    fn on_system(&mut self, line: &str) -> Result<(), TermError> {
        if !line.is_empty() {
            self.tx
                .send(UiEvent::System(line.to_string()))
                .map_err(|_| TermError::Message("ui channel closed".into()))?;
        }
        Ok(())
    }
}

/// M2 审批 gate：非白名单命令一律拒绝，并把被拒命令通知 UI 显示。
/// （真正的浮层审批属 M3；这里避免回合悬挂又给用户可见反馈。）
struct DenyNotifyGate {
    tx: Sender<UiEvent>,
}

impl ApprovalGate for DenyNotifyGate {
    fn decide(&mut self, req: &CommandApprovalRequest) -> Result<ApprovalDecision, TermError> {
        let _ = self.tx.send(UiEvent::Denied {
            command: req.command.clone(),
            args: req.args.clone(),
        });
        Ok(ApprovalDecision::Deny)
    }
}

/// 键盘读取线程：阻塞在 `event::read()`，按键转为 `UiEvent::Key`。
fn spawn_key_reader(tx: Sender<UiEvent>) {
    thread::spawn(move || {
        loop {
            match event::read() {
                Ok(Event::Key(key)) => {
                    if tx.send(UiEvent::Key(key)).is_err() {
                        break;
                    }
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });
}

/// 持久 worker：单 current-thread runtime 串行执行回合与刷新任务，
/// 避免同一 `reqwest::Client` 跨多个临时 runtime 复用的连接池问题；
/// UI 退出（job_tx drop）后 worker 自行退出。
fn spawn_worker(client: ServeClient, tx: Sender<UiEvent>) -> Sender<WorkerJob> {
    let (job_tx, job_rx) = mpsc::channel::<WorkerJob>();
    thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("worker runtime");
        while let Ok(job) = job_rx.recv() {
            match job {
                WorkerJob::Turn(job) => run_turn_job(&rt, &client, *job, &tx),
                WorkerJob::RefreshSessions => {
                    let result = rt.block_on(fetch_web_sessions(&client));
                    let event = UiEvent::Sessions(match result {
                        Ok(list) => Ok(list),
                        Err(e) => Err(e.to_string()),
                    });
                    if tx.send(event).is_err() {
                        break;
                    }
                }
                WorkerJob::RefreshStatus => {
                    let result = rt.block_on(fetch_serve_defaults(&client));
                    if tx.send(UiEvent::Status(result)).is_err() {
                        break;
                    }
                }
            }
        }
    });
    job_tx
}

fn run_turn_job(
    rt: &tokio::runtime::Runtime,
    client: &ServeClient,
    job: TurnRequest,
    tx: &Sender<UiEvent>,
) {
    let result = rt.block_on(async {
        let mut sink = UiSink { tx: tx.clone() };
        let mut gate: Box<dyn ApprovalGate> = if job.yes {
            Box::new(AutoAllowOnce)
        } else {
            Box::new(DenyNotifyGate { tx: tx.clone() })
        };
        run_chat_stream_sink(
            client,
            &job.opts,
            &mut sink,
            gate.as_mut(),
            Some(&job.cancel),
        )
        .await
    });
    let (outcome, error) = match result {
        Ok(o) => (o, None),
        Err(TermError::Interrupted) => (ChatStreamOutcome::default(), None),
        Err(e) => (ChatStreamOutcome::default(), Some(e.to_string())),
    };
    let _ = tx.send(UiEvent::TurnDone { outcome, error });
}

async fn fetch_serve_defaults(client: &ServeClient) -> Result<ServeDefaults, String> {
    let v: serde_json::Value = client
        .get_json("/status?view=shell")
        .await
        .map_err(|e| e.to_string())?;
    Ok(ServeDefaults::from_status(&v))
}

/// 全屏会话的共享 UI 状态与依赖。
struct TuiApp<'a> {
    client: &'a ServeClient,
    overrides: &'a mut SessionPrefs,
    yes: bool,
    st: UiState,
    rx: Receiver<UiEvent>,
    job_tx: Sender<WorkerJob>,
    cancel: Option<StreamCancel>,
    exit: bool,
    /// `/quit` 请求在回合结束后退出（回合进行中先取消并等待）。
    quit_pending: bool,
}

/// 控制斜杠（本地处理，不发给模型）。`/` 开头的未知命令视为普通消息。
enum Control {
    Quit,
    Model(Option<String>),
    Mode(Option<String>),
    Role(Option<String>),
    Status,
    ConvNew,
    ConvRefresh,
    ConvUse(String),
    ConvUnknown(String),
}

fn is_ctrl_c(key: &KeyEvent) -> bool {
    key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)
}

fn is_ctrl_o(key: &KeyEvent) -> bool {
    key.code == KeyCode::Char('o') && key.modifiers.contains(KeyModifiers::CONTROL)
}

fn clear_word(arg: &str) -> bool {
    matches!(arg, "off" | "none" | "clear")
}

/// 设置/查询一个字符串型 override 槽，返回给用户看的回显。
fn set_override_field(slot: &mut Option<String>, arg: Option<String>, name: &str) -> String {
    match arg {
        Some(v) if clear_word(&v) => {
            *slot = None;
            format!("{name} override cleared")
        }
        Some(v) => {
            *slot = Some(v.clone());
            format!("{name} override: {v}")
        }
        None => match slot.as_deref() {
            Some(v) if !v.trim().is_empty() => format!("{name} override: {v}"),
            _ => format!("{name} override: (none — 使用 serve 默认；/{name} <值> 设置)"),
        },
    }
}

/// 设置 `/mode` override（校验 ask/plan/act），返回给用户看的回显。
fn set_mode_field(slot: &mut Option<String>, arg: Option<String>) -> String {
    match arg {
        Some(v) if clear_word(&v) => {
            *slot = None;
            "mode override cleared".to_string()
        }
        Some(v) if matches!(v.as_str(), "ask" | "plan" | "act") => {
            *slot = Some(v.clone());
            format!("mode override: {v}")
        }
        Some(v) => format!("invalid session mode '{v}'; 可选 ask / plan / act（off 清除）"),
        None => match slot.as_deref() {
            Some(v) if !v.trim().is_empty() => format!("mode override: {v}"),
            _ => "mode override: (serve 默认；/mode ask|plan|act 设置)".to_string(),
        },
    }
}

/// 解析控制斜杠：`/model [x]` `/mode [ask|plan|act]` `/role [id]` `/status`
/// `/conv`(刷新) `/conv new` `/quit`。返回 `None` 表示按普通消息处理。
fn parse_control(text: &str) -> Option<Control> {
    let t = text.trim();
    let rest = t.strip_prefix('/')?.trim();
    let (head, arg) = match rest.split_once(char::is_whitespace) {
        Some((h, a)) => (h.trim(), a.trim().to_string()),
        None => (rest, String::new()),
    };
    let head = head.to_ascii_lowercase();
    let arg_opt = (!arg.is_empty()).then_some(arg);
    match head.as_str() {
        "quit" | "exit" | "q" => Some(Control::Quit),
        "model" => Some(Control::Model(arg_opt)),
        "mode" => Some(Control::Mode(arg_opt)),
        "role" => Some(Control::Role(arg_opt)),
        "status" => Some(Control::Status),
        "conv" => Some(match arg_opt {
            None => Control::ConvRefresh,
            Some(a) => match a.split_whitespace().next().unwrap_or("") {
                "new" | "clear" => Control::ConvNew,
                "list" | "ls" | "show" => Control::ConvRefresh,
                "use" | "switch" => match a.split_whitespace().nth(1) {
                    Some(id) if !id.is_empty() => Control::ConvUse(id.to_string()),
                    _ => Control::ConvUnknown(a),
                },
                _ => Control::ConvUnknown(a),
            },
        }),
        _ => None,
    }
}

impl TuiApp<'_> {
    fn on_ctrl_c(&mut self) {
        if !self.st.running {
            self.exit = true;
            return;
        }
        if !self.st.cancel_sent {
            self.st.cancel_sent = true;
            self.st
                .push_line(LineKind::System, "正在取消回合…（再按一次 Ctrl+C 强退）");
            if let Some(c) = &self.cancel {
                c.cancel();
            }
        } else {
            self.exit = true;
        }
    }

    fn use_selected(&mut self) {
        let Some(resume) = self.st.selected_resume() else {
            self.st.push_line(
                LineKind::System,
                "该会话尚无 server conversation_id（先在 Web 端聊一轮绑定）",
            );
            return;
        };
        let title = self
            .st
            .sessions
            .get(self.st.selected)
            .map(|r| r.title.trim())
            .filter(|s| !s.is_empty())
            .unwrap_or("(untitled)")
            .to_string();
        self.switch_to_conv(resume, Some(&title));
    }

    /// 切到指定 `conversation_id`：回合在跑时拒绝；否则清 transcript 从空开始。
    fn switch_to_conv(&mut self, resume: String, title: Option<&str>) {
        if self.st.running {
            self.st
                .push_line(LineKind::System, "回合进行中：结束后再切换会话");
            return;
        }
        self.st.conversation_id = Some(resume);
        self.st.reset_transcript();
        self.st.focus = Focus::Input;
        let label = title.unwrap_or("(by id)");
        self.st.push_line(
            LineKind::System,
            &format!("已切换到会话「{label}」（新对话从空开始）"),
        );
    }

    fn new_session(&mut self) {
        if self.st.running {
            self.st
                .push_line(LineKind::System, "回合进行中：结束后再新建会话");
            return;
        }
        self.st.conversation_id = None;
        self.st.reset_transcript();
        self.st.focus = Focus::Input;
        self.st
            .push_line(LineKind::System, "已开始新会话（conversation_id 已清空）");
    }

    fn refresh_sessions(&mut self) {
        if self.job_tx.send(WorkerJob::RefreshSessions).is_err() {
            self.st
                .push_line(LineKind::System, "刷新会话失败：worker 已退出");
        }
    }

    fn refresh_status(&mut self) {
        if self.job_tx.send(WorkerJob::RefreshStatus).is_err() {
            self.st
                .push_line(LineKind::System, "刷新状态失败：worker 已退出");
        }
    }

    /// 回车 / Ctrl+O：先按控制斜杠处理，否则提交消息（回合在跑则保留输入并提示）。
    fn on_submit(&mut self) {
        let text = self.st.current_input();
        let trimmed = text.trim().to_string();
        if trimmed.is_empty() {
            return;
        }
        if let Some(control) = parse_control(&trimmed) {
            self.st.take_input();
            self.apply_control(control);
            return;
        }
        if self.st.running {
            self.st
                .push_line(LineKind::System, "回合进行中：Ctrl+C 可取消，完成后再发送");
            return;
        }
        self.st.take_input();
        self.start_turn(&trimmed);
    }

    fn apply_control(&mut self, control: Control) {
        match control {
            Control::Quit => {
                if self.st.running {
                    if !self.st.cancel_sent {
                        self.st.cancel_sent = true;
                        if let Some(c) = &self.cancel {
                            c.cancel();
                        }
                        self.st.push_line(
                            LineKind::System,
                            "正在取消回合…完成后退出（再按一次 Ctrl+C 强退）",
                        );
                    } else {
                        self.exit = true;
                    }
                    self.quit_pending = true;
                } else {
                    self.exit = true;
                }
            }
            Control::Model(arg) => {
                let echo = set_override_field(&mut self.overrides.model, arg, "model");
                self.st.push_line(LineKind::System, &echo);
            }
            Control::Mode(arg) => {
                let echo = set_mode_field(&mut self.overrides.session_mode, arg);
                self.st.push_line(LineKind::System, &echo);
            }
            Control::Role(arg) => {
                let echo = set_override_field(&mut self.overrides.agent_role, arg, "role");
                self.st.push_line(LineKind::System, &echo);
            }
            Control::Status => {
                self.refresh_status();
                self.st.push_line(LineKind::System, "正在刷新 serve 状态…");
            }
            Control::ConvNew => {
                self.new_session();
            }
            Control::ConvRefresh => {
                self.refresh_sessions();
                self.st.push_line(LineKind::System, "正在刷新会话列表…");
            }
            Control::ConvUse(id) => {
                self.switch_to_conv(id, None);
            }
            Control::ConvUnknown(arg) => {
                self.st.push_line(
                    LineKind::System,
                    &format!("不支持的 /conv 子命令：{arg}（支持 new / list / use <id>）"),
                );
            }
        }
    }

    fn on_key(&mut self, key: KeyEvent) {
        if is_ctrl_c(&key) {
            self.on_ctrl_c();
            return;
        }
        if self.st.focus == Focus::Sidebar {
            self.on_sidebar_key(key);
            return;
        }
        self.on_input_key(key);
    }

    fn on_sidebar_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Esc => self.st.focus = Focus::Input,
            KeyCode::Up => self.st.move_selection(true),
            KeyCode::Down => self.st.move_selection(false),
            KeyCode::Enter => self.use_selected(),
            KeyCode::Char('n') => self.new_session(),
            KeyCode::Char('r') => self.refresh_sessions(),
            _ => {}
        }
    }

    fn on_input_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => self.on_submit(),
            KeyCode::Char('o') if is_ctrl_o(&key) => self.on_submit(),
            KeyCode::Char('c') | KeyCode::Char('d') | KeyCode::Char('z')
                if key.modifiers.contains(KeyModifiers::CONTROL) => {}
            KeyCode::Tab => {
                let wide_enough = self.st.sidebar_visible;
                if wide_enough && !self.st.sessions.is_empty() {
                    self.st.selected = self
                        .st
                        .sessions
                        .iter()
                        .position(|r| self.st.row_in_use(r))
                        .unwrap_or(0);
                    self.st.focus = Focus::Sidebar;
                }
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.st.insert_char(ch);
            }
            KeyCode::Backspace => self.st.backspace(),
            KeyCode::Left => self.st.cursor_left(),
            KeyCode::Right => self.st.cursor_right(),
            KeyCode::Home => self.st.cursor_home(),
            KeyCode::End => self.st.cursor_end(),
            KeyCode::Up => self.on_scroll(true),
            KeyCode::Down => self.on_scroll(false),
            _ => {}
        }
    }

    fn on_scroll(&mut self, up: bool) {
        if up {
            self.st.view_offset = self.st.view_offset.saturating_add(3);
        } else {
            self.st.view_offset = self.st.view_offset.saturating_sub(3);
        }
    }

    fn on_turn_done(&mut self, outcome: ChatStreamOutcome, error: Option<String>) {
        self.st.running = false;
        self.st.cancel_sent = false;
        self.cancel = None;
        if let Some(cid) = outcome.conversation_id {
            self.st.conversation_id = Some(cid);
        }
        if let Some(msg) = error {
            self.st
                .push_line(LineKind::System, &format!("回合出错：{msg}"));
        }
        // 回合后会话列表可能变化（新会话绑定 id / 标题），静默刷新。
        self.refresh_sessions();
        if self.quit_pending {
            self.exit = true;
        }
    }

    fn on_msg(&mut self, msg: UiEvent) {
        match msg {
            UiEvent::Key(key) => self.on_key(key),
            UiEvent::Text(delta) => self.st.stream_delta(LineKind::Assistant, &delta),
            UiEvent::Thinking(delta) => self.st.stream_delta(LineKind::Thinking, &delta),
            UiEvent::System(line) => self.st.push_line(LineKind::System, line.trim()),
            UiEvent::Denied { command, args } => {
                let preview = if args.trim().is_empty() {
                    command
                } else {
                    format!("{command} {args}")
                };
                self.st
                    .push_line(LineKind::System, &format!("已拒绝命令审批：{preview}"));
            }
            UiEvent::TurnDone { outcome, error } => self.on_turn_done(outcome, error),
            UiEvent::Sessions(result) => match result {
                Ok(list) => {
                    self.st.active_session_id = list.active_session_id;
                    self.st.replace_sessions(list.sessions);
                }
                Err(e) => {
                    self.st
                        .push_line(LineKind::System, &format!("拉取会话列表失败：{e}"));
                }
            },
            UiEvent::Status(result) => match result {
                Ok(defaults) => self.st.serve_defaults = Some(defaults),
                Err(e) => {
                    self.st
                        .push_line(LineKind::System, &format!("拉取 serve 状态失败：{e}"));
                }
            },
        }
    }

    fn start_turn(&mut self, message: &str) {
        let prefs = self.overrides.prefs();
        let opts = ChatStreamOptions {
            message: message.to_string(),
            approval_session_id: new_approval_session_id(),
            conversation_id: self.st.conversation_id.clone(),
            client_llm: prefs.client_llm.map(|cl| ClientLlmFields {
                api_key: cl.api_key.map(str::to_string),
                model: cl.model.map(str::to_string),
                api_base: cl.api_base.map(str::to_string),
            }),
            agent_role: prefs.agent_role.map(str::to_string),
            session_mode: prefs.session_mode.map(str::to_string),
            stream_resume: None,
        };
        self.st.push_line(LineKind::User, message);
        self.st.running = true;
        self.st.cancel_sent = false;
        let cancel = StreamCancel::new();
        self.cancel = Some(cancel.clone());
        let job = TurnRequest {
            opts,
            cancel,
            yes: self.yes,
        };
        if self.job_tx.send(WorkerJob::Turn(Box::new(job))).is_err() {
            self.st.running = false;
            self.st.push_line(LineKind::System, "回合 worker 已退出");
        }
    }

    /// 状态行显示值：本地 override 优先（`*` 标记），否则回退 serve 默认。
    fn status_info(&self) -> StatusInfo {
        let overrides = &self.overrides;
        let defaults = self.st.serve_defaults.as_ref();
        let effective = |local: Option<&str>, remote: Option<&String>| -> Option<String> {
            match local.filter(|s| !s.trim().is_empty()) {
                Some(v) => Some(format!("{v}*")),
                None => remote.cloned(),
            }
        };
        StatusInfo {
            api_base: self.client.config().api_base.clone(),
            model: effective(
                overrides.model.as_deref(),
                defaults.and_then(|d| d.model.as_ref()),
            ),
            role: effective(
                overrides.agent_role.as_deref(),
                defaults.and_then(|d| d.role.as_ref()),
            ),
            mode: effective(
                overrides.session_mode.as_deref(),
                defaults.and_then(|d| d.mode.as_ref()),
            ),
            running: self.st.running,
            cancel_sent: self.st.cancel_sent,
        }
    }

    /// 事件循环：drain 通道 → 渲染 → 节流 sleep。
    fn run_loop(&mut self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
        loop {
            if self.exit {
                return Ok(());
            }
            // 每帧读取终端尺寸：窄屏隐藏左栏；若焦点滞留已隐藏的左栏则收回输入框。
            let size = terminal.backend().size().context("terminal size")?;
            self.st.sidebar_visible = size.width >= SIDEBAR_MIN_WIDTH;
            if !self.st.sidebar_visible && self.st.focus == Focus::Sidebar {
                self.st.focus = Focus::Input;
            }
            let mut any = false;
            while let Ok(msg) = self.rx.try_recv() {
                any = true;
                self.on_msg(msg);
                if self.exit {
                    return Ok(());
                }
            }
            let info = self.status_info();
            terminal
                .draw(|f| render::draw(f, &self.st, &info))
                .context("tui draw failed")?;
            let _ = io::stdout().flush();
            thread::sleep(Duration::from_millis(if any { 2 } else { 50 }));
        }
    }
}

/// raw mode + 备用屏幕的 RAII 复原。
struct ScreenGuard;

impl Drop for ScreenGuard {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

/// 全屏 TUI 入口（M2）：状态行 + 左栏会话 + 流式 transcript + 单行输入。
pub async fn run_tui(client: &ServeClient, overrides: &mut SessionPrefs, yes: bool) -> Result<()> {
    enable_raw_mode().context("enable raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).context("enter alternate screen")?;
    let _guard = ScreenGuard;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout)).context("init terminal")?;

    let (tx, rx) = mpsc::channel::<UiEvent>();
    spawn_key_reader(tx.clone());
    let job_tx = spawn_worker(client.clone(), tx.clone());
    // 启动即拉一次会话与 serve 默认状态（失败以系统行提示）。
    let _ = job_tx.send(WorkerJob::RefreshSessions);
    let _ = job_tx.send(WorkerJob::RefreshStatus);

    let mut app = TuiApp {
        client,
        overrides,
        yes,
        st: UiState::new(),
        rx,
        job_tx,
        cancel: None,
        exit: false,
        quit_pending: false,
    };
    terminal.clear().context("clear screen")?;
    app.run_loop(&mut terminal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_control_known_slashes() {
        assert!(matches!(parse_control("/quit"), Some(Control::Quit)));
        assert!(matches!(parse_control("/EXIT"), Some(Control::Quit)));
        assert!(matches!(parse_control("/status"), Some(Control::Status)));
        assert!(matches!(parse_control("/conv"), Some(Control::ConvRefresh)));
        assert!(matches!(parse_control("/conv new"), Some(Control::ConvNew)));
        assert!(matches!(
            parse_control("/model gpt-x"),
            Some(Control::Model(Some(v))) if v == "gpt-x"
        ));
        assert!(matches!(
            parse_control("/model"),
            Some(Control::Model(None))
        ));
        assert!(matches!(
            parse_control("/mode act"),
            Some(Control::Mode(Some(v))) if v == "act"
        ));
        assert!(matches!(
            parse_control("/role coder"),
            Some(Control::Role(Some(v))) if v == "coder"
        ));
    }

    #[test]
    fn parse_control_conv_use_and_unknown() {
        assert!(matches!(
            parse_control("/conv use c1"),
            Some(Control::ConvUse(v)) if v == "c1"
        ));
        assert!(matches!(
            parse_control("/conv bogus"),
            Some(Control::ConvUnknown(_))
        ));
        assert!(matches!(
            parse_control("/conv use"),
            Some(Control::ConvUnknown(_))
        ));
    }

    #[test]
    fn parse_control_passes_plain_through() {
        assert!(parse_control("hello").is_none());
        assert!(parse_control("/my-skill").is_none());
        assert!(parse_control(" /model ").is_some());
    }

    #[test]
    fn override_field_set_clear_query() {
        let mut slot = None;
        let echo = set_override_field(&mut slot, Some("gpt-x".into()), "model");
        assert!(echo.contains("gpt-x"));
        assert_eq!(slot.as_deref(), Some("gpt-x"));
        let echo = set_override_field(&mut slot, Some("off".into()), "model");
        assert!(echo.contains("cleared"));
        assert!(slot.is_none());
        let echo = set_override_field(&mut slot, None, "model");
        assert!(echo.contains("(none"));
    }

    #[test]
    fn mode_field_validates() {
        let mut slot = None;
        let bad = set_mode_field(&mut slot, Some("bogus".into()));
        assert!(bad.contains("invalid"));
        assert!(slot.is_none());
        let ok = set_mode_field(&mut slot, Some("plan".into()));
        assert!(ok.contains("plan"));
        assert_eq!(slot.as_deref(), Some("plan"));
    }
}
