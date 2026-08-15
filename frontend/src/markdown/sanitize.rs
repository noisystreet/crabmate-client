//! 聊天 Markdown HTML 的 ammonia 白名单：任务列表 checkbox、`code` 的 `language-*`、GFM alert class。

use std::borrow::Cow;

const ALERT_CLASSES: &[&str] = &[
    "markdown-alert-note",
    "markdown-alert-tip",
    "markdown-alert-important",
    "markdown-alert-warning",
    "markdown-alert-caution",
];

/// 净化 pulldown-cmark 输出：保留只读任务列表、围栏语言 class、GFM alert class。
pub fn clean_chat_html(body: &str) -> String {
    let cleaned = unwrap_anchors_without_href(&chat_ammonia_builder().clean(body).to_string());
    demote_chat_headings(&cleaned)
}

/// 消息内 `h1`/`h2` 降为 `h3`/`h4`，避免抢页面大纲层级。
fn demote_chat_headings(html: &str) -> String {
    html.replace("<h1", "<h3")
        .replace("</h1>", "</h3>")
        .replace("<h2", "<h4")
        .replace("</h2>", "</h4>")
}

fn find_anchor_open(s: &str) -> Option<usize> {
    let mut search = 0usize;
    while let Some(rel) = s[search..].find("<a") {
        let abs = search + rel;
        let next = s.as_bytes().get(abs + 2).copied();
        if next.is_none_or(|b| b == b' ' || b == b'>' || b == b'\t' || b == b'\n' || b == b'/') {
            return Some(abs);
        }
        search = abs + 2;
    }
    None
}

fn open_tag_has_href(open: &str) -> bool {
    open.as_bytes()
        .windows(5)
        .any(|w| w.eq_ignore_ascii_case(b"href="))
}

/// ammonia 剥掉非法 `href` 后会留下无链接的 `<a target=_blank>`；拆成纯文本避免空跳转。
fn unwrap_anchors_without_href(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    loop {
        let Some(rel) = find_anchor_open(rest) else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..rel]);
        let from_a = &rest[rel..];
        let Some(gt) = from_a.find('>') else {
            out.push_str(from_a);
            break;
        };
        let open = &from_a[..=gt];
        if open_tag_has_href(open) {
            out.push_str(&from_a[..=gt]);
            rest = &from_a[gt + 1..];
            continue;
        }
        let inner = &from_a[gt + 1..];
        if let Some(end) = inner.find("</a>") {
            out.push_str(&inner[..end]);
            rest = &inner[end + 4..];
        } else {
            out.push_str(inner);
            break;
        }
    }
    out
}

fn chat_ammonia_builder() -> ammonia::Builder<'static> {
    let mut builder = ammonia::Builder::default();
    builder.link_rel(Some("noopener noreferrer"));
    builder.add_tags(["input"]);
    builder.add_tag_attributes("input", ["checked"]);
    // 任何漏网的 `<input>` 都强制成只读 checkbox，避免 `type=text` 等可交互控件。
    builder.set_tag_attribute_value("input", "type", "checkbox");
    builder.set_tag_attribute_value("input", "disabled", "disabled");
    builder.set_tag_attribute_value("a", "target", "_blank");
    builder.set_tag_attribute_value("img", "referrerpolicy", "no-referrer");
    builder.add_tag_attributes("code", ["class"]);
    builder.add_tag_attributes("pre", ["class"]);
    builder.add_tag_attributes("blockquote", ["class"]);
    builder.attribute_filter(filter_safe_classes);
    builder
}

fn filter_safe_classes<'u>(el: &str, attr: &str, val: &'u str) -> Option<Cow<'u, str>> {
    if attr != "class" {
        return Some(Cow::Borrowed(val));
    }
    match el {
        "code" | "pre" => keep_language_classes(val),
        "blockquote" => keep_alert_classes(val),
        _ => Some(Cow::Borrowed(val)),
    }
}

fn keep_alert_classes(val: &str) -> Option<Cow<'static, str>> {
    let kept: Vec<&str> = val
        .split_whitespace()
        .filter(|class| ALERT_CLASSES.contains(class))
        .collect();
    if kept.is_empty() {
        None
    } else {
        Some(Cow::Owned(kept.join(" ")))
    }
}

fn keep_language_classes(val: &str) -> Option<Cow<'static, str>> {
    let kept: Vec<&str> = val
        .split_whitespace()
        .filter(|class| is_language_class(class))
        .collect();
    if kept.is_empty() {
        None
    } else {
        Some(Cow::Owned(kept.join(" ")))
    }
}

fn is_language_class(class: &str) -> bool {
    let Some(rest) = class.strip_prefix("language-") else {
        return false;
    };
    !rest.is_empty()
        && rest
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'+' | b'.' | b'-' | b'#'))
}

#[cfg(test)]
mod tests {
    use super::{clean_chat_html, is_language_class};

    #[test]
    fn language_class_token_rules() {
        assert!(is_language_class("language-rust"));
        assert!(is_language_class("language-c++"));
        assert!(is_language_class("language-c#"));
        assert!(!is_language_class("language-"));
        assert!(!is_language_class("evil"));
        assert!(!is_language_class("language-rs;alert(1)"));
    }

    #[test]
    fn strips_non_language_code_class() {
        let h = clean_chat_html("<code class=\"evil language-rust\">x</code>");
        assert!(h.contains("language-rust"), "got {h:?}");
        assert!(!h.contains("evil"), "got {h:?}");
    }

    #[test]
    fn injected_text_input_becomes_disabled_checkbox() {
        let h = clean_chat_html("<input type=\"text\" name=\"x\" value=\"y\">");
        let lower = h.to_lowercase();
        assert!(lower.contains("<input"), "got {h:?}");
        assert!(lower.contains("type=\"checkbox\""), "got {h:?}");
        assert!(lower.contains("disabled"), "got {h:?}");
        assert!(!lower.contains("type=\"text\""), "got {h:?}");
        assert!(!lower.contains("name="), "got {h:?}");
        assert!(!lower.contains("value="), "got {h:?}");
    }

    #[test]
    fn keeps_gfm_alert_class_on_blockquote() {
        let h = clean_chat_html("<blockquote class=\"markdown-alert-note evil\">n</blockquote>");
        assert!(h.contains("markdown-alert-note"), "got {h:?}");
        assert!(!h.contains("evil"), "got {h:?}");
    }

    #[test]
    fn unwraps_anchor_when_href_was_stripped() {
        let h = clean_chat_html("<a target=\"_blank\" rel=\"noopener noreferrer\">x</a>");
        assert!(!h.contains("<a"), "got {h:?}");
        assert!(h.contains('x'), "got {h:?}");
    }

    #[test]
    fn keeps_anchor_with_href() {
        let h = clean_chat_html("<a href=\"https://example.com\">x</a>");
        assert!(h.contains("<a"), "got {h:?}");
        assert!(h.contains("https://example.com"), "got {h:?}");
    }

    #[test]
    fn demotes_h1_and_h2() {
        let h = clean_chat_html("<h1>t</h1><h2>s</h2><h3>k</h3>");
        assert!(!h.contains("<h1"), "got {h:?}");
        assert!(!h.contains("<h2"), "got {h:?}");
        assert!(h.contains("<h3>t</h3>"), "got {h:?}");
        assert!(h.contains("<h4>s</h4>"), "got {h:?}");
        assert!(h.contains("<h3>k</h3>"), "got {h:?}");
    }
}
