//! 左栏连接区用到的 authority 计算（窗口注册表 + 本窗口每条 lane）。
//!
//! 单独成文件是行数门禁的要求（`nav_rail_connections.rs` 贴着 920 行），与
//! `nav_rail_lanes.rs` / `nav_rail_other_windows.rs` 同一处置：`#[path]` 只换文件，
//! 模块路径仍是 `nav_rail_connections::authorities`。
//!
//! 全在这里的理由：「什么算已占用」的口径只该有一份 —— 可连实例的剔除
//! （`ctl_rows_for_rail`）与「其它窗口」的剔除（`other_window_rows`）读同一组 authority。

use leptos::prelude::*;

use crabmate_client_api::connections::ConnectionSnapshot;
use crabmate_client_api::url::api_base_authority;

use crate::app::lane_actions;

/// 窗口级连接注册表里已占用的 authority。
fn snapshot_authorities(snapshots: &[ConnectionSnapshot]) -> Vec<String> {
    snapshots
        .iter()
        .map(|snapshot| snapshot.authority.clone())
        .filter(|authority| !authority.is_empty())
        .collect()
}

/// 本窗口每条 lane 已占用的 authority。
///
/// 空基址 = 同 Origin（设置页那颗按钮新建的 lane），用 `fallback`（窗口当前基址）兜底 ——
/// 否则那条 lane 会被算成「没占用」，同一个 Origin 的实例又出现在「可连实例」里。
///
/// 抽成自由函数而不是写在组件里：`NavRailConnections` 贴着 `lizard` 的 CCN>10 门禁。
fn lane_authorities(rows: &[crate::app::app_signals::LaneRailRow], fallback: &str) -> Vec<String> {
    rows.iter()
        .map(|row| {
            let base = row.api_base.get();
            let base = if base.trim().is_empty() {
                fallback.to_string()
            } else {
                base
            };
            api_base_authority(&base)
        })
        .filter(|authority| !authority.is_empty())
        .collect()
}

/// 窗口注册表里**不属于本窗口**的那些（「其它窗口」分组）。
///
/// 判据是「这条记录的 authority 不在**本窗口任意一条 lane** 的基址里」，**不是**
/// 「不等于当前前台 lane 的基址」。后者正是这里踩过的坑：`api_base` 随切 lane 而变，
/// 切到第二条 lane 之后，窗口注册表里那条初始连接的记录立刻「不等于前台基址」，
/// 于是它被当成「另一个窗口」冒出来 —— 症状就是「点一下切 lane，多出一个其他窗口」。
pub(super) fn other_window_rows(
    snapshots: &[ConnectionSnapshot],
    lane_authorities: &[String],
) -> Vec<ConnectionSnapshot> {
    snapshots
        .iter()
        .filter(|snapshot| !lane_authorities.contains(&snapshot.authority))
        .cloned()
        .collect()
}

/// 本窗口每条 lane 占用的 authority（响应式）。空基址 = 同 Origin，用窗口当前基址兜底。
pub(super) fn lane_authority_signal(fallback: RwSignal<String>) -> Signal<Vec<String>> {
    Signal::derive(move || {
        let lane_rows = lane_actions::rail().map_or_else(Vec::new, |(r, _)| r.get());
        lane_authorities(&lane_rows, &fallback.get())
    })
}

/// 已占用的 authority（响应式）：窗口级注册表 + 本窗口每条 lane 的基址。
///
/// 两个来源缺一不可：#561 S2 之后「连接」= 在当前窗口新建一条 lane，它不在窗口注册表里；
/// 而别的窗口 / 老路径仍只体现在注册表里。
pub(super) fn taken_authorities_signal(
    rows: Signal<Vec<ConnectionSnapshot>>,
    lane_taken: Signal<Vec<String>>,
) -> Signal<Vec<String>> {
    Signal::derive(move || {
        let mut out = snapshot_authorities(&rows.get());
        out.extend(lane_taken.get());
        out
    })
}
