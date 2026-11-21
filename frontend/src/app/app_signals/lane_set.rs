//! 窗口内的 lane 容器（#561 阶段 3）：**当前只持有一条 lane**。
//!
//! # 这一步解决什么
//!
//! 一条 lane 的运行时不止「一组信号」：还有接线产物（`refresh_*` / `new_session` /
//! `insert_workspace_file_ref`）与聊天列壳（`ChatColumnShell`）。改造前它们全是
//! `AppShellCtx` 上的**单个字段** —— 只要再开一条 lane，这些产物就会互相覆盖（后建的
//! 把先建的挤掉，切回去时第一条的发送闭包已经丢了）。
//!
//! 现在一条 lane 的运行时整体打包成 [`LaneRuntime`]，容器持 `Vec<LaneRuntime>`；壳一律
//! 经 [`AppShellCtx::signals`](super::super::AppShellCtx::signals) 等访问器取当前那条。
//!
//! 本步仍是 **N=1 等价重构**：容器恒一条，取到的就是改造前那份，单连接零行为变化
//! （frontend 826 条测试守这条）。
//!
//! # 为什么用 `Rc<RefCell<Vec<_>>>` 而不是 `RwSignal<Vec<_>>`
//!
//! `AppSignals` 与 `ChatColumnShell` 都含 `Rc` / `Arc<dyn Fn>`，不满足 Leptos
//! `RwSignal<T>` 要求的 `Send + Sync`。所以「有几条 lane」用 `Rc<RefCell<..>>` 存，
//! 只把**当前下标**做成 `RwSignal`（它是 `usize`，满足约束）—— 将来加 lane 时 bump
//! 一个版本号信号即可让视图重绘，不必让整个 `Vec` 进响应式系统。
//!
//! # 为什么现在还没有 `add_lane` / `set_active`
//!
//! 它们要等「新建 lane 的入口」落地才有调用者；提前写出来会被 `clippy -D warnings`
//! 判 `dead_code`（#558 把 1b 标为「故意没做」就是要避开这种白名单）。容器形状先按
//! 将来需要的样子定好，加方法时不必再改结构。

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use leptos::prelude::*;

use super::super::app_shell_wire_phases::ShellWiringOutput;
use super::super::chat::ChatColumnShell;
use super::AppSignals;

/// 一条 lane 的**全部**运行时：信号 + 接线产物 + 聊天列壳。
///
/// 三者必须一起按 lane 存放 —— 分开存就会出现「信号是这条、发送闭包是那条」。
pub(crate) struct LaneRuntime {
    pub signals: AppSignals,
    /// 阶段 1–5 的接线产物（`refresh_*` / `new_session` / 流式壳 …）。
    pub wiring: ShellWiringOutput,
    /// 聊天列视图壳（由 `signals` + `wiring` 组装，见 [`LaneRuntime::new`]）。
    pub chat_column: ChatColumnShell,
}

impl LaneRuntime {
    /// 由信号与接线产物组装一条 lane 的运行时。
    ///
    /// 组装表达式与改造前 `bootstrap_app_shell` 里那段逐字一致 —— 那一步只跑一次，
    /// 现在搬到这里，为的是「每条 lane 都要有一份」。
    fn new(signals: AppSignals, wiring: ShellWiringOutput) -> Self {
        let chat_column = ChatColumnShell {
            app: signals.clone(),
            stream_shell: wiring.chat_stream_shell.clone(),
            stream_busy_memos: wiring.stream_busy_memos,
            run_send_message: wiring.chat_wires.run_send_message.clone(),
            trigger_stop: Arc::clone(&wiring.chat_wires.cancel_stream),
            stream_follow_up: wiring.chat_wires.stream_follow_up,
            insert_workspace_file_ref: wiring.insert_workspace_file_ref,
        };
        Self {
            signals,
            wiring,
            chat_column,
        }
    }
}

/// 窗口内的 lane 集合 + 当前前台下标。
#[derive(Clone)]
pub(crate) struct LaneSet {
    /// 每条 lane 一份完整运行时。
    lanes: Rc<RefCell<Vec<LaneRuntime>>>,
    /// 当前前台下标。**唯一真源**：所有「我现在看的是哪条」都从这里读（现在恒 0）。
    active: RwSignal<usize>,
}

impl LaneSet {
    /// 建一个**只有一条 lane** 的容器。
    ///
    /// `signals` 必须是 `AppSignals::new_lane(&window, active_holder())` 的产物，
    /// `wiring` 是对它的 `run_shell_wiring_in_order` —— 与改造前的启动路径逐字等价。
    pub(crate) fn new_single(signals: AppSignals, wiring: ShellWiringOutput) -> Self {
        Self {
            lanes: Rc::new(RefCell::new(vec![LaneRuntime::new(signals, wiring)])),
            active: RwSignal::new(0),
        }
    }

    /// 对**当前前台**那条 lane 的运行时借用一次。
    ///
    /// 用借用而不是 clone 整份：`LaneRuntime` 里的 `ChatColumnShell` 含一堆 `Arc`，
    /// 而壳内要在 121 处取信号 —— 每处都 clone 一份运行时等于把「便宜」变成「不便宜」。
    ///
    /// 下标越界（理论上不可能：现在没有改下标的入口）时退回第一条 —— 宁可画错一条，
    /// 也别让一处 panic 把整屏打空。
    pub(crate) fn with_current<R>(&self, f: impl FnOnce(&LaneRuntime) -> R) -> R {
        let lanes = self.lanes.borrow();
        let last = lanes.len().saturating_sub(1);
        let index = self.active.get_untracked().min(last);
        let lane = lanes.get(index).expect("LaneSet 至少持有一条 lane");
        f(lane)
    }

    /// 当前前台那条 lane 的信号（唯一高频入口，单独开一个省一次闭包）。
    pub(crate) fn current_signals(&self) -> AppSignals {
        self.with_current(|lane| lane.signals.clone())
    }
}
