//! 网络 / 前台恢复后的**受控**重试：`online` 与 `visibilitychange → visible` → 只补失败项。
//!
//! issue #23：Android 切 Wi-Fi/蜂窝、App 回前台、桌面睡眠唤醒或 serve 重启之后，`/status`
//! 会停在 `Failed` 再也不动。根因是 [`StatusFetchPhase::allows_auto_fetch`] 只放行 `Idle`，
//! 而初始化那条 Effect 只依赖 `initialized`（一生只变一次）—— 于是除了手点重试没有出口。
//!
//! 为什么不是「把 `Failed` 也放进 `allows_auto_fetch`」：那条谓词同时被初始化 Effect 使用，
//! 一旦放行，服务端不可达时它会反复自旋。恢复必须是**外部事件驱动 + 有闸门**的：
//!
//! | 闸 | 作用 |
//! |---|---|
//! | 前台 lane | 一个事件只让**用户正看着的那条**去补；后台 lane 等切过去或自己的轮询 |
//! | 只补失败项 | `Ready` / `Idle` 一律不碰，切标签不会顺带刷新健康数据 |
//! | 并发闸 | 目标正在 `Fetching` / 加载中 → 跳过（`refresh_status` 本身也幂等） |
//! | 冷却 + 退避 | `online` 与 `visible` 常同时到达；连续失败时按 1s→2s→4s… 上限 30s 退避 |
//!
//! prefs 侧只**读**不写：`user_prefs_allows_persist` 已经限定「只有 `Ready` / `SaveFailed`
//! 才 PUT」，恢复路径递增 `user_prefs_reload_nonce` 只会重新 GET，不会用默认值覆盖服务端
//! 已有偏好（issue #23 验收第 2 条）。

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use crabmate_client_api::lane_phase::LanePhase;
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use crate::app::shell::status::fetch_state::StatusFetchPhase;
use crate::app::shell::status::tasks_state::StatusTasksSignals;
use crate::user_prefs_sync_state::UserPrefsSyncPhase;

/// 两次恢复动作之间的最小间隔（毫秒）：`online` 与 `visibilitychange` 常同时到达。
const MIN_INTERVAL_MS: f64 = 1_500.0;

/// 连续失败退避的上限（毫秒）。
const BACKOFF_MAX_MS: f64 = 30_000.0;

/// 恢复闸的纯状态（不碰 DOM，便于单测）。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RecoveryGate {
    /// 上次真正发起恢复的 `Date.now()`；`0` = 从未。
    last_attempt_ms: f64,
    /// 已发起的恢复次数（两路都恢复健康时清零）。退避按它分档。
    attempts: u32,
}

impl RecoveryGate {
    /// 记一次已发起的恢复。
    #[must_use]
    pub fn attempted(self, now_ms: f64) -> Self {
        Self {
            last_attempt_ms: now_ms,
            attempts: self.attempts.saturating_add(1),
        }
    }

    /// 恢复健康：退避额度清零。
    #[must_use]
    pub fn healed(self) -> Self {
        Self::default()
    }
}

/// 已经尝试过 `attempts` 次时，下一次要等多久。
///
/// `0` = 还没试过 → 立刻就可以（首次恢复不该凭空多等）；之后从 [`MIN_INTERVAL_MS`]
/// 起翻倍：1.5s、3s、6s、12s、24s，封顶 [`BACKOFF_MAX_MS`]。
#[must_use]
pub fn backoff_ms(attempts: u32) -> f64 {
    if attempts == 0 {
        return 0.0;
    }
    // 指数封顶 5（1.5s * 32 = 48s > 30s 上限），避免大数移位溢出。
    let exp = (attempts - 1).min(5);
    (MIN_INTERVAL_MS * f64::from(1u32 << exp)).min(BACKOFF_MAX_MS)
}

/// 这次事件能不能触发恢复。
///
/// 只管**节奏**（冷却 + 退避）。「不重复发同一个请求」由目标自身保证：`/status` 只在
/// `Failed` 时补（`Fetching` 时它不可能是 `Failed`），prefs 的重载 nonce 是幂等的。
///
/// 这里**刻意不接收**「status 是否在途」—— 那会把两路绑在一起：`/status` 正在跑时
/// prefs 的恢复会被无谓地跳过（两件事互相独立）。
#[must_use]
pub fn should_recover(gate: RecoveryGate, now_ms: f64) -> bool {
    // `backoff_ms(0) == 0`：从未尝试过时 `wait` 就是 0，事件立即生效。
    let wait = backoff_ms(gate.attempts);
    now_ms - gate.last_attempt_ms >= wait
}

/// 这一轮该补哪几路 —— 纯决策，便于单测。
///
/// 两路互不牵连：`/status` 在途（`Fetching`）不影响 prefs 的重读，反过来也一样。
/// 之前这里共用一个「在途闸」，导致 `status=Fetching` 时 prefs 的恢复被一起吞掉。
#[must_use]
pub fn recovery_targets(status: StatusFetchPhase, prefs: UserPrefsSyncPhase) -> (bool, bool) {
    (
        status == StatusFetchPhase::Failed,
        prefs == UserPrefsSyncPhase::LoadFailed,
    )
}

/// 事件恢复需要的信号。收成一个结构体，避免 6 个位置参数撞上 `fn-param` 门禁。
pub struct ConnectivityRecoveryArgs {
    pub status: StatusTasksSignals,
    /// `/status` 的刷新闭包（**per-lane**：后台 lane 拿到的是别人家的 serve）。
    pub refresh_status: Arc<dyn Fn() + Send + Sync>,
    pub prefs_phase: RwSignal<UserPrefsSyncPhase>,
    pub prefs_reload_nonce: RwSignal<u64>,
    /// 本 lane 的前后台相位：只有 `Foreground` 才动手。
    pub phase: RwSignal<LanePhase>,
}

fn now_ms() -> f64 {
    js_sys::Date::now()
}

/// 挂 `online` + `visibilitychange`：恢复失败的状态与偏好拉取。
///
/// 两个事件都要挂 —— WebView 里 Android 切网不一定发 `online`，切前台也不一定发 `online`；
/// 只挂一个必然漏掉另一条路径（issue #23 的 Android 场景正是两者交替发生）。
pub fn wire_connectivity_recovery(args: ConnectivityRecoveryArgs) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(document) = window.document() else {
        return;
    };

    let gate = Rc::new(Cell::new(RecoveryGate::default()));
    let args = Rc::new(args);
    let try_recover: Rc<dyn Fn()> = Rc::new({
        let gate = Rc::clone(&gate);
        let args = Rc::clone(&args);
        move || recover_if_needed(&args, &gate)
    });

    let on_online: Closure<dyn FnMut()> = {
        let try_recover = Rc::clone(&try_recover);
        Closure::wrap(Box::new(move || try_recover()) as Box<dyn FnMut()>)
    };
    let _ = window.add_event_listener_with_callback("online", on_online.as_ref().unchecked_ref());
    on_online.forget();

    let on_visible: Closure<dyn FnMut()> = {
        let try_recover = Rc::clone(&try_recover);
        let doc = document.clone();
        Closure::wrap(Box::new(move || {
            // 只认「变成可见」这一半：切走时没必要打请求。
            if !doc.hidden() {
                try_recover();
            }
        }) as Box<dyn FnMut()>)
    };
    let _ = document
        .add_event_listener_with_callback("visibilitychange", on_visible.as_ref().unchecked_ref());
    on_visible.forget();
}

/// 一次恢复尝试：判前台 → 判有无失败项 → 过闸 → 只补失败的那几路。
fn recover_if_needed(args: &ConnectivityRecoveryArgs, gate: &Cell<RecoveryGate>) {
    // 前台闸：后台 lane 的 `/status` 属于另一个 serve，一次事件不该把所有 lane 都唤醒。
    if !matches!(args.phase.get_untracked(), LanePhase::Foreground) {
        return;
    }
    let (recover_status, recover_prefs) = recovery_targets(
        args.status.status_fetch_phase.get_untracked(),
        args.prefs_phase.get_untracked(),
    );
    if !recover_status && !recover_prefs {
        // 都健康：把退避额度清掉，下次真出问题从最短冷却重新算。
        gate.set(gate.get().healed());
        return;
    }
    let now = now_ms();
    if !should_recover(gate.get(), now) {
        return;
    }
    gate.set(gate.get().attempted(now));
    if recover_status {
        // 绕过 `allows_auto_fetch`（它只放行 Idle）：这里已由前台 + 冷却 + 退避把住，
        // `refresh_status` 内部还会挡掉 `Fetching`。
        (args.refresh_status)();
    }
    if recover_prefs {
        args.prefs_reload_nonce.update(|n| *n = n.wrapping_add(1));
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BACKOFF_MAX_MS, MIN_INTERVAL_MS, RecoveryGate, backoff_ms, recovery_targets, should_recover,
    };

    /// 退避序列：1s / 2s / 4s / 8s / 16s / 30s（封顶）—— 失败越多等越久，但有上界，
    /// 否则长时间离线之后恢复要等十几分钟。
    #[test]
    fn backoff_grows_then_caps() {
        assert_eq!(backoff_ms(0), 0.0, "还没试过：不必等待");
        assert_eq!(backoff_ms(1), MIN_INTERVAL_MS);
        assert_eq!(backoff_ms(2), MIN_INTERVAL_MS * 2.0);
        assert_eq!(backoff_ms(3), MIN_INTERVAL_MS * 4.0);
        assert_eq!(backoff_ms(4), MIN_INTERVAL_MS * 8.0);
        assert_eq!(backoff_ms(u32::MAX), BACKOFF_MAX_MS, "大数不溢出且封顶");
    }

    /// 两路互不牵连：`/status` 在途时 prefs 的恢复**不能**被吞掉。
    ///
    /// 回归要点：早先这里共用一个「在途闸」，`status=Fetching` 会把 prefs 的
    /// `LoadFailed` 一起挡掉 —— 表现为「网络回来了但偏好仍是错误态」。
    #[test]
    fn a_fetching_status_does_not_block_prefs_recovery() {
        use crate::app::shell::status::fetch_state::StatusFetchPhase as S;
        use crate::user_prefs_sync_state::UserPrefsSyncPhase as P;

        assert_eq!(recovery_targets(S::Fetching, P::LoadFailed), (false, true));
        assert_eq!(recovery_targets(S::Failed, P::LoadFailed), (true, true));
        assert_eq!(recovery_targets(S::Failed, P::Ready), (true, false));
        assert_eq!(recovery_targets(S::Fetching, P::Ready), (false, false));
        // `Idle` 不补：那是「还没拉过」，由初始化的 Effect 负责，恢复不该抢。
        assert_eq!(recovery_targets(S::Idle, P::Ready), (false, false));
    }

    /// 冷却：`online` 与 `visible` 同时到达时只放行一次。
    #[test]
    fn cooldown_swallows_the_second_event() {
        let gate = RecoveryGate::default();
        assert!(should_recover(gate, 10_000.0), "首次放行");
        let gate = gate.attempted(10_000.0);
        assert!(
            !should_recover(gate, 10_000.5),
            "同一批事件里的第二个应被冷却吞掉"
        );
        assert!(
            should_recover(gate, 10_000.0 + MIN_INTERVAL_MS),
            "冷却过后放行"
        );
    }

    /// 连续失败：第二次恢复要等满退避，而不是 1.5s 就再来。
    #[test]
    fn consecutive_attempts_widen_the_window() {
        let gate = RecoveryGate::default().attempted(0.0).attempted(0.0);
        // 已尝试两次 → 等 3s，而不是 1.5s 就再来。
        assert!(!should_recover(gate, 2_000.0));
        assert!(should_recover(gate, 3_000.0));
    }

    /// 恢复健康后额度清零：下次失败不必一上来就等 30s。
    #[test]
    fn healed_resets_the_backoff() {
        let gate = RecoveryGate::default()
            .attempted(0.0)
            .attempted(0.0)
            .attempted(0.0);
        assert_eq!(gate.healed(), RecoveryGate::default());
    }
}
