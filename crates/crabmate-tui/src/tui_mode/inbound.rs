//! 入站事件：把「谁发的、事件去哪」从注释约定变成类型（issue #347）。
//!
//! 改动前有两条发送端并存，用途只写在注释里：`LaneTx` 给自己的事件打 `LaneId` 标签，
//! 裸 `Sender<UiEvent>` 发全局输入与外壳动作。漏包一次标签，某条 lane 的事件就会被
//! 当成**全局输入**落到当前聚焦的 lane 上 —— 静默串台：不崩溃，很难查。
//!
//! 三个变体把三件事分开，误用变成编译期错误：
//!
//! - [`Inbound::Input`]：键盘 / 鼠标 / 粘贴 / 尺寸，作用于**当前** lane；
//! - [`Inbound::FromLane`]：某条 lane 的出站事件，按 `LaneId` 分发，找不到即丢弃；
//! - [`Inbound::Shell`]：外壳动作（会改 `TuiApp::lanes` 的那些），lane 根本收不到 ——
//!   所以 `LaneApp::on_msg` 里不再需要为它们留空 arm。

use crabmate_client_api::ctl::{ToggleFailure, ToggleOutcome};

use super::worker::{LaneId, UiEvent};

/// 进入 [`super::TuiApp::on_msg`] 的唯一事件类型。
pub(super) enum Inbound {
    /// 全局输入：谁都不认领，交给**当前** lane。
    ///
    /// 变体**不做**限制：`Input(UiEvent::TurnDone { .. })` 同样能编译，并会落到当前
    /// lane（依旧串台）。要堵死这一点得引入只含输入变体的载荷类型，让 `Input` 收那个
    /// 类型；当前形态覆盖的是「忘了打 lane 标签」这个真发生过的错误。
    Input(UiEvent),
    /// 某条 lane 自己的出站事件（常规出口是 [`super::worker::LaneTx`]，它负责贴上
    /// `LaneId`；持有裸 `Sender<Inbound>` 的地方也能手写，但没有理由这么做）。
    FromLane(LaneId, UiEvent),
    /// 外壳动作：只有 [`super::TuiApp`] 能处理。
    Shell(ShellEvent),
}

/// 只有外壳能处理的动作：它们改的是 `TuiApp::lanes` 本身，单条 lane 手上没有那份 `&mut`。
pub(super) enum ShellEvent {
    /// 控制面回包后装配一条新 lane（`/lane attach` / `/lane new`）。
    ///
    /// 目标 lane 此刻还不存在，按 `LaneId` 分发无处可去 —— 这正是它必须留在外壳的原因。
    Ready {
        /// 实例名（将来那条 lane 的 `label`）。
        name: String,
        /// `Ok(serve 根 URL)` / `Err(面向用户的失败原因)`。
        result: Result<String, String>,
    },
    /// 摘掉本进程的一条连接（`/lane detach [<name>]`，缺省 = 当前 lane）：被摘的可能正是
    /// 发起命令的那条 lane。
    Detach {
        /// 要摘掉的那条连接的 `label`。
        name: String,
    },
    /// 取消某条 lane 的回合（`/lane cancel`）：`LaneApp` 只握着自己的 `cancel`，
    /// 却可能被要求取消别人。
    Cancel {
        /// 要取消的那条连接的 `label`。
        name: String,
    },
    /// 翻声明 `enabled` 的回执（`/lane close|open`）：关掉后还要顺手摘掉本进程那条连接。
    Toggled(Result<ToggleOutcome, ToggleFailure>),
}
