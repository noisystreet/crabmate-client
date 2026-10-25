//! `crabmate-tui tui`：全屏模式（M3–M4）。
//!
//! 布局（仿 Desktop）：左栏会话 + 主区流式 transcript +
//! **右栏工作区目录树（宽屏默认显示；Ctrl+W 聚焦右栏浏览）** + 底栏（多行）输入 +
//! 审批浮层 + 底部状态栏。回合/拉取在持久 worker 线程的 current-thread tokio runtime 中
//! 串行执行（`run_chat_stream_sink` / `fetch_web_sessions` / `fetch_workspace_dir` /
//! `fetch_workspace_projects` / `/status`），避免全屏事件循环与 IO 互相阻塞。raw mode 下
//! Ctrl+C 是按键事件，取消经 [`StreamCancel`] 走"外部取消"通道，不复用文本模式的
//! `ctrl_c` 信号路径。

mod approve;
mod background;
mod background_cmd;
mod body_cache;
mod clipboard;
// `controls` / `tool_summary` 提到 `pub(crate)`：清除词与控制字符剥离在 crate 根也要用。
pub(crate) mod controls;
mod drafts;
mod event_loop;
mod exit_flush;
mod help;
mod history;
mod hotkeys;
mod layout;
mod md;
mod mouse;
mod palette;
mod paste;
mod read_jobs;
mod render;
mod resume;
mod serve_defaults;
mod session_index;
mod settings;
mod settings_panel;
mod sidebar_keys;
mod slash_menu;
mod spinner;
mod state;
mod status;
mod status_stack;
// `caps` 子命令要打印探测结果，`main` 侧需直接读（见 `term_image.rs` 头注释）。
pub(crate) mod term_image;
#[cfg(test)]
mod test_support;
mod text;
pub(crate) mod tool_summary;
mod turn_send;
mod worker;
mod workspace_cmd;
mod workspace_keys;
mod workspace_tree;
mod ws_sidebar;

use std::io;
use std::sync::mpsc::{self, Receiver, Sender};

use anyhow::{Context, Result};
use crossterm::cursor::SetCursorStyle;
use crossterm::event::{DisableBracketedPaste, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::style::Print;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::serve::{
    ApprovalDecision, ChatStreamOutcome, ServeClient, StreamCancel, StreamResume, WorkspaceDirData,
};

use self::approve::{decision_for_key, decision_summary};
use self::controls::{Control, clear_word, parse_control, set_mode_field, set_override_field};
use self::render::StatusInfo;
use self::settings::{PersistedSettings, normalize};
use self::settings_panel::SettingsPanel;
use self::worker::{UiEvent, WorkerJob, spawn_key_reader, spawn_worker};
use super::SessionPrefs;
use state::{Focus, LineKind, UiState};

/// 全屏会话的共享 UI 状态与依赖。
struct TuiApp<'a> {
    client: &'a ServeClient,
    overrides: &'a mut SessionPrefs,
    yes: bool,
    /// `--no-keyring`：设置面板里 API 密钥行禁写（也不参与 key_set 判定）。
    no_keyring: bool,
    st: UiState,
    /// 会话级输入草稿：切会话 / 新建时换入换出（见 `drafts`）。
    drafts: drafts::Drafts,
    /// 生成中 spinner 帧（按墙钟推进；见 `spinner` 模块）。
    spinner: spinner::Spinner,
    rx: Receiver<UiEvent>,
    /// UI 事件发送端：读请求 `spawn` 出去后自己回包，不经 worker 队列。
    tx: Sender<UiEvent>,
    job_tx: Sender<WorkerJob>,
    cancel: Option<StreamCancel>,
    exit: bool,
    /// `/quit`（`/exit`、`/q` 同义）请求在回合结束后退出（回合进行中先取消并等待）。
    quit_pending: bool,
    /// transcript 物理行的行级缓存（只重建变化的行，见 `body_cache`）。
    body: body_cache::BodyCache,
    /// 设置面板（`/settings` / F2 打开；`None` = 关闭）。
    panel: Option<SettingsPanel>,
    /// serve user-data 设置快照（启动拉取 / 保存成功后更新；随轮与状态行合成用）。
    persisted: Option<PersistedSettings>,
    /// 上一帧的终端尺寸：鼠标命中检测要按同一套分区重算矩形。
    last_size: (u16, u16),
    /// `/conv new` 的会话索引状态（新建 / 在途 / 待起名 / 待绑定，见 `session_index`）。
    index: session_index::UiIndexState,
    /// 可 `/resume` 的断点（serve 侧 job 可能仍在跑；保留规则见 `resume` 模块）。
    resume: Option<resume::ResumePoint>,
    /// 在途回合的原始输入：回合结束后据此决定要不要留断点（`/resume` 需重发同一轮）。
    sent_turn: Option<resume::TurnEcho>,
    /// `/background` 设置的 kitty 像素背景；`None` = 不画（默认关闭）。
    bg: Option<background::Background>,
}

impl TuiApp<'_> {
    /// 先把 body 缓存推到最新：流式期只重建被改动的那一行，其余行复用。
    fn prepare_body(&mut self, terminal_width: u16) {
        let width = render::chat_body_width(self.st.sidebar_visible, terminal_width);
        self.body.ensure(&self.st, width);
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
        // 选中的正是 `/conv new` 刚建、还没发过消息的空行：它已经是当前会话，
        // 「使用」它等于原地打转（`switch_to_conv` 还会把这条空行回收掉）。
        if session_index::selection_is_pending_blank(
            self.index.local_new_session.as_deref(),
            &self.st.sessions,
            self.st.selected,
        ) {
            self.st.push_line(
                LineKind::System,
                "这就是当前新会话（还没有 server conversation_id）：直接输入即可",
            );
            return;
        }
        let Some(resume) = self.st.selected_resume() else {
            self.st.push_line(
                LineKind::System,
                "该会话还没有 server conversation_id：先发一条消息（首轮结束会自动绑定）",
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
        // 切走前回收「/conv new 建的空行」并把服务端 active 指过去（对齐 Web reclaim）。
        self.leave_current_for_switch(&resume);
        // 断点属于上一个会话的 transcript：换会话后不可续，避免正文落到别人的记录里。
        self.drop_resume_point();
        self.st.focus = Focus::Input;
        // 草稿按会话换入换出：当前内容存回旧会话，再取目标会话那一份（见 `drafts`）。
        self.swap_draft_in(Some(&resume));
        // 切换 = 换一份 transcript（同 Desktop）：清空后回放该会话的历史。
        history::replay_into(self, resume, title);
    }

    fn refresh_sessions(&mut self) {
        if self.job_tx.send(WorkerJob::RefreshSessions).is_err() {
            self.st
                .push_line(LineKind::System, "刷新会话失败：worker 已退出");
        }
    }

    /// 拉取根目录列表；`keep` 为 true 时保留已展开目录与光标（用户手动 `r` 刷新）。
    fn refresh_workspace(&mut self, keep: bool) {
        if self
            .job_tx
            .send(WorkerJob::RefreshWorkspace { keep })
            .is_err()
        {
            // worker 已退出：回滚在途标记，否则后续 `r` 会被去重逻辑永久吞掉。
            self.st.ws_root_pending = false;
            self.st
                .push_line(LineKind::System, "刷新工作区失败：worker 已退出");
        }
    }

    /// 根目录列表结果：`keep` 时保留展开态并重拉子目录，否则整体重置。
    fn on_workspace_root(&mut self, result: Result<WorkspaceDirData, String>, keep: bool) {
        match result {
            Ok(data) => {
                let refetch = if keep {
                    self.st.ws_root_replace_keep(data)
                } else {
                    self.st.ws_root_replace(data);
                    Vec::new()
                };
                // 保留展开态时子缓存已作废，逐个重拉（worker 串行，父目录先到）。
                for rel in refetch {
                    self.fetch_ws_dir(rel);
                }
            }
            Err(e) => {
                // 手动刷新失败时树还在，`ws_root_failed` 不会置侧栏错误；必须显式提示，
                // 否则用户只看到「正在刷新…」后毫无反馈。
                let fresh = self.st.ws_root_failed(&e);
                if (fresh && self.st.focus == Focus::Workspace) || keep {
                    self.st
                        .push_line(LineKind::System, &format!("拉取工作区失败：{e}"));
                }
            }
        }
    }

    /// 聚焦右栏工作区目录树（Ctrl+W / 会话栏 w）；根列表未就绪时先拉一次。
    fn focus_workspace(&mut self) {
        if !self.st.ws_ready && !self.st.ws_root_pending {
            self.st.ws_begin_root_fetch();
            self.refresh_workspace(false);
        }
        self.st.focus = Focus::Workspace;
    }

    fn refresh_status(&mut self) {
        if self.job_tx.send(WorkerJob::RefreshStatus).is_err() {
            self.st
                .push_line(LineKind::System, "刷新状态失败：worker 已退出");
        }
    }

    /// 启动时拉取一次 user-data 设置快照（prefs + llm-overrides）→ 持久层。
    fn load_user_settings(&mut self) {
        if self.job_tx.send(WorkerJob::LoadUserSettings).is_err() {
            self.st
                .push_line(LineKind::System, "拉取设置失败：worker 已退出");
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
            // 忙时不再丢弃：文本进排队槽，回合正常结束后自动发出（Esc 清队）。
            self.queue_submit(&trimmed);
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
            Control::Background(arg) => self.set_background(arg),
            Control::Copy(arg) => self.copy_last(arg),
            Control::Workspace(arg) => self.workspace_command(arg),
            Control::Resume => self.resume_turn(),
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
            Control::Settings => self.open_settings(),
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

    /// `/help`：向 transcript 打印本地命令与快捷键说明（文案表见 [`help`]）。
    fn show_help(&mut self) {
        for line in help::LINES {
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

    /// `/copy` 与 Ctrl+Y：复制最近一条消息，结果写成一行系统提示（流程见 [`clipboard::run`]）。
    fn copy_last(&mut self, arg: Option<String>) {
        let notice = clipboard::run(&self.st.lines, arg.as_deref());
        self.st.push_line(LineKind::System, &notice);
    }

    fn on_key(&mut self, key: KeyEvent) {
        // 审批浮层优先：只认决策键，忽略其它输入（防多重审批叠栈）。
        // ←/→ 只移动选中项（`decision_for_key` 返回 None），Enter 才按选中项提交。
        if self.st.approval.is_some() {
            if let Some(decision) = decision_for_key(&key, &mut self.st.approval_choice) {
                let via_ctrl_abort = is_ctrl_abort(&key);
                self.answer_approval(decision, via_ctrl_abort);
            }
            return;
        }
        if is_ctrl_c(&key) {
            self.on_ctrl_c();
            return;
        }
        // 顶层快捷键（F6 设置 / F3 状态栈）：打包成一个判定，给这条 if 链腾出 CCN。
        if self.on_hotkey(&key) {
            return;
        }
        // 设置面板打开：按键全部交给面板（不再走 Workspace/Sidebar/输入分发）。
        if self.panel.is_some() {
            self.on_settings_key(key);
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
        // composer 的 `/` 补全菜单打开时先吃方向键 / Tab / Enter / Esc（对齐 Desktop 浮层）。
        if self.on_slash_menu_key(&key) {
            return;
        }
        self.on_input_key(key);
    }

    /// 顶层（与焦点无关）快捷键的分派：F6 设置面板 / F3 状态栈 / Ctrl+Y 复制。
    ///
    /// 判定表在 [`hotkeys`]：这里只按结果分派，不再往上面那条 if 链（和本方法的
    /// 历史实现）里加分支 —— 两者都贴着 CCN≤10 门禁。
    fn on_hotkey(&mut self, key: &KeyEvent) -> bool {
        match hotkeys::of(key) {
            Some(hotkeys::TopHotkey::Settings) => self.toggle_settings(),
            Some(hotkeys::TopHotkey::StatusStack) => self.st.toggle_status_stack(),
            Some(hotkeys::TopHotkey::Copy) => self.copy_last(None),
            None => return false,
        }
        true
    }
    /// `/` 补全菜单接管按键；返回 `true` 表示已消费。只在前缀态里成立，所以
    /// `/zzz`（无候选）之类不会把方向键从 transcript 滚动里抢走。
    fn on_slash_menu_key(&mut self, key: &KeyEvent) -> bool {
        self.st.slash_menu_open() && self.slash_menu_key(key)
    }

    /// F6 开关设置面板：关闭时若脏字段则先弹确认（Esc 同语义）。
    fn toggle_settings(&mut self) {
        if self.panel.is_none() {
            // 面板接管键盘：会话栏那次「待确认删除」留到回来再按 y 就是误删，先作废。
            self.st.confirm_delete = None;
            self.open_settings();
        } else {
            self.on_settings_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        }
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

    fn on_input_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter if is_alt_enter(&key) => self.st.insert_newline(),
            KeyCode::Enter => self.on_submit(),
            KeyCode::Char('o') if is_ctrl_o(&key) => self.on_submit(),
            // Ctrl+W：宽屏时聚焦右栏工作区目录树（窄屏忽略，侧栏本就隐藏）。
            KeyCode::Char('w') if is_ctrl_w(&key) && self.st.sidebar_visible => {
                self.focus_workspace();
            }
            KeyCode::Char('e') if is_ctrl_e(&key) => self.st.toggle_thinking_with_notice(),
            KeyCode::Char('c') | KeyCode::Char('d') | KeyCode::Char('z')
                if key.modifiers.contains(KeyModifiers::CONTROL) => {}
            // Esc：清掉排队消息（审批浮层 / `/` 菜单 / 设置面板的 Esc 在前面的层就消费了）。
            KeyCode::Esc => {
                self.clear_queued_on_esc();
            }
            KeyCode::Tab => self.focus_sidebar_on_current(),
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
        // 草稿变了：收敛 `/` 菜单的选中项与「Esc 关闭」状态，并确保 skill 候选已拉过。
        self.st.slash_after_input();
        self.maybe_fetch_skills();
    }

    fn on_turn_done(
        &mut self,
        outcome: ChatStreamOutcome,
        error: Option<String>,
        interrupted: Option<StreamResume>,
    ) {
        // 排队处置取决于这一轮怎么结束：先记标志（下面会复位 / move 走 error）；中断也算非正常。
        let cancelled = self.st.cancel_sent;
        let failed = error.is_some() || interrupted.is_some();
        self.st.running = false;
        self.st.cancel_sent = false;
        self.cancel = None;
        // 断点：只有「serve 侧 job 可能仍在跑」才留（规则与 repl 同源，见 `resume`）。
        self.record_turn_outcome(&outcome, interrupted);
        // 只读的设置面板随回合结束自动解锁（保留面板，不需重开）。
        if let Some(panel) = self.panel.as_mut() {
            panel.unlock_after_turn();
        }
        if let Some(cid) = outcome.conversation_id {
            self.st.conversation_id = Some(cid.clone());
            // `/conv new` 建的本地行还没有服务端 id：首轮结束就补上（左栏 `>` 与
            // `/conv use`、Web 端续聊都靠它）。
            self.bind_new_session(&cid);
        }
        if let Some(msg) = error {
            self.st
                .push_line(LineKind::System, &format!("回合出错：{msg}"));
        }
        // 回合后会话列表可能变化（新会话绑定 id / 标题），静默刷新。
        self.refresh_sessions();
        if self.quit_pending {
            self.exit = true;
            return;
        }
        // 忙时排队的下一句：正常结束才自动发出（取消 / 出错退回输入框，见 `drafts`）。
        self.flush_queued_after_turn(failed, cancelled);
    }

    fn on_msg(&mut self, msg: UiEvent) {
        match msg {
            UiEvent::Key(key) => self.on_key(key),
            // 粘贴整段进输入框（不改焦点、不提交）：见 `paste` 模块头注释。
            UiEvent::Paste(text) => self.on_paste(text),
            // 鼠标：右栏工作区目录树的点击/滚轮（审批凥层/设置面板打开时内部直接忽略）。
            UiEvent::Mouse(m) => self.on_mouse(m),
            // 尺寸变化只为唤醒：真值在 `draw_frame` 前现查（`sync_terminal_size`）。
            // 变体刻意不携带尺寸，所以这里没有任何可缓存的 w/h。
            UiEvent::Resize => {}
            UiEvent::Text(delta) => self.st.stream_delta(LineKind::Assistant, &delta),
            UiEvent::Skills(result) => self.st.set_skills(result),
            UiEvent::Thinking(delta) => self.st.stream_delta(LineKind::Thinking, &delta),
            UiEvent::System(line) => self.st.push_line(LineKind::System, line.trim()),
            UiEvent::ToolStart {
                tool_call_id,
                name,
                note,
            } => self.st.tool_start(&tool_call_id, &name, note.as_deref()),
            UiEvent::ToolEnd {
                tool_call_id,
                name,
                ok,
                note,
            } => self.st.tool_end(&tool_call_id, &name, ok, note.as_deref()),
            UiEvent::Approval { prompt } => {
                // 审批浮层优先级高于设置面板：面板（只读）开着时先关掉再弹审批，
                // 避免按键被面板截走导致审批悬挂。
                self.panel = None;
                // 审批期间只认决策键，会话栏的待确认删除留到审批结束就是误删。
                self.st.confirm_delete = None;
                self.st.begin_approval(&prompt.req, prompt.answer);
            }
            UiEvent::ApprovalExpired { notice } => {
                // 浮层还挂着就撤掉：它手里那条 `answer` 已断开，留着只会误导 ——
                // 用户按 Enter 会静默失败，而屏幕上仍显示「等你确认」。
                self.st.approval = None;
                self.st.push_line(LineKind::System, &notice);
            }
            UiEvent::TurnDone {
                outcome,
                error,
                interrupted,
            } => self.on_turn_done(outcome, error, interrupted),
            UiEvent::Sessions(result) => match result {
                Ok(list) => {
                    self.st.active_session_id = list.active_session_id;
                    self.st.replace_sessions(list.sessions);
                    // 待定位的新建行只能在刷新回来的列表里按 id 找（见 `session_index`）。
                    self.apply_pending_select();
                    // 启动后第一次拿到列表：回放上次在用的会话（只做一次）。
                    history::auto_replay_if_active(self);
                }
                Err(e) => {
                    self.st
                        .push_line(LineKind::System, &format!("拉取会话列表失败：{e}"));
                }
            },
            UiEvent::ConversationHistory {
                conversation_id,
                result,
            } => history::on_result(self, &conversation_id, result),
            UiEvent::SessionIndex { result, op } => self.on_session_index(result, op),
            UiEvent::Status(result) => match result {
                Ok(defaults) => self.st.serve_defaults = Some(defaults),
                Err(e) => {
                    self.st
                        .push_line(LineKind::System, &format!("拉取 serve 状态失败：{e}"));
                }
            },
            UiEvent::WorkspacePath(path) => self.st.workspace_path = path,
            UiEvent::Workspace { result, keep } => self.on_workspace_root(result, keep),
            UiEvent::WorkspaceDir {
                rel,
                generation,
                result,
            } => {
                // 代数不符 = 中途切过工作区：这条结果属于旧工作区，丢弃。否则会把 A 的
                // 目录内容写进 B 的缓存，用户随后在 B 展开同名目录会看到 A 的内容。
                if generation == self.st.ws_generation {
                    match result {
                        Ok(data) => self.st.ws_dir_ok(&rel, data),
                        Err(e) => {
                            if self.st.ws_dir_failed(&rel) {
                                self.st.push_line(
                                    LineKind::System,
                                    &format!("展开 {rel} 失败：{e}（← 可重试）"),
                                );
                            }
                        }
                    }
                }
            }
            UiEvent::WsProjects(result) => self.st.ws_pick_projects(result),
            UiEvent::WsProjectSwitch(result) => self.on_ws_project_switch(result),
            UiEvent::WorkspaceQuery(result) => self.on_workspace_query(result),
            UiEvent::WorkspaceSet(result) => self.on_workspace_set(result),
            UiEvent::UserData(result) => self.on_user_data(result),
            UiEvent::SavedLlm(result) => self.on_settings_saved_llm(result),
            UiEvent::SavedPrefs(result) => self.on_settings_saved_prefs(result),
        }
    }

    fn start_turn(&mut self, message: &str) {
        // `/conv new` 建的空行还没有名字：首条消息就是它的标题（与 Web 同规则）。
        self.retitle_new_session_for_first_turn(message);
        let opts = self.build_turn_opts(message, None);
        self.st.push_line(LineKind::User, message);
        self.dispatch_turn(opts);
    }

    /// 右栏状态栈占用的行数：**渲染与鼠标命中必须用同一份**。
    ///
    /// `draw` 是自由函数、`on_mouse` 是方法，两处各算一遍的话，行数一漂就是
    /// 「点击右栏点到了别的行」或「状态栈底部空一行」—— 都属于难归因的症状。
    pub(super) fn status_stack_rows(&self) -> u16 {
        status_stack::stack_rows(
            &self.status_info(),
            render::SIDEBAR_WIDTH.saturating_sub(2) as usize,
            self.st.status_stack_collapsed,
        )
    }

    /// 状态行显示值：override（`*`）＞ user-data 已存值 ＞ serve 默认。
    fn status_info(&self) -> StatusInfo {
        let overrides = &self.overrides;
        let stored = self.persisted.as_ref();
        let defaults = self.st.serve_defaults.as_ref();
        let show =
            |local: &Option<String>, stored: Option<&str>, remote: Option<&str>| match normalize(
                local,
            ) {
                Some(v) => Some(format!("{v}*")),
                None => match stored.map(str::trim).filter(|s| !s.is_empty()) {
                    Some(v) => Some(v.to_string()),
                    None => remote
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string),
                },
            };
        StatusInfo {
            api_base: self.client.config().api_base.clone(),
            model: show(
                &overrides.model,
                stored.and_then(|p| p.model.as_deref()),
                defaults.and_then(|d| d.model.as_deref()),
            ),
            role: show(
                &overrides.agent_role,
                stored.and_then(|p| p.role.as_deref()),
                defaults.and_then(|d| d.role.as_deref()),
            ),
            mode: show(
                &overrides.session_mode,
                stored.and_then(|p| p.session_mode.as_deref()),
                defaults.and_then(|d| d.mode.as_deref()),
            ),
            running: self.st.running,
            cancel_sent: self.st.cancel_sent,
            spinner: self.spinner.frame(),
            view_offset: self.st.view_offset,
            search_term: self.st.search_term().map(str::to_string),
            search_total: self.st.search_total,
        }
    }
}

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
const MOUSE_ON: &str = "\x1b[?1000h\x1b[?1002h\x1b[?1006h";
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
    let job_tx = spawn_worker(client.clone(), tx.clone());

    let mut app = TuiApp {
        client,
        overrides,
        yes,
        no_keyring,
        st: UiState::new(),
        drafts: drafts::Drafts::default(),
        spinner: spinner::Spinner::new(),
        rx,
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
    // 启动即拉一次会话 / 工作区根 / serve 默认状态 / user-data 设置快照（失败以系统行提示）。
    app.refresh_sessions();
    app.st.ws_begin_root_fetch();
    app.refresh_workspace(false);
    app.refresh_status();
    app.load_user_settings();
    let res = app.run_loop(&mut terminal);
    // 显式删背景图：退出备用屏幕时终端本会清空，但别把图像配额留到下一次运行。
    app.clear_background();
    // 退出落盘（有界 best-effort）：`st.running` 仍为真 = 回合中被强退，不再等它落定。
    let force = app.st.running;
    let warn = app.flush_before_exit(force).await;
    drop(guard);
    if let Some(warn) = warn {
        eprintln!("{warn}");
    }
    res
}
