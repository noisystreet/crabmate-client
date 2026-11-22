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

use crabmate_client_api::connections::ConnectionStatus;
use crabmate_client_api::lane_attach::MAX_LANES;
use crabmate_client_api::lane_phase::LaneEvent;

use crate::api::connection_scope;

use super::super::app_shell_wire_phases::{ShellWiringOutput, run_shell_wiring_in_order};
use super::super::chat::ChatColumnShell;
use super::{AppSignals, WindowSignals};

/// 一条 lane 在**左栏那一行**上要显示的东西（#569 对齐 TUI 的 lane 行）。
///
/// 全是 `RwSignal`（`Copy` + `Send + Sync`），所以能塞进 `RwSignal<Vec<_>>` 再进视图闭包 ——
/// 换成 `Rc<LaneRuntime>` 会当场撞上 §5.2 那道 `Rc cannot be sent between threads safely`。
/// 写入面同样只有一处：`app_shell_effects::connection_report`（它已经在算同一批数字，
/// 再算一遍就是「左栏与连接区两个读数」的老问题）。
#[derive(Clone, Copy)]
pub(crate) struct LaneRailRow {
    /// 行主体：连接名（ctl 实例名 / 用户给的名字）；空 = 用基址兜底。
    pub name: RwSignal<String>,
    /// 基址（空 = 同 Origin）。行上不显示，进 `title`。
    pub api_base: RwSignal<String>,
    /// 这条 lane 此刻在做什么（`·●▲×`）。
    pub status: RwSignal<ConnectionStatus>,
    /// 后台有新输出、用户还没切过去看。
    pub unread: RwSignal<bool>,
    /// 上下文占比短文本（`62%`），与状态栈同一根轴线。
    pub context: RwSignal<Option<String>>,
}

impl LaneRailRow {
    pub(crate) fn new(name: Option<String>, api_base: String) -> Self {
        Self {
            name: RwSignal::new(name.unwrap_or_default()),
            api_base: RwSignal::new(api_base),
            status: RwSignal::new(ConnectionStatus::Idle),
            unread: RwSignal::new(false),
            context: RwSignal::new(None),
        }
    }
}

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
    /// 左栏每行要显示的读数；下标与 `lanes` 对齐（`rail_rows[i]` 描述 `lanes[i]`）。
    ///
    /// 与 `lanes` 分开存的原因：`LaneRuntime` 含 `Rc`，进不了响应式系统与视图闭包；
    /// 这里只放 `Copy` 信号句柄，视图读它就能跟着每条 lane 的状态变。
    rail_rows: RwSignal<Vec<LaneRailRow>>,
}

impl LaneSet {
    /// 建一个**只有一条 lane** 的容器。
    ///
    /// `signals` 必须是 `AppSignals::new_lane(&window, active_holder())` 的产物，
    /// `wiring` 是对它的 `run_shell_wiring_in_order` —— 与改造前的启动路径逐字等价。
    pub(crate) fn new_single(
        window: WindowSignals,
        row: LaneRailRow,
        signals: AppSignals,
        wiring: ShellWiringOutput,
    ) -> Self {
        Self {
            window,
            lanes: Rc::new(RefCell::new(vec![Rc::new(LaneRuntime::new(
                signals, wiring,
            ))])),
            active: RwSignal::new(0),
            rail_rows: RwSignal::new(vec![row]),
        }
    }

    /// 左栏行数据的响应式来源（下标与 [`Self::len`] 对齐）。
    pub(crate) fn rail_rows(&self) -> RwSignal<Vec<LaneRailRow>> {
        self.rail_rows
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

    /// 还能再建一条 lane 吗（`MAX_LANES` 已达）。
    ///
    /// 上限与 TUI 同源：`crabmate_client_api::lane_attach::MAX_LANES`（那份注释写着「Web 沿用
    /// 同一值」），`Alt+1..8` 也只到 8。不设上限的话，点「连接」能堆出任意多条 lane ——
    /// 每条都带一整套接线与后台轮询。
    pub(crate) fn is_full(&self) -> bool {
        self.len() >= MAX_LANES
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
    pub(crate) fn add_lane(
        &self,
        scope: connection_scope::ScopeHolder,
        name: Option<&str>,
        api_base: &str,
    ) -> (Rc<LaneRuntime>, LaneRailRow) {
        // 行先建：接线的 phase 5 要往里写读数（见 `run_shell_wiring_in_order`）。
        let row = LaneRailRow::new(name.map(str::to_string), api_base.to_string());
        let signals = AppSignals::new_lane(&self.window, scope);
        let wiring = run_shell_wiring_in_order(&signals, row);
        let runtime = Rc::new(LaneRuntime::new(signals, wiring));
        self.lanes.borrow_mut().push(Rc::clone(&runtime));
        self.rail_rows.update(|rows| rows.push(row));
        (runtime, row)
    }

    /// 当前前台下标（响应式句柄，给视图层的 `display` 切换订阅）。
    pub(crate) fn active_signal(&self) -> RwSignal<usize> {
        self.active
    }

    /// 切到第 `index` 条 lane（#561 S3）。越界不切，返回 `false`。
    ///
    /// 三件事一起做，**顺序无关但必须都在**：移动「前台默认基址」的指针
    /// （[`connection_scope::activate`]）、改 `active` 下标（视图据此只换 `display`）、
    /// 给两条 lane 各推进一次 `LanePhase`（旧的 → `Background`，新的 → `Foreground`）。
    /// 少了最后一步，「后台降频 / 后台审批」就会读到错的阶段。
    pub(crate) fn set_active(&self, index: usize) -> bool {
        // 先把要写的 `(phase, 新值)` 收集出来，**释放 `lanes` 借用之后**再写信号：
        // `phase` 有真实订阅者（`connection_report` 的 Effect 读它定可见性），若在持借用
        // 时写信号、而重算路径又碰容器，就是一次重入 panic。
        let (target, phase_updates) = {
            let lanes = self.lanes.borrow();
            if index >= lanes.len() {
                return false;
            }
            let updates: Vec<_> = lanes
                .iter()
                .enumerate()
                .map(|(i, runtime)| {
                    let phase = runtime.signals.phase;
                    let next = phase.get_untracked().next(LaneEvent::FocusChanged {
                        foreground: i == index,
                    });
                    (phase, next)
                })
                .filter(|(phase, next)| phase.get_untracked() != *next)
                .collect();
            (Rc::clone(&lanes[index]), updates)
        };
        for (phase, next) in phase_updates {
            phase.set(next);
        }
        connection_scope::activate(&target.signals.scope);
        self.active.set(index);
        true
    }

    /// 相对切换（`Alt+→` / `Alt+←` / `Ctrl+N`）；单 lane 时返回 `false`（no-op）。
    pub(crate) fn cycle(&self, delta: isize) -> bool {
        let len = self.len();
        if len <= 1 {
            return false;
        }
        let next = next_index(self.active.get_untracked(), delta, len);
        self.set_active(next)
    }

    /// 跳到下一条**在等审批**的 lane（`F4`）；没有就返回 `false`（留在原地）。
    ///
    /// 后台审批是这条 lane 上唯一「用户看不见但必须能被叫出来」的事件，所以它单独一键。
    pub(crate) fn focus_next_pending_approval(&self) -> bool {
        let len = self.len();
        if len <= 1 {
            return false;
        }
        let current = self.active.get_untracked();
        let target = {
            let lanes = self.lanes.borrow();
            (1..=len).map(|step| (current + step) % len).find(|index| {
                lanes[*index]
                    .signals
                    .approval
                    .pending_approval
                    .get_untracked()
                    .is_some()
            })
        };
        match target {
            Some(index) => self.set_active(index),
            None => false,
        }
    }
}

/// 循环取模的下一格（纯函数，便于单测）：`current + delta` 落在 `[0, len)`。
#[must_use]
fn next_index(current: usize, delta: isize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    (current as isize + delta).rem_euclid(len as isize) as usize
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
    use super::{lane_storage_label, next_index};

    #[test]
    fn next_index_wraps_both_ways() {
        assert_eq!(next_index(0, 1, 3), 1);
        assert_eq!(next_index(2, 1, 3), 0, "末条 → 第一条（循环）");
        assert_eq!(next_index(0, -1, 3), 2, "第一条 → 末条（循环）");
        assert_eq!(next_index(1, -1, 2), 0);
        assert_eq!(next_index(0, 1, 1), 0, "单 lane 原地");
        assert_eq!(next_index(0, 1, 0), 0, "空容器不 panic");
    }

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
