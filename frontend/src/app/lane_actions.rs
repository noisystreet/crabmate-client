//! 窗口级「新建一条 lane」的动作注册表（#561 **S2**）。
//!
//! # 为什么是一张注册表，而不是一路传参
//!
//! 「连接」改的是**窗口级**的 lane 容器（[`AppShellCtx`](super::AppShellCtx)），但它的两个
//! 入口都在很深的子视图里（设置页的 `SettingsConnectionNewWindowBlock`、左栏 ctl 行的
//! 「连接」），中间隔着 `SettingsPageViewInput` / `CtlRow` 一长串参数表。穿参数要改 6 处签名，
//! 而这个动作**每窗口唯一** —— 与 `confirm_dialog::register_shell_confirm` 同一个形状：
//! 注册一次，入口零参取用。
//!
//! # 为什么参数里只有基址
//!
//! 设置页那颗按钮给 `None`（沿用当前基址 / 同 Origin），左栏 ctl 行给目标实例的地址 ——
//! 这是两处入口**唯一**的差别。其余（存储域后缀、owner、挂载点）都由 `App` 侧决定。
//!
//! 句柄是 `Rc<dyn Fn(Option<String>)>`：`on:click` 回调允许捕获含 `Rc` 的值（§5.2 实测过
//! 这条边界），所以不需要 `Send + Sync`。

use std::cell::RefCell;
use std::rc::Rc;

use leptos::prelude::RwSignal;

/// 「新建 lane」动作的句柄：参数是这条 lane 要连的基址（`None` = 沿用当前基址 / 同 Origin）。
type NewLaneAction = Rc<dyn Fn(Option<String>)>;

/// 左栏 lane 列表的数据源（#561 S3b）：每条 lane 的展示地址 + 当前前台下标。
///
/// 两个都是 `RwSignal`（`Copy` 且 `Send + Sync`），所以能直接进 `move ||` 响应式闭包 ——
/// 换成 `Rc<RefCell<Vec<_>>>` 会在那里报 `Rc cannot be sent between threads safely`
/// （§5.2 实测的那道边界）。切换动作走 [`switch_to`]，同样不进闭包。
type LaneRailSignals = (RwSignal<Vec<String>>, RwSignal<usize>);

/// 点左栏某一行时切过去。
type SwitchToAction = Rc<dyn Fn(usize)>;

thread_local! {
    /// 本窗口的「新建 lane」动作；由 `App` 挂载时注册一次。
    static NEW_LANE: RefCell<Option<NewLaneAction>> = const { RefCell::new(None) };
    /// 左栏 lane 列表的数据源。
    static RAIL: RefCell<Option<LaneRailSignals>> = const { RefCell::new(None) };
    /// 点左栏某一行时切过去。
    static SWITCH_TO: RefCell<Option<SwitchToAction>> = const { RefCell::new(None) };
}

/// 注册本窗口的「新建 lane」动作（`App` 调一次）。
pub(crate) fn register_new_lane(action: NewLaneAction) {
    NEW_LANE.with(|cell| *cell.borrow_mut() = Some(action));
}

/// 跑一次「新建 lane」。
///
/// 未注册时静默返回：非桌面壳（Android / 纯浏览器）与单测里没有 `App` 挂载，入口本来也不
/// 该出现在那条路径上（`NavRailConnections` 只在桌面壳渲染）。
pub(crate) fn new_lane(base: Option<String>) {
    let action = NEW_LANE.with(|cell| cell.borrow().clone());
    if let Some(action) = action {
        action(base);
    }
}

/// 注册左栏 lane 列表的数据源与切换动作（`App` 调一次）。
pub(crate) fn register_rail(
    labels: RwSignal<Vec<String>>,
    active: RwSignal<usize>,
    switch_to: Rc<dyn Fn(usize)>,
) {
    RAIL.with(|cell| *cell.borrow_mut() = Some((labels, active)));
    SWITCH_TO.with(|cell| *cell.borrow_mut() = Some(switch_to));
}

/// 左栏 lane 列表的数据源；未注册时 `None`（非桌面壳 / 单测）。
pub(crate) fn rail() -> Option<LaneRailSignals> {
    RAIL.with(|cell| *cell.borrow())
}

/// 切到第 `index` 条 lane（未注册时静默返回）。
pub(crate) fn switch_to(index: usize) {
    let action = SWITCH_TO.with(|cell| cell.borrow().clone());
    if let Some(action) = action {
        action(index);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_without_registration_is_a_no_op() {
        // 注册表是 thread_local：本线程没注册过，调用必须不 panic。
        new_lane(Some("http://127.0.0.1:18080".to_string()));
    }
}
