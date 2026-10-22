//! `TuiApp` 的测试构造辅助（仅 `#[cfg(test)]`）。
//!
//! 事件循环里有几处「接线」只能在 `TuiApp` 层验证 —— 草稿换入换出、排队消息的去向、
//! 回放失败后的回滚 —— 它们都不是纯函数，单测得先造出一个 `TuiApp`。
//!
//! 这份构造器原本是 `workspace_cmd` 测试里的私有辅助。多个模块各写一份必然漂移，
//! 而漂移的表现是「某个测试的初始状态与真实启动不一致」这种**假通过**，所以提到这里共用。

use std::sync::mpsc;

use crate::SessionPrefs;
use crate::serve::{ConnectionConfig, ServeClient};

use super::TuiApp;
use super::spinner::Spinner;
use super::state::UiState;
use super::worker::{UiEvent, WorkerJob};

/// 空的会话级 override（`*` 覆盖），让状态行回落到 serve 默认。
pub(super) fn test_prefs() -> SessionPrefs {
    SessionPrefs {
        api_key: None,
        model: None,
        api_base: None,
        agent_role: None,
        session_mode: None,
    }
}

/// 不连网的 `ServeClient`。
///
/// 端口 1 必然连不上：接线测试只关心「状态怎么变」，任何真发出去的请求都应该失败得
/// 明明白白，而不是悄悄打到某个本地 serve 上、让断言依赖外部环境。
pub(super) fn test_client() -> ServeClient {
    ServeClient::new(ConnectionConfig::new("http://127.0.0.1:1", "")).expect("client")
}

/// 构造一个最小 `TuiApp`：`yes` / `no_keyring` 都打开，其余为启动初始态。
///
/// `rx` / `tx` 必须是同一对：单测没有 tokio runtime，读请求会退回队列；但若将来某个
/// 测试跑在 `#[tokio::test]` 下，读请求就真的会 `spawn`，那时发到一个没人读的通道里
/// 会让断言假通过。`job_tx` 同理 —— 要断言「发了哪些 worker 任务」就把接收端留在
/// 调用方。
pub(super) fn test_app<'a>(
    client: &'a ServeClient,
    prefs: &'a mut SessionPrefs,
    rx: mpsc::Receiver<UiEvent>,
    tx: mpsc::Sender<UiEvent>,
    job_tx: mpsc::Sender<WorkerJob>,
) -> TuiApp<'a> {
    TuiApp {
        client,
        overrides: prefs,
        yes: true,
        no_keyring: true,
        st: UiState::new(),
        drafts: Default::default(),
        spinner: Spinner::new(),
        rx,
        tx,
        job_tx,
        cancel: None,
        exit: false,
        quit_pending: false,
        prepared: Vec::new(),
        body_key: None,
        panel: None,
        persisted: None,
        last_size: (0, 0),
        index: Default::default(),
        resume: None,
        sent_turn: None,
        bg: None,
    }
}
