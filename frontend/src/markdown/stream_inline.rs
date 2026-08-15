//! 流式活跃行：只渲染**已成对**的行内标记，半截保持转义字面量。

use super::autolink::http_url_end_at;

fn push_escaped_char(out: &mut String, c: char) {
    match c {
        '&' => out.push_str("&amp;"),
        '<' => out.push_str("&lt;"),
        '>' => out.push_str("&gt;"),
        '"' => out.push_str("&quot;"),
        _ => out.push(c),
    }
}

fn push_escaped_str(out: &mut String, s: &str) {
    for c in s.chars() {
        push_escaped_char(out, c);
    }
}

/// 成对扫描之外：当前位置若已是 `http(s)://` URL 则写成安全 `<a>`。
fn try_push_autolink(out: &mut String, text: &str, i: usize) -> Option<usize> {
    let end = http_url_end_at(text, i)?;
    let url = &text[i..end];
    out.push_str("<a target=\"_blank\" rel=\"noopener noreferrer\" href=\"");
    push_escaped_str(out, url);
    out.push_str("\">");
    push_escaped_str(out, url);
    out.push_str("</a>");
    Some(end)
}

/// 在 `from` 起查找闭合 delimiter；要求非空内容且不跨换行。
fn find_closing_delim(s: &str, from: usize, delim: &str) -> Option<usize> {
    if from >= s.len() {
        return None;
    }
    let rest = &s[from..];
    let mut search_at = 0usize;
    while let Some(rel) = rest[search_at..].find(delim) {
        let abs = from + search_at + rel;
        if abs > from {
            let inner = &s[from..abs];
            if !inner.is_empty() && !inner.contains('\n') {
                return Some(abs);
            }
        }
        search_at += rel + delim.len();
        if search_at >= rest.len() {
            break;
        }
    }
    None
}

fn try_code(out: &mut String, text: &str, i: usize) -> Option<usize> {
    if !text[i..].starts_with('`') {
        return None;
    }
    let end = find_closing_delim(text, i + 1, "`")?;
    out.push_str("<code>");
    push_escaped_str(out, &text[i + 1..end]);
    out.push_str("</code>");
    Some(end + 1)
}

fn try_strong(out: &mut String, text: &str, i: usize) -> Option<usize> {
    if !text[i..].starts_with("**") {
        return None;
    }
    let end = find_closing_delim(text, i + 2, "**")?;
    out.push_str("<strong>");
    out.push_str(&stream_inline(&text[i + 2..end], false));
    out.push_str("</strong>");
    Some(end + 2)
}

fn try_strike(out: &mut String, text: &str, i: usize) -> Option<usize> {
    if !text[i..].starts_with("~~") {
        return None;
    }
    let end = find_closing_delim(text, i + 2, "~~")?;
    out.push_str("<del>");
    push_escaped_str(out, &text[i + 2..end]);
    out.push_str("</del>");
    Some(end + 2)
}

fn try_em_star(out: &mut String, text: &str, i: usize) -> Option<usize> {
    if text[i..].starts_with("**") || !text[i..].starts_with('*') {
        return None;
    }
    let end = find_closing_delim(text, i + 1, "*")?;
    out.push_str("<em>");
    push_escaped_str(out, &text[i + 1..end]);
    out.push_str("</em>");
    Some(end + 1)
}

fn prev_char_is_alnum(text: &str, i: usize) -> bool {
    text[..i]
        .chars()
        .next_back()
        .is_some_and(|c| c.is_ascii_alphanumeric())
}

fn next_char_is_alnum(text: &str, i: usize) -> bool {
    text[i..]
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric())
}

fn try_em_underscore(out: &mut String, text: &str, i: usize) -> Option<usize> {
    if !text[i..].starts_with('_') || prev_char_is_alnum(text, i) {
        return None;
    }
    let end = find_closing_delim(text, i + 1, "_")?;
    let close_at = end + 1;
    if next_char_is_alnum(text, close_at) {
        return None;
    }
    out.push_str("<em>");
    push_escaped_str(out, &text[i + 1..end]);
    out.push_str("</em>");
    Some(close_at)
}

fn consume_incomplete_http_link(
    out: &mut String,
    text: &str,
    open_at: usize,
    url_at: usize,
) -> Option<usize> {
    let url_end = http_url_end_at(text, url_at)?;
    push_escaped_str(out, &text[open_at..url_end]);
    Some(url_end)
}

fn is_safe_stream_href(url: &str) -> bool {
    if url.is_empty() || url.contains(|c: char| c.is_whitespace() || matches!(c, '"' | '<' | '>')) {
        return false;
    }
    let b = url.as_bytes();
    (b.len() > 8 && b[..8].eq_ignore_ascii_case(b"https://"))
        || (b.len() > 7 && b[..7].eq_ignore_ascii_case(b"http://"))
}

fn push_md_link(out: &mut String, label: &str, url: &str) {
    out.push_str("<a target=\"_blank\" rel=\"noopener noreferrer\" href=\"");
    push_escaped_str(out, url);
    out.push_str("\">");
    push_escaped_str(out, label);
    out.push_str("</a>");
}

/// `[label](` 的 label 与 URL 起点；半截或非法则 `None`。
fn parse_md_link_label(text: &str, i: usize) -> Option<(&str, usize)> {
    if !text[i..].starts_with('[') {
        return None;
    }
    let rest = &text[i + 1..];
    let close_br = rest.find(']')?;
    if close_br == 0 || rest[..close_br].contains('\n') {
        return None;
    }
    if !rest[close_br + 1..].starts_with('(') {
        return None;
    }
    Some((&rest[..close_br], i + 1 + close_br + 2))
}

fn finish_http_md_link(
    out: &mut String,
    text: &str,
    open_at: usize,
    url_at: usize,
    url_end: usize,
    label: &str,
) -> Option<usize> {
    if text.get(url_end..).is_some_and(|s| s.starts_with(')')) {
        let url = &text[url_at..url_end];
        if !is_safe_stream_href(url) {
            return None;
        }
        push_md_link(out, label, url);
        Some(url_end + 1)
    } else {
        consume_incomplete_http_link(out, text, open_at, url_at)
    }
}

fn try_md_link(out: &mut String, text: &str, i: usize) -> Option<usize> {
    let (label, url_at) = parse_md_link_label(text, i)?;
    if let Some(url_end) = http_url_end_at(text, url_at) {
        return finish_http_md_link(out, text, i, url_at, url_end, label);
    }
    None
}

fn stream_inline(text: &str, allow_strong: bool) -> String {
    let mut out = String::with_capacity(text.len().saturating_mul(2));
    let mut i = 0usize;
    while i < text.len() {
        if let Some(n) = try_code(&mut out, text, i) {
            i = n;
            continue;
        }
        if allow_strong && let Some(n) = try_strong(&mut out, text, i) {
            i = n;
            continue;
        }
        if let Some(n) = try_strike(&mut out, text, i) {
            i = n;
            continue;
        }
        if let Some(n) = try_em_star(&mut out, text, i) {
            i = n;
            continue;
        }
        if let Some(n) = try_em_underscore(&mut out, text, i) {
            i = n;
            continue;
        }
        if let Some(n) = try_md_link(&mut out, text, i) {
            i = n;
            continue;
        }
        if let Some(n) = try_push_autolink(&mut out, text, i) {
            i = n;
            continue;
        }
        let c = text[i..].chars().next().unwrap_or('\0');
        push_escaped_char(&mut out, c);
        i += c.len_utf8();
    }
    out
}

/// 流式活跃行：仅渲染**已成对**的行内标记，半截 `**` / `` ` `` / `*` / `[text](url)` 保持转义字面量。
#[must_use]
pub fn stream_inline_safe_html(text: &str) -> String {
    if text.is_empty() {
        String::new()
    } else {
        stream_inline(text, true)
    }
}

#[cfg(test)]
mod tests {
    use super::stream_inline_safe_html;

    #[test]
    fn balanced_then_incomplete_bold() {
        let h = stream_inline_safe_html("**ok** and **no");
        assert!(h.contains("<strong>ok</strong>"), "got {h}");
        assert!(h.contains("**no"), "got {h}");
        assert_eq!(h.matches("<strong>").count(), 1, "got {h}");
    }

    #[test]
    fn paired_italic_star_and_incomplete() {
        let h = stream_inline_safe_html("见 *斜体* 与 *半");
        assert!(h.contains("<em>斜体</em>"), "got {h}");
        assert!(h.contains("*半"), "got {h}");
        assert_eq!(h.matches("<em>").count(), 1, "got {h}");
    }

    #[test]
    fn underscore_italic_skips_snake_case() {
        let h = stream_inline_safe_html("foo_bar_baz 与 _em_");
        assert!(h.contains("foo_bar_baz"), "got {h}");
        assert!(!h.contains("<em>bar</em>"), "got {h}");
        assert!(h.contains("<em>em</em>"), "got {h}");
    }

    #[test]
    fn complete_markdown_link_and_incomplete() {
        let h = stream_inline_safe_html("见 [site](https://example.com) 与 [no](https://x");
        assert!(h.contains("href=\"https://example.com\""), "got {h}");
        assert!(h.contains(">site</a>"), "got {h}");
        assert!(h.contains("[no](https://x"), "got {h}");
        assert_eq!(h.matches("<a ").count(), 1, "got {h}");
    }

    #[test]
    fn javascript_markdown_link_stays_literal() {
        let h = stream_inline_safe_html("[x](javascript:alert(1))");
        assert!(!h.contains("<a "), "got {h}");
        assert!(h.contains("javascript:alert(1)"), "got {h}");
    }

    #[test]
    fn markdown_link_keeps_balanced_parens_in_url() {
        let h = stream_inline_safe_html("[wiki](https://en.wikipedia.org/wiki/Foo_(bar))");
        assert!(
            h.contains("href=\"https://en.wikipedia.org/wiki/Foo_(bar)\""),
            "got {h}"
        );
        assert!(h.contains(">wiki</a>"), "got {h}");
        assert_eq!(h.matches("<a ").count(), 1, "got {h}");
    }
}
