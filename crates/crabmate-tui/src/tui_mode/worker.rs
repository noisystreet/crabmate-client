//! 全屏 TUI 的 worker 面：键盘事件线程、回合/拉取持久 worker（单 current-thread
//! runtime 串行执行，避免 reqwest 连接池跨多个临时 runtime 复用问题）以及 UI 事件。
//!
//! 拆分出本模块以控制 `mod.rs` 单文件行数（fn-nloc 门禁 ≤ 920）与函数复杂度。

use std::sync::mpsc::{self, Sender};
use std::thread;

use crossterm::event::{self, Event, KeyEvent, MouseButton, MouseEvent, MouseEventKind};

use crabmate_client_api::paths;
use crabmate_client_api::prompt_tokens::TiktokenPromptTokensSnapshot;

use crate::serve::{
    ApprovalGate, AutoAllowOnce, ChatStreamOptions, ChatStreamOutcome,
    ConversationMessagesResponse, ServeClient, SessionIndexApplied, SessionIndexOp, StreamCancel,
    StreamResume, StreamSink, TermError, WebSessionsList, WorkspaceDirData, WorkspaceProjectsData,
    apply_web_session_op, fetch_conversation_messages, fetch_llm_overrides, fetch_user_data_prefs,
    fetch_web_sessions, fetch_workspace, fetch_workspace_dir, fetch_workspace_projects,
    put_llm_overrides, put_user_data_prefs, run_chat_stream_sink, set_workspace,
    switch_workspace_project,
};

use super::approve::{APPROVAL_WAIT, ApprovalPrompt, OverlayApprovalGate};
use super::serve_defaults::ServeDefaults;
use super::settings::{LlmSave, PersistedSettings, PrefsSave, merge_llm_save, merge_prefs_save};
use super::slash_menu::SkillsPayload;

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
    /// 已持久化会话的历史分页（`GET /conversation/messages`）：只读回放。
    /// 失败只提示，不阻断输入（当前会话仍可继续聊）。
    ConversationHistory {
        conversation_id: String,
        result: Result<ConversationMessagesResponse, String>,
    },
    /// 会话索引写入（`/conv new` 新建或复用空行 / 首轮后绑定 `conversation_id` / 切走回收）结果。
    SessionIndex {
        result: Result<SessionIndexApplied, String>,
        op: SessionIndexOp,
    },
    Status(Result<ServeDefaults, String>),
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
    WorkspaceSet(Result<String, String>),
    /// 启动时 user-data 设置快照（prefs + llm-overrides）拉取结果（合成持久层）。
    UserData(Result<PersistedSettings, String>),
    /// `llm-overrides` 保存结果：`Ok` 回传已应用动作（面板据此清 staged）。
    SavedLlm(Result<LlmSave, String>),
    /// `prefs` 保存结果：`Ok` 回传已应用动作（面板据此清 staged）。
    SavedPrefs(Result<PrefsSave, String>),
    /// skill 目录（`GET /skills`，供 `/` 补全菜单的 skill 候选）。
    Skills(Result<SkillsPayload, String>),
}

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
    RefreshStatus,
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

    fn on_prompt_tokens(&mut self, snap: &TiktokenPromptTokensSnapshot) -> Result<(), TermError> {
        self.tx
            .send(UiEvent::PromptTokens(Box::new(snap.clone())))
            .map_err(|_| TermError::Message("ui channel closed".into()))
    }

    fn on_tool_start(
        &mut self,
        tool_call_id: &str,
        name: &str,
        note: Option<&str>,
    ) -> Result<(), TermError> {
        self.tx
            .send(UiEvent::ToolStart {
                tool_call_id: tool_call_id.to_string(),
                name: name.to_string(),
                note: note.map(str::to_string),
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
pub(super) fn spawn_worker(client: ServeClient, tx: Sender<UiEvent>) -> Sender<WorkerJob> {
    let (job_tx, job_rx) = mpsc::channel::<WorkerJob>();
    thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("worker runtime");
        while let Ok(job) = job_rx.recv() {
            match job {
                WorkerJob::Turn(job) => run_turn_job(&rt, &client, *job, &tx),
                WorkerJob::RefreshSessions => job_refresh_sessions(&rt, &client, &tx),
                WorkerJob::ConversationHistory {
                    conversation_id,
                    before_index,
                } => job_conversation_history(&rt, &client, &conversation_id, before_index, &tx),
                WorkerJob::SessionIndex(op) => job_session_index(&rt, &client, op, &tx),
                WorkerJob::RefreshWorkspace { keep } => {
                    job_refresh_workspace_root(&rt, &client, &tx, keep);
                }
                WorkerJob::WorkspaceDir { rel, generation } => {
                    job_workspace_dir(&rt, &client, &rel, generation, &tx);
                }
                WorkerJob::RefreshStatus => job_refresh_status(&rt, &client, &tx),
                WorkerJob::WsProjects => job_ws_projects(&rt, &client, &tx),
                WorkerJob::WsSwitchProject(name) => job_ws_switch_project(&rt, &client, &name, &tx),
                WorkerJob::WorkspaceQuery => job_workspace_query(&rt, &client, &tx),
                WorkerJob::WorkspaceSet(path) => job_workspace_set(&rt, &client, &path, &tx),
                WorkerJob::LoadUserSettings => job_load_user_settings(&rt, &client, &tx),
                WorkerJob::SaveLlm(save) => job_save_llm(&rt, &client, save, &tx),
                WorkerJob::SavePrefs(save) => job_save_prefs(&rt, &client, save, &tx),
                WorkerJob::Skills => job_skills(&rt, &client, &tx),
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
    });
    // 先送 History（UI 侧据此绑定 `conversation_id`），再送快照，水位才落在正确的会话上。
    if let Some(snap) = snapshot {
        let _ = tx.send(UiEvent::PromptTokens(Box::new(snap)));
    }
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

/// serve 默认偏好（`/status?view=shell`）。
fn job_refresh_status(rt: &tokio::runtime::Runtime, client: &ServeClient, tx: &Sender<UiEvent>) {
    let result = rt.block_on(fetch_serve_defaults(client));
    let _ = tx.send(UiEvent::Status(result));
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
fn job_workspace_set(
    rt: &tokio::runtime::Runtime,
    client: &ServeClient,
    path: &str,
    tx: &Sender<UiEvent>,
) {
    let result = rt
        .block_on(set_workspace(client, path))
        .map_err(|e| e.to_string());
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
