//! 按**显示列**截断 / 折行 / 水平滑窗的共用工具（CJK / 全角标点按 2 列计）。
//!
//! 从 `render.rs` 抽出：一是 `render.rs` 已贴着 920 行门禁，二是这条规则被状态栏、左右
//! 侧栏、设置面板、composer 共用，留在渲染模块里容易被当成「只给状态行用」。
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

/// 水平滚动窗口：返回（可见起点显示列、光标在可见文本中的显示列）。
///
/// **不变式：`start` 落在字符边界上，且 `shown_cursor < width`。** 两条都得成立：
/// [`cell_window`] 只整字跳过，起点若落在宽字符中间，实际渲染会比 `start` 少算 1 列，
/// 光标于是被画到末尾汉字的**中间**（`shown_cursor == width` 时更糟——越界后被
/// 光标定位 clamp 到 `width - 1`，正好压在后半个宽字上）。
///
/// 历史实现直接返回 `cursor + 1 - width`（再 `.min(total - width)`），两个取值都可能是
/// 半个宽字的偏移，于是「一行汉字铺满后光标落在末字中间」。
///
/// 光标贴右缘、右侧留 1 列：起点至少要 `cursor + 1 - width`；再向列号增大方向取到最近的
/// 字符起点（宁可多滚半个宽字出去，也不切半字），这样 `cursor - start ≤ width - 1` 恒成立。
///
/// 边界：`width == 1` 且光标前是宽字符时无字可显示（空窗口、光标列 0）——1 列放不下任何
/// 汉字，与其画半个字，不如留白。
pub(super) fn horizontal_window(text: &str, cursor_cell: usize, width: usize) -> (usize, usize) {
    if width == 0 {
        return (0, 0);
    }
    // 光标落在行内：不用滚动。注意判据是 `cursor_cell < width` 而不是「文本装得下」——
    // 一行正好铺满（`cursor_cell == width`）时光标位就不在行内了，必须滚。
    if cursor_cell < width {
        return (0, cursor_cell);
    }
    let want = cursor_cell.saturating_add(1).saturating_sub(width);
    // `min(cursor_cell)` 是防御而非算法需要：`cursor_cell` 正常由「同一段文本的光标前缀」
    // 量宽得来，该前提下结果必然 ≤ 它；万一调用方传了不自洽的值，这里退化成「不滚动」，
    // 而不是让裸减法下溢（debug 下 panic，release 下回绕成巨大列号被 `as u16` 截断）。
    let start = char_start_at_or_after(text, want).min(cursor_cell);
    (start, cursor_cell - start)
}

/// 返回 `≥ cell` 的最小**字符起点**（显示列）；走出文本尾部时返回文本总宽。
///
/// 唯一的硬前提是 `cell ≤ 光标列`：只在这个前提下结果才必然 ≤ 光标列。调用方做减法前
/// 仍宜 clamp（见 [`horizontal_window`]），免得输入不自洽时下溢。
fn char_start_at_or_after(text: &str, cell: usize) -> usize {
    let mut used = 0usize;
    for ch in text.chars() {
        if used >= cell {
            return used;
        }
        used += ch.width().unwrap_or(0);
    }
    used
}

/// 从显示列 `start` 起取 `cells` 列的可见串；宽字符整体跳/取，不切半字。
///
/// 只能整字跳，所以 `start` 落在宽字符中间时会把那个字整字画出来（实际起点比 `start`
/// 少算 1 列）。需要光标对齐的调用方请用 [`horizontal_window`] 算 `start`，它保证落在
/// 字符边界上。
pub(super) fn cell_window(text: &str, start: usize, cells: usize) -> String {
    let mut out = String::new();
    let mut skipped = 0usize;
    let mut taken = 0usize;
    for ch in text.chars() {
        let cw = ch.width().unwrap_or(0);
        if skipped + cw <= start {
            skipped += cw;
            continue;
        }
        if taken + cw > cells {
            break;
        }
        out.push(ch);
        taken += cw;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 窗口不变式的表驱动校验：起点必是字符边界（`cell_window` 从该处起算不丢列），
    /// 且光标列落在可视宽内——否则光标会压在后半个宽字上或被裁到行外。
    #[test]
    fn horizontal_window_keeps_cursor_inside_and_on_char_boundary() {
        for (content, cursor, width) in [
            ("", 0, 4),
            ("abc", 3, 4),
            ("abcdefgh", 8, 4),
            ("abcdefgh", 8, 0),
            ("crabmate> 你好", 14, 6),
            ("你好你好你", 10, 4),
            ("你好你好你", 6, 3),
            ("a你b好c", 7, 2),
        ] {
            let (start, c) = horizontal_window(content, cursor, width);
            if width == 0 {
                assert_eq!((start, c), (0, 0), "{content:?} @ {width}");
                continue;
            }
            assert!(
                start <= cursor,
                "{content:?} @ {cursor}/{width}：起点越过光标"
            );
            assert!(c < width, "{content:?} @ {cursor}/{width}：光标列 {c} 越界");
            // 起点落在字符边界 ⇔ 从 `start` 起的切片宽度正好等于「总宽 - start」。
            let full = UnicodeWidthStr::width(content);
            let tail = UnicodeWidthStr::width(cell_window(content, start, full).as_str());
            assert_eq!(
                tail,
                full - start,
                "{content:?} @ {cursor}/{width}：起点切了半个宽字"
            );
        }
    }

    /// `cell_window` 只能整字跳过：起点落在宽字符中间时把那个字整字画出，故调用方须先经
    /// [`horizontal_window`] 对齐到字符边界。
    #[test]
    fn cell_window_keeps_wide_char_boundary() {
        assert_eq!(cell_window("你好世界", 2, 4), "好世");
        assert_eq!(cell_window("你a好b", 3, 3), "好b");
    }

    /// 一行汉字正好铺满：起点必须退到字符边界，否则光标压在后半个字上。
    #[test]
    fn wide_char_line_full_cursor_keeps_char_boundary() {
        let content = "你好你好你";
        let (start, c) = horizontal_window(content, 10, 4);
        assert_eq!(start, 8, "起点须落在宽字符边界，而不是切半字");
        // 右缘对齐受宽字 2 列限制，光标列只能到 width-2（宁可右缘空 2 列，也不压半个字）。
        assert_eq!(cell_window(content, start, 4), "你");
        assert_eq!(c, 2);
    }

    /// 调用方传了不自洽的 `cursor_cell`（超出文本宽）也不能下溢 panic：起点仍钳在文本尾部。
    #[test]
    fn horizontal_window_clamps_inconsistent_cursor() {
        let (start, _c) = horizontal_window("ab", 99, 4);
        assert_eq!(
            start, 2,
            "起点应钳到文本总宽，而不是越过 cursor_cell 后下溢"
        );
    }

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
