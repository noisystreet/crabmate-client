//! 聊天 Markdown HTML 的 ammonia 白名单：任务列表 checkbox、`code` 的 `language-*` class。

use std::borrow::Cow;

/// 净化 pulldown-cmark 输出：保留只读任务列表与围栏语言 class，其余走 ammonia 默认。
pub fn clean_chat_html(body: &str) -> String {
    chat_ammonia_builder().clean(body).to_string()
}

fn chat_ammonia_builder() -> ammonia::Builder<'static> {
    let mut builder = ammonia::Builder::default();
    builder.link_rel(Some("noopener noreferrer"));
    builder.add_tags(["input"]);
    builder.add_tag_attributes("input", ["checked"]);
    // 任何漏网的 `<input>` 都强制成只读 checkbox，避免 `type=text` 等可交互控件。
    builder.set_tag_attribute_value("input", "type", "checkbox");
    builder.set_tag_attribute_value("input", "disabled", "disabled");
    builder.add_tag_attributes("code", ["class"]);
    builder.add_tag_attributes("pre", ["class"]);
    builder.attribute_filter(filter_code_language_class);
    builder
}

fn filter_code_language_class<'u>(el: &str, attr: &str, val: &'u str) -> Option<Cow<'u, str>> {
    if attr != "class" || (el != "code" && el != "pre") {
        return Some(Cow::Borrowed(val));
    }
    keep_language_classes(val)
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
}
