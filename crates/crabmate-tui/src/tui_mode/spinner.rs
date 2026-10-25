//! 生成中 spinner：帧号 + 按墙钟推进。
//!
//! 事件循环本来就每帧无条件重绘（有消息 2ms、空闲 50ms），所以动画不需要额外
//! 定时器；只需按墙钟推进帧号，渲染侧索引 `render::SPINNER_FRAMES` 即可。
//! 关键：必须按时间推进而非按帧号自增 —— 否则有消息时（2ms/帧）会比空闲时
//! （50ms/帧）快约 25 倍，看起来像抽搐。

use std::time::{Duration, Instant};

/// 换帧间隔（渲染约 20fps，即 2–3 帧换一次）。
const SPINNER_INTERVAL: Duration = Duration::from_millis(120);

/// 生成中动画状态。非生成中归零，保证下一回合从首帧开始。
pub(super) struct Spinner {
    frame: usize,
    at: Instant,
}

impl Spinner {
    pub(super) fn new() -> Self {
        Self {
            frame: 0,
            at: Instant::now(),
        }
    }

    /// 推进一帧（仅在 `spinning` 且距上次换帧已达间隔时）。
    /// `spinning` 由调用方给，避免本模块依赖 `UiState`。
    pub(super) fn advance(&mut self, spinning: bool) {
        if !spinning {
            self.frame = 0;
            self.at = Instant::now();
            return;
        }
        if self.at.elapsed() >= SPINNER_INTERVAL {
            self.frame = self.frame.wrapping_add(1);
            self.at = Instant::now();
        }
    }

    /// 当前帧号（渲染侧按 `% SPINNER_FRAMES.len()` 索引）。
    pub(super) fn frame(&self) -> usize {
        self.frame
    }

    /// 距下一次换帧还剩多久（已过期返回 0）。
    ///
    /// 事件循环据此算唤醒 deadline：生成中不能像静态那样长等到兜底超时，
    /// 否则 spinner 会「120ms 一帧」退化成「1s 一帧」。
    pub(super) fn remaining(&self) -> Duration {
        SPINNER_INTERVAL.saturating_sub(self.at.elapsed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_resets_to_first_frame() {
        let mut s = Spinner {
            frame: 5,
            at: Instant::now(),
        };
        s.advance(false);
        assert_eq!(s.frame(), 0, "非生成中归零，相位不残留到下一回合");
    }

    #[test]
    fn advance_within_interval_keeps_frame() {
        let mut s = Spinner {
            frame: 3,
            at: Instant::now(),
        };
        // 刚换过帧，间隔未到：不应推进。
        s.advance(true);
        assert_eq!(s.frame(), 3);
    }

    #[test]
    fn advances_once_interval_elapsed() {
        let past = Instant::now()
            .checked_sub(SPINNER_INTERVAL)
            .expect("系统时钟不支持回退 Instant");
        let mut s = Spinner { frame: 0, at: past };
        s.advance(true);
        assert_eq!(s.frame(), 1);
    }
}
