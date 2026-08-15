//! 将正文中的裸 `http://` / `https://` 收成链接（代码块、已有链接内不处理）。
//! pulldown-cmark 0.13 的 CommonMark 自动链接只认 `<https://…>`，GFM 裸 URL 需自行扫描。

use pulldown_cmark::{CowStr, Event, LinkType, Tag, TagEnd};

const HTTP: &str = "http://";
const HTTPS: &str = "https://";

/// 改写解析事件：段落内裸 URL → Autolink；`SoftBreak` → `HardBreak`。
pub fn rewrite_events<'a, I>(events: I) -> Vec<Event<'a>>
where
    I: Iterator<Item = Event<'a>>,
{
    let mut out = Vec::new();
    let mut skip = 0u32;
    for ev in events {
        apply_event(&mut out, &mut skip, ev);
    }
    out
}

fn apply_event<'a>(out: &mut Vec<Event<'a>>, skip: &mut u32, ev: Event<'a>) {
    match ev {
        Event::Start(tag) => {
            if skip_start(&tag) {
                *skip = skip.saturating_add(1);
            }
            out.push(Event::Start(tag));
        }
        Event::End(end) => {
            if skip_end(&end) {
                *skip = skip.saturating_sub(1);
            }
            out.push(Event::End(end));
        }
        Event::SoftBreak => out.push(Event::HardBreak),
        Event::Text(text) if *skip == 0 => {
            if next_http_url(text.as_ref(), 0).is_none() {
                out.push(Event::Text(text));
            } else {
                push_autolinked_text(out, text.as_ref());
            }
        }
        other => out.push(other),
    }
}

fn skip_start(tag: &Tag<'_>) -> bool {
    matches!(
        tag,
        Tag::CodeBlock(_) | Tag::Link { .. } | Tag::Image { .. }
    )
}

fn skip_end(end: &TagEnd) -> bool {
    matches!(end, TagEnd::CodeBlock | TagEnd::Link | TagEnd::Image)
}

fn push_autolinked_text<'a>(out: &mut Vec<Event<'a>>, text: &str) {
    let mut i = 0usize;
    while let Some((start, end)) = next_http_url(text, i) {
        if start > i {
            out.push(Event::Text(CowStr::from(text[i..start].to_string())));
        }
        push_autolink_events(out, &text[start..end]);
        i = end;
    }
    if i < text.len() {
        out.push(Event::Text(CowStr::from(text[i..].to_string())));
    }
}

fn push_autolink_events<'a>(out: &mut Vec<Event<'a>>, url: &str) {
    let owned = url.to_string();
    out.push(Event::Start(Tag::Link {
        link_type: LinkType::Autolink,
        dest_url: CowStr::from(owned.clone()),
        title: CowStr::from(""),
        id: CowStr::from(""),
    }));
    out.push(Event::Text(CowStr::from(owned)));
    out.push(Event::End(TagEnd::Link));
}

/// 若 `s[i..]` 以合法 http(s) URL 开头，返回该 URL 的字节终点（不含）。
pub fn http_url_end_at(s: &str, i: usize) -> Option<usize> {
    let (start, end) = next_http_url(s, i)?;
    (start == i).then_some(end)
}

fn next_http_url(s: &str, mut from: usize) -> Option<(usize, usize)> {
    while from < s.len() {
        let rel = find_scheme_rel(&s[from..])?;
        let start = from + rel;
        if url_start_ok(s, start) {
            if let Some(end) = scan_url_end(s, start) {
                return Some((start, end));
            }
        }
        from = start.saturating_add(1);
    }
    None
}

fn find_scheme_rel(s: &str) -> Option<usize> {
    let mut i = 0usize;
    while i < s.len() {
        if scheme_len_at(s, i).is_some() {
            return Some(i);
        }
        let ch = s[i..].chars().next()?;
        i += ch.len_utf8();
    }
    None
}

fn url_start_ok(s: &str, start: usize) -> bool {
    if start == 0 {
        return true;
    }
    let Some(prev) = s[..start].chars().next_back() else {
        return true;
    };
    !prev.is_ascii_alphanumeric() && prev != '_'
}

fn starts_with_ignore_ascii(s: &str, prefix: &str) -> bool {
    let sb = s.as_bytes();
    let pb = prefix.as_bytes();
    sb.len() >= pb.len() && sb[..pb.len()].eq_ignore_ascii_case(pb)
}

fn scheme_len_at(s: &str, start: usize) -> Option<usize> {
    let rest = s.get(start..)?;
    if starts_with_ignore_ascii(rest, HTTPS) {
        Some(HTTPS.len())
    } else if starts_with_ignore_ascii(rest, HTTP) {
        Some(HTTP.len())
    } else {
        None
    }
}

fn scan_url_end(s: &str, start: usize) -> Option<usize> {
    let scheme_len = scheme_len_at(s, start)?;
    let mut i = start + scheme_len;
    if i >= s.len() {
        return None;
    }
    let first = s[i..].chars().next()?;
    if !host_start_ok(first) {
        return None;
    }
    while i < s.len() {
        let Some(c) = s[i..].chars().next() else {
            break;
        };
        if !is_url_body_char(c) {
            break;
        }
        i += c.len_utf8();
    }
    i = trim_url_end(s, start, i);
    if i <= start + scheme_len {
        None
    } else {
        Some(i)
    }
}

fn host_start_ok(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '[' || is_non_ascii_url_char(c)
}

fn is_url_body_char(c: char) -> bool {
    if c.is_ascii_alphanumeric() {
        return true;
    }
    if c.is_ascii() {
        return is_ascii_url_punct(c);
    }
    is_non_ascii_url_char(c)
}

fn is_ascii_url_punct(c: char) -> bool {
    matches!(
        c,
        '-' | '.'
            | '_'
            | '~'
            | ':'
            | '/'
            | '?'
            | '#'
            | '['
            | ']'
            | '@'
            | '!'
            | '$'
            | '&'
            | '\''
            | '('
            | ')'
            | '*'
            | '+'
            | ','
            | ';'
            | '='
            | '%'
    )
}

fn is_non_ascii_url_char(c: char) -> bool {
    !c.is_whitespace() && !c.is_control() && !is_cjk_url_stop(c)
}

fn is_cjk_url_stop(c: char) -> bool {
    matches!(
        c,
        '。' | '，' | '；' | '：' | '！' | '？' | '、' | '）' | '】' | '」' | '』'
    )
}

fn trim_url_end(s: &str, start: usize, mut end: usize) -> usize {
    while let Some(c) = s.get(start..end).and_then(|t| t.chars().next_back()) {
        if !should_trim_trailing(s, start, end, c) {
            break;
        }
        end -= c.len_utf8();
    }
    end
}

fn should_trim_trailing(s: &str, start: usize, end: usize, c: char) -> bool {
    if matches!(c, '.' | ',' | ':' | ';' | '!' | '?') {
        return true;
    }
    match c {
        ')' => unmatched_close(s, start, end, '(', ')'),
        ']' => unmatched_close(s, start, end, '[', ']'),
        '}' => unmatched_close(s, start, end, '{', '}'),
        _ => false,
    }
}

fn unmatched_close(s: &str, start: usize, end: usize, open: char, close: char) -> bool {
    let slice = &s[start..end];
    let n_open = slice.chars().filter(|&ch| ch == open).count();
    let n_close = slice.chars().filter(|&ch| ch == close).count();
    n_close > n_open
}

#[cfg(test)]
mod tests {
    use super::{http_url_end_at, next_http_url};

    #[test]
    fn finds_https_in_prose() {
        let s = "见 https://example.com/path 尾";
        let (a, b) = next_http_url(s, 0).expect("url");
        assert_eq!(&s[a..b], "https://example.com/path");
    }

    #[test]
    fn strips_trailing_period() {
        let s = "https://example.com.";
        let (a, b) = next_http_url(s, 0).expect("url");
        assert_eq!(&s[a..b], "https://example.com");
    }

    #[test]
    fn strips_unmatched_paren() {
        let s = "(https://example.com)";
        let (a, b) = next_http_url(s, 0).expect("url");
        assert_eq!(&s[a..b], "https://example.com");
    }

    #[test]
    fn keeps_balanced_paren_in_path() {
        let s = "https://en.wikipedia.org/wiki/Foo_(bar)";
        let (a, b) = next_http_url(s, 0).expect("url");
        assert_eq!(&s[a..b], s);
    }

    #[test]
    fn rejects_javascript_scheme() {
        assert!(next_http_url("javascript:alert(1)", 0).is_none());
    }

    #[test]
    fn rejects_glued_to_identifier() {
        assert!(next_http_url("xhttps://example.com", 0).is_none());
    }

    #[test]
    fn prefix_at_index() {
        let s = "见 https://example.com";
        let i = s.find("https").unwrap();
        assert_eq!(http_url_end_at(s, i), Some(s.len()));
        assert!(http_url_end_at(s, 0).is_none());
    }

    #[test]
    fn scheme_is_case_insensitive() {
        let s = "HTTPS://EXAMPLE.COM/Path";
        let (a, b) = next_http_url(s, 0).expect("url");
        assert_eq!(&s[a..b], s);
    }

    #[test]
    fn keeps_cjk_path() {
        let s = "https://example.com/文档";
        let (a, b) = next_http_url(s, 0).expect("url");
        assert_eq!(&s[a..b], s);
    }

    #[test]
    fn cjk_period_is_not_part_of_url() {
        let s = "https://example.com/文档。";
        let (a, b) = next_http_url(s, 0).expect("url");
        assert_eq!(&s[a..b], "https://example.com/文档");
    }

    #[test]
    fn ipv6_literal() {
        let s = "http://[::1]/health";
        let (a, b) = next_http_url(s, 0).expect("url");
        assert_eq!(&s[a..b], s);
    }
}
