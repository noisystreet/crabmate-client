//! 助手正文的**行内 Markdown 轻渲染**辅助（ratatui span 层，无 transcript 状态）。
//!
//! token 由共享 [`crabmate_client_api::markdown_inline`] 提供；这里把 token 映射成
//! 样式并折成按显示宽度的物理行。搜索高亮叠加规则也在此统一（锚定整行反色、
//! 命中改黄前景且保留修饰位），保证与普通行的高亮观感一致。

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use unicode_width::UnicodeWidthChar;

use crabmate_client_api::{InlineSpan, parse_inline_markdown};

/// 行内 markdown 片段 → 渲染样式（前景仅在需要区分时给出，其余随基色）。
pub(crate) fn md_span_style(span: &InlineSpan) -> Style {
    match span {
        InlineSpan::Text(_) => Style::new(),
        InlineSpan::Bold(_) => Style::new().add_modifier(Modifier::BOLD),
        InlineSpan::Italic(_) => Style::new().add_modifier(Modifier::ITALIC),
        InlineSpan::InlineCode(_) => Style::new().fg(Color::Cyan),
        InlineSpan::Strike(_) => Style::new().add_modifier(Modifier::CROSSED_OUT),
        InlineSpan::Link { .. } => Style::new()
            .fg(Color::Blue)
            .add_modifier(Modifier::UNDERLINED),
    }
}

/// 片段文本（Link 取展示文本）。
fn span_text(span: &InlineSpan) -> &str {
    match span {
        InlineSpan::Text(t)
        | InlineSpan::Bold(t)
        | InlineSpan::Italic(t)
        | InlineSpan::InlineCode(t)
        | InlineSpan::Strike(t) => t,
        InlineSpan::Link { text, .. } => text,
    }
}

/// 按显示宽度断行（普通纯文本路径用；规则同 styled 版）。
/// `\n` 强制断行，控制字符丢弃，空输入产出单个空行。
pub(crate) fn wrap_physical(text: &str, width: usize) -> Vec<String> {
    let mut rows: Vec<String> = Vec::new();
    let mut row = String::new();
    let mut row_width = 0usize;
    for ch in text.chars() {
        if ch == '\n' {
            if !row.is_empty() {
                rows.push(std::mem::take(&mut row));
                row_width = 0;
            }
            continue;
        }
        if ch.is_control() {
            continue;
        }
        let cw = ch.width().unwrap_or(0);
        if !row.is_empty() && row_width + cw > width {
            rows.push(std::mem::take(&mut row));
            row_width = 0;
        }
        row.push(ch);
        row_width += cw;
    }
    if !row.is_empty() {
        rows.push(row);
    }
    if rows.is_empty() {
        rows.push(String::new());
    }
    rows
}

/// 单段（不含换行）行内文本 → 带 markdown 样式的字符流。
pub(crate) fn inline_styled_chars(part: &str) -> Vec<(Style, char)> {
    let mut out = Vec::new();
    for span in parse_inline_markdown(part) {
        let st = md_span_style(&span);
        out.extend(span_text(&span).chars().map(|c| (st, c)));
    }
    out
}

/// 该行是否为代码围栏开关行（`` ``` `` / `~~~`，容忍行首缩进）。
fn is_fence_switch(line: &str) -> bool {
    let s = line.trim_start();
    s.starts_with("```") || s.starts_with("~~~")
}

/// 行首至多 3 个空格的缩进宽度（字节数）。
fn leading_indent(line: &str) -> usize {
    line.bytes().take(3).take_while(|&b| b == b' ').count()
}

/// `b` 起点是否为合法 UTF-8 边界并取首字符；调用方保证切片从 ASCII 边界开始。
fn first_char(b: &[u8]) -> Option<char> {
    std::str::from_utf8(b).ok().and_then(|s| s.chars().next())
}

/// 标记后紧贴的 CJK 等非 ASCII 字母正文（对应共享 normalize 的 `-规范`/`1.下一步` 规则）。
fn glued_cjk_body(b: &[u8]) -> bool {
    first_char(b).is_some_and(|c| c.is_alphabetic() && !c.is_ascii())
}

/// 行首 ATX 标题匹配：返回（`#` 个数, 内容起始字节偏移）。
/// 与 normalize 语义对齐：`#` 后可跟空格/tab，也允许紧贴正文（`###规范`/`#Title`）。
fn heading_match(rest: &[u8]) -> Option<(usize, usize)> {
    let mut h = 0usize;
    while h < rest.len() && h < 6 && rest[h] == b'#' {
        h += 1;
    }
    if h == 0 {
        return None;
    }
    match rest.get(h) {
        None => Some((h, h)),
        Some(b' ') | Some(b'\t') => Some((h, h + 1)),
        Some(b'#') => None,
        Some(_) => Some((h, h)),
    }
}

/// 引用 `>` 匹配：`>正文`（紧贴）/ `> 正文`（分隔）都可。
fn quote_match(rest: &[u8]) -> Option<(usize, usize)> {
    if rest.first() != Some(&b'>') {
        return None;
    }
    Some((
        1,
        match rest.get(1) {
            Some(b' ') | Some(b'\t') => 2,
            _ => 1,
        },
    ))
}

/// 无序列表 `-`/`+`（CJK 紧贴放行）；`*` 仅空格/tab 分隔（紧贴按斜体语义）。
fn unordered_match(rest: &[u8]) -> Option<(usize, usize)> {
    let c0 = *rest.first()?;
    if matches!(c0, b'-' | b'+') {
        return match rest.get(1) {
            Some(b' ') | Some(b'\t') => Some((1, 2)),
            Some(_) if glued_cjk_body(&rest[1..]) => Some((1, 1)),
            _ => None,
        };
    }
    if c0 == b'*' {
        return match rest.get(1) {
            Some(b' ') | Some(b'\t') => Some((1, 2)),
            _ => None,
        };
    }
    None
}

/// 有序列表 `1. `/`1) `（≤9 位；CJK 紧贴放行）。
fn ordered_match(rest: &[u8]) -> Option<(usize, usize)> {
    let mut d = 0usize;
    while d < rest.len() && d < 9 && rest[d].is_ascii_digit() {
        d += 1;
    }
    if d == 0 || !matches!(rest.get(d), Some(b'.') | Some(b')')) {
        return None;
    }
    match rest.get(d + 1) {
        Some(b' ') | Some(b'\t') => Some((d + 1, d + 2)),
        Some(_) if glued_cjk_body(&rest[d + 1..]) => Some((d + 1, d + 1)),
        _ => None,
    }
}

/// 行首列表/引用匹配：返回（标记字节数, 内容起始字节偏移）。
fn list_or_quote_match(rest: &[u8]) -> Option<(usize, usize)> {
    quote_match(rest)
        .or_else(|| unordered_match(rest))
        .or_else(|| ordered_match(rest))
}

/// 单行：行首标记（标题/列表/引用）轻渲染 + 其余部分行内 md。
/// 标记弱化为灰色；标题正文整体加粗（行内样式叠加）。宽容识别只影响样式判定，
/// 不增删字符（分隔空格/tab 原样保留；tab 仅会在折行时被丢弃）。
fn styled_part_chars(part: &str) -> Vec<(Style, char)> {
    let indent = leading_indent(part);
    let rest = &part.as_bytes()[indent..];
    let (marker_len, content_at, heading) = if let Some((h, ca)) = heading_match(rest) {
        (h, ca, true)
    } else if let Some((m, ca)) = list_or_quote_match(rest) {
        (m, ca, false)
    } else {
        return inline_styled_chars(part);
    };
    let mut out: Vec<(Style, char)> = Vec::new();
    out.extend(part[..indent].chars().map(|c| (Style::new(), c)));
    let marker: String = part[indent..indent + marker_len].to_string();
    out.extend(marker.chars().map(|c| (Style::new().fg(Color::Gray), c)));
    if content_at > marker_len {
        out.push((
            Style::new(),
            rest.get(marker_len).copied().unwrap_or(b' ') as char,
        ));
    }
    let body = inline_styled_chars(&part[indent + content_at..]);
    if heading {
        out.extend(
            body.into_iter()
                .map(|(s, c)| (s.add_modifier(Modifier::BOLD), c)),
        );
    } else {
        out.extend(body);
    }
    out
}

/// 整段助手文本 → 带样式的字符流（段间补 `\n` 作强制断行，标记不跨段解析）。
/// 处于 `` ``` `` / `~~~` 围栏内的行整行按纯文本，避免代码内容里的 `**`/反引号/
/// `[t](u)` 等被误当成行内样式而吞字符。
pub(crate) fn assistant_styled_text(text: &str) -> Vec<(Style, char)> {
    let mut out = Vec::new();
    let mut first = true;
    let mut in_fence = false;
    for part in text.split('\n') {
        if !first {
            out.push((Style::new(), '\n'));
        }
        let toggles = is_fence_switch(part);
        if toggles {
            in_fence = !in_fence;
        }
        if toggles || in_fence {
            out.extend(part.chars().map(|c| (Style::new(), c)));
        } else {
            out.extend(styled_part_chars(part));
        }
        first = false;
    }
    out
}

/// 按显示宽度把带样式字符流折成物理行（控制字符丢弃；规则同普通行 wrap）。
pub(crate) fn wrap_styled_chars(styled: &[(Style, char)], width: usize) -> Vec<Vec<(Style, char)>> {
    let mut rows: Vec<Vec<(Style, char)>> = Vec::new();
    let mut row: Vec<(Style, char)> = Vec::new();
    let mut row_width = 0usize;
    for &(st, ch) in styled {
        if ch == '\n' {
            if !row.is_empty() {
                rows.push(std::mem::take(&mut row));
                row_width = 0;
            }
            continue;
        }
        if ch.is_control() {
            continue;
        }
        let cw = ch.width().unwrap_or(0);
        if !row.is_empty() && row_width + cw > width {
            rows.push(std::mem::take(&mut row));
            row_width = 0;
        }
        row.push((st, ch));
        row_width += cw;
    }
    if !row.is_empty() {
        rows.push(row);
    }
    if rows.is_empty() {
        rows.push(Vec::new());
    }
    rows
}

/// 行基色叠加 markdown 片段样式（md 未给前景时随基色）。
pub(crate) fn md_row_style(md: Style, base: Style) -> Style {
    let fg = md.fg.or(base.fg);
    let mut s = Style::new();
    if let Some(c) = fg {
        s = s.fg(c);
    }
    s.add_modifier(md.add_modifier | base.add_modifier)
}

/// 搜索高亮叠加：锚定行整体反色；其余命中行改黄色前景（保留修饰位）。
pub(crate) fn highlight_override(s: Style, matched: bool, anchor: bool) -> Style {
    if anchor {
        return Style::new()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD);
    }
    if matched {
        return Style::new().fg(Color::Yellow).add_modifier(s.add_modifier);
    }
    s
}

fn same_style(a: &Style, b: &Style) -> bool {
    a.fg == b.fg && a.bg == b.bg && a.add_modifier == b.add_modifier
}

/// 一行带样式字符 → span 序列（连续同款式字符合并）。
pub(crate) fn styled_row_spans(
    row: &[(Style, char)],
    base: Style,
    matched: bool,
    anchor: bool,
) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut cur: Option<(String, Style)> = None;
    for &(md, ch) in row {
        let style = highlight_override(md_row_style(md, base), matched, anchor);
        match &mut cur {
            Some((t, s)) if same_style(s, &style) => t.push(ch),
            _ => {
                if let Some((t, s)) = cur.take() {
                    spans.push(Span::styled(t, s));
                }
                cur = Some((ch.to_string(), style));
            }
        }
    }
    if let Some((t, s)) = cur.take() {
        spans.push(Span::styled(t, s));
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bold_code_and_link_get_expected_styles() {
        let chars = inline_styled_chars("a **b** `c` [d](https://e)");
        assert!(
            chars
                .iter()
                .any(|(s, ch)| { s.add_modifier.contains(Modifier::BOLD) && *ch == 'b' })
        );
        assert!(
            chars
                .iter()
                .any(|(s, ch)| s.fg == Some(Color::Cyan) && *ch == 'c')
        );
        assert!(
            chars
                .iter()
                .any(|(s, ch)| { s.add_modifier.contains(Modifier::UNDERLINED) && *ch == 'd' })
        );
        assert!(!chars.iter().any(|(_, ch)| *ch == 'h' || *ch == 't'));
    }

    #[test]
    fn unterminated_marker_has_no_bold() {
        let chars = inline_styled_chars("尾 **未闭合");
        assert!(
            chars
                .iter()
                .all(|(s, _)| !s.add_modifier.contains(Modifier::BOLD))
        );
        let rows = wrap_styled_chars(&chars, 200);
        let spans = styled_row_spans(&rows[0], Style::new(), false, false);
        let joined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(joined.contains("**未闭合"));
    }

    #[test]
    fn matched_highlight_keeps_bold_but_turns_yellow() {
        let chars = inline_styled_chars("a **b** c");
        let rows = wrap_styled_chars(&chars, 200);
        let spans = styled_row_spans(&rows[0], Style::new().fg(Color::LightGreen), true, false);
        assert!(spans.iter().any(|s| s.style.fg == Some(Color::Yellow)
            && s.style.add_modifier.contains(Modifier::BOLD)
            && s.to_string().contains('b')));
    }

    #[test]
    fn anchor_row_overrides_whole_row() {
        let chars = inline_styled_chars("**b**");
        let rows = wrap_styled_chars(&chars, 200);
        let spans = styled_row_spans(&rows[0], Style::new(), false, true);
        assert!(
            spans
                .iter()
                .all(|s| s.style.bg == Some(Color::Yellow) && s.style.fg == Some(Color::Black))
        );
    }

    #[test]
    fn code_fence_content_is_plain_and_literal() {
        let text = "前 **A**\n```\nls *.rs **x** `t`\n```\n后 **B**";
        let chars = assistant_styled_text(text);
        let styled = |c: char| {
            chars
                .iter()
                .filter(|(_, ch)| *ch == c)
                .any(|(s, _)| !s.add_modifier.is_empty() || s.fg.is_some())
        };
        // 围栏外强调生效
        assert!(styled('A'));
        assert!(styled('B'));
        // 围栏内整行纯文本：星号/反引号原样保留、无样式
        assert!(!styled('x'));
        assert!(!styled('`'));
        assert!(!styled('*'));
        let joined: String = chars.iter().map(|(_, ch)| *ch).collect();
        assert!(joined.contains("ls *.rs **x** `t`"), "got {joined:?}");
    }

    #[test]
    fn wrap_splits_wide_chars() {
        let rows = wrap_physical("你好世界abc", 5);
        assert_eq!(rows, vec!["你好", "世界a", "bc"]);
        assert_eq!(wrap_physical("你好世界", 4), vec!["你好", "世界"]);
    }

    #[test]
    fn wrap_handles_newline() {
        let rows = wrap_physical("a\nbcd", 10);
        assert_eq!(rows, vec!["a", "bcd"]);
    }

    #[test]
    fn wrap_drops_control_chars() {
        let rows = wrap_physical("a\u{1b}[31mb", 10);
        assert_eq!(rows, vec!["a[31mb"]);
        assert!(!rows[0].contains('\u{1b}'));
    }

    fn has_gray(chars: &[(Style, char)], c: char) -> bool {
        chars
            .iter()
            .any(|(s, ch)| *ch == c && s.fg == Some(Color::Gray))
    }

    fn has_bold(chars: &[(Style, char)], c: char) -> bool {
        chars
            .iter()
            .any(|(s, ch)| *ch == c && s.add_modifier.contains(Modifier::BOLD))
    }

    #[test]
    fn heading_is_bold_and_marker_gray() {
        let chars = assistant_styled_text("# 标题");
        assert!(has_gray(&chars, '#'));
        assert!(has_bold(&chars, '标'));
        assert!(has_bold(&chars, '题'));
    }

    #[test]
    fn list_and_quote_markers_are_gray_content_plain() {
        let ul = assistant_styled_text("- 项目");
        assert!(has_gray(&ul, '-'));
        assert!(!has_bold(&ul, '项'));
        let ol = assistant_styled_text("1. 有序");
        assert!(has_gray(&ol, '1'));
        assert!(has_gray(&ol, '.'));
        let quote = assistant_styled_text("> 引用");
        assert!(has_gray(&quote, '>'));
        // 完整文本原样保留（标记字符仍在、未被样式吞掉）
        let joined: String = quote.iter().map(|(_, ch)| *ch).collect();
        assert_eq!(joined, "> 引用");
    }

    #[test]
    fn heading_keeps_inline_code_and_no_marker_loss() {
        let chars = assistant_styled_text("# 看 `x` 用");
        assert!(chars.iter().any(|(s, ch)| {
            *ch == 'x' && s.fg == Some(Color::Cyan) && s.add_modifier.contains(Modifier::BOLD)
        }));
        let joined: String = chars.iter().map(|(_, ch)| *ch).collect();
        assert_eq!(joined, "# 看 x 用");
    }

    #[test]
    fn spaced_asterisks_mid_line_not_treated_as_list() {
        let chars = assistant_styled_text("a * b * c");
        let joined: String = chars.iter().map(|(_, ch)| *ch).collect();
        assert_eq!(joined, "a * b * c");
        assert!(!has_bold(&chars, 'b') && !chars.iter().any(|(s, _)| s.fg == Some(Color::Gray)));
    }

    #[test]
    fn glued_markers_match_desktop_normalize_semantics() {
        // `###规范` / `-规范` / `1.下一步` / `>正文`：与共享 normalize 补空格后的效果一致
        let h = assistant_styled_text("###规范");
        assert!(has_gray(&h, '#'));
        assert!(has_bold(&h, '规'));
        let ul = assistant_styled_text("-规范");
        assert!(has_gray(&ul, '-'));
        let ol = assistant_styled_text("1.下一步");
        assert!(has_gray(&ol, '1'));
        assert!(has_gray(&ol, '.'));
        let q = assistant_styled_text(">正文");
        assert!(has_gray(&q, '>'));
        // 文本逐字保留（不增删字符）
        for input in ["###规范", "-规范", "1.下一步", ">正文"] {
            let chars = assistant_styled_text(input);
            let joined: String = chars.iter().map(|(_, ch)| *ch).collect();
            assert_eq!(joined, input, "原文应逐字保留: {input}");
        }
    }

    #[test]
    fn ascii_flags_and_asterisk_emphasis_stay_untouched() {
        // normalize 对 ASCII 粘连不补空格：`-rf` 不是列表
        let rf = assistant_styled_text("-rf");
        assert!(!rf.iter().any(|(s, _)| s.fg == Some(Color::Gray)));
        // 行首 `*强调*` 走斜体而非列表
        let em = assistant_styled_text("*强调*");
        assert!(
            em.iter()
                .any(|(s, ch)| *ch == '强' && s.add_modifier.contains(Modifier::ITALIC))
        );
        assert!(!em.iter().any(|(s, _)| s.fg == Some(Color::Gray)));
    }

    #[test]
    fn heading_tab_separator_accepted() {
        let chars = assistant_styled_text("#\t标题");
        assert!(has_gray(&chars, '#'));
        assert!(has_bold(&chars, '标'));
    }
}
