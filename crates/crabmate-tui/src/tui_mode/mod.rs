//! `crabmate-tui tui`：全屏模式（M3–M4）。
//!
//! 布局：顶栏(工作区) + 左栏（会话 / Ctrl+W 工作区目录树）+ 主区流式 transcript +
//! 底栏（多行）输入 + 审批浮层 + 底部状态行。回合/拉取在持久 worker 线程的
//! current-thread tokio runtime 中串行执行（`run_chat_stream_sink` / `fetch_web_sessions`
//! / `fetch_workspace_dir` / `/status`），避免全屏事件循环与 IO 互相阻塞。raw mode 下
//! Ctrl+C 是按键事件，取消经 [`StreamCancel`] 走"外部取消"通道，不复用文本模式的
//! `ctrl_c` 信号路径。

mod approve;
mod controls;
mod md;
mod render;
mod serve_defaults;
mod state;
mod worker;
mod workspace_tree;
mod ws_sidebar;

use std::io::{self, Write};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::cursor::SetCursorStyle;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::{Backend, CrosstermBackend};

use crabmate_tui_core::{
    ApprovalDecision, ChatStreamOptions, ChatStreamOutcome, ClientLlmFields, ServeClient,
    StreamCancel, new_approval_session_id,
};

use self::approve::{decision_for_key, decision_summary};
use self::controls::{Control, clear_word, parse_control, set_mode_field, set_override_field};
use self::render::{BodyRow, SIDEBAR_MIN_WIDTH, StatusInfo, build_body_rows, chat_body_width};
use self::worker::{TurnRequest, UiEvent, WorkerJob, spawn_key_reader, spawn_worker};
use super::SessionPrefs;
use state::{Focus, LineKind, UiState};

/// body 物理行 memo 的失效键：内容代数（`content_rev`）+ 宽度 + 搜索词/锚点。
/// `view_offset`/焦点/审批等"draw 期"因素不在此列（窗口裁剪每帧实时做）。
#[derive(PartialEq, Debug)]
struct BodyMemoKey {
    width: usize,
    rev: u64,
    needle: Option<String>,
    cursor: Option<usize>,
}

fn body_memo_key(st: &UiState, terminal_width: u16) -> BodyMemoKey {
    BodyMemoKey {
        width: chat_body_width(st.sidebar_visible, terminal_width),
        rev: st.content_rev,
        needle: st.search_term().map(str::to_string),
        cursor: st.search_cursor,
    }
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
    /// 帧间 memo：内容/宽度/搜索未变时复用，避免每帧全量 tokenize+wrap。
    prepared: Vec<BodyRow>,
    body_key: Option<BodyMemoKey>,
}

impl TuiApp<'_> {
    /// 计算当前帧的 body 行：memo 命中直接复用；否则重建。
    fn prepare_body(&mut self, terminal_width: u16) {
        let key = body_memo_key(&self.st, terminal_width);
        if self.body_key.as_ref() != Some(&key) {
            self.prepared = build_body_rows(&self.st, key.width);
            self.body_key = Some(key);
        }
    }
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

/// Ctrl+W：输入焦点下进入左栏工作区目录树。
fn is_ctrl_w(key: &KeyEvent) -> bool {
    key.code == KeyCode::Char('w') && key.modifiers.contains(KeyModifiers::CONTROL)
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

    fn refresh_workspace(&mut self) {
        if self.job_tx.send(WorkerJob::RefreshWorkspace).is_err() {
            self.st
                .push_line(LineKind::System, "刷新工作区失败：worker 已退出");
        }
    }

    fn fetch_ws_dir(&mut self, rel: String) {
        if self.job_tx.send(WorkerJob::WorkspaceDir(rel)).is_err() {
            self.st
                .push_line(LineKind::System, "拉取目录失败：worker 已退出");
        }
    }

    fn fetch_ws_projects(&mut self) {
        if self.job_tx.send(WorkerJob::WsProjects).is_err() {
            self.st
                .push_line(LineKind::System, "拉取项目池失败：worker 已退出");
        }
    }

    fn switch_ws_project(&mut self, name: String) {
        if self.job_tx.send(WorkerJob::WsSwitchProject(name)).is_err() {
            self.st
                .push_line(LineKind::System, "切换项目失败：worker 已退出");
        }
    }

    /// 进入工作区目录树视图（Ctrl+W / 会话栏 w）；根列表未就绪时先拉一次。
    fn focus_workspace(&mut self) {
        if !self.st.ws_ready && !self.st.ws_root_pending {
            self.st.ws_begin_root_fetch();
            self.refresh_workspace();
        }
        self.st.focus = Focus::Workspace;
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
            "按键：Alt+Enter 换行 · Ctrl+E 思考展开/折叠 · Ctrl+W 工作区目录树 · PgUp/PgDn 翻页 · Ctrl+End 回底部",
            "工作区树：↑↓选 Enter/→展开 ◀收起/回父 r刷新 w回会话列表 Tab/Esc 回输入",
            "工作区未设置：按 p 从项目池选择（选择视图：↑↓选 Enter切换 r刷新 Esc 返回）",
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
        if self.st.focus == Focus::Workspace {
            self.on_workspace_key(key);
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
            // w：切到工作区目录树（与工作区视图内的 w 互逆）。
            KeyCode::Char('w') => self.focus_workspace(),
            _ => {}
        }
    }

    /// 工作区目录树视图按键：浏览目录层级（Enter/→ 展开，← 收起或回父目录）。
    fn on_workspace_key(&mut self, key: KeyEvent) {
        if self.st.ws_pick.is_some() {
            self.on_workspace_pick_key(key);
            return;
        }
        match key.code {
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Esc => self.st.focus = Focus::Input,
            KeyCode::Up => self.st.ws_move(true),
            KeyCode::Down => self.st.ws_move(false),
            KeyCode::Enter | KeyCode::Right => {
                if let Some(rel) = self.st.ws_toggle_dir() {
                    self.fetch_ws_dir(rel);
                }
            }
            KeyCode::Left => self.st.ws_left(),
            KeyCode::Char('r') => {
                self.st.push_line(LineKind::System, "正在刷新工作区目录…");
                self.refresh_workspace();
            }
            // w：回到左栏会话列表（w 两侧对称切换）。
            KeyCode::Char('w') => self.st.focus = Focus::Sidebar,
            // 工作区未设置（顶栏路径为空）时：p 进入项目池「选择工作区」。
            KeyCode::Char('p') if self.st.workspace_path.is_none() && self.st.ws_open_pick() => {
                self.fetch_ws_projects();
            }
            _ => {}
        }
    }

    /// 项目池「选择工作区」子视图按键。
    fn on_workspace_pick_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Tab | KeyCode::BackTab => self.st.ws_close_pick(),
            KeyCode::Up => self.st.ws_pick_move(true),
            KeyCode::Down => self.st.ws_pick_move(false),
            KeyCode::Char('r') => {
                if self.st.ws_pick_refresh() {
                    self.fetch_ws_projects();
                }
            }
            KeyCode::Enter | KeyCode::Right => {
                if let Some(name) = self.st.ws_pick_begin_switch() {
                    self.st
                        .push_line(LineKind::System, &format!("正在切换到项目「{name}」…"));
                    self.switch_ws_project(name);
                }
            }
            // w：关闭选择并回到左栏会话。
            KeyCode::Char('w') => {
                self.st.ws_close_pick();
                self.st.focus = Focus::Sidebar;
            }
            _ => {}
        }
    }

    fn on_input_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter if is_alt_enter(&key) => self.st.insert_newline(),
            KeyCode::Enter => self.on_submit(),
            KeyCode::Char('o') if is_ctrl_o(&key) => self.on_submit(),
            // Ctrl+W：宽屏时进入工作区目录树（窄屏忽略，侧栏本就隐藏）。
            KeyCode::Char('w') if is_ctrl_w(&key) && self.st.sidebar_visible => {
                self.focus_workspace();
            }
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
            UiEvent::WorkspacePath(path) => self.st.workspace_path = path,
            UiEvent::Workspace(result) => match result {
                Ok(data) => self.st.ws_root_replace(data),
                Err(e) => {
                    if self.st.ws_root_failed(&e) && self.st.focus == Focus::Workspace {
                        self.st
                            .push_line(LineKind::System, &format!("拉取工作区失败：{e}"));
                    }
                }
            },
            UiEvent::WorkspaceDir { rel, result } => match result {
                Ok(data) => self.st.ws_dir_ok(&rel, data),
                Err(e) => {
                    if self.st.ws_dir_failed(&rel) {
                        self.st.push_line(
                            LineKind::System,
                            &format!("展开 {rel} 失败：{e}（← 可重试）"),
                        );
                    }
                }
            },
            UiEvent::WsProjects(result) => self.st.ws_pick_projects(result),
            UiEvent::WsProjectSwitch(result) => self.on_ws_project_switch(result),
        }
    }

    /// 项目池切换结果：成功打印路径、关闭选择视图并刷新根列表；失败留在视图提示。
    fn on_ws_project_switch(&mut self, result: Result<String, String>) {
        if let Ok(path) = &result {
            self.st
                .push_line(LineKind::System, &format!("已切换到工作区：{path}"));
        }
        self.st.ws_pick_switch_result(result);
        if self.st.ws_pick.is_none() {
            self.refresh_workspace();
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
            if !self.st.sidebar_visible
                && matches!(self.st.focus, Focus::Sidebar | Focus::Workspace)
            {
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
            // 内容/宽度/搜索未变时复用已构建的 body 物理行，避免每帧全量 tokenize+wrap。
            self.prepare_body(size.width);
            terminal
                .draw(|f| render::draw(f, &self.st, &info, &self.prepared))
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
        prepared: Vec::new(),
        body_key: None,
    };
    terminal.clear().context("clear screen")?;
    // 启动即拉一次会话 / 工作区根 / serve 默认状态（失败以系统行提示）。
    app.refresh_sessions();
    app.st.ws_begin_root_fetch();
    app.refresh_workspace();
    app.refresh_status();
    app.run_loop(&mut terminal)
}

#[cfg(test)]
mod memo_tests {
    use super::*;
    use state::LineKind;

    #[test]
    fn memo_key_changes_on_render_inputs() {
        let mut s = UiState::new();
        let k0 = body_memo_key(&s, 120);
        s.push_line(LineKind::User, "a");
        assert_ne!(body_memo_key(&s, 120), k0, "新增行应失效");
        let k1 = body_memo_key(&s, 120);
        s.stream_delta(LineKind::Assistant, "b");
        assert_ne!(body_memo_key(&s, 120), k1, "流式追加应失效");
        let k2 = body_memo_key(&s, 120);
        s.start_search("b");
        assert_ne!(body_memo_key(&s, 120), k2, "搜索词/锚点应失效");
        let k3 = body_memo_key(&s, 120);
        s.toggle_thinking();
        assert_ne!(body_memo_key(&s, 120), k3, "折叠切换应失效");
        // draw 期因素（滚动）不应失效
        let k4 = body_memo_key(&s, 120);
        s.view_offset = 5;
        assert_eq!(body_memo_key(&s, 120), k4, "滚动不应失效");
        // 宽度变化应失效
        assert_ne!(body_memo_key(&s, 120), body_memo_key(&s, 121), "宽度应失效");
    }
}
