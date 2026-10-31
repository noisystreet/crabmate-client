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
mod ctl;
mod drafts;
mod event_loop;
mod exit_flush;
mod help;
mod history;
mod hotkeys;
mod inbound;
mod lane_cmd;
mod lane_list;
mod layout;
mod md;
mod mouse;
mod palette;
mod pane_width;
mod paste;
mod permission_intent;
mod read_jobs;
mod render;
mod resume;
mod serve_defaults;
mod session_index;
mod settings;
mod settings_panel;
mod shell;
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
mod usage_hydrate;
mod worker;
mod worker_ctl;
mod workspace_cmd;
mod workspace_keys;
mod workspace_tree;
mod ws_sidebar;

use std::borrow::Cow;
use std::sync::mpsc::{Receiver, Sender};

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::serve::{
    ApprovalDecision, ChatStreamOutcome, ServeClient, StreamCancel, StreamResume, WorkspaceDirData,
};

use self::approve::decision_summary;
use self::controls::{Control, clear_word, parse_control, set_mode_field, set_override_field};
use self::hotkeys::{is_alt_enter, is_ctrl, is_ctrl_c, is_ctrl_e, is_ctrl_o, is_ctrl_w};
use self::inbound::{Inbound, ShellEvent};
use self::render::StatusInfo;
use self::settings::{PersistedSettings, normalize};
use self::settings_panel::SettingsPanel;
use self::shell::LaneTemplate;
use self::worker::{LaneId, UiEvent, WorkerJob};
use super::SessionPrefs;
use state::{Focus, LineKind, UiState};

// 入口 re-export：`main.rs` 调 `tui_mode::run_tui`（实现已拆到 `shell` 模块）。
pub use self::shell::run_tui;
// 连接数上限与启动校验放 `shell`（装配 lane 的那一层），`main` 只留一行调用 ——
// `main.rs` 顶着 `fn-nloc-ratchet` 的 920 行/文件上限。
pub(crate) use self::shell::check_lane_capacity;

/// 一条连接（lane）的完整 UI 状态与依赖。
///
/// 由原 `TuiApp` 整体改名而来：`tui_mode` 下近 400 处 `self.st` / `self.cancel` 等
/// 访问因此**一行都不用改**。多 lane 只在外层 [`TuiApp`] 增一层路由，见
/// `docs/design/tui_multi_server.md` §4.1。
struct LaneApp<'a> {
    /// 本条 lane 的标识：SSE 流事件按它分发（见外层 [`TuiApp::on_msg`]）。
    id: LaneId,
    /// 左栏下半 lane 列表里显示的名字。W2 用 `local`；W3 起来自 profile 名。
    label: String,
    /// 本条 lane 的连接。
    ///
    /// `Cow` 而非 `&'a ServeClient`：启动装配借 `main` 里那份（零拷贝），而
    /// `/lane attach|new` 在运行期造出来的客户端没有可借的对象，只能自己持有。
    /// 它 Deref 到 `&ServeClient`，所以其余代码一处都不用改。
    client: Cow<'a, ServeClient>,
    /// 本条 lane 自己的表达层（`/model` `/mode` `/role` `/perm` 与 CLI 初值）。
    ///
    /// **自有而非借用**：多 lane 各自一份，`/model` 只影响当前 lane；
    /// `&mut` 借用会让第二条 lane 无法构造（见 `docs/design/tui_multi_server.md` §4.1）。
    overrides: SessionPrefs,
    yes: bool,
    /// `--no-keyring`：设置面板里 API 密钥行禁写（也不参与 key_set 判定）。
    no_keyring: bool,
    st: UiState,
    /// 会话级输入草稿：切会话 / 新建时换入换出（见 `drafts`）。
    drafts: drafts::Drafts,
    /// 生成中 spinner 帧（按墙钟推进；见 `spinner` 模块）。
    spinner: spinner::Spinner,
    /// UI 事件发送端：读请求 `spawn` 出去后自己回包，不经 worker 队列。
    ///
    /// 接收端**不在这里**：所有 lane 共用一个通道，`rx` 由外层 [`TuiApp`] 独占持有，
    /// 否则多条 lane 会彼此抢消息（见 `docs/design/tui_multi_server.md` §4.2）。
    tx: Sender<Inbound>,
    job_tx: Sender<WorkerJob>,
    cancel: Option<StreamCancel>,
    /// 本条 lane 发起的**进程级**退出请求（`/quit`、第二次 Ctrl+C、`quit_pending` 落地）。
    ///
    /// 语义是「退出整个 TUI」，不是「只摘掉这条 lane」（那个用 `/lane detach`），所以
    /// 消费方是外壳：`TuiApp::exit_requested` 扫**全部** lane。只判当前 lane 会让后台
    /// lane 的请求一直悬着，直到用户切过去才突然掀掉整个 TUI（issue #338）。
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
    /// `crabmatectl` 控制面 socket 路径；`None` = 控制面未启用（`/ctl` 会说明原因）。
    ///
    /// 全局资源（一个 socket 管所有实例）却存在**每条 lane** 上：`/ctl` 由当前 lane 的
    /// worker 执行，结果写进当前 lane 的 transcript，所以按 lane 存一份读起来最自然；
    /// 各 lane 的值在构造时相同，不存在「谁改了谁」的问题。
    ctl_socket: Option<std::path::PathBuf>,
}

/// 多 lane 外壳：持有 N 条 lane 与当前聚焦的那条。
///
/// 键盘 / 鼠标输入路由给当前 lane，SSE 与回合同步结果按 `LaneId` 分发；lane 的新增
/// （`/lane attach|new`）与切换都是**外壳动作** —— 单条 lane 无权改 `lanes`，
/// 否则每条都得知道全局有几条（见 `docs/design/tui_multi_server.md` §4.2）。
struct TuiApp<'a> {
    lanes: Vec<LaneApp<'a>>,
    /// 当前聚焦的 lane 下标。W1 恒为 0。
    active: usize,
    /// 所有 lane 共用的事件通道接收端：全进程只有这一个消费者，按标签分发。
    rx: Receiver<Inbound>,
    /// 新增 lane 的装配模板（见 [`LaneTemplate`]）。
    template: LaneTemplate,
    /// 下一条 lane 的编号。**不复用**已退出 lane 的号：`LaneId` 单调递增才没有 ABA。
    next_lane_id: u32,
}

impl<'a> TuiApp<'a> {
    /// 事件入口：三种来源各走各的路（见 [`Inbound`]）。
    ///
    /// 分类由**类型**保证，不靠调用方记得包一层（issue #347）：lane 的出站事件只能经
    /// `LaneTx` 变成 [`Inbound::FromLane`]，外壳动作走 [`Inbound::Shell`]，键盘 / 鼠标 /
    /// 粘贴 / 尺寸是 [`Inbound::Input`]。
    fn on_msg(&mut self, msg: Inbound) {
        match msg {
            // 全局输入：作用于**当前** lane。切 lane 与点 lane 列表是外壳动作，必须在
            // 进入 lane 之前拦截（lane 无权切自己，否则每条都要知道全局有几条）。
            Inbound::Input(ev) => {
                if let UiEvent::Key(k) = &ev {
                    match hotkeys::of(k) {
                        Some(hotkeys::TopHotkey::NextLane) => return self.switch_lane(1),
                        Some(hotkeys::TopHotkey::PrevLane) => return self.switch_lane(-1),
                        // 跳到在等审批的那条：同样必须在外壳层做（要知道全局有几条、
                        // 谁在等），而且不能让这次按键漏进 lane 的输入框。
                        Some(hotkeys::TopHotkey::NextApproval) => {
                            return self.focus_next_approval();
                        }
                        _ => {}
                    }
                }
                // 左栏下半的 lane 列表：点一下切过去，这次点击不进入 lane。
                if let UiEvent::Mouse(m) = &ev
                    && let Some(idx) = self.lane_at_click(m)
                {
                    self.active = idx;
                    return;
                }
                self.lane_mut().on_msg(ev);
            }
            // 某条 lane 的出站事件：按 id 分发。
            Inbound::FromLane(id, ev) => {
                // 后台 lane 弹审批先在本 lane 留一句提示：浮层只画当前 lane，不说一声
                // 就是最长 600s 的静默停摆（见 issue #337）。提示归提示，事件仍要
                // 分派给**它自己**那条 lane —— 浮层建在它的 `st` 上。
                self.announce_background_approval(id, &ev);
                // 目标 lane 已关闭：丢弃，绝不把增量写进别的 transcript —— 串台比丢一条
                // 增量难查得多，而这正是 `Inbound` 存在的理由。
                if let Some(lane) = self.lanes.iter_mut().find(|l| l.id == id) {
                    lane.on_msg(ev);
                }
            }
            Inbound::Shell(ev) => self.on_shell(ev),
        }
    }

    /// 外壳动作：只有这一层能改 `lanes`（单条 lane 没有那份 `&mut`）。
    fn on_shell(&mut self, ev: ShellEvent) {
        match ev {
            // 新 lane 的装配：目标 lane 此刻还不存在，按 id 分发无处可去。
            ShellEvent::Ready { name, result } => self.on_lane_ready(name, result),
            // 摘连接：`LaneApp` 手上没有 `lanes` 的 `&mut`（见 `shell::detach_lane`）。
            ShellEvent::Detach { name } => self.detach_lane(&name),
            // 取消某条 lane 的回合：`LaneApp` 只握着自己的 `cancel`。
            ShellEvent::Cancel { name } => self.cancel_lane(&name),
            // 翻声明的回执：关掉声明后还要顺手摘掉本进程那条连接。
            ShellEvent::Toggled(result) => self.on_lane_toggled(result),
        }
    }
}

impl LaneApp<'_> {
    /// 先把 body 缓存推到最新：流式期只重建被改动的那一行，其余行复用。
    fn prepare_body(&mut self, terminal_width: u16) {
        let width = render::chat_body_width(self.st.sidebar_visible, terminal_width);
        self.body.ensure(&self.st, width);
    }
}

impl LaneApp<'_> {
    fn on_ctrl_c(&mut self) {
        // 第一次 = 取消（走统一收尾：卡在审批浮层时也立刻撤掉，否则 gate 要等满
        // `APPROVAL_WAIT` 才收尾，而屏幕早已写着「正在取消回合…」—— issue #346）；
        // 第二次 = 强退。空闲 lane 上直接退出。
        if !self.request_stop("正在取消回合…（再按一次 Ctrl+C 强退）") {
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
        // 权限意图属于**上一个**会话：切走即清（同 Desktop 的 owner 规则）。留着它等于把
        // 「为 A 选的档」静默用到 B —— 选低了会报错，选高了不会。`/mode` `/role` 不清。
        self.overrides.permission_level = None;
        // 断点属于上一个会话的 transcript：换会话后不可续，避免正文落到别人的记录里。
        self.drop_resume_point();
        self.st.focus = Focus::Input;
        // 草稿按会话换入换出：当前内容存回旧会话，再取目标会话那一份（见 `drafts`）。
        self.swap_draft_in(Some(&resume));
        // 切换 = 换一份 transcript（同 Desktop）：清空后回放该会话的历史。
        history::replay_into(self, resume, title);
        // 权限投影同样是**按会话**算的：宁可短暂不显示，也别拿上一个会话的档位撑场面。
        self.forget_permission_view();
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

    /// 拉 serve 侧事实（`/status` + `/permission`），**带当前会话**（见 `WorkerJob::RefreshStatus`）。
    pub(super) fn refresh_status(&mut self) {
        if self
            .job_tx
            .send(WorkerJob::RefreshStatus {
                conversation_id: self.st.conversation_id.clone(),
            })
            .is_err()
        {
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
                // 忙时先取消并等这一轮收尾，走与 Ctrl+C 同一个收尾入口（**撤审批浮层**
                // 也在里面 —— 少了它，卡审批的回合要等满 `APPROVAL_WAIT` 才收尾，
                // 退出就白等 600 s，issue #346）；已经在取消中则直接退出。
                let stopping = self.request_stop(
                    "正在取消回合…完成后退出整个 TUI（所有连接；只摘一条用 /lane detach，再按一次 Ctrl+C 强退）",
                );
                if !stopping {
                    self.exit = true;
                }
                if self.st.running {
                    self.quit_pending = true;
                }
            }
            Control::Help => self.show_help(),
            Control::Find(arg) => self.do_find(arg),
            Control::Background(arg) => self.set_background(arg),
            Control::Copy(arg) => self.copy_last(arg),
            Control::Ctl => self.fetch_ctl_list(),
            Control::Lane(Ok(cmd)) => self.lane_command(cmd),
            // 用法提示写进 transcript：命令在输入框里已经消失，不留痕等于什么都没发生。
            Control::Lane(Err(usage)) => self.st.push_line(LineKind::System, &usage),
            Control::Workspace(arg) => self.workspace_command(arg),
            Control::Resume => self.resume_turn(),
            Control::Older => history::load_older(self),
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
            Control::Perm(arg) => self.submit_permission_slash(arg),
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
        // 审批浮层优先：只认决策键与「看完整命令」的键（Ctrl+E 展开 / 展开后滚动），
        // 其余输入一律忽略（防多重审批叠栈）。
        if self.st.approval.is_some() {
            self.on_approval_key(&key);
            return;
        }
        if is_ctrl_c(&key) {
            self.on_ctrl_c();
            return;
        }
        // 权限档位菜单（状态行 `perm` 段点开）：模态浮层，先吃方向键 / Enter / Esc。
        if self.on_permission_menu_key(&key) {
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
            // 切 lane / 跳到待审批 lane 都是外壳动作，`TuiApp::on_msg` 已拦截，正常路径
            // 到不了这里；留着只为穷尽分支（万一有人直接调 lane 的 on_key，也按「已消费」
            // 处理，免得方向键再漏给输入框）。
            Some(
                hotkeys::TopHotkey::NextLane
                | hotkeys::TopHotkey::PrevLane
                | hotkeys::TopHotkey::NextApproval,
            ) => {}
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
            // 到顶还继续 PgUp，就是「我要看更早的」：静默续翻一页（拒绝理由不落行）。
            KeyCode::PageUp => {
                let was_at_top = self.st.body_at_top;
                self.st.scroll_page(true);
                if was_at_top {
                    history::load_older_on_top(self);
                }
            }
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
            // 收下 id；换了会话就顺带带新 id 重拉权限投影（理由见 `adopt_turn_conversation`）。
            self.adopt_turn_conversation(&cid);
            // `/conv new` 建的本地行还没有服务端 id：首轮结束就补上（左栏 `>` 与
            // `/conv use`、Web 端续聊都靠它）。
            self.bind_new_session(&cid);
            // cid 一到，新会话的 `~` 基线就不再适用（`UiState::context_text` 只认「无 cid」）：
            // 这一轮若没带实测，补一次用量水合，否则状态行会从 `~12k` 掉回占位 `—`。
            self.hydrate_usage_if_unmeasured(&cid);
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
            UiEvent::PromptTokens(snap) => self.st.set_prompt_tokens(*snap),
            // 回合末补拉的用量（`GET /conversation/messages` 自带快照）：带 cid，
            // 回包晚到（用户已切走）时丢弃，不把上个会话的数字挂到当前会话上。
            UiEvent::UsageHydrated {
                conversation_id,
                snapshot,
            } => self.on_usage_hydrated(&conversation_id, snapshot),
            // 压缩 / 裁剪后水位回落（ADR 0005）：只增不减会让数字虚高。
            UiEvent::ContextTrim(after) => self.st.reset_used(after),
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
                    // 启动后第一次拿到列表：只有 CLI 显式给了 conversation_id 才回放；
                    // 不再自动续 serve 的 active 会话（见 `history::replay_explicit_conversation`）。
                    history::replay_explicit_conversation(self);
                }
                Err(e) => {
                    self.st
                        .push_line(LineKind::System, &format!("拉取会话列表失败：{e}"));
                }
            },
            UiEvent::ConversationHistory {
                conversation_id,
                before_index,
                result,
                snapshot,
            } => history::on_result(self, &conversation_id, before_index, result, snapshot),
            UiEvent::SessionIndex { result, op } => self.on_session_index(result, op),
            UiEvent::Status {
                conversation_id,
                result,
            } => self.on_serve_status(conversation_id, result),
            UiEvent::PermissionWritten {
                level,
                outcome,
                generation,
            } => self.on_permission_written(&level, outcome, generation),
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
            UiEvent::CtlList(result) => ctl::on_ctl_list(self, result),
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
            render::WORKSPACE_PANE_WIDTH.saturating_sub(2) as usize,
            self.st.status_stack_collapsed,
        )
    }

    /// 状态行显示值：override（`*`）＞ user-data 已存值 ＞ serve 默认。
    fn status_info(&self) -> StatusInfo {
        let overrides = &self.overrides;
        let stored = self.persisted.as_ref();
        let defaults = self.st.serve_defaults.as_ref();
        // `perm` 段两侧：口径（含「无已绑定会话就不并列」）见 `perm_status_values`。
        let (perm_effective, perm_intent) = self.perm_status_values();
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
            // 上下文用量：段恒显示；没有实测时是占位 `—`，新会话则回落 `GET /status` 的
            // system 基线并带 `~`（见 `UiState::context_text`）。
            context: self.st.context_text(
                stored.and_then(|p| p.context_tokens.as_deref()),
                defaults
                    .and_then(|d| d.baseline_prompt_tokens(stored.and_then(|p| p.role.as_deref()))),
            ),
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
            // 权限等级随 `ServeDefaults` 一起来（worker 同任务拉 `/permission`）；两侧怎么取
            // 见 `perm_status_values`，不同时渲染成 `意图→生效`（与 Desktop chip 同口径）。
            permission: perm_effective,
            permission_intent: perm_intent,
            spinner: self.spinner.frame(),
            view_offset: self.st.view_offset,
            search_term: self.st.search_term().map(str::to_string),
            search_total: self.st.search_total,
        }
    }
}
