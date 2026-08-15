//! 闭合围栏：在已净化 HTML 上加语言标签与复制按钮（不走 ammonia，chrome 由本模块生成）。

fn language_from_code_open(open: &str) -> Option<&str> {
    let i = open.find("language-")?;
    let rest = &open[i + "language-".len()..];
    let n = rest
        .bytes()
        .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'+' | b'.' | b'-' | b'#'))
        .count();
    if n == 0 { None } else { Some(&rest[..n]) }
}

fn skip_ws(s: &str, mut i: usize) -> usize {
    while s
        .as_bytes()
        .get(i)
        .is_some_and(|b| *b == b' ' || *b == b'\n' || *b == b'\t')
    {
        i += 1;
    }
    i
}

fn find_code_pre_close(s: &str) -> Option<usize> {
    let code_end = s.find("</code>")?;
    let after = &s[code_end + 7..];
    let ws = skip_ws(after, 0);
    if !after[ws..].starts_with("</pre>") {
        return None;
    }
    Some(code_end + 7 + ws + 6)
}

fn push_escaped(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
}

fn wrap_pre_html(lang: Option<&str>, inner_pre: &str) -> String {
    let mut out = String::with_capacity(inner_pre.len() + 160);
    out.push_str("<div class=\"md-code-block\"><div class=\"md-code-block__bar\">");
    if let Some(lang) = lang {
        out.push_str("<span class=\"md-code-block__lang\">");
        push_escaped(&mut out, lang);
        out.push_str("</span>");
    } else {
        out.push_str("<span class=\"md-code-block__lang\"></span>");
    }
    out.push_str(
        "<button type=\"button\" class=\"md-code-block__copy\" data-md-copy-code>\
         <span class=\"md-code-block__copy-en\">Copy</span>\
         <span class=\"md-code-block__copy-zh\">复制</span></button>",
    );
    out.push_str("</div>");
    out.push_str(inner_pre);
    out.push_str("</div>");
    out
}

fn wrap_one_pre(from_pre: &str) -> Option<(String, usize)> {
    const PRE: &str = "<pre>";
    if !from_pre.starts_with(PRE) {
        return None;
    }
    let i = skip_ws(from_pre, PRE.len());
    if !from_pre[i..].starts_with("<code") {
        return None;
    }
    let gt = from_pre[i..].find('>')?;
    let open_end = i + gt + 1;
    let lang = language_from_code_open(&from_pre[i..open_end]);
    let consumed = find_code_pre_close(&from_pre[open_end..])? + open_end;
    Some((wrap_pre_html(lang, &from_pre[..consumed]), consumed))
}

/// 把已净化的 `<pre><code>` 包上语言条与 Copy 按钮。
pub fn decorate_fenced_code_blocks(html: &str) -> String {
    let mut out = String::with_capacity(html.len().saturating_add(html.len() / 6));
    let mut rest = html;
    while let Some(idx) = rest.find("<pre>") {
        out.push_str(&rest[..idx]);
        let from_pre = &rest[idx..];
        if let Some((wrapped, consumed)) = wrap_one_pre(from_pre) {
            out.push_str(&wrapped);
            rest = &from_pre[consumed..];
        } else {
            out.push_str(PRE_TAG);
            rest = &from_pre[PRE_TAG.len()..];
        }
    }
    out.push_str(rest);
    out
}

const PRE_TAG: &str = "<pre>";

#[cfg(test)]
mod tests {
    use super::decorate_fenced_code_blocks;

    #[test]
    fn wraps_language_fence_with_toolbar() {
        let h = decorate_fenced_code_blocks(
            "<pre><code class=\"language-rust\">let x = 1;</code></pre>",
        );
        assert!(h.contains("md-code-block"), "got {h:?}");
        assert!(h.contains("data-md-copy-code"), "got {h:?}");
        assert!(h.contains(">Copy</span>"), "got {h:?}");
        assert!(h.contains(">复制</span>"), "got {h:?}");
        assert!(h.contains(">rust</span>"), "got {h:?}");
        assert!(h.contains("language-rust"), "got {h:?}");
        assert!(h.contains("<pre><code"), "got {h:?}");
    }

    #[test]
    fn wraps_plain_fence_without_lang_label() {
        let h = decorate_fenced_code_blocks("<pre><code>plain</code></pre>");
        assert!(h.contains("md-code-block"), "got {h:?}");
        assert!(h.contains("data-md-copy-code"), "got {h:?}");
        assert!(!h.contains(">plain</span>"), "got {h:?}");
    }
}
