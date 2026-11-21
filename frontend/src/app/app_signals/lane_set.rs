//! 窗口内的 lane 容器（#561 阶段 3）：**当前只持有一条 lane**。
//!
//! # 这一步解决什么
//!
//! 一条 lane 的运行时不止「一组信号」：还有接线产物（`refresh_*` / `new_session` /
//! `insert_workspace_file_ref`）与聊天列壳（`ChatColumnShell`）。改造前它们全是
//! `AppShellCtx` 上的**单个字段** —— 只要再开一条 lane，这些产物就会互相覆盖（后建的
//! 把先建的挤掉，切回去时第一条的发送闭包已经丢了）。
//!
//! 现在一条 lane 的运行时整体打包成 [`LaneRuntime`]，容器持 `Vec<Rc<LaneRuntime>>`；壳一律
//! 经 [`AppShellCtx::current_runtime`](super::super::AppShellCtx::current_runtime) 取当前那条，
//! 之后按 lane 生成视图入参（`app_signals::lane_view_inputs`）。
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

use crate::api::connection_scope;

use super::super::app_shell_wire_phases::{ShellWiringOutput, run_shell_wiring_in_order};
use super::super::chat::ChatColumnShell;
use super::{AppSignals, WindowSignals};

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
    /// 窗口级信号的**唯一一份**（S2 建新 lane 时传给 `AppSignals::new_lane`）。
    /// 新 lane 复制它的句柄 —— 主题 / 语言 / 列宽因此不随 lane 走。
    window: WindowSignals,
    /// 每条 lane 一份完整运行时。**外面再套一层 `Rc`**：`App` 需要「取一次、传给多个
    /// 方法」（S1 的 14 个信号构造方法都要按指定 lane 生成），而 `RefCell` 借用无法跨
    /// 多次调用持有；`Rc::clone` 是常数开销且不持借用。
    lanes: Rc<RefCell<Vec<Rc<LaneRuntime>>>>,
    /// 当前前台下标。**唯一真源**：所有「我现在看的是哪条」都从这里读（现在恒 0）。
    active: RwSignal<usize>,
}

impl LaneSet {
    /// 建一个**只有一条 lane** 的容器。
    ///
    /// `signals` 必须是 `AppSignals::new_lane(&window, active_holder())` 的产物，
    /// `wiring` 是对它的 `run_shell_wiring_in_order` —— 与改造前的启动路径逐字等价。
    pub(crate) fn new_single(
        window: WindowSignals,
        signals: AppSignals,
        wiring: ShellWiringOutput,
    ) -> Self {
        Self {
            window,
            lanes: Rc::new(RefCell::new(vec![Rc::new(LaneRuntime::new(
                signals, wiring,
            ))])),
            active: RwSignal::new(0),
        }
    }

    /// 取当前前台那条 lane 的运行时（`Rc` 句柄，常数开销）。
    ///
    /// 下标越界（理论上不可能：现在没有改下标的入口）时退回**最后一条** —— 宁可画错一条，
    /// 也别让一处 panic 把整屏打空。
    pub(crate) fn current_runtime(&self) -> Rc<LaneRuntime> {
        let lanes = self.lanes.borrow();
        let last = lanes.len().saturating_sub(1);
        let index = self.active.get_untracked().min(last);
        Rc::clone(lanes.get(index).expect("LaneSet 至少持有一条 lane"))
    }

    /// 窗口里有几条 lane。
    pub(crate) fn len(&self) -> usize {
        self.lanes.borrow().len()
    }

    /// 下一条新 lane 的 `localStorage` 域后缀（§7 的 `-l<n>`）。
    ///
    /// 取「壳注入的连接标识」做基；没有注入（纯浏览器 / Android）时给 `None` —— 与主 lane
    /// 同口径，不凭空造域。
    pub(crate) fn next_storage_label(&self) -> Option<String> {
        lane_storage_label(
            crate::api::storage_scope::connection_key().as_deref(),
            self.len(),
        )
    }

    /// 新建一条 lane 并 push 进容器（#561 S2），返回它的运行时句柄。
    ///
    /// **必须在 `mount_to` 的闭包里调用**：`run_shell_wiring_in_order` 里那约 29 个
    /// `Effect::new` 需要一个 owner，`mount_to` 会为新子树建一个。在普通事件回调里直接调
    /// 本方法，那些 Effect 会挂到 `App` 的 owner 上 —— 关 lane 时清不掉。
    ///
    /// 前台下标不动：新 lane 先在后台，切前台是 S3 的事。
    pub(crate) fn add_lane(&self, scope: connection_scope::ScopeHolder) -> Rc<LaneRuntime> {
        let signals = AppSignals::new_lane(&self.window, scope);
        let wiring = run_shell_wiring_in_order(&signals);
        let runtime = Rc::new(LaneRuntime::new(signals, wiring));
        self.lanes.borrow_mut().push(Rc::clone(&runtime));
        runtime
    }
}

/// 新 lane 的存储域后缀（§7）：`<注入 key>-l<index>`。
///
/// `None` = 不分域（没有注入 key 时与主 lane 同口径）。`index` 传**当前 lane 数**，所以
/// 第一条新 lane 是 `-l1` —— 主 lane 保持裸键，已升级用户的既有数据不受影响。
#[must_use]
fn lane_storage_label(base_key: Option<&str>, index: usize) -> Option<String> {
    base_key.map(|key| format!("{key}-l{index}"))
}

#[cfg(test)]
mod tests {
    use super::lane_storage_label;

    #[test]
    fn new_lane_label_appends_the_lane_suffix_to_the_injected_key() {
        assert_eq!(
            lane_storage_label(Some("127-0-0-1-8080"), 1).as_deref(),
            Some("127-0-0-1-8080-l1")
        );
        assert_eq!(
            lane_storage_label(Some("127-0-0-1-8080"), 2).as_deref(),
            Some("127-0-0-1-8080-l2")
        );
    }

    #[test]
    fn no_injected_key_means_no_lane_scoping() {
        // 纯浏览器 / Android 没有注入标识：不凭空造域，否则它们的键名会漂移。
        assert_eq!(lane_storage_label(None, 1), None);
    }
}
