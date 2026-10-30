//! 全屏 TUI 的 worker 面：键盘事件线程、回合/拉取持久 worker（单 current-thread
//! runtime 串行执行，避免 reqwest 连接池跨多个临时 runtime 复用问题）以及 UI 事件。
//!
//! 拆分出本模块以控制 `mod.rs` 单文件行数（fn-nloc 门禁 ≤ 920）与函数复杂度。

use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::thread;

use crossterm::event::{self, Event, KeyEvent, MouseButton, MouseEvent, MouseEventKind};

use crabmate_client_api::conversation_state::ConversationStateOutcome;
use crabmate_client_api::ctl::CtlInstance;
use crabmate_client_api::paths;
use crabmate_client_api::permission::PermissionView;
use crabmate_client_api::prompt_tokens::TiktokenPromptTokensSnapshot;

use crate::serve::{
    ApprovalGate, AutoAllowOnce, ChatStreamOptions, ChatStreamOutcome,
    ConversationMessagesResponse, ServeClient, SessionIndexApplied, SessionIndexOp, StreamCancel,
    StreamResume, StreamSink, TermError, WebSessionsList, WorkspaceDirData, WorkspaceProjectsData,
    WorkspaceSetError, apply_web_session_op, fetch_conversation_messages, fetch_llm_overrides,
    fetch_user_data_prefs, fetch_web_sessions, fetch_workspace, fetch_workspace_dir,
    fetch_workspace_projects, put_llm_overrides, put_user_data_prefs, run_chat_stream_sink,
    set_workspace, switch_workspace_project, write_permission_level,
};

use super::approve::{APPROVAL_WAIT, ApprovalPrompt, OverlayApprovalGate};
// 控制面（unix socket）的三类 job 在 `worker_ctl`：它们不碰 reqwest 连接池。
use super::serve_defaults::ServeDefaults;
use super::settings::{LlmSave, PersistedSettings, PrefsSave, merge_llm_save, merge_prefs_save};
use super::slash_menu::SkillsPayload;
use super::worker_ctl::{job_ctl_attach, job_ctl_list, job_ctl_new};

/// UI 事件（键盘线程与回合/拉取线程共同的生产者）。
pub(super) enum UiEvent {
    Key(KeyEvent),
    /// 括号粘贴（bracketed paste）：终端把一次粘贴包成一个事件，**含其中的换行**。
    ///
    /// 与 [`UiEvent::Key`] 分开是因为换行在这条路径上不该被当成提交
    /// （`on_input_key` 里 `KeyCode::Enter => on_submit`）：多行文本要整段进输入框。
    Paste(String),
    /// 鼠标事件（目前只用于右栏工作区目录树的点击/滚轮，见 `mouse.rs`）。
    Mouse(MouseEvent),
    /// 终端尺寸变化：**只作唤醒源，刻意不携带尺寸**。
    /// 真值永远在 draw 前 ioctl 现查（见 `event_loop` 的 `sync_terminal_size`）——
    /// 事件里放一份 w/h 只会诱发「缓存下来省一次 ioctl」这类漏 resize bug。
    Resize,
    Text(String),
    Thinking(String),
    System(String),
    ToolStart {
        tool_call_id: String,
        name: String,
        /// 事件自带的「要执行什么」（`summary` / `argsPreview`）；没有则为空。
        note: Option<String>,
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
        /// 传输中断且已拿到 job 句柄时的续流点（`TermError::InterruptedStream`）；
        /// 由 worker 在错误变成字符串之前取出，供 `/resume` 使用。
        interrupted: Option<StreamResume>,
    },
    Sessions(Result<WebSessionsList, String>),
    /// 上下文用量快照（`tiktokenPromptTokens`（`RUN_FINISHED` / `conversation_saved`））：更新状态行上下文段与会话内水位。
    ///
    /// `Box` 是因为它比其它变体大一个量级（7 个 `Option` + 2 个 `String`），没必要让每个
    /// `UiEvent` 都撑到那个尺寸。
    PromptTokens(Box<TiktokenPromptTokensSnapshot>),
    /// 上下文用量**回落**（`timeline_log` 的 `kind=context_trim`）：压缩 / 裁剪后的用量。
    ContextTrim(u32),
    /// 回合末补的用量快照（`GET /conversation/messages` 的 `tiktoken_prompt_tokens`）。
    ///
    /// 与 [`UiEvent::PromptTokens`] 同口径，但**带 `conversation_id`**：这条回包是异步的，
    /// 到达时用户可能已经切走 —— 那个数字属于上一个会话，不能落到当前会话的水位上。
    UsageHydrated {
        conversation_id: String,
        snapshot: Option<TiktokenPromptTokensSnapshot>,
    },
    /// 已持久化会话的历史分页（`GET /conversation/messages`）：只读回放。
    /// 失败只提示，不阻断输入（当前会话仍可继续聊）。
    ConversationHistory {
        conversation_id: String,
        result: Result<ConversationMessagesResponse, String>,
        /// 水合响应自带的用量快照：随回包一起走 `should_apply_reply` 门控
        /// （回包被丢弃时快照也不该生效，与 Web 端同口径）。
        snapshot: Option<TiktokenPromptTokensSnapshot>,
        /// `None` = 最新页（切换会话回放）；`Some` = `/older` 前插页，值即请求用的游标。
        before_index: Option<u32>,
    },
    /// 会话索引写入（`/conv new` 新建或复用空行 / 首轮后绑定 `conversation_id` / 切走回收）结果。
    SessionIndex {
        result: Result<SessionIndexApplied, String>,
        op: SessionIndexOp,
    },
    /// serve 侧事实（`/status?view=shell` + `/permission`）。
    ///
    /// **带 `conversation_id`**：请求是带着发起时的会话走的（`/permission` 按会话算），回包
    /// 却是异步的 —— 到达时用户可能已经切走或 `/conv new`，那份投影属于上一个会话，落到
    /// 新会话的状态行就是「显示别人的档位」。收口在 [`LaneApp::on_serve_status`]。
    Status {
        conversation_id: Option<String>,
        result: Result<ServeDefaults, String>,
    },
    /// 权限等级意图落盘结果（`POST /conversation/state`）。
    ///
    /// 分两层不是冗余：`Err` = 请求**根本没拿到响应**（建请求 / 网络 / 读 body），
    /// `Ok(outcome)` = 拿到了 HTTP 响应，按共享层分类。合并会凭空造出一个不存在的状态码，
    /// 也就分不清「服务端拒绝了」与「压根没连上」。
    PermissionWritten {
        level: String,
        outcome: Result<ConversationStateOutcome, String>,
        /// 发起时的代数：与 `UiState::permission_generation` 不等即丢弃（连选两档时只有
        /// 最后一次落地，否则慢的那个回包会把界面刷回旧档）。
        generation: u64,
    },
    /// 工作区根路径（`GET /workspace`，随会话刷新顺带拉取；只更新右栏标题，不动目录树）。
    WorkspacePath(Option<String>),
    /// 工作区根目录列表（`GET /workspace`）：路径给右栏标题，entries 给目录树。
    /// `keep` 区分用户手动刷新（保留展开态）与首次/切换工作区（重置）。
    Workspace {
        result: Result<WorkspaceDirData, String>,
        keep: bool,
    },
    /// 已展开子目录的列表（`GET /workspace?path=`）结果。
    WorkspaceDir {
        rel: String,
        /// 发起时的 [`UiState::ws_generation`]：回包时代数已变 = 中途切过工作区，
        /// 这条结果属于旧工作区，必须丢弃（否则会把 A 的目录内容写进 B 的缓存）。
        generation: u64,
        result: Result<WorkspaceDirData, String>,
    },
    /// 审批等待超时（[`APPROVAL_WAIT`]）。
    ///
    /// UI 必须据此撤掉浮层：否则它继续画着一个早已按「拒绝」处理掉的请求，
    /// 用户按 Enter 时那条 `answer` 早已断开，静默失败、屏幕上什么都不说。
    ApprovalExpired {
        notice: String,
    },
    /// 项目池列表（`GET /workspace/projects`）。
    WsProjects(Result<WorkspaceProjectsData, String>),
    /// 项目切换（`POST /workspace/projects`）结果：成功为 serve 端工作区路径。
    WsProjectSwitch(Result<String, String>),
    /// `/workspace`（空参）查询结果：serve 当前工作区根。
    WorkspaceQuery(Result<String, String>),
    /// `/workspace <路径>` 设置结果：成功为生效路径（空串＝serve 默认根）。
    ///
    /// 失败带 `kind`（不是裸字符串）：409 `WORKSPACE_ALREADY_SET`（一实例一工作区）
    /// 要能提示「另起 lane」，而不是被当成普通网络错误让人重试。
    WorkspaceSet(Result<String, WorkspaceSetError>),
    /// `crabmatectl` 控制面的实例列表（unix socket 上的 `{"op":"list"}`）。
    ///
    /// 失败给**已经面向用户**的文案（`CtlCallError::user_message`）：控制面的三类失败
    /// （连不上 / 读写失败 / 被拒绝）处置各不相同，压成一个裸字符串再让 UI 重新猜分类
    /// 没有意义 —— 分类留在 `serve::ctl`，UI 只负责显示。
    CtlList(Result<Vec<CtlInstance>, String>),
    /// 运行期新增 lane 的**外壳级**结果：`Ok(base_url)` = 可以连。
    ///
    /// 与 [`UiEvent::CtlList`] 不同，这条**不带 lane 标签**（worker 直接写通道，不经
    /// [`UiSink`]）：新增 lane 改的是 `TuiApp::lanes`，而目标 lane 此刻还不存在，
    /// 按 `LaneId` 分发只会被当成「已关闭的 lane」丢掉。
    CtlLaneReady {
        /// 实例名（将来那条 lane 的 `label`）。
        name: String,
        /// `Ok(serve 根 URL)` / `Err(面向用户的失败原因)`。
        result: Result<String, String>,
    },
    /// 摘掉本进程的一条连接（`/lane detach <name>`）：**外壳动作**，不动声明与进程。
    ///
    /// 与 [`UiEvent::CtlLaneReady`] 同路且同理：改的是 `TuiApp::lanes`，单条 lane 没有
    /// 那份 `&mut`；而且被摘的可能正是发起命令的那条 lane，按 `LaneId` 分发会落空。
    LaneDetach {
        /// 要摘掉的那条连接的 `label`。
        name: String,
    },
    /// 启动时 user-data 设置快照（prefs + llm-overrides）拉取结果（合成持久层）。
    UserData(Result<PersistedSettings, String>),
    /// `llm-overrides` 保存结果：`Ok` 回传已应用动作（面板据此清 staged）。
    SavedLlm(Result<LlmSave, String>),
    /// `prefs` 保存结果：`Ok` 回传已应用动作（面板据此清 staged）。
    SavedPrefs(Result<PrefsSave, String>),
    /// skill 目录（`GET /skills`，供 `/` 补全菜单的 skill 候选）。
    Skills(Result<SkillsPayload, String>),
    /// 带来源 lane 的事件：SSE 流增量与回合结果都由某条 lane 产生。
    ///
    /// `Box` 只为打破递归类型；解包集中在外层 `TuiApp::on_msg` 一处完成，
    /// lane 内的业务 `match` 仍按裸变体写（见 `docs/design/tui_multi_server.md` §4.2）。
    Lane(LaneId, Box<UiEvent>),
}

/// 一条 lane（连接）的稳定标识：单调递增、不回收（避免 ABA）。
///
/// **裸变体 = 全局输入**：键盘 / 鼠标 / 粘贴 / 尺寸从不带标签（`spawn_key_reader`
/// 直接发裸 `UiEvent`），由外层路由给当前 lane。只有 SSE 流与回合结果带标签。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct LaneId(pub(super) u32);

/// 回合后台任务：由持久 worker 线程串行消费。
pub(super) struct TurnRequest {
    pub opts: ChatStreamOptions,
    pub cancel: StreamCancel,
    pub yes: bool,
}

/// worker 任务：回合 / 会话与工作区刷新 / 状态刷新（复用同一 runtime 与连接池）。
pub(super) enum WorkerJob {
    Turn(Box<TurnRequest>),
    RefreshSessions,
    /// 拉一页已持久化会话历史（只读回放）；`before_index = None` 取最新页。
    ConversationHistory {
        conversation_id: String,
        before_index: Option<u32>,
    },
    /// 回合末补一次用量快照：只取 `GET /conversation/messages` 的 `tiktoken_prompt_tokens`，
    /// 消息正文丢弃（回放另有 [`WorkerJob::ConversationHistory`]）。
    ///
    /// 流帧 `tiktokenPromptTokens` 不一定每轮都来；缺了这一轮，状态行会从新会话基线的
    /// `~12k` 掉回占位 `—`。Desktop 用同一招兜底（流收尾后 `bump_session_hydrate_nonce`）。
    RefreshUsage {
        conversation_id: String,
    },
    /// 写会话索引（`GET` → 改 → 全量 `PUT`）：新建空白行 / 绑定 `conversation_id` /
    /// 删除行（删除时会连服务端对话一起清，见 `apply_web_session_op`）。
    SessionIndex(SessionIndexOp),
    /// 拉取根目录列表；`keep` 为 true 时保留展开态与光标（用户手动刷新）。
    RefreshWorkspace {
        keep: bool,
    },
    /// 展开目录 → 拉取该目录子列表。
    WorkspaceDir {
        rel: String,
        generation: u64,
    },
    /// 拉 serve 侧事实（`/status?view=shell` + `/permission`）。
    ///
    /// **带 `conversation_id`**：`GET /permission` 的生效档是按会话算的（会话列里的意图 +
    /// min 链），裸请求回的是「配置默认档 + 无会话意图」。不带 id 就会在状态行显示一个
    /// 与实际生效档无关的值 —— 实测同一台 serve：裸请求 `approved/config`，带 id
    /// `unrestricted/persisted`。
    RefreshStatus {
        conversation_id: Option<String>,
    },
    /// 项目池列表（`GET /workspace/projects`）。
    WsProjects,
    /// 切换项目池内项目（`POST /workspace/projects`）。
    WsSwitchProject(String),
    /// 查询当前工作区根（`GET /workspace`）：`/workspace` 空参。
    WorkspaceQuery,
    /// 按路径设置工作区根（`POST /workspace`，空串＝恢复 serve 默认）：`/workspace <路径>`。
    WorkspaceSet(String),
    /// 拉取 user-data 设置快照（prefs + llm-overrides）→ 持久层。
    LoadUserSettings,
    /// 保存 llm-overrides：先 GET 再改 `client_llm.{model,api_base}` 再全量 PUT。
    SaveLlm(LlmSave),
    /// 保存 prefs：先 GET 再改 `cm_role/session_mode` 再全量 PUT。
    SavePrefs(PrefsSave),
    /// 拉 skill 目录（`GET /skills`）：只给 `/` 补全菜单用。
    Skills,
    /// 拉 `crabmatectl` 控制面的实例列表（unix socket，不是 HTTP）。
    ///
    /// 跑在**当前 lane 的 worker** 上而不是另起线程：`/ctl` 是低频手动命令，串行执行
    /// 换来「谁执行就写进谁的 transcript」的自然语义；代价是最坏阻塞该 lane 5 s
    /// （`serve::ctl` 的 IO 超时），期间该 lane 的其它拉取排队 —— 可接受。
    CtlList {
        /// 控制面 socket 路径（`--ctl-socket` 或默认路径）。
        socket: PathBuf,
    },
    /// 连上控制面里**已就绪**的实例（`/lane attach <name>`）：只读 `list` 取 `base_url`。
    CtlAttach {
        /// 控制面 socket 路径。
        socket: PathBuf,
        /// 实例名（`instances.toml` 的键，也是将来那条 lane 的 `label`）。
        name: String,
    },
    /// 新增声明并等它就绪（`/lane new <name> [workspace]`）。
    ///
    /// `add` 只写盘，真正起实例是 ctl 主循环的下一轮收敛，所以这里必须自己轮询
    /// `list` 才拿得到 `base_url`（`add` 的响应里没有端口）。轮询会**阻塞本 lane 的
    /// worker** 至多 [`LANE_READY_TIMEOUT`]：期间该 lane 的其它拉取排队。
    CtlNew {
        /// 控制面 socket 路径。
        socket: PathBuf,
        /// 新实例名。
        name: String,
        /// 声明的 `workspace`（Server 的 socket `add` 要求必填）。
        workspace: String,
    },
    /// 写会话级权限等级意图（`POST /conversation/state`，空闲态落盘）。
    ///
    /// 与 `/perm` 写下的本地 override 不是一回事：override 只影响**下一条** `/chat`，
    /// 退出进程就没了；这里落库后，下次进同一个会话仍是这一档（Desktop chip 同语义）。
    WritePermission {
        conversation_id: String,
        level: String,
        /// 发起时的代数：连选两档时只有最后一次落地（同 Desktop chip 的 `generation`）。
        generation: u64,
    },
}

/// 全屏模式 sink：把流事件发往 UI 事件通道，并标注来源 lane。
struct UiSink {
    /// 产生这些流事件的 lane。UI 侧据此把增量写进对应的 transcript。
    lane: LaneId,
    tx: Sender<UiEvent>,
}

impl UiSink {
    /// 所有出站事件都带 lane 标签；通道关闭即视为 UI 已退出。
    fn emit(&self, ev: UiEvent) -> Result<(), TermError> {
        self.tx
            .send(UiEvent::Lane(self.lane, Box::new(ev)))
            .map_err(|_| TermError::Message("ui channel closed".into()))
    }
}

impl StreamSink for UiSink {
    fn on_text(&mut self, delta: &str) -> Result<(), TermError> {
        if !delta.is_empty() {
            self.emit(UiEvent::Text(delta.to_string()))?;
        }
        Ok(())
    }

    fn on_reasoning(&mut self, delta: &str) -> Result<(), TermError> {
        if !delta.is_empty() {
            self.emit(UiEvent::Thinking(delta.to_string()))?;
        }
        Ok(())
    }

    fn on_system(&mut self, line: &str) -> Result<(), TermError> {
        if !line.is_empty() {
            self.emit(UiEvent::System(line.to_string()))?;
        }
        Ok(())
    }

    fn on_context_trim(&mut self, after: u32) -> Result<(), TermError> {
        self.emit(UiEvent::ContextTrim(after))?;
        Ok(())
    }

    fn on_prompt_tokens(&mut self, snap: &TiktokenPromptTokensSnapshot) -> Result<(), TermError> {
        self.emit(UiEvent::PromptTokens(Box::new(snap.clone())))
    }

    fn on_tool_start(
        &mut self,
        tool_call_id: &str,
        name: &str,
        note: Option<&str>,
    ) -> Result<(), TermError> {
        self.emit(UiEvent::ToolStart {
            tool_call_id: tool_call_id.to_string(),
            name: name.to_string(),
            note: note.map(str::to_string),
        })
    }

    fn on_tool_end(
        &mut self,
        tool_call_id: &str,
        name: &str,
        ok: Option<bool>,
        note: Option<&str>,
    ) -> Result<(), TermError> {
        self.emit(UiEvent::ToolEnd {
            tool_call_id: tool_call_id.to_string(),
            name: name.to_string(),
            ok,
            note: note.map(str::to_string),
        })
    }
}

/// 键盘/鼠标读取线程：阻塞在 `event::read()`，按键与鼠标事件转为 `UiEvent`。
pub(super) fn spawn_key_reader(tx: Sender<UiEvent>) {
    thread::spawn(move || {
        loop {
            match event::read() {
                Ok(Event::Key(key)) => {
                    if tx.send(UiEvent::Key(key)).is_err() {
                        break;
                    }
                }
                // 括号粘贴：整段粘贴是一个事件（换行也在里面），交给输入框一次性插入。
                Ok(Event::Paste(text)) => {
                    if tx.send(UiEvent::Paste(text)).is_err() {
                        break;
                    }
                }
                // 只转发会被处理的那几类：捕获鼠标后终端会持续上报指针移动/拖动，
                // 这类事件每条进 channel 都会让主循环把节流降到 2ms 白刷整帧。
                Ok(Event::Mouse(m)) if is_actionable_mouse(m.kind) => {
                    if tx.send(UiEvent::Mouse(m)).is_err() {
                        break;
                    }
                }
                // 尺寸变化只作唤醒源：真值在 `draw_frame` 前现查（`sync_terminal_size`）。
                Ok(Event::Resize(..)) => {
                    if tx.send(UiEvent::Resize).is_err() {
                        break;
                    }
                }
                Ok(Event::Mouse(_)) => {}
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });
}

/// 鼠标事件里真正会被 `on_mouse` 处理的那几类；其余在读取线程就丢弃。
///
/// 捕获鼠标后终端会持续上报指针移动（`Moved`）与拖动（`Drag`），而 `run_loop`
/// 每收到一条消息就把节流从 50ms 降到 2ms 并整帧重绘——这类事件一条也不该进去。
pub(super) fn is_actionable_mouse(kind: MouseEventKind) -> bool {
    matches!(
        kind,
        MouseEventKind::Down(MouseButton::Left)
            | MouseEventKind::ScrollUp
            | MouseEventKind::ScrollDown
    )
}

/// 持久 worker：单 current-thread runtime 串行执行回合与刷新任务，
/// 避免同一 `reqwest::Client` 跨多个临时 runtime 复用的连接池问题；
/// UI 退出（job_tx drop）后 worker 自行退出。
pub(super) fn spawn_worker(
    lane: LaneId,
    client: ServeClient,
    tx: Sender<UiEvent>,
) -> Sender<WorkerJob> {
    let (job_tx, job_rx) = mpsc::channel::<WorkerJob>();
    thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("worker runtime");
        while let Ok(job) = job_rx.recv() {
            match job {
                WorkerJob::Turn(job) => run_turn_job(&rt, &client, *job, &tx, lane),
                WorkerJob::RefreshSessions => job_refresh_sessions(&rt, &client, &tx),
                WorkerJob::ConversationHistory {
                    conversation_id,
                    before_index,
                } => job_conversation_history(&rt, &client, &conversation_id, before_index, &tx),
                WorkerJob::RefreshUsage { conversation_id } => {
                    job_refresh_usage(&rt, &client, &conversation_id, &tx);
                }
                WorkerJob::SessionIndex(op) => job_session_index(&rt, &client, op, &tx),
                WorkerJob::RefreshWorkspace { keep } => {
                    job_refresh_workspace_root(&rt, &client, &tx, keep);
                }
                WorkerJob::WorkspaceDir { rel, generation } => {
                    job_workspace_dir(&rt, &client, &rel, generation, &tx);
                }
                WorkerJob::RefreshStatus { conversation_id } => {
                    job_refresh_status(&rt, &client, conversation_id.as_deref(), &tx)
                }
                WorkerJob::WsProjects => job_ws_projects(&rt, &client, &tx),
                WorkerJob::WsSwitchProject(name) => job_ws_switch_project(&rt, &client, &name, &tx),
                WorkerJob::WorkspaceQuery => job_workspace_query(&rt, &client, &tx),
                WorkerJob::WorkspaceSet(path) => job_workspace_set(&rt, &client, &path, &tx),
                WorkerJob::LoadUserSettings => job_load_user_settings(&rt, &client, &tx),
                WorkerJob::SaveLlm(save) => job_save_llm(&rt, &client, save, &tx),
                WorkerJob::SavePrefs(save) => job_save_prefs(&rt, &client, save, &tx),
                WorkerJob::Skills => job_skills(&rt, &client, &tx),
                WorkerJob::CtlList { socket } => job_ctl_list(&socket, &tx),
                WorkerJob::CtlAttach { socket, name } => job_ctl_attach(&socket, &name, &tx),
                WorkerJob::CtlNew {
                    socket,
                    name,
                    workspace,
                } => job_ctl_new(&socket, &name, &workspace, &tx),
                WorkerJob::WritePermission {
                    conversation_id,
                    level,
                    generation,
                } => job_write_permission(&rt, &client, &conversation_id, &level, generation, &tx),
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
    lane: LaneId,
) {
    let result = rt.block_on(async {
        let mut sink = UiSink {
            lane,
            tx: tx.clone(),
        };
        let mut gate: Box<dyn ApprovalGate> = if job.yes {
            Box::new(AutoAllowOnce)
        } else {
            Box::new(OverlayApprovalGate {
                tx: tx.clone(),
                wait: APPROVAL_WAIT,
            })
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
    let (outcome, error, interrupted) = split_turn_result(result);
    let _ = tx.send(UiEvent::TurnDone {
        outcome,
        error,
        interrupted,
    });
}

/// 回合结果 → (摘要, 错误文案, 续流点)。
///
/// `TermError::InterruptedStream` 在变成字符串之前被拆出来：UI 侧 `/resume`
/// 需要 `job_id` / `after_seq` 两个数字，字符串里只留给人看。用户强退
/// （`TermError::Interrupted`）沿用原语义——不报错，也不留续流点。
fn split_turn_result(
    result: Result<ChatStreamOutcome, TermError>,
) -> (ChatStreamOutcome, Option<String>, Option<StreamResume>) {
    match result {
        Ok(o) => (o, None, None),
        Err(TermError::Interrupted) => (ChatStreamOutcome::default(), None, None),
        Err(e) => {
            // 先借用取续流点，再让错误自己渲染文案：格式串只留在 `TermError` 的
            // Display 里，不会在 worker 这边多一份需要同步修改的副本。
            let interrupted = match &e {
                TermError::InterruptedStream {
                    job_id, after_seq, ..
                } => Some(StreamResume {
                    job_id: *job_id,
                    after_seq: *after_seq,
                }),
                _ => None,
            };
            (
                ChatStreamOutcome::default(),
                Some(e.to_string()),
                interrupted,
            )
        }
    }
}

async fn fetch_serve_defaults(client: &ServeClient) -> Result<ServeDefaults, String> {
    let v: serde_json::Value = client
        .get_json(paths::STATUS_SHELL)
        .await
        .map_err(|e| e.to_string())?;
    Ok(ServeDefaults::from_status(&v))
}

/// 写会话索引（`GET` 最新 → 改 → 全量 `PUT`）：新建空白行或把 `conversation_id` 绑到指定行。
fn job_session_index(
    rt: &tokio::runtime::Runtime,
    client: &ServeClient,
    op: SessionIndexOp,
    tx: &Sender<UiEvent>,
) {
    let result = rt
        .block_on(apply_web_session_op(client, &op))
        .map_err(|e| e.to_string());
    let _ = tx.send(UiEvent::SessionIndex { result, op });
}

/// 会话列表 + 工作区路径刷新（静默；发送失败忽略，worker 随 UI 退出而停）。
fn job_refresh_sessions(rt: &tokio::runtime::Runtime, client: &ServeClient, tx: &Sender<UiEvent>) {
    let result = rt
        .block_on(fetch_web_sessions(client))
        .map_err(|e| e.to_string());
    let _ = tx.send(UiEvent::Sessions(result));
    let path = rt.block_on(fetch_workspace(client)).ok().and_then(|w| {
        let p = w.path.trim();
        (!p.is_empty()).then(|| p.to_string())
    });
    let _ = tx.send(UiEvent::WorkspacePath(path));
}

/// 拉一页会话历史（只读回放）。失败如实回传，提示文案由 UI 决定。
fn job_conversation_history(
    rt: &tokio::runtime::Runtime,
    client: &ServeClient,
    conversation_id: &str,
    before_index: Option<u32>,
    tx: &Sender<UiEvent>,
) {
    let fetched = rt.block_on(fetch_conversation_messages(
        client,
        conversation_id,
        before_index,
    ));
    // 水合响应自带当前上下文快照：新会话 / 切换会话即可显示初始用量（与 Desktop 同口径），
    // 不必等回合末的 `RUN_FINISHED`。
    let snapshot = fetched
        .as_ref()
        .ok()
        .and_then(|r| r.tiktoken_prompt_tokens.clone());
    let result = fetched.map_err(|e| e.to_string());
    let _ = tx.send(UiEvent::ConversationHistory {
        conversation_id: conversation_id.to_string(),
        result,
        snapshot,
        before_index,
    });
}

/// 回合末补用量快照：只留 `tiktoken_prompt_tokens`，消息正文丢弃。
///
/// 失败静默：状态行的用量段不是关键路径，拉不到就维持现状（占位 `—`），
/// 不必往 transcript 里写一条与用户操作无关的报错。
fn job_refresh_usage(
    rt: &tokio::runtime::Runtime,
    client: &ServeClient,
    conversation_id: &str,
    tx: &Sender<UiEvent>,
) {
    let snapshot = rt
        .block_on(fetch_conversation_messages(client, conversation_id, None))
        .ok()
        .and_then(|r| r.tiktoken_prompt_tokens);
    let _ = tx.send(UiEvent::UsageHydrated {
        conversation_id: conversation_id.to_string(),
        snapshot,
    });
}

/// 工作区根目录列表（工作区路径 + 目录树数据）。
fn job_refresh_workspace_root(
    rt: &tokio::runtime::Runtime,
    client: &ServeClient,
    tx: &Sender<UiEvent>,
    keep: bool,
) {
    let result = rt
        .block_on(fetch_workspace_dir(client, None))
        .map_err(|e| e.to_string());
    let _ = tx.send(UiEvent::Workspace { result, keep });
}

/// 展开目录的子列表。
fn job_workspace_dir(
    rt: &tokio::runtime::Runtime,
    client: &ServeClient,
    rel: &str,
    generation: u64,
    tx: &Sender<UiEvent>,
) {
    let result = rt
        .block_on(fetch_workspace_dir(client, Some(rel)))
        .map_err(|e| e.to_string());
    let _ = tx.send(UiEvent::WorkspaceDir {
        rel: rel.to_string(),
        generation,
        result,
    });
}

/// serve 默认偏好（`/status?view=shell`）+ 当前生效权限投影（`/permission`）。
///
/// 两个端点合进同一个任务：状态行要的是**同一时刻**的 serve 侧事实，分两次任务到达顺序随机，
/// 会短暂拼出"旧 model + 新等级"这种没发生过的组合。
fn job_refresh_status(
    rt: &tokio::runtime::Runtime,
    client: &ServeClient,
    conversation_id: Option<&str>,
    tx: &Sender<UiEvent>,
) {
    let mut result = rt.block_on(fetch_serve_defaults(client));
    let view = rt.block_on(fetch_permission_view(client, conversation_id));
    if let Ok(defaults) = result.as_mut() {
        defaults.permission_view = view;
    }
    let _ = tx.send(UiEvent::Status {
        conversation_id: conversation_id.map(str::to_string),
        result,
    });
}

/// `GET /permission` 的完整投影：**404（老 serve）与任何失败都返回 `None`**。
///
/// 状态行少一段，而不是显示一个猜来的档位——等级是策略事实，猜错的方向可能是危险的
/// （把 `chat` 显示成 `unrestricted` 才最糟）。需要细节时看 Desktop 设置页或 `/status`
/// 里的 `permission_level_default` / `permission_level_max`。
///
/// 存**整个** view（不再只取 `level`）：切换菜单的选项集就是 `available`，与 Desktop chip
/// 吃同一份投影，避免「菜单列五档、服务端只让选三档」这种自造的分歧。
///
/// **带 `conversation_id`**：这是按会话算的投影（会话列里的意图 + min 链）。裸请求回的是
/// 「配置默认档 + 无会话意图」，与状态行要回答的「此刻这个会话是哪一档」是两个问题 ——
/// 混用会让状态行在刷新后跳回默认档，看起来像刚选的档自己丢掉了。
async fn fetch_permission_view(
    client: &ServeClient,
    conversation_id: Option<&str>,
) -> Option<PermissionView> {
    let view = client
        .get_json_optional::<PermissionView>(&paths::permission(None, conversation_id))
        .await
        .ok()
        .flatten()?;
    (!view.level.trim().is_empty()).then_some(view)
}

/// 权限等级意图落盘（`POST /conversation/state`）。
///
/// 只发 `permission_level` 一列：服务端是部分更新，顺带回传 `session_mode` / `agent_role`
/// 会把过期值写回去（Desktop `apply_level` 同一条约束）。
///
/// 结局**不在这一层判定**：拿到响应就交给共享层
/// [`classify_conversation_state_response`] —— 这条路由的 404 有「老 serve 没路由」与
/// 「会话不存在」两种含义，两处各写一份判别就是新的漂移点。
fn job_write_permission(
    rt: &tokio::runtime::Runtime,
    client: &ServeClient,
    conversation_id: &str,
    level: &str,
    generation: u64,
    tx: &Sender<UiEvent>,
) {
    let outcome = rt
        .block_on(write_permission_level(client, conversation_id, level))
        .map_err(|e| e.to_string());
    let _ = tx.send(UiEvent::PermissionWritten {
        level: level.to_string(),
        outcome,
        generation,
    });
}

/// skill 目录（`GET /skills`）：只给 `/` 补全菜单的 skill 候选，失败如实回传。
fn job_skills(rt: &tokio::runtime::Runtime, client: &ServeClient, tx: &Sender<UiEvent>) {
    let result = rt.block_on(async {
        client
            .get_json::<SkillsPayload>(paths::SKILLS)
            .await
            .map_err(|e| e.to_string())
    });
    let _ = tx.send(UiEvent::Skills(result));
}

/// 项目池列表（`GET /workspace/projects`）。
fn job_ws_projects(rt: &tokio::runtime::Runtime, client: &ServeClient, tx: &Sender<UiEvent>) {
    let result = rt
        .block_on(fetch_workspace_projects(client))
        .map_err(|e| e.to_string());
    let _ = tx.send(UiEvent::WsProjects(result));
}

/// 切换项目池内项目（`POST /workspace/projects`）。
fn job_ws_switch_project(
    rt: &tokio::runtime::Runtime,
    client: &ServeClient,
    name: &str,
    tx: &Sender<UiEvent>,
) {
    let result = rt
        .block_on(switch_workspace_project(client, name))
        .map_err(|e| e.to_string());
    let _ = tx.send(UiEvent::WsProjectSwitch(result));
}

/// `/workspace`（空参）：读 serve 当前工作区根（`GET /workspace`）。
///
/// `fetch_workspace` 不像 `fetch_workspace_dir` 那样检查体内 `error`（2xx 也可能带
/// 语义错误），这里补上：否则用户会看到「当前工作区：」空一行而不是失败原因。
fn job_workspace_query(rt: &tokio::runtime::Runtime, client: &ServeClient, tx: &Sender<UiEvent>) {
    let result = match rt.block_on(fetch_workspace(client)) {
        Ok(w) => match w.error.as_deref().map(str::trim) {
            Some(e) if !e.is_empty() => Err(e.to_string()),
            _ => Ok(w.path),
        },
        Err(e) => Err(e.to_string()),
    };
    let _ = tx.send(UiEvent::WorkspaceQuery(result));
}

/// `/workspace <路径>`：按路径设置 serve 工作区根（`POST /workspace`）。
///
/// 与项目池那条（`POST /workspace/projects`）是同一个后端状态的两种入口：项目池只能选
/// 池内已有项目，这里把路径原样发出去（客户端不做白名单，能否落地由 serve 决定；
/// 空串＝恢复 serve 默认）。
///
/// 失败**不压成字符串**：`WorkspaceSetError` 带 `kind`，UI 层才能把 409
/// `WORKSPACE_ALREADY_SET` 提示成「另起 lane」而不是「重试」。
fn job_workspace_set(
    rt: &tokio::runtime::Runtime,
    client: &ServeClient,
    path: &str,
    tx: &Sender<UiEvent>,
) {
    let result = rt.block_on(set_workspace(client, path));
    let _ = tx.send(UiEvent::WorkspaceSet(result));
}

/// 启动时拉取 user-data 设置快照（`GET /user-data/prefs` + `GET /user-data/llm-overrides`）
/// 并合成持久层；任一失败整体报错（面板/合成回退 override 与 serve 默认）。
fn job_load_user_settings(
    rt: &tokio::runtime::Runtime,
    client: &ServeClient,
    tx: &Sender<UiEvent>,
) {
    let result: Result<PersistedSettings, String> = rt.block_on(async {
        let prefs = fetch_user_data_prefs(client)
            .await
            .map_err(|e| e.to_string())?;
        let llm = fetch_llm_overrides(client)
            .await
            .map_err(|e| e.to_string())?;
        Ok(PersistedSettings::from_snapshot(&prefs, &llm))
    });
    let _ = tx.send(UiEvent::UserData(result));
}

/// 保存 llm-overrides：先 GET 现值，只改 `client_llm.{model,api_base,temperature,
/// llm_thinking_mode,llm_context_tokens}` 再全量 PUT
/// （合并保真，不覆盖 `executor_llm/saved_models/execution_mode`）。
fn job_save_llm(
    rt: &tokio::runtime::Runtime,
    client: &ServeClient,
    save: LlmSave,
    tx: &Sender<UiEvent>,
) {
    let result: Result<LlmSave, String> = rt.block_on(async {
        let dto = fetch_llm_overrides(client)
            .await
            .map_err(|e| e.to_string())?;
        put_llm_overrides(client, &merge_llm_save(dto, &save))
            .await
            .map_err(|e| e.to_string())?;
        Ok(save)
    });
    let _ = tx.send(UiEvent::SavedLlm(result));
}

/// 保存 prefs：先 GET 现值，只改 `cm_role/session_mode/disable_readonly_tool_ttl_cache`
/// 再全量 PUT（合并保真，不覆盖 `locale/theme/布局/IDE` 等字段）。
fn job_save_prefs(
    rt: &tokio::runtime::Runtime,
    client: &ServeClient,
    save: PrefsSave,
    tx: &Sender<UiEvent>,
) {
    let result: Result<PrefsSave, String> = rt.block_on(async {
        let dto = fetch_user_data_prefs(client)
            .await
            .map_err(|e| e.to_string())?;
        put_user_data_prefs(client, &merge_prefs_save(dto, &save))
            .await
            .map_err(|e| e.to_string())?;
        Ok(save)
    });
    let _ = tx.send(UiEvent::SavedPrefs(result));
}

#[cfg(test)]
mod tests {
    use super::is_actionable_mouse;
    use crossterm::event::{MouseButton, MouseEventKind};

    #[test]
    fn pointer_motion_and_drags_are_dropped() {
        // 指针划过 / 按住拖动：一条都不进 channel（否则主循环按 2ms 整帧重绘）。
        assert!(!is_actionable_mouse(MouseEventKind::Moved));
        assert!(!is_actionable_mouse(MouseEventKind::Drag(
            MouseButton::Left
        )));
        assert!(!is_actionable_mouse(MouseEventKind::Up(MouseButton::Left)));
    }

    #[test]
    fn clicks_and_wheel_are_kept() {
        assert!(is_actionable_mouse(MouseEventKind::Down(MouseButton::Left)));
        assert!(is_actionable_mouse(MouseEventKind::ScrollUp));
        assert!(is_actionable_mouse(MouseEventKind::ScrollDown));
    }

    #[test]
    fn non_left_clicks_are_dropped() {
        // 右键 / 中键没有绑定动作，`on_mouse` 也是忽略。
        assert!(!is_actionable_mouse(MouseEventKind::Down(
            MouseButton::Right
        )));
        assert!(!is_actionable_mouse(MouseEventKind::Down(
            MouseButton::Middle
        )));
    }
}
