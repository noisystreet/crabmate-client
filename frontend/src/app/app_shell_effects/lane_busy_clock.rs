//! 左栏 lane 行的**在途耗时**（#577）：起点、秒差与窗口级心跳。
//!
//! TUI 的 lane 行早就显示这一段（`12s` / `2m05s` / `1h02m`），Web 左栏此前只喂了上下文占比
//! 与未读点 —— 同一份口径（`crabmate_client_api::lane_row::tail_parts`），缺的只是数据。
//!
//! - **格式不在这里**：秒数交给 `tail_parts` → `busy_duration_text`，两端同一份实现；
//! - **起点**由 [`next_turn_started`] 定，且**幂等**：写它的那条 `Effect` 在流式期间会因为
//!   上下文用量变化反复重跑，若每次都覆盖成「现在」，秒数就永远停在 0；
//! - **推进**是窗口级的：一个 `now` 信号 + 至多一个 1 s 定时器（[`wire_lane_busy_clock`]），
//!   全空闲就停。这与壳侧 `desktop_connections::spawn_busy_ticker` 是**两个**时钟，各自服务
//!   不同的读者：壳那条给「别的窗口读我这一行」用（`Entry::started_at` 按 window label 记），
//!   本模块给「本窗口 N 条 lane 各自的秒数」用 —— 多 lane 下壳只有一个 `started_at`，秒数会混
//!   （`docs/design/desktop_single_window_lanes.md` §8.1）。
//!
//! 两个纯函数都收 `now_ms` 而不是自己取时钟：`Effect` 在 host 目标上跑不到（测试里没有浏览器
//! 时钟），判断藏在闭包里就永远测不到 —— 与 `connection_report::rail_metrics` 同一条理由。

use leptos::prelude::*;

use crate::app::app_signals::LaneRailRow;

/// 心跳间隔。`busy_secs` 是整秒读数，比它更快只是多一次重绘。
#[cfg(target_arch = "wasm32")]
const TICK_MS: u64 = 1_000;

/// 下一帧的「回合起点」（毫秒）。
///
/// - 空闲（`busy == false`）→ `None`：行尾那一段**整个不出现**（不是 `0s`），与 TUI 的降级
///   规则相同；
/// - 在跑但还没有起点 → 记下 `at_ms`（回合**发出**的那一刻，首 token 之前的排队也算等）；
/// - 在跑且已有起点 → 原样返回。
///
/// 参数叫 `at_ms` 而不是 `now_ms`：后者是本模块的取时钟函数，同名会读成「在传函数」。
#[must_use]
pub(crate) fn next_turn_started(previous: Option<f64>, busy: bool, at_ms: f64) -> Option<f64> {
    if !busy {
        return None;
    }
    previous.or(Some(at_ms))
}

/// 已耗时（整秒）；没有起点时不画。
///
/// 回拨 / 起点落在未来（用户改了系统时间、或拿墙钟做刻度）时饱和到 0：宁可一个 `0s`，也不要
/// 一个巨大的数字或一次 panic —— 这条路径每秒都会被求值。`elapsed == 0` 仍返回 `Some(0)`
/// （刚发出那一帧显示 `0s`，与 TUI 一致），**别**「优化」成 `None`：那会让行尾在头一秒闪一下
/// 再出现。
#[must_use]
pub(crate) fn busy_secs_from(started_ms: Option<f64>, at_ms: f64) -> Option<u64> {
    let elapsed = at_ms - started_ms?;
    if !elapsed.is_finite() || elapsed <= 0.0 {
        return Some(0);
    }
    Some((elapsed / 1_000.0) as u64)
}

/// 心跳此刻该做什么。
///
/// 抽成枚举 + 纯函数是为了**能测**：起/停决策原本埋在 wasm-only 的 `Effect` 闭包里，host 目标
/// 上跑不到，等于没测 —— 而「漏停」（常驻定时器）与「停了不再起」（秒数冻结）正是这条改动
/// 最容易写错的两处。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TickerAction {
    /// 有 lane 在跑，但还没起：现在起。
    Start,
    /// 全空闲，但定时器还挂着：现在收。
    Stop,
    /// 状态没变。
    Keep,
}

/// 由「有没有 lane 在跑」与「定时器在不在」推出该做什么。
#[must_use]
pub(crate) fn ticker_action(any_busy: bool, running: bool) -> TickerAction {
    match (any_busy, running) {
        (true, false) => TickerAction::Start,
        (false, true) => TickerAction::Stop,
        _ => TickerAction::Keep,
    }
}

/// 当前时刻（毫秒，单调）。
///
/// 用 `performance.now()` 而不是 `Date::now()`：后者是墙钟，NTP 校时或用户改时间会让读数跳一
/// 下（TUI 为此用的是 `Instant`）。`Performance` feature 已在 `frontend/Cargo.toml` 打开。
#[cfg(target_arch = "wasm32")]
pub(crate) fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|window| window.performance())
        .map_or(0.0, |performance| performance.now())
}

/// 非 wasm（host 测试 / SSR）：没有浏览器时钟。调用方都带 `now_ms` 参数，这里只兜个底。
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn now_ms() -> f64 {
    0.0
}

/// 装上窗口级的在途心跳（`App` 挂载时调**一次**）。
///
/// 为什么是窗口级：N 条 lane 读的是同一个刻度，每 lane 一个 `setInterval` 在 8 条 lane 时
/// 每秒唤醒 8 次。也不做成全局常驻：全空闲就 `clear`，最常见的静止状态一次都不唤醒。
#[cfg(target_arch = "wasm32")]
pub(crate) fn wire_lane_busy_clock(rows: RwSignal<Vec<LaneRailRow>>, now: RwSignal<f64>) {
    use std::cell::RefCell;
    use std::time::Duration;

    use leptos_dom::helpers::{IntervalHandle, set_interval_with_handle};

    thread_local! {
        /// **本页面**那唯一一个定时器。`IntervalHandle` 不满足 `Send + Sync`、进不了 `RwSignal`，
        /// 所以与 `lane_actions` 的注册表同形：线程局部。
        ///
        /// 前提：**一个页面只调一次**本函数（`App` 挂载时）。同页若出现第二个实例，它会看到
        /// `running == true` 而永不启动自己的心跳 —— 那条 lane 的秒数会静默冻结（症状像「秒数
        /// 不动」，很难查）。真的要多窗口，应该是每个 WebView 页面各一份 wasm 实例，而不是
        /// 同页复用这套注册表。
        static TICKER: RefCell<Option<IntervalHandle>> = const { RefCell::new(None) };
    }

    // 起 / 停各自成函数：`Effect` 里只剩一个 `match`，CCN 留给上面那个纯函数（`ticker_action`）。
    fn start(now: RwSignal<f64>) {
        if let Ok(handle) =
            set_interval_with_handle(move || now.set(now_ms()), Duration::from_millis(TICK_MS))
        {
            TICKER.with(|cell| *cell.borrow_mut() = Some(handle));
        }
    }

    fn stop() {
        if let Some(handle) = TICKER.with(|cell| cell.borrow_mut().take()) {
            handle.clear();
        }
    }

    // 回到前台先补一次刻度：隐藏期间浏览器会节流甚至挂起 `setInterval`，而秒数只在 tick 时前进
    // —— 不补的话切回来会先看到一段旧读数。`focus` 覆盖桌面壳唤醒 / 窗口失焦再聚焦，
    // `visibilitychange` 覆盖浏览器切标签页；两个都挂上，多补一次只是多一次重绘。
    // 与页面同寿：页面活着就一直要补，没有摘下来的时刻。
    {
        use wasm_bindgen::JsCast;
        use wasm_bindgen::closure::Closure;

        let bump = Closure::<dyn FnMut()>::new(move || now.set(now_ms()));
        let callback = bump.as_ref().unchecked_ref();
        if let Some(window) = web_sys::window() {
            let _ = window.add_event_listener_with_callback("focus", callback);
            if let Some(document) = window.document() {
                let _ = document.add_event_listener_with_callback("visibilitychange", callback);
            }
        }
        bump.forget();
    }

    // 页面卸载 / owner 被 dispose 时收掉定时器：否则在没有行可画之后它仍会每秒醒来一次。
    on_cleanup(stop);

    Effect::new(move |_| {
        // 逐条读行内的起点：`rows.get()` 只订阅容器（加 lane 时重跑），行内信号要各自读。
        let any_busy = rows.with(|rows| rows.iter().any(|row| row.turn_started_at.get().is_some()));
        let running = TICKER.with(|cell| cell.borrow().is_some());
        match ticker_action(any_busy, running) {
            TickerAction::Start => start(now),
            TickerAction::Stop => stop(),
            TickerAction::Keep => {}
        }
    });
}

/// 非 wasm（host 测试 / SSR）：没有定时器可起，也没有行会 `busy`。
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn wire_lane_busy_clock(rows: RwSignal<Vec<LaneRailRow>>, now: RwSignal<f64>) {
    let _ = (rows, now);
}

#[cfg(test)]
mod tests {
    use super::{TickerAction, busy_secs_from, next_turn_started, ticker_action};

    #[test]
    fn the_ticker_ticks_only_while_something_is_in_flight() {
        // 漏掉 `Start` → 秒数冻结；漏掉 `Stop` → 静止时每秒醒一次（笔记本上看得出来）。
        assert_eq!(ticker_action(true, false), TickerAction::Start);
        assert_eq!(ticker_action(false, true), TickerAction::Stop);
        assert_eq!(ticker_action(true, true), TickerAction::Keep);
        assert_eq!(ticker_action(false, false), TickerAction::Keep);
    }

    #[test]
    fn an_idle_frame_clears_the_start() {
        // 回合结束 → 起点清空，行尾那段整个消失（不是 `0s`）。
        assert_eq!(next_turn_started(Some(1_000.0), false, 5_000.0), None);
        assert_eq!(next_turn_started(None, false, 5_000.0), None);
    }

    #[test]
    fn a_busy_frame_records_the_start_only_once() {
        let started = next_turn_started(None, true, 1_000.0);
        assert_eq!(started, Some(1_000.0));
        // 关键一条：流式期间 Effect 会因为 context 变化反复重跑，起点**不能**跟着往后跳。
        assert_eq!(next_turn_started(started, true, 2_500.0), Some(1_000.0));
        assert_eq!(next_turn_started(started, true, 60_000.0), Some(1_000.0));
    }

    #[test]
    fn the_elapsed_seconds_follow_the_start() {
        assert_eq!(busy_secs_from(None, 10_000.0), None);
        // 刚发出：与 TUI 一样显示 `0s`，而不是空。
        assert_eq!(busy_secs_from(Some(1_000.0), 1_000.0), Some(0));
        assert_eq!(busy_secs_from(Some(1_000.0), 1_999.0), Some(0));
        assert_eq!(busy_secs_from(Some(1_000.0), 13_000.0), Some(12));
        assert_eq!(busy_secs_from(Some(0.0), 125_000.0), Some(125));
    }

    #[test]
    fn a_clock_going_backwards_saturates_instead_of_panicking() {
        // 墙钟回拨 / NTP 校正：读数落到 0，不出现负数和巨大数字。
        assert_eq!(busy_secs_from(Some(5_000.0), 4_000.0), Some(0));
        assert_eq!(busy_secs_from(Some(f64::NAN), 4_000.0), Some(0));
    }
}
