//! 行内 Markdown 轻渲染 token（纯逻辑，无 IO / DOM）。
//!
//! 只做**行内子集**：粗体 / 斜体 / 行内码 / 链接 / 删除线。未闭合的标记一律按
//! **纯文本**输出（流式半段文本不会闪烁样式）；下划线不参与强调（避免 `_` 与
//! 标识符/CJK 冲突）。调用方各自渲染：WASM → HTML span，ratatui → 带样式 Span。

/// 行内片段：一个原子文本块或一种样式化文本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InlineSpan {
    Text(String),
    Bold(String),
    Italic(String),
    InlineCode(String),
    Strike(String),
    Link { text: String, url: String },
}

/// 某标记的解析结果：`(下一个下标, 片段)`。
type SpanHit = (usize, InlineSpan);

/// 把一段（无 `\n` 的）行内文本解析为片段序列；相邻纯文本会合并。
#[must_use]
pub fn parse_inline_markdown(text: &str) -> Vec<InlineSpan> {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<InlineSpan> = Vec::new();
    let mut buf = String::new();
    let mut i = 0usize;

    while i < chars.len() {
        let hit = backtick_span(&chars, i)
            .or_else(|| strike_span(&chars, i))
            .or_else(|| bold_span(&chars, i))
            .or_else(|| italic_span(&chars, i))
            .or_else(|| link_span(&chars, i));
        if let Some((next, span)) = hit {
            if !buf.is_empty() {
                out.push(InlineSpan::Text(std::mem::take(&mut buf)));
            }
            out.push(span);
            i = next;
            continue;
        }
        buf.push(chars[i]);
        i += 1;
    }
    if !buf.is_empty() {
        out.push(InlineSpan::Text(buf));
    }
    out
}

/// `` `code` `` / `` ``a`b`` ``（run 1~2；更长按字面）。
fn backtick_span(chars: &[char], i: usize) -> Option<SpanHit> {
    if chars[i] != '`' {
        return None;
    }
    let run = run_len(chars, i, '`');
    if run > 2 {
        return None;
    }
    let end = find_run(chars, i + run, '`', run)?;
    let inner: String = chars[i + run..end].iter().collect();
    Some((end + run, InlineSpan::InlineCode(inner)))
}

/// `~~strike~~`（两侧不得紧贴空白，与 CommonMark flanking 一致）。
fn strike_span(chars: &[char], i: usize) -> Option<SpanHit> {
    if run_len(chars, i, '~') < 2 {
        return None;
    }
    let end = find_run(chars, i + 2, '~', 2)?;
    if !well_flanked(chars, i + 2, end) {
        return None;
    }
    let inner: String = chars[i + 2..end].iter().collect();
    Some((end + 2, InlineSpan::Strike(inner)))
}

/// `**bold**`（两侧不得紧贴空白）。
fn bold_span(chars: &[char], i: usize) -> Option<SpanHit> {
    if run_len(chars, i, '*') < 2 {
        return None;
    }
    let end = find_run(chars, i + 2, '*', 2)?;
    if !well_flanked(chars, i + 2, end) {
        return None;
    }
    let inner: String = chars[i + 2..end].iter().collect();
    Some((end + 2, InlineSpan::Bold(inner)))
}

/// `*italic*`（仅单个 `*`、不含在 `**` run 里；两侧不得紧贴空白）。
fn italic_span(chars: &[char], i: usize) -> Option<SpanHit> {
    if chars[i] != '*' || run_len(chars, i, '*') > 1 {
        return None;
    }
    let end = find_run(chars, i + 1, '*', 1)?;
    if !well_flanked(chars, i + 1, end) {
        return None;
    }
    let inner: String = chars[i + 1..end].iter().collect();
    Some((end + 1, InlineSpan::Italic(inner)))
}

/// 强调内容非空且首尾不得紧贴空白（避免 `a * b * c` / `** a **` 被误强调）。
fn well_flanked(chars: &[char], open_end: usize, close_start: usize) -> bool {
    open_end < close_start
        && !chars[open_end].is_whitespace()
        && !chars[close_start - 1].is_whitespace()
}

/// `[text](url)`（text 原样，不嵌套解析）。
fn link_span(chars: &[char], i: usize) -> Option<SpanHit> {
    if chars[i] != '[' {
        return None;
    }
    let (close, url_start, url_end) = try_link(chars, i)?;
    let inner: String = chars[i + 1..close].iter().collect();
    let url: String = chars[url_start..url_end].iter().collect();
    Some((url_end + 1, InlineSpan::Link { text: inner, url }))
}

/// 从 `start` 起连续 `needle` 字符数。
fn run_len(chars: &[char], start: usize, needle: char) -> usize {
    let mut n = 0usize;
    let mut i = start;
    while i < chars.len() && chars[i] == needle {
        n += 1;
        i += 1;
    }
    n
}

/// 从 `start` 起找下一段长度 ≥ `want` 的 `needle` run，返回其起始下标。
fn find_run(chars: &[char], start: usize, needle: char, want: usize) -> Option<usize> {
    let mut i = start;
    while i < chars.len() {
        if chars[i] == needle && run_len(chars, i, needle) >= want {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// 解析 `[text](url)`：成功返回 `(text 结束下标, url 起始, url 结束)`。
fn try_link(chars: &[char], open: usize) -> Option<(usize, usize, usize)> {
    let close = chars[open + 1..].iter().position(|&c| c == ']')? + open + 1;
    if chars.get(close + 1) != Some(&'(') {
        return None;
    }
    let url_start = close + 2;
    let url_end = chars[url_start..].iter().position(|&c| c == ')')? + url_start;
    Some((close, url_start, url_end))
}

#[cfg(test)]
mod tests {
    use super::{InlineSpan, parse_inline_markdown};

    fn text(s: &str) -> InlineSpan {
        InlineSpan::Text(s.to_string())
    }

    #[test]
    fn bold_italic_and_plain_mix() {
        assert_eq!(
            parse_inline_markdown("a **b** c *d* e"),
            vec![
                text("a "),
                InlineSpan::Bold("b".into()),
                text(" c "),
                InlineSpan::Italic("d".into()),
                text(" e"),
            ]
        );
    }

    #[test]
    fn inline_code_single_and_double_backtick() {
        assert_eq!(
            parse_inline_markdown("`x`"),
            vec![InlineSpan::InlineCode("x".into())]
        );
        // 双反引号允许内容里带单个反引号
        assert_eq!(
            parse_inline_markdown("``a`b``"),
            vec![InlineSpan::InlineCode("a`b".into())]
        );
    }

    #[test]
    fn link_keeps_text_raw() {
        assert_eq!(
            parse_inline_markdown("[site](https://example.com)"),
            vec![InlineSpan::Link {
                text: "site".into(),
                url: "https://example.com".into()
            }]
        );
    }

    #[test]
    fn strike_parses() {
        assert_eq!(
            parse_inline_markdown("~~x~~"),
            vec![InlineSpan::Strike("x".into())]
        );
    }

    #[test]
    fn unterminated_markers_stay_literal() {
        assert_eq!(parse_inline_markdown("**x"), vec![text("**x")]);
        assert_eq!(parse_inline_markdown("a *b"), vec![text("a *b")]);
        assert_eq!(parse_inline_markdown("`x"), vec![text("`x")]);
        assert_eq!(parse_inline_markdown("[x](u"), vec![text("[x](u")]);
    }

    #[test]
    fn trailing_opener_after_closed_pair_is_literal() {
        assert_eq!(
            parse_inline_markdown("**a** tail **"),
            vec![InlineSpan::Bold("a".into()), text(" tail **")]
        );
    }

    #[test]
    fn underscore_is_not_emphasis() {
        assert_eq!(parse_inline_markdown("_x_"), vec![text("_x_")]);
        assert_eq!(parse_inline_markdown("a_b"), vec![text("a_b")]);
    }

    #[test]
    fn spaced_asterisks_are_not_emphasis() {
        // 两侧空白不成强调（CommonMark flanking）：乘法/通配符等不误伤且不吞星号
        assert_eq!(parse_inline_markdown("a * b * c"), vec![text("a * b * c")]);
        assert_eq!(parse_inline_markdown("** a **"), vec![text("** a **")]);
        assert_eq!(parse_inline_markdown("~~ a ~~"), vec![text("~~ a ~~")]);
        assert_eq!(
            parse_inline_markdown("ls *.rs *.toml"),
            vec![text("ls *.rs *.toml")]
        );
    }

    #[test]
    fn emphasis_keeps_working_when_flanked() {
        assert_eq!(
            parse_inline_markdown("*b*"),
            vec![InlineSpan::Italic("b".into())]
        );
        assert_eq!(
            parse_inline_markdown("**b**"),
            vec![InlineSpan::Bold("b".into())]
        );
        assert_eq!(
            parse_inline_markdown("a**b**c"),
            vec![text("a"), InlineSpan::Bold("b".into()), text("c")]
        );
    }

    #[test]
    fn cjk_text_untouched() {
        assert_eq!(
            parse_inline_markdown("你好**世界**"),
            vec![text("你好"), InlineSpan::Bold("世界".into())]
        );
    }
}
