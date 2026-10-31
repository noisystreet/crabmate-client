//! 生成中 spinner：帧号由墙钟推导（不累积计数）。
//!
//! 帧号只取决于「这一轮发出到现在过了多久」，所以谁都不需要每帧去推进它 ——
//! 多 lane 并行时后台 lane 的动画天然跟着时间走（它既不进 `draw_frame`，
//! 也没有自己的调度）。曾经是每 lane 一份计数器由 `draw_frame` 推进，结果
//! 只有当前 lane 被推，后台 lane 的帧号永远停在切走时那一格。
//!
//! 关键：必须按时间推导而非按帧号自增 —— 否则有消息时（2ms/帧）会比空闲时
//! （50ms/帧）快约 25 倍，看起来像抽搐。

use std::time::Duration;

/// 换帧间隔（渲染约 20fps，即 2–3 帧换一次）。
pub(super) const SPINNER_INTERVAL: Duration = Duration::from_millis(120);

/// 回合发出后过了 `elapsed` 时的帧号（渲染侧按 `% SPINNER_FRAMES.len()` 索引）。
///
/// 非生成中传 `Duration::ZERO`：帧号回到首帧，相位不残留到下一轮。
pub(super) fn frame_at(elapsed: Duration) -> usize {
    (elapsed.as_millis() / SPINNER_INTERVAL.as_millis()) as usize
}

/// 距下一次换帧还有多久（事件循环据此算唤醒 deadline）。
///
/// 正好落在帧边界上时返回**一整个**间隔：那一刻的帧已经该画了，下一帧还在后面。
pub(super) fn next_tick(elapsed: Duration) -> Duration {
    let step = SPINNER_INTERVAL.as_millis();
    let into = elapsed.as_millis() % step;
    Duration::from_millis((step - into) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_starts_at_zero_and_steps_once_per_interval() {
        assert_eq!(frame_at(Duration::ZERO), 0, "回合刚开始是首帧");
        assert_eq!(frame_at(SPINNER_INTERVAL - Duration::from_millis(1)), 0);
        assert_eq!(frame_at(SPINNER_INTERVAL), 1);
        assert_eq!(frame_at(SPINNER_INTERVAL * 3 + Duration::from_millis(5)), 3);
    }

    #[test]
    fn next_tick_counts_down_to_the_next_frame() {
        assert_eq!(
            next_tick(Duration::ZERO),
            SPINNER_INTERVAL,
            "刚换过帧：等满一个间隔"
        );
        // 落在边界上不能算 0，否则 `recv_timeout(0)` 变成忙转。
        assert_eq!(next_tick(SPINNER_INTERVAL), SPINNER_INTERVAL);
        assert_eq!(
            next_tick(SPINNER_INTERVAL + Duration::from_millis(1)),
            Duration::from_millis(119)
        );
    }
}
