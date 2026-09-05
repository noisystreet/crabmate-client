//! 全屏 TUI 的 worker 面：键盘事件线程、回合/拉取持久 worker（单 current-thread
//! runtime 串行执行，避免 reqwest 连接池跨多个临时 runtime 复用问题）以及 UI 事件。
//!
//! 拆分出本模块以控制 `mod.rs` 单文件行数（fn-nloc 门禁 ≤ 920）与函数复杂度。

use std::sync::mpsc::{self, Sender};
use std::thread;

use crossterm::event::{self, Event, KeyEvent};

use crabmate_tui_core::{
    ApprovalGate, AutoAllowOnce, ChatStreamOptions, ChatStreamOutcome, ServeClient, StreamCancel,
    StreamSink, TermError, WebSessionsList, WorkspaceDirData, fetch_web_sessions, fetch_workspace,
    fetch_workspace_dir, run_chat_stream_sink,
};

use super::approve::{ApprovalPrompt, OverlayApprovalGate};
use super::serve_defaults::ServeDefaults;

/// UI 事件（键盘线程与回合/拉取线程共同的生产者）。
pub(super) enum UiEvent {
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
    /// 工作区根路径（`GET /workspace`，随会话刷新顺带拉取；只更新顶栏，不动目录树）。
    WorkspacePath(Option<String>),
    /// 工作区根目录列表（`GET /workspace`）：路径给顶栏，entries 给目录树。
    Workspace(Result<WorkspaceDirData, String>),
    /// 已展开子目录的列表（`GET /workspace?path=`）结果。
    WorkspaceDir {
        rel: String,
        result: Result<WorkspaceDirData, String>,
    },
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
    RefreshWorkspace,
    /// 展开目录 → 拉取该目录子列表。
    WorkspaceDir(String),
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
pub(super) fn spawn_key_reader(tx: Sender<UiEvent>) {
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
                WorkerJob::RefreshWorkspace => job_refresh_workspace_root(&rt, &client, &tx),
                WorkerJob::WorkspaceDir(rel) => job_workspace_dir(&rt, &client, &rel, &tx),
                WorkerJob::RefreshStatus => job_refresh_status(&rt, &client, &tx),
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

/// 会话列表 + 顶栏工作区路径刷新（静默；发送失败忽略，worker 随 UI 退出而停）。
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

/// 工作区根目录列表（顶栏路径 + 目录树数据）。
fn job_refresh_workspace_root(
    rt: &tokio::runtime::Runtime,
    client: &ServeClient,
    tx: &Sender<UiEvent>,
) {
    let result = rt
        .block_on(fetch_workspace_dir(client, None))
        .map_err(|e| e.to_string());
    let _ = tx.send(UiEvent::Workspace(result));
}

/// 展开目录的子列表。
fn job_workspace_dir(
    rt: &tokio::runtime::Runtime,
    client: &ServeClient,
    rel: &str,
    tx: &Sender<UiEvent>,
) {
    let result = rt
        .block_on(fetch_workspace_dir(client, Some(rel)))
        .map_err(|e| e.to_string());
    let _ = tx.send(UiEvent::WorkspaceDir {
        rel: rel.to_string(),
        result,
    });
}

/// serve 默认偏好（`/status?view=shell`）。
fn job_refresh_status(rt: &tokio::runtime::Runtime, client: &ServeClient, tx: &Sender<UiEvent>) {
    let result = rt.block_on(fetch_serve_defaults(client));
    let _ = tx.send(UiEvent::Status(result));
}
