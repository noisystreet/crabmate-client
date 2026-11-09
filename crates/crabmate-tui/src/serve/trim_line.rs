//! `context_trim` 的一行提示文案（issue #478）：服务端压缩 / 裁剪了窗口，用量回落。
//!
//! 放在 `serve` 层而不是 `tui_mode`：全屏 `tui` 与文本模式（`chat` / `repl`）都要说
//! 同一件事，而 `serve` 是两者的共同下层。文案只写一遍，差别只在 `before` 的有无。
//!
//! 刻意**不写成因**：契约只给了 `after_tokens`，到底是历史裁剪、工具输出压缩还是摘要，
//! 客户端无从得知。凭一个 `kind` 猜括号里的理由，等于把「读代码才知道」换成
//! 「读文案被误导」—— 猜错的方向换了，猜的成本没变。

use crabmate_client_api::context_axis::format_compact_tokens;

/// 一次 `context_trim` 的用户可见读数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextTrimNotice {
    /// 压缩前的显示水位。
    ///
    /// `None` = 本地**没有可信**的前置值（还没绑定会话，或本轮此前没有过实测水位）。
    /// 用 `Option` 而不是 `0` 当哨兵：缺前置是常态而非异常，编码成 0 会渲染出
    /// 「0 → 251.1k」这种用户从未见过的数字。
    pub before: Option<u32>,
    /// 服务端给的裁剪后用量（`detail.after_tokens`）。
    pub after: u32,
    /// 本轮第几次压缩（≥1）：多次时文案标出次数，否则读者会以为只压了一次。
    pub count: u32,
}

impl ContextTrimNotice {
    /// 只有裁剪后用量 —— 文本模式（`chat` / `repl`）没有会话水位可报，只能报最小事实。
    #[must_use]
    pub fn after_only(after: u32) -> Self {
        Self {
            before: None,
            after,
            count: 1,
        }
    }
}

/// 提示行正文：`上下文已压缩：312.4k → 251.1k`。
///
/// 数字走 `format_compact_tokens`，与状态行 `ctx` 段同一个口径 —— 同一份用量两种写法，
/// 正是 ADR 0005 要消灭的那种漂移（用户没法把公告和状态行的数字对上）。
#[must_use]
pub fn trim_line_text(notice: &ContextTrimNotice) -> String {
    let after = format_compact_tokens(notice.after);
    let body = match notice.before {
        Some(before) => format!("{} → {after}", format_compact_tokens(before)),
        None => after,
    };
    if notice.count > 1 {
        format!("上下文已压缩：{body}（本轮 {} 次）", notice.count)
    } else {
        format!("上下文已压缩：{body}")
    }
}

#[cfg(test)]
mod tests {
    use super::{ContextTrimNotice, trim_line_text};

    fn notice(before: Option<u32>, after: u32, count: u32) -> ContextTrimNotice {
        ContextTrimNotice {
            before,
            after,
            count,
        }
    }

    #[test]
    fn shows_the_drop_when_a_before_value_is_known() {
        assert_eq!(
            trim_line_text(&notice(Some(312_400), 251_100, 1)),
            "上下文已压缩：312.4k → 251.1k"
        );
    }

    /// 没有可信前置值时只报当前量：编一个 0 出来，用户会以为自己的会话曾经是空的。
    #[test]
    fn omits_the_arrow_without_a_before_value() {
        assert_eq!(
            trim_line_text(&ContextTrimNotice::after_only(251_100)),
            "上下文已压缩：251.1k"
        );
    }

    /// 文本模式的退化形态与全屏的退化形态必须逐字一致，否则同一事件两种说法。
    #[test]
    fn after_only_matches_an_explicit_none() {
        assert_eq!(
            trim_line_text(&ContextTrimNotice::after_only(40_000)),
            trim_line_text(&notice(None, 40_000, 1))
        );
    }

    /// 小于一万的量不缩写（与状态行同一分界）。
    #[test]
    fn small_amounts_stay_plain_integers() {
        assert_eq!(
            trim_line_text(&notice(Some(9_800), 4_200, 1)),
            "上下文已压缩：9800 → 4200"
        );
    }

    /// 一轮多次压缩：一次比一次少，读者要知道还有第二次，否则会以为屏幕数字又「自己掉了」。
    #[test]
    fn counts_repeated_trims_in_the_same_turn() {
        assert_eq!(
            trim_line_text(&notice(Some(312_400), 200_000, 2)),
            "上下文已压缩：312.4k → 200.0k（本轮 2 次）"
        );
    }
}
