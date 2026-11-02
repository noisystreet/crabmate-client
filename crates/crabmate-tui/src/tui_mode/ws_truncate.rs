//! 侧栏名称在窄栏下的省略策略（纯字符串处理，供 `ws_sidebar` 调用）。
//!
//! 抽成独立模块的两个理由：
//! 1. `ws_sidebar.rs` 自带一整套渲染测试，已接近 920 行/文件的 `fn-nloc-ratchet`
//!    门禁，再往里堆实现会挤掉后续改动的余量；
//! 2. 省略规则是纯函数，能在不搭 `UiState` 的前提下把宽字符 / 边界宽度测干净。
//!
//! 为什么不再用「只保留尾部」：`…module_name.rs` 在窄栏里会把同目录的几个长名压成
//! `…me.rs` / `…le.rs` 这种几乎一样的行 —— 尾部只有扩展名相同，前缀才是区分度所在。
//! 反过来「只保留头部」又会丢扩展名，而扩展名是终端里判断文件类型的主要线索。

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::text::truncate_display;

/// 超宽不足这么多列时不做中段省略。
///
/// 只差一两列的情况下，中段省略会从名字中间挖掉一两个字符（`abc…xyz`），看起来像
/// 名字本身被改过；这种情况退回「保留开头 + 尾部 `…`」更符合直觉。
const MIDDLE_MIN_SAVING: usize = 4;

/// 头部在省略预算里的占比（分子 / 分母）：头 2 : 尾 3。
///
/// 尾部多给是有意的：扩展名通常 2–8 列，加上名字后半段比前几个字符更能定位文件。
const HEAD_RATIO_NUM: usize = 2;
const HEAD_RATIO_DEN: usize = 5;

/// 省略号之后剩下的预算小于这么多列时，不做中段省略，退回「`…` + 尾部」。
///
/// 深层目录里名称预算经常只剩七八列（缩进 2 列/层 + `▸ ` 已经吃掉大半），这时按
/// 2:3 分只会切出 `mo…me.rs` 这种东西——头尾各两三个字符，反而是 `…name.rs` 更能
/// 认出文件。中段省略的收益只在预算够两头各放一段时才成立。
const TAIL_ONLY_BELOW_BUDGET: usize = 10;

/// 名称超宽时的省略：保留开头与末尾，中间用 `…`，结果宽度 ≤ `width`。
///
/// `width == 0` 返回空串，`width == 1` 只放得下 `…`（与 `truncate_display` 同口径：
/// 省略号本身要占 1 列）。
pub(super) fn middle_ellipsis(name: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let name_w = UnicodeWidthStr::width(name);
    if name_w <= width {
        return name.to_string();
    }
    if width == 1 {
        return "…".to_string();
    }
    if name_w <= width + MIDDLE_MIN_SAVING {
        return truncate_display(name, width);
    }
    // `width ≥ 2`，故 `budget ≥ 1`，省略号有位置可放。
    let budget = width - 1;
    if budget < TAIL_ONLY_BELOW_BUDGET {
        // 预算太窄：头部留不出有意义的字符，退回「`…` + 尾部」（扩展名优先）。
        return format!("…{}", take_suffix(name, budget));
    }
    let head_budget = budget * HEAD_RATIO_NUM / HEAD_RATIO_DEN;
    let tail_budget = budget - head_budget;
    let head = take_prefix(name, head_budget);
    let tail = take_suffix(name, tail_budget);
    // `head` / `tail` 各自按显示宽度累加，不会把宽字符截成半个，故总宽必然 ≤ budget + 1。
    format!("{head}…{tail}")
}

/// 取尽可能长的前缀，使显示宽度 ≤ `budget`。
fn take_prefix(s: &str, budget: usize) -> String {
    let mut out = String::new();
    let mut used = 0usize;
    for ch in s.chars() {
        let w = ch.width().unwrap_or(0);
        if used + w > budget {
            break;
        }
        out.push(ch);
        used += w;
    }
    out
}

/// 取尽可能长的后缀，使显示宽度 ≤ `budget`（从尾部往前累加，再翻回来）。
fn take_suffix(s: &str, budget: usize) -> String {
    let mut tail: Vec<char> = Vec::new();
    let mut used = 0usize;
    for ch in s.chars().rev() {
        let w = ch.width().unwrap_or(0);
        if used + w > budget {
            break;
        }
        tail.push(ch);
        used += w;
    }
    tail.into_iter().rev().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_returns_name_unchanged() {
        assert_eq!(middle_ellipsis("main.rs", 20), "main.rs");
        // 恰好等宽：不省略（边界差一列最容易写错）。
        assert_eq!(middle_ellipsis("1234567", 7), "1234567");
    }

    #[test]
    fn narrow_keeps_both_ends_and_the_extension() {
        let out = middle_ellipsis("crabmate_super_long_module_name.rs", 18);
        assert!(UnicodeWidthStr::width(out.as_str()) <= 18, "{out}");
        assert!(out.contains('…'), "应做中段省略：{out}");
        assert!(out.starts_with("cra"), "保留开头：{out}");
        assert!(out.ends_with(".rs"), "保留扩展名：{out}");
        // 头部与尾部都不为空 —— 纯尾部省略的回归守卫。
        let (head, tail) = out.split_once('…').expect("含省略号");
        assert!(!head.is_empty() && !tail.is_empty(), "{out}");
    }

    #[test]
    fn head_and_tail_both_present_for_ascii() {
        let out = middle_ellipsis("abcdefghijklmnopqrstuvwxyz", 12);
        assert!(UnicodeWidthStr::width(out.as_str()) <= 12, "{out}");
        assert_eq!(out, "abcd…tuvwxyz", "头 4 : 尾 7 的分配（预算 11）");
    }

    #[test]
    fn wide_chars_are_not_split() {
        // 每个汉字 2 列：预算 11 是奇数，尾部累加到 7 列会超，只能停在上一个完整汉字。
        let out = middle_ellipsis("配置说明文档非常长的名字.md", 11);
        assert!(UnicodeWidthStr::width(out.as_str()) <= 11, "{out}");
        assert_eq!(out, "配置…字.md", "不得切出半个宽字符：{out}");
    }

    #[test]
    fn tiny_widths() {
        assert_eq!(middle_ellipsis("main.rs", 0), "");
        assert_eq!(middle_ellipsis("main.rs", 1), "…");
        // 2 列：省略号 + 1 列尾部（头部预算算下来是 0，允许为空）。
        let two = middle_ellipsis("main.rs", 2);
        assert_eq!(UnicodeWidthStr::width(two.as_str()), 2, "{two}");
    }

    #[test]
    fn small_overflow_falls_back_to_tail_ellipsis() {
        // 只差 1 列：不做中段省略，避免从名字中间挖字。
        let out = middle_ellipsis("12345678", 7);
        assert_eq!(out, "123456…");
    }
}
