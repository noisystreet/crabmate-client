//! 按**显示列**截断文本的共用工具（CJK / 全角标点按 2 列计）。
//!
//! 从 `render.rs` 抽出：一是 `render.rs` 已贴着 920 行门禁，二是这条规则被状态栏、左右
//! 侧栏、设置面板共用，留在渲染模块里容易被当成「只给状态行用」。
//!
//! **不变式：返回值显示宽恒 ≤ `width`**（连结尾的 `…` 一起算）。
//!
//! 历史实现是「放不下就停，再无条件补一个 `…`」，所以内容正好铺满预算时结果会比 `width`
//! 宽 1 列。调用方按 `width` 算出来的预算（状态栏分段、侧栏选中行竖条）就会被 `Paragraph`
//! 悄悄裁掉末列——这类偏差在宽终端上看不出来，只在窄终端掉最后一个字。现在改成先给 `…`
//! 预留 1 列。

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// 按显示宽截断，超出则以 `…` 结尾；结果显示宽 ≤ `width`。
pub(super) fn truncate_display(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    // 装得下就原样返回：多数调用点走的都是这条，保持零分配之外的行为不变。
    if UnicodeWidthStr::width(text) <= width {
        return text.to_string();
    }
    // 需要截断：给 `…` 预留 1 列（`width` 已保证 ≥ 1，故 `budget` 不会回绕）。
    let budget = width - 1;
    let mut out = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let cw = ch.width().unwrap_or(0);
        if used + cw > budget {
            break;
        }
        out.push(ch);
        used += cw;
    }
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_that_fits_is_returned_unchanged() {
        assert_eq!(truncate_display("abc", 3), "abc");
        assert_eq!(truncate_display("你好", 4), "你好");
        assert_eq!(truncate_display("", 4), "");
        assert_eq!(truncate_display("abc", 0), "");
    }

    #[test]
    fn truncated_text_never_exceeds_width() {
        // 核心回归：内容正好铺满预算时，不能因为「补一个 …」而多出第 width+1 列。
        for (text, width) in [
            ("abcdef", 4),
            ("abcdef", 1),
            ("你好吗", 3),
            ("中", 1),
            ("tool_name (done) — a long note", 12),
        ] {
            let out = truncate_display(text, width);
            let w = UnicodeWidthStr::width(out.as_str());
            assert!(w <= width, "{text} @ {width} → 「{out}」{w} 列，超出预算");
        }
        assert_eq!(truncate_display("abcdef", 4), "abc…");
        assert_eq!(truncate_display("你好吗", 3), "你…");
    }
}
