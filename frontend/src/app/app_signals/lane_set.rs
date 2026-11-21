//! 窗口内的 lane 容器（#561 阶段 3）：**当前只持有一条 lane**。
//!
//! # 这一步解决什么
//!
//! 改造前「当前这条 lane 的信号」是 `AppShellCtx` 上的一个 `pub signals` 字段，壳内
//! **121 处**直接访问它。那种形状下「切 lane」无从下手：视图闭包捕获的是**具体句柄**
//! （`RwSignal` 是 `Copy`），换一条 lane 得挨个改 121 处。
//!
//! 现在改成：容器持 `Vec<AppSignals>` + 当前下标，壳一律经
//! [`AppShellCtx::signals`](super::super::AppShellCtx::signals) 取 —— 于是「当前是哪条」
//! **只有一个入口**。将来切 lane 只需让这个入口返回另一条，或按 lane 重建子树。
//!
//! 本步是 **N=1 等价重构**：容器恒一条，`current()` 恒等于改造前那份 `AppSignals`，
//! 所以单连接行为零变化（frontend 826 条测试守这条）。
//!
//! # 为什么现在只有 `new_single` 与 `current`
//!
//! `set_active` / `len` / `active_index` 要等「第二条 lane 从哪来」定了才有消费者 ——
//! 提前写出来会被 `clippy -D warnings` 判 `dead_code`，而补 `#[expect(dead_code)]`
//! 正是 #558 把 1b 标为「故意没做」要避开的形态。同理 `active` 下标**现在恒为 0**，
//! 但它已经是唯一真源，不必等到那时再引入。
//!
//! # 与 TUI 的对应
//!
//! 对应 `TuiApp` 的 `lanes: Vec<LaneApp>` + `active: usize`
//! （`crates/crabmate-tui/src/tui_mode/mod.rs`）。差别只在承载方式：TUI 是所有权模型
//! （`Vec<LaneApp>` 独占），Web 是信号模型（`AppSignals` 里全是 Copy 句柄，取一份就是
//! 「另一条 lane 的视图」）。

use leptos::prelude::*;

use super::{AppSignals, WindowSignals};
use crate::api::connection_scope;

/// 窗口内的 lane 集合 + 当前前台下标。
#[derive(Clone)]
pub(crate) struct LaneSet {
    /// 每条 lane 一份：`AppSignals` 自带它自己的 `scope`（#558 项 1b）与 `phase`
    /// （#561，当前恒 `Foreground`）。
    lanes: Vec<AppSignals>,
    /// 当前前台下标。**唯一真源**：所有「我现在看的是哪条」都从这里读（现在恒 0）。
    active: RwSignal<usize>,
}

impl LaneSet {
    /// 建一个**只有一条 lane** 的容器。
    ///
    /// `scope` 用 `active_holder()`（就是 `AppSignals::new()` 取的那一份），所以与改造前
    /// 的启动路径逐字等价。
    pub(crate) fn new_single(window: &WindowSignals) -> Self {
        let first = AppSignals::new_lane(window, connection_scope::active_holder());
        Self {
            lanes: vec![first],
            active: RwSignal::new(0),
        }
    }

    /// 当前前台那条 lane 的一组信号。
    ///
    /// 下标越界（理论上不可能：现在没有改下标的入口）时退回第一条 —— 宁可画错一条，
    /// 也别让一处 panic 把整屏打空。
    pub(crate) fn current(&self) -> AppSignals {
        self.lanes
            .get(self.active.get_untracked())
            .or_else(|| self.lanes.first())
            .cloned()
            .expect("LaneSet 至少持有一条 lane")
    }
}
