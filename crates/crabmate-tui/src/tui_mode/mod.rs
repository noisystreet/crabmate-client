//! `crabmate-tui tui`：全屏模式（M3）。
//!
//! 布局：状态行 + 左栏会话 + 主区流式 transcript + 底栏（多行）输入 + 审批浮层。
//! 回合/拉取在持久 worker 线程的 current-thread tokio runtime 中串行执行
//! （`run_chat_stream_sink` / `fetch_web_sessions` / `/status`），避免全屏事件循环
//! 与 IO 互相阻塞。raw mode 下 Ctrl+C 是按键事件，取消经 [`StreamCancel`] 走
//! "外部取消"通道，不复用文本模式的 `ctrl_c` 信号路径。

mod approve;
mod controls;
mod render;
mod state;

use std::io::{self, Write};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::cursor::SetCursorStyle;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::{Backend, CrosstermBackend};

use crabmate_tui_core::{
    ApprovalDecision, ApprovalGate, AutoAllowOnce, ChatStreamOptions, ChatStreamOutcome,
    ClientLlmFields, ServeClient, StreamCancel, StreamSink, TermError, WebSessionsList,
    fetch_web_sessions, new_approval_session_id, run_chat_stream_sink,
};

use self::approve::{ApprovalPrompt, OverlayApprovalGate, decision_for_key, decision_summary};
use self::controls::{Control, clear_word, parse_control, set_mode_field, set_override_field};
use self::render::{SIDEBAR_MIN_WIDTH, StatusInfo};
use super::SessionPrefs;
use state::{Focus, LineKind, ServeDefaults, UiState};

/// UI 事件（键盘线程与回合/拉取线程共同的生产者）。
enum UiEvent {
    Key(KeyEvent),
    Text(String),
    Thinking(String),
    System(String),
    ToolStart {
        tool_call_id: String,
        name: String,
    },
    ToolEnd {
        tool_call_id: String,
        name: String,
        ok: Option<bool>,
        note: Option<String>,
    },
    Approval {
        prompt: ApprovalPrompt,
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

    fn on_tool_start(&mut self, tool_call_id: &str, name: &str) -> Result<(), TermError> {
        self.tx
            .send(UiEvent::ToolStart {
                tool_call_id: tool_call_id.to_string(),
                name: name.to_string(),
            })
            .map_err(|_| TermError::Message("ui channel closed".into()))
    }

    fn on_tool_end(
        &mut self,
        tool_call_id: &str,
        name: &str,
        ok: Option<bool>,
        note: Option<&str>,
    ) -> Result<(), TermError> {
        self.tx
            .send(UiEvent::ToolEnd {
                tool_call_id: tool_call_id.to_string(),
                name: name.to_string(),
                ok,
                note: note.map(str::to_string),
            })
            .map_err(|_| TermError::Message("ui channel closed".into()))
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
            Box::new(OverlayApprovalGate { tx: tx.clone() })
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

fn is_ctrl_c(key: &KeyEvent) -> bool {
    key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)
}

/// Ctrl+C / Ctrl+D（审批浮层里的"中断式拒绝"；回合取消需在浮层关闭后再按一次）。
fn is_ctrl_abort(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('d'))
}

fn is_ctrl_o(key: &KeyEvent) -> bool {
    key.code == KeyCode::Char('o') && key.modifiers.contains(KeyModifiers::CONTROL)
}

fn is_ctrl_e(key: &KeyEvent) -> bool {
    key.code == KeyCode::Char('e') && key.modifiers.contains(KeyModifiers::CONTROL)
}

fn is_ctrl(key: &KeyEvent, code: KeyCode) -> bool {
    key.code == code && key.modifiers.contains(KeyModifiers::CONTROL)
}

fn is_alt_enter(key: &KeyEvent) -> bool {
    key.code == KeyCode::Enter && key.modifiers.contains(KeyModifiers::ALT)
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
            Control::Help => self.show_help(),
            Control::Find(arg) => self.do_find(arg),
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

    /// `/help`：向 transcript 打印本地命令与快捷键说明。
    fn show_help(&mut self) {
        for line in [
            "本地命令：/quit /help /status /mode /role /model /find /conv",
            "/mode ask|plan|act · /role <id> · /model <name>（off 清除，随对话生效）",
            "/find <词> 搜索并高亮 · /find（空参）跳下一处 · /find off 清除",
            "/conv list 刷新 · /conv use <id> 切换 · /conv new 新会话",
            "按键：Alt+Enter 换行 · Ctrl+E 思考展开/折叠 · PgUp/PgDn 翻页 · Ctrl+End 回底部",
            "审批浮层：Enter=一次 · a=始终 · Esc/n=拒绝 · Ctrl+C 先拒绝、回合随后继续需再按取消",
        ] {
            self.st.push_line(LineKind::System, line);
        }
    }

    /// `/find`：设置搜索词/跳下一处/清除（Ctrl+E 之外的本地只读命令）。
    fn do_find(&mut self, arg: Option<String>) {
        match arg {
            Some(t) if clear_word(&t) => {
                self.st.clear_search();
                self.st.push_line(LineKind::System, "搜索已清除");
            }
            Some(term) => {
                if self.st.start_search(&term) == 0 {
                    let needle = self.st.search_term().unwrap_or(&term);
                    self.st.push_line(
                        LineKind::System,
                        &format!("「{needle}」无匹配（新内容到达后可再 /find 跳转）"),
                    );
                }
            }
            None => {
                if !self.st.search_active() {
                    self.st.push_line(
                        LineKind::System,
                        "当前无搜索词：/find <词> 开始搜索（/find 空参跳下一处）",
                    );
                } else if self.st.next_search_hit().is_none() {
                    self.st.push_line(LineKind::System, "无更多匹配行");
                }
            }
        }
    }

    fn on_key(&mut self, key: KeyEvent) {
        // 审批浮层优先：只认决策键，忽略其它输入（防多重审批叠栈）。
        if self.st.approval.is_some() {
            if let Some(decision) = decision_for_key(&key) {
                let via_ctrl_abort = is_ctrl_abort(&key);
                self.answer_approval(decision, via_ctrl_abort);
            }
            return;
        }
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

    /// 关闭审批浮层并把决策回传 SSE gate；写一条结果系统行。
    /// `via_ctrl_abort`：Ctrl+C/D 触发的拒绝不等于取消回合，提示用户还需再按一次。
    fn answer_approval(&mut self, decision: ApprovalDecision, via_ctrl_abort: bool) {
        let Some(overlay) = self.st.approval.take() else {
            return;
        };
        let preview = overlay.preview();
        let _ = overlay.answer.send(decision);
        let summary = decision_summary(&decision);
        self.st
            .push_line(LineKind::System, &format!("{summary}命令审批：{preview}"));
        if matches!(decision, ApprovalDecision::Deny) && via_ctrl_abort {
            self.st.push_line(
                LineKind::System,
                "回合仍在跑：再按一次 Ctrl+C 可取消当前回合",
            );
        }
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
            KeyCode::Enter if is_alt_enter(&key) => self.st.insert_newline(),
            KeyCode::Enter => self.on_submit(),
            KeyCode::Char('o') if is_ctrl_o(&key) => self.on_submit(),
            KeyCode::Char('e') if is_ctrl_e(&key) => {
                self.st.toggle_thinking();
                let state = if self.st.thinking_visible() {
                    "展开"
                } else {
                    "折叠"
                };
                self.st.push_line(
                    LineKind::System,
                    &format!("思考内容已{state}（Ctrl+E 切换）"),
                );
            }
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
            KeyCode::Home if is_ctrl(&key, KeyCode::Home) => self.st.scroll_top(),
            KeyCode::End if is_ctrl(&key, KeyCode::End) => self.st.scroll_bottom(),
            KeyCode::Home => self.st.cursor_home(),
            KeyCode::End => self.st.cursor_end(),
            KeyCode::PageUp => self.st.scroll_page(true),
            KeyCode::PageDown => self.st.scroll_page(false),
            // 多行编辑时 ↑/↓ 走行内光标；单行仍保留"滚动 transcript"的旧行为。
            KeyCode::Up if self.st.is_multiline() => self.st.move_cursor_vert(true),
            KeyCode::Down if self.st.is_multiline() => self.st.move_cursor_vert(false),
            KeyCode::Up => self.st.scroll_lines(true, 3),
            KeyCode::Down => self.st.scroll_lines(false, 3),
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
            UiEvent::ToolStart { tool_call_id, name } => self.st.tool_start(&tool_call_id, &name),
            UiEvent::ToolEnd {
                tool_call_id,
                name,
                ok,
                note,
            } => self.st.tool_end(&tool_call_id, &name, ok, note.as_deref()),
            UiEvent::Approval { prompt } => {
                self.st.begin_approval(&prompt.req, prompt.answer);
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
        // 新回合：清上轮工具行映射并回到最新视图（搜索词保留、锚点释放）。
        self.st.reset_run_tools();
        self.st.view_offset = 0;
        self.st.release_search_anchor();
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
            view_offset: self.st.view_offset,
            search_term: self.st.search_term().map(str::to_string),
            search_total: self.st.search_total,
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
            // PgUp/PgDn 页步 ≈ 聊天可视区行数（状态行 + composer 占几行）。
            self.st.page_rows = (size.height as usize).saturating_sub(4).max(1);
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
    // 输入行只显示文本，用闪烁竖条光标提示输入位置。
    execute!(stdout, SetCursorStyle::BlinkingBar).context("set cursor style")?;
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
