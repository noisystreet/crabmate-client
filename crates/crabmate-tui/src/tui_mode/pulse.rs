//! lane 行「在跑」指示的呼吸相位。
//!
//! 取代原来的 Braille 转圈：字形固定为 `●`，只让**颜色亮度**走一条三角波。这样
//! 行宽恒定（换帧不挤动折行点），也不必为一个动画多占一列 —— 窄侧栏里那一列是
//! 名字的空间。
//!
//! 相位与帧号同源（[`spinner::frame_at`] 的拍数），于是两条性质白拿：
//! - 后台 lane 不进 `draw_frame` 也照样在呼吸（墙钟推导，谁也不推谁）；
//! - 唤醒源与空帧跳过判定沿用 `lane_list::spinner_turning`，不必新增第二条节拍。

use std::time::Duration;

use ratatui::style::Color;

use super::spinner;

/// 一个方向的档数：相位走 `0..STEPS` 再走回来（三角波，不是锯齿）。
///
/// 呼吸要「亮 → 暗 → 亮」；锯齿会在最暗处一帧跳回最亮，看起来像闪断而不是呼吸。
pub(super) const STEPS: u64 = 4;

/// 每档停留几拍。[`spinner`] 一拍 120ms → 240ms/档，与状态行仪器条同一节奏；
/// 走完一个完整呼吸（亮→暗→亮）约 1.2s，慢到不抢注意力、快到一眼看出在动。
const TICKS_PER_STEP: u64 = 2;

/// 最暗档保留基础色的百分比。
///
/// 再暗就贴上行底色（`overlay_bg #1f2033`）：后台 lane 的那颗点会**消失一瞬**，
/// 读起来像「这条停了」，而不是「它还在动」。
const FLOOR_PCT: u32 = 55;

/// 回合发出后过了 `elapsed` 时的呼吸档位（`0..STEPS`，0 = 最亮）。
pub(super) fn step_at(elapsed: Duration) -> u64 {
    let ticks = spinner::frame_at(elapsed) as u64 / TICKS_PER_STEP;
    // 三角波：上升半程 `0..STEPS`，下降半程往回走（端点不重复数两次）。
    let period = STEPS * 2 - 2;
    let t = ticks % period;
    if t < STEPS { t } else { period - t }
}

/// 档位 → 亮度百分比；越界钳到两端（`usize` 帧号可能极大）。
fn pct(step: u64) -> u32 {
    let last = (STEPS - 1) as u32;
    let step = u32::try_from(step).unwrap_or(u32::MAX).min(last);
    FLOOR_PCT + (100 - FLOOR_PCT) * (last - step) / last
}

/// 按档位把 `base` 压暗。
///
/// 非 `Rgb` 原样返回：16 色 / 无色模式下没有可插值的通道，于是这一行**视觉上静止**
/// —— 那不是缺陷。`NO_COLOR` 从来就不该靠颜色说话，`●` 字形本身还在，而
/// 「在跑」的语义仍由符号区分（`·` 空闲 / `▲` 审批 / `×` 退出）。
pub(super) fn color_at(base: Color, step: u64) -> Color {
    match base {
        Color::Rgb(r, g, b) => {
            let k = pct(step);
            let dim = |c: u8| (u32::from(c) * k / 100) as u8;
            Color::Rgb(dim(r), dim(g), dim(b))
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一档的时长（与 [`TICKS_PER_STEP`] 对齐，测试里不重复写 240ms）。
    fn beat() -> Duration {
        Duration::from_millis(120) * TICKS_PER_STEP as u32
    }

    /// 三角波：走到底再回来，不会在最暗处跳回最亮。
    #[test]
    fn step_is_a_triangle_not_a_sawtooth() {
        let at = |n: u32| step_at(beat() * n);
        let seq: Vec<u64> = (0..6).map(at).collect();
        assert_eq!(seq, vec![0, 1, 2, 3, 2, 1], "一个周期是上去再下来");
        assert_eq!(at(6), 0, "周期回绕到最亮");
    }

    /// 最亮档就是原色，最暗档**不暗到 0** —— 后台 lane 的点不许「消失一瞬」。
    #[test]
    fn brightness_stays_between_floor_and_full() {
        let base = Color::Rgb(0x8f, 0xa4, 0xea);
        assert_eq!(color_at(base, 0), base, "0 档不压暗");
        let darkest = color_at(base, STEPS - 1);
        let Color::Rgb(r, _, b) = darkest else {
            panic!("Rgb 进去应仍是 Rgb：{darkest:?}");
        };
        assert_eq!(u32::from(r), 0x8f * FLOOR_PCT / 100);
        assert!(u32::from(r) > 0 && u32::from(b) > 0, "不许压到全黑");
        // 越界档位钳到最暗，不 panic。
        assert_eq!(color_at(base, 99), darkest);
    }

    /// 16 色 / 无色模式原样返回：这一行静止，靠 `●` 字形兜底。
    #[test]
    fn non_rgb_colors_are_left_alone() {
        assert_eq!(color_at(Color::LightBlue, 2), Color::LightBlue);
        assert_eq!(color_at(Color::Reset, 3), Color::Reset);
    }
}
