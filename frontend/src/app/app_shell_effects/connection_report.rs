//! 把本窗口的**连接状态与未读**上报给桌面壳（左栏「连接」区的数据源）。
//!
//! 左栏那行（`nav_rail_connections`）画的是壳注册表里的快照，而
//! 快照里的 `status` / `unread` 只有**本窗口自己**知道 —— 壳不会去猜。这个模块就是
//! 那条一直缺的通道：
//!
//! - **状态**：`TurnLifecycleState`（回合在不在跑）+ `pending_approval`（有没有审批
//!   在等）合成一个 [`ConnectionStatus`]，变了才上报；
//! - **未读**：窗口失焦期间回合在跑 → 置位，窗口重新聚焦 → 清零。
//!
//! 判定本身是 `crabmate-client-api` 的纯函数（`connection_status_from_turn` /
//! `connection_unread_after`）：TUI 的 lane 行用的是同一套口径，写在这里一遍、那里
//! 一遍迟早会分叉。本模块只负责「读到信号 → 调壳」。
//!
//! **必须先登记再上报。** 壳按 window label 查注册表，未登记的 label 会被
//! `set_status` 静默丢弃（返回 `None` 就直接 return）；而 `register_connection` 是
//! `install_connections_feed` 里 spawn 出去的一次异步调用，首帧的 `Effect` 很可能
//! 比它先跑。所以这里订阅 [`connections_registered`]，在登记完成那一刻补发第一帧。
//!
//! - **行尾读数**（`context` / `busy_secs`，issue #414）：上下文用量随这一帧一起算、算完就报
//!   （短文本 `62%`，与状态栈同一根轴线）；在途耗时只报一个 `busy` 开关，秒数由壳按
//!   `started_at` 推进（`desktop_connections::spawn_busy_ticker`）—— 前端不每秒发 IPC，
//!   因为「别人那条连接」的开始时刻本来也不在它手上。
//!
//!   **左栏那条 lane 行的秒数走另一条路**（#577）：壳侧的 `started_at` 按 window label 记，
//!   一个窗口 N 条 lane 时只有一个秒表，秒数会混（`docs/design/desktop_single_window_lanes.md`
//!   §8.1）。所以起点在这里写进 `row.turn_started_at`，本窗口自己按行推进
//!   （`lane_busy_clock`）—— 壳那条 ticker 保留给「别的窗口读我这一行」的单 lane 路径。
//!
//! 刻意**不做**的一件事（留给后续 issue，需要新的壳命令或 UI 入口）：
//!
//! - `rename_connection`：入口是左栏本窗口那行的右键菜单（#415）。它只改显示名、与状态上报
//!   无关，所以不在这里。

use leptos::prelude::*;

use crabmate_client_api::connections::{
    ConnectionStatus, connection_status_from_turn, connection_unread_after,
};
use crabmate_client_api::context_axis::context_axis_pct_text;

use crate::app::turn_lifecycle::{TurnLifecycleState, turn_lifecycle_coarse_busy};
use crate::chat_session_state::ChatSessionSignals;
use crate::tauri_shell::{
    tauri_report_connection_metrics, tauri_report_connection_status,
    tauri_report_connection_unread, tauri_shell_available,
};

use super::super::connection_feed::connections_registered;
use super::lane_busy_clock::{next_turn_started, now_ms};
use crate::app::shell::status::context::{context_axis_apply_watermark, status_bar_context_axis};
use crate::app::shell::status::tasks_state::StatusTasksSignals;

/// 上报所需的信号（都是「本窗口此刻在干什么」的既有来源）。
pub struct ConnectionReportSignals {
    /// 单轮 `/chat/stream` 的粗粒度生命周期；这里只取「有没有回合在跑」。
    pub turn_lifecycle: RwSignal<TurnLifecycleState>,
    /// 待审批三元组：`Some` 即「有审批在等」。
    pub pending_approval: RwSignal<Option<(String, String, String)>>,
    /// 会话域：上下文用量的第一来源（实测 tiktoken 快照 / 当前会话）。
    pub chat: ChatSessionSignals,
    /// `/status` 与任务域：拿不到实测时的 `system` 基线。
    pub status: StatusTasksSignals,
    /// 本机 `context_tokens` 设置的世代号：改了设置就得重算分母。
    pub client_llm_storage_tick: RwSignal<u64>,
    /// 当前 agent 角色：`/status` 的基线按角色给。
    pub selected_agent_role: RwSignal<Option<String>>,
    /// 这条 lane 的生命周期阶段（#561 §8.2）：未读要判「窗口在前台 **且** 这条 lane
    /// 是前台那条」。单 lane 时恒 `Foreground`，退化成原来的「窗口是否失焦」。
    pub phase: RwSignal<crabmate_client_api::lane_phase::LanePhase>,
    /// 左栏那一行的读数（#569 对齐 TUI 的 lane 行）。
    ///
    /// 状态 / 未读 / 上下文占比**就在这里一起算**：这四项本来就由本模块产出（同一根轴线、
    /// 同一套 `connection_*` 谓词），再在左栏算一遍就会出现「同一台 serve 两个读数」。
    pub row: crate::app::app_signals::LaneRailRow,
}

/// lane 是否**真正可见**（#561 §8.2 的口径）：窗口在前台 **且** 这条 lane 是前台那条。
///
/// 抽成纯函数是为了能测：`Effect` 在 host 目标第一句就 `tauri_shell_available()` 早退，
/// 判断藏在闭包里就永远没有测试 —— 而「后台 lane 即使窗口有焦点也不算可见」正是
/// 多 lane 下最容易写错的一条（照旧只判窗口焦点，后台 lane 的新输出就永远不会置未读）。
fn lane_visible(window_focused: bool, phase: crabmate_client_api::lane_phase::LanePhase) -> bool {
    window_focused && phase.is_foreground()
}

/// 行尾读数：`(上下文占比短文本, 在途开关)`。
///
/// 抽成普通函数是为了能测：`Effect` 在 host 目标第一句就 `tauri_shell_available()` 早退，
/// 判断藏在闭包里就永远没有测试 —— 而这里最容易写错的一条正是「和状态栈是同一个数」。
fn rail_metrics(
    chat: ChatSessionSignals,
    status: StatusTasksSignals,
    client_llm_storage_tick: RwSignal<u64>,
    selected_agent_role: RwSignal<Option<String>>,
    turn_busy: bool,
) -> (Option<String>, bool) {
    let context =
        status_bar_context_axis(chat, status, client_llm_storage_tick, selected_agent_role)
            // 水位必须在这一步施加：`status_bar_context_axis` 只算轴线，状态栈 chip、用量页
            // 与 TUI 的 lane 行都在拿到轴线后各自施加一次。漏掉它，服务端 `used` 短暂回落时
            // （工具输出被截断等）连接行会往下跳一下再涨回来，而状态栈那时仍是峰值 ——
            // 同一个会话两个百分比，正是 ADE 0005 要消灭的形态。
            .map(|axis| context_axis_apply_watermark(axis, chat))
            .and_then(context_axis_pct_text);
    (context, turn_busy)
}

/// 装上状态 / 未读 / 行尾读数上报。
///
/// 非桌面壳（Android / Web / 测试）整段早退：那里既没有 `report_connection_*` 命令，
/// 也没有第二个窗口需要读这条读数。
///
/// `context` 与 `busy` 合成一条命令、且**同一条记账**：两个字段分开记，会出现「context
/// 变了但 busy 没变」时多报一次、或反过来漏报一次。
pub fn wire_connection_status_report(sig: ConnectionReportSignals) {
    // **读数照算，只有上报依赖壳**（#569）：左栏那条 lane 行要显示状态 / 未读 / 上下文占比，
    // 它和「上报给桌面壳」是同一批数字。整段早退会让左栏那行永远停在 Invariant（Idle · 无读数）。
    let shell = tauri_shell_available();

    let registered = Memo::new(move |_| connections_registered().get());
    let focused = window_focused_signal();
    // 上一次**发送**的值（不是上一次想发的值）：登记完成前不发，那一段不能记账。
    let reported_status = StoredValue::new(None::<ConnectionStatus>);
    let reported_unread = StoredValue::new(None::<bool>);
    let reported_metrics = StoredValue::new(None::<(Option<String>, bool)>);
    // 本地镜像：壳里那份未读只是回音，需要「保持原值」的那一位在窗口这一侧。
    let unread = StoredValue::new(false);

    Effect::new(move |_| {
        let turn_busy = turn_lifecycle_coarse_busy(sig.turn_lifecycle.get());
        let status = connection_status_from_turn(turn_busy, sig.pending_approval.get().is_some());
        // #561 §8.2：可见 = 窗口在前台 **且** 这条 lane 是前台那条。
        // 单 lane 时 phase 恒 Foreground，这里退化成改动前的「只看窗口焦点」。
        let visible = lane_visible(focused.get(), sig.phase.get());
        let next_unread = connection_unread_after(visible, turn_busy, unread.get_value());
        unread.set_value(next_unread);
        let (context, busy) = rail_metrics(
            sig.chat,
            sig.status,
            sig.client_llm_storage_tick,
            sig.selected_agent_role,
            turn_busy,
        );

        // 左栏那条 lane 行：同一批数字顺带写进去（#569）。
        sig.row.status.set(status);
        sig.row.unread.set(next_unread);
        sig.row.context.set(context.clone());
        // #577：行尾在途耗时的**起点**。与上面三行同批、且在下面「上报」的 early-return 之前
        // —— 非桌面壳（Android / Web）也要有这个读数；写到早退之后，左栏那行就永远没有秒数。
        // 幂等由 `next_turn_started` 保证：流式期间 `context` 每帧都在变、这条 Effect 会重跑
        // 很多次，每次覆盖成「现在」会让秒数永远停在 0。刻度与推进见 `lane_busy_clock`。
        let previous_start = sig.row.turn_started_at.get_untracked();
        let started = next_turn_started(previous_start, turn_busy, now_ms());
        // 只在与上一帧不同时写：这个信号是本 Effect 自己写的，无脑 `set` 会白跑一遍重算。
        if previous_start != started {
            sig.row.turn_started_at.set(started);
        }

        // 还没进注册表：**不能**把想报的值记成「已上报」—— 否则登记完成那一刻若
        // 状态再没变过，就永远不会发，症状与「从来没上报」一模一样。
        if !shell || !registered.get() {
            return;
        }
        if reported_status.get_value() != Some(status) {
            reported_status.set_value(Some(status));
            tauri_report_connection_status(status);
        }
        if reported_unread.get_value() != Some(next_unread) {
            reported_unread.set_value(Some(next_unread));
            tauri_report_connection_unread(next_unread);
        }
        let next_metrics = (context, busy);
        if reported_metrics.get_value() != Some(next_metrics.clone()) {
            reported_metrics.set_value(Some(next_metrics.clone()));
            tauri_report_connection_metrics(next_metrics.0, next_metrics.1);
        }
    });
}

/// 窗口是否在前台（`focus` / `blur` 两个事件维护）。
///
/// 用 DOM 事件而不是壳的 `is_focused()`：前端本来就要在浏览器 / Android 上编译，
/// 而「窗口失焦」在 WebView 里与 `window` 的 `blur` 是同一件事 —— 不必为它多开一条
/// 仅在桌面可用的 IPC。
#[cfg(target_arch = "wasm32")]
fn window_focused_signal() -> RwSignal<bool> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;

    let focused = RwSignal::new(document_has_focus());
    if let Some(window) = web_sys::window() {
        let on_focus = Closure::<dyn FnMut()>::new(move || focused.set(true));
        let on_blur = Closure::<dyn FnMut()>::new(move || focused.set(false));
        let _ = window.add_event_listener_with_callback("focus", on_focus.as_ref().unchecked_ref());
        let _ = window.add_event_listener_with_callback("blur", on_blur.as_ref().unchecked_ref());
        // 监听器与页面同寿（与 `app_shell_effects::viewport` / `sync_dom` 同一做法）：
        // 失焦判定在页面活着的时候一直要用，没有需要摘下来的时刻。
        on_focus.forget();
        on_blur.forget();
    }
    focused
}

/// 非 wasm（测试 / SSR）：没有窗口可失焦，恒按前台处理。
#[cfg(not(target_arch = "wasm32"))]
fn window_focused_signal() -> RwSignal<bool> {
    RwSignal::new(true)
}

/// 读一次当前是否在前台。
///
/// 拿不到就按「在前台」处理：宁可少画一个 `•`，也别让一个清不掉的未读点出现 ——
/// 那种点会让用户以为有东西一直没看。
#[cfg(target_arch = "wasm32")]
fn document_has_focus() -> bool {
    web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.has_focus().ok())
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::lane_visible;
    use crabmate_client_api::lane_phase::LanePhase;

    /// 单 lane 的既有行为：只看窗口焦点（`phase` 恒 `Foreground`，等于改动前）。
    #[test]
    fn a_single_lane_falls_back_to_window_focus() {
        assert!(lane_visible(true, LanePhase::Foreground));
        assert!(!lane_visible(false, LanePhase::Foreground));
    }

    /// 多 lane 的关键一条：窗口有焦点，但**不是前台那条** → 看不见，新输出要置未读。
    ///
    /// 照旧只判窗口焦点的话，后台 lane 的未读永远不会亮（用户永远不知道它说了话）。
    #[test]
    fn a_background_lane_is_not_visible_even_with_window_focus() {
        assert!(!lane_visible(true, LanePhase::Background));
    }

    /// 关闭中 / 已关闭的 lane 也不可见：它的输出不该再点亮任何东西。
    #[test]
    fn closing_and_closed_lanes_are_never_visible() {
        for phase in [LanePhase::Closing, LanePhase::Closed] {
            assert!(!lane_visible(true, phase), "{phase:?}");
            assert!(!lane_visible(false, phase), "{phase:?}");
        }
    }
}
