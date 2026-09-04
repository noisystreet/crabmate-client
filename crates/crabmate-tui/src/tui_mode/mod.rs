//! `crabmate-tui tui`：全屏模式入口（M1 MVP）。
//!
//! 布局：状态行 + 主区流式 transcript + 底栏单行输入。回合在独立线程的
//! current-thread tokio runtime 中执行（`run_chat_stream_sink`），避免全屏
//! 事件循环与 SSE 消费互相阻塞；raw mode 下 Ctrl+C 是按键事件，取消经
//! [`StreamCancel`] 走"外部取消"通道，不复用文本模式的 `ctrl_c` 信号路径。

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
use ratatui::backend::CrosstermBackend;

use crabmate_tui_core::{
    ApprovalDecision, ApprovalGate, AutoAllowOnce, ChatStreamOptions, ChatStreamOutcome,
    ClientLlmFields, CommandApprovalRequest, ServeClient, StreamCancel, StreamSink, TermError,
    new_approval_session_id, run_chat_stream_sink,
};

use super::SessionPrefs;
use state::{LineKind, UiState};

use self::render::StatusInfo;

/// UI 事件（键盘线程与回合线程共同的生产者）。
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

/// M1 审批 gate：非白名单命令一律拒绝，并把被拒命令通知 UI 显示。
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

/// 回合后台任务：由持久 worker 线程串行消费。
struct TurnRequest {
    opts: ChatStreamOptions,
    cancel: StreamCancel,
    yes: bool,
}

/// 持久回合 worker：单 current-thread runtime 串行执行 `/chat/stream`，
/// 避免同一 `reqwest::Client` 跨多个临时 runtime 复用的连接池问题；
/// 每回合结束发 `TurnDone`，UI 退出（job_tx drop）后 worker 自行退出。
fn spawn_turn_worker(client: ServeClient, tx: Sender<UiEvent>) -> Sender<TurnRequest> {
    let (job_tx, job_rx) = mpsc::channel::<TurnRequest>();
    thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("turn runtime");
        while let Ok(job) = job_rx.recv() {
            let result = rt.block_on(async {
                let mut sink = UiSink { tx: tx.clone() };
                let mut gate: Box<dyn ApprovalGate> = if job.yes {
                    Box::new(AutoAllowOnce)
                } else {
                    Box::new(DenyNotifyGate { tx: tx.clone() })
                };
                run_chat_stream_sink(
                    &client,
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
            if tx.send(UiEvent::TurnDone { outcome, error }).is_err() {
                break;
            }
        }
    });
    job_tx
}

/// 全屏会话的共享 UI 状态与依赖。
struct TuiApp<'a> {
    client: &'a ServeClient,
    overrides: &'a SessionPrefs,
    yes: bool,
    st: UiState,
    rx: Receiver<UiEvent>,
    job_tx: Sender<TurnRequest>,
    cancel: Option<StreamCancel>,
    exit: bool,
}

fn is_ctrl_c(key: &KeyEvent) -> bool {
    key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)
}

fn is_ctrl_o(key: &KeyEvent) -> bool {
    key.code == KeyCode::Char('o') && key.modifiers.contains(KeyModifiers::CONTROL)
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

    /// 回车 / Ctrl+O：提交输入（`/quit` `/exit` 直接退出）。
    fn on_submit(&mut self) {
        let text = self.st.current_input();
        let trimmed = text.trim().to_string();
        if trimmed.is_empty() {
            return;
        }
        if matches!(trimmed.as_str(), "/quit" | "/exit") {
            if self.st.running {
                self.st.cancel_sent = true;
                if let Some(c) = &self.cancel {
                    c.cancel();
                }
            }
            self.exit = true;
            return;
        }
        if self.st.running {
            self.st
                .push_line(LineKind::System, "回合进行中：Ctrl+C 可取消，完成后再发送");
            return;
        }
        self.start_turn(&trimmed);
    }

    fn on_scroll(&mut self, up: bool) {
        if up {
            self.st.view_offset = self.st.view_offset.saturating_add(3);
        } else {
            self.st.view_offset = self.st.view_offset.saturating_sub(3);
        }
    }

    fn on_key(&mut self, key: KeyEvent) {
        if is_ctrl_c(&key) {
            self.on_ctrl_c();
            return;
        }
        match key.code {
            KeyCode::Enter => self.on_submit(),
            KeyCode::Char('o') if is_ctrl_o(&key) => self.on_submit(),
            KeyCode::Char('c') | KeyCode::Char('d') | KeyCode::Char('z')
                if key.modifiers.contains(KeyModifiers::CONTROL) => {}
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
        self.st.take_input();
        self.st.running = true;
        self.st.cancel_sent = false;
        let cancel = StreamCancel::new();
        self.cancel = Some(cancel.clone());
        let job = TurnRequest {
            opts,
            cancel,
            yes: self.yes,
        };
        if self.job_tx.send(job).is_err() {
            self.st.running = false;
            self.st.push_line(LineKind::System, "回合 worker 已退出");
        }
    }

    fn status_info(&self) -> StatusInfo<'_> {
        StatusInfo {
            api_base: self.client.config().api_base.as_str(),
            model: non_empty(self.overrides.model.as_deref()),
            role: non_empty(self.overrides.agent_role.as_deref()),
            mode: non_empty(self.overrides.session_mode.as_deref()),
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
            let mut any = false;
            while let Ok(msg) = self.rx.try_recv() {
                any = true;
                self.on_msg(msg);
                if self.exit {
                    return Ok(());
                }
            }
            terminal
                .draw(|f| render::draw(f, &self.st, &self.status_info()))
                .context("tui draw failed")?;
            let _ = io::stdout().flush();
            thread::sleep(Duration::from_millis(if any { 2 } else { 50 }));
        }
    }
}

fn non_empty(v: Option<&str>) -> Option<&str> {
    v.map(str::trim).filter(|s| !s.is_empty())
}

/// raw mode + 备用屏幕的 RAII 复原。
struct ScreenGuard;

impl Drop for ScreenGuard {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

/// 全屏 TUI 入口（M1）：状态行 + 流式 transcript + 单行输入 + 发送/取消。
pub async fn run_tui(client: &ServeClient, overrides: &SessionPrefs, yes: bool) -> Result<()> {
    enable_raw_mode().context("enable raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).context("enter alternate screen")?;
    let _guard = ScreenGuard;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout)).context("init terminal")?;

    let (tx, rx) = mpsc::channel::<UiEvent>();
    spawn_key_reader(tx.clone());
    let job_tx = spawn_turn_worker(client.clone(), tx.clone());
    let mut app = TuiApp {
        client,
        overrides,
        yes,
        st: UiState::new(),
        rx,
        job_tx,
        cancel: None,
        exit: false,
    };
    terminal.clear().context("clear screen")?;
    app.run_loop(&mut terminal)
}
