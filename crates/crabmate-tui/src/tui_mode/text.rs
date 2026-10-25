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

/// tab 展开成几个空格（与 `state_paste` 的粘贴归一化同口径）。
const TAB_AS_SPACES: &str = "    ";

/// 按显示宽**折行**（保留原文的 `\n`），结果每行显示宽恒 ≤ `width`。
///
/// 与 [`truncate_display`] 互补：那个丢掉溢出部分，这个把溢出挪到下一行。审批浮层
/// 「看完整命令」用的就是后者——它的原始缺陷正是「截断成一行、看不到后面还有什么」。
///
/// 归一化口径与粘贴一致：`\t` 展开成 4 空格、其它控制字符（含 CRLF 的 `\r`）丢弃，
/// 否则裸 `\x1b` 会被 ratatui 当成转义序列的开头画乱整屏。
pub(super) fn wrap_display(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let mut out: Vec<String> = Vec::new();
    for logical in text.split('\n') {
        let expanded = logical.replace('\t', TAB_AS_SPACES);
        let mut line = String::new();
        let mut used = 0usize;
        for ch in expanded.chars() {
            if ch.is_control() {
                continue;
            }
            let cw = ch.width().unwrap_or(0);
            // `used > 0` 兜住「单字符比整行还宽」：硬放一行，别在原地死循环。
            if used + cw > width && used > 0 {
                out.push(std::mem::take(&mut line));
                used = 0;
            }
            line.push(ch);
            used += cw;
        }
        out.push(line);
    }
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

    #[test]
    fn wrap_splits_on_width_and_keeps_logical_newlines() {
        // 语义换行保留：三行输入仍是三行，不会被折行抹平。
        assert_eq!(wrap_display("ab\ncd", 8), vec!["ab", "cd"]);
        // 超宽折成两行（而不是截断丢内容）。
        assert_eq!(wrap_display("abcdef", 4), vec!["abcd", "ef"]);
        // CJK 按 2 列计：宽度 4 只放得下两个汉字。
        assert_eq!(wrap_display("你好吗", 4), vec!["你好", "吗"]);
        assert_eq!(wrap_display("", 4), vec![""]);
        assert!(wrap_display("abc", 0).is_empty());
    }

    #[test]
    fn wrap_drops_control_chars_and_expands_tabs() {
        // 裸 `\x1b` / CRLF 的 `\r` 都不能进渲染（前者会画乱整屏）。
        assert_eq!(wrap_display("a\u{1b}b\r\nc", 8), vec!["ab", "c"]);
        assert_eq!(wrap_display("\tx", 8), vec!["    x"]);
    }

    #[test]
    fn wrapped_lines_never_exceed_the_width() {
        let long = "x".repeat(100);
        for (text, width) in [
            ("abcdefg", 3),
            ("这是一段比较长的中文命令 --flag value", 7),
            (long.as_str(), 13),
        ] {
            for line in wrap_display(text, width) {
                let w = UnicodeWidthStr::width(line.as_str());
                assert!(w <= width, "「{line}」{w} 列，超出 {width}");
            }
        }
    }
}
