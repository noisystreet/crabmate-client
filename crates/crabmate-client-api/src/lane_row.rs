//! lane 行的**读数内容与降级规则**（纯逻辑，无 IO、不依赖宽度库）。
//!
//! TUI 与 Web 左栏的 lane 行是同一件事：一条 lane 一行，行尾挂「上下文占比 / 在途耗时 / 未读点」。
//! 三样在窄栏里都放不下，**先丢谁**必须两端一致 —— 各写一遍的症状是「同一台机器上，TUI 留着
//! 耗时、Web 却把它丢了」，而这种偏差只在窗口变窄时才露头，最难被发现。
//!
//! 规则来自 TUI 的实测结论（`docs/design/tui_multi_server.md` §4.6）：
//!
//! - 最右的未读点**最重要** —— 它就是「切过去看看」的理由，宁可少一个数字也不能丢它；
//! - 其次是耗时（回合还在跑，秒数在涨，是"它没卡住"的证据）；
//! - 再次才是上下文占比；
//! - 装不下时**从左边丢**，且读数整体不超过半行 —— 这一行首先是「哪条 lane」。
//!
//! 本模块只产出**有序的字符串片段**与「按给定宽度裁剪」的结果；怎么排成一行（终端 `Span` 还是
//! DOM 元素）由各端决定。宽度计算以 `width_of` 回调注入：终端传 `UnicodeWidthStr::width`（CJK 按
//! 2 列）；浏览器不做宽度裁剪、直接取 [`tail_parts`] 交给 CSS。
//! **别传 `str::len`** —— 那是字节数，`•` 一个字符 3 字节，会把读数误判成放不下；
//! 真要按"字数"算就传 `str::chars().count()`。

use crate::connections::busy_duration_text;

/// 名字至少留几列，才值得在行尾画读数；不够就整段尾部让位给名字。
pub const LANE_LABEL_MIN: usize = 4;

/// 一条 lane 行的读数来源（只取渲染需要的那几项）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LaneReadout<'a> {
    /// 上下文用量短文本（`62%` / `~1%`）；没有实测也没有基线时 `None`。
    pub context: Option<&'a str>,
    /// 在途回合已耗时（秒）；空闲时 `None`。
    pub busy_secs: Option<u64>,
    /// 后台产生了新输出、用户还没切过去看。
    pub unread: bool,
}

/// 行尾读数的有序片段：`[上下文, 耗时, 未读]`。
///
/// 顺序即优先级（**从右往左**越来越重要）：裁剪时从这一列的第一个开始丢。
#[must_use]
pub fn tail_parts(readout: LaneReadout<'_>) -> Vec<String> {
    let mut parts = Vec::with_capacity(3);
    if let Some(ctx) = readout.context.map(str::trim).filter(|s| !s.is_empty()) {
        parts.push(ctx.to_string());
    }
    if let Some(secs) = readout.busy_secs {
        // 与 TUI lane 行、桌面连接行同一份格式（`12s` / `2m05s` / `1h02m`）。
        parts.push(busy_duration_text(secs));
    }
    if readout.unread {
        parts.push(String::from("•"));
    }
    parts
}

/// 尾部读数可用的宽度：名字要让出 [`LANE_LABEL_MIN`]，整体不超过半行。
#[must_use]
pub fn tail_room(width: usize, head_width: usize) -> usize {
    width
        .saturating_sub(head_width + 1 + LANE_LABEL_MIN)
        .min(width / 2)
}

/// 在 `room` 宽度内保留最重要的若干段（**从左边丢**），拼成一行尾读数。
///
/// `width_of` 由调用方给：终端传 `UnicodeWidthStr::width`（CJK 按 2 列），
/// 只做「有几个字」判断的调用方可以传 `str::len`。
#[must_use]
pub fn tail_within(parts: &[String], room: usize, width_of: impl Fn(&str) -> usize) -> String {
    let mut kept = parts;
    while !kept.is_empty() && joined_width(kept, &width_of) > room {
        kept = &kept[1..];
    }
    kept.join(" ")
}

/// 拼接后的显示宽度（段间各一个空格）。
fn joined_width(parts: &[String], width_of: &impl Fn(&str) -> usize) -> usize {
    if parts.is_empty() {
        return 0;
    }
    let glyphs: usize = parts.iter().map(|p| width_of(p.as_str())).sum();
    glyphs + parts.len() - 1
}

/// 标题里那段语言无关的计数后缀：`[2/8]` + `↓m`（没画全）+ `▲k`（在等审批）。
///
/// 文案（「连接」/「lanes」）留在各端 i18n：这里只保证**数字口径**两端一致 ——
/// `n` 是已连接条数、`max` 是上限、`hidden` 是「这里没画全」的条数、`pending` 是待审批 lane 数。
/// 后缀的顺序也固定：先说「连了几条」，再说「没画全」，最后说「有东西在等人」。
#[must_use]
pub fn count_suffix(
    connected: usize,
    max_lanes: usize,
    hidden: usize,
    pending_approvals: usize,
) -> String {
    let mut text = format!("[{connected}/{max_lanes}]");
    if hidden > 0 {
        text.push_str(&format!(" ↓{hidden}"));
    }
    if pending_approvals > 0 {
        text.push_str(&format!(" ▲{pending_approvals}"));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn readout(context: Option<&str>, busy: Option<u64>, unread: bool) -> LaneReadout<'_> {
        LaneReadout {
            context,
            busy_secs: busy,
            unread,
        }
    }

    #[test]
    fn parts_are_ordered_by_importance_from_the_right() {
        // 顺序即优先级：最右的未读点最重要，所以它在最后。
        assert_eq!(
            tail_parts(readout(Some("62%"), Some(12), true)),
            vec!["62%".to_string(), "12s".to_string(), "•".to_string()]
        );
        assert_eq!(tail_parts(readout(None, None, false)), Vec::<String>::new());
    }

    #[test]
    fn a_blank_context_is_not_a_part() {
        // 空串/空白不是「一个读数」，别在行尾留一段空白。
        assert_eq!(
            tail_parts(readout(Some("   "), None, false)),
            Vec::<String>::new()
        );
    }

    #[test]
    fn narrow_rows_drop_from_the_left_and_keep_the_unread_dot() {
        let parts = tail_parts(readout(Some("62%"), Some(125), true));
        // 三样放不下 → 先丢 ctx，再丢耗时；未读点留到最后。
        let width = |s: &str| s.chars().count();
        assert_eq!(tail_within(&parts, 1, width), "•");
        assert_eq!(tail_within(&parts, 7, width), "2m05s •");
        assert_eq!(tail_within(&parts, 11, width), "62% 2m05s •");
    }

    #[test]
    fn a_zero_room_keeps_nothing() {
        let parts = tail_parts(readout(Some("62%"), None, true));
        assert_eq!(tail_within(&parts, 0, |s: &str| s.chars().count()), "");
    }

    #[test]
    fn the_room_leaves_a_minimum_for_the_name_and_never_exceeds_half() {
        // 60 列宽：尾部最多 30（半行）。
        assert_eq!(tail_room(60, 1), 30);
        // 9 列宽：扣掉符号 1 + 空格 1 + 名字下限 4 后剩 3，比半行（4）小。
        assert_eq!(tail_room(9, 1), 3);
        // 极窄时不能算出负数。
        assert_eq!(tail_room(4, 1), 0);
    }

    #[test]
    fn the_count_suffix_only_mentions_what_is_true() {
        assert_eq!(count_suffix(2, 8, 0, 0), "[2/8]");
        assert_eq!(count_suffix(8, 8, 3, 0), "[8/8] ↓3");
        assert_eq!(count_suffix(2, 8, 0, 1), "[2/8] ▲1");
        assert_eq!(count_suffix(8, 8, 3, 2), "[8/8] ↓3 ▲2");
    }
}
