//! 聊天气泡内 Markdown → 安全 HTML（`ammonia` 白名单），供助手消息渲染。
//!
//! 解析前会做 `crabmate_client_api::markdown_normalize::normalize_markdown_for_render`
//! （文本规范化已下沉到 `crabmate-client-api` 共享，TUI/Web 同源），
//! 降低模型常见围栏误写导致的整段被吃进代码块等问题。

mod autolink;
mod code_block;
mod sanitize;
mod stream_inline;
pub(crate) mod workspace_image;

use pulldown_cmark::{Options, Parser, html};

pub(crate) use stream_inline::stream_inline_safe_html;

use crabmate_client_api::markdown_normalize::normalize_markdown_for_render;

/// 将 Markdown 转为经净化的 HTML 片段（不含外层 `<html>`）。
/// 段落内单换行按硬换行输出 `<br />`，避免在 `white-space: normal` 下被收成空格。
pub fn to_safe_html(md: &str) -> String {
    if md.trim().is_empty() {
        return String::new();
    }
    let md = normalize_markdown_for_render(md);
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_GFM);
    let parser = Parser::new_ext(&md, opts);
    let events = autolink::rewrite_events(parser);
    let mut body = String::new();
    html::push_html(&mut body, events.into_iter());
    let cleaned = sanitize::clean_chat_html(&body);
    code_block::decorate_fenced_code_blocks(&cleaned)
}

/// 调试：不做 Markdown 解析，将纯文本转义为可安全写入 `innerHTML` 的片段（换行 → `<br />`）。
pub fn plaintext_to_safe_html(text: &str) -> String {
    if text.trim().is_empty() {
        return String::new();
    }
    let mut out = String::with_capacity(text.len().saturating_mul(2));
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("<br />"),
            '\r' => {}
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{normalize_markdown_for_render, plaintext_to_safe_html, to_safe_html};

    #[test]
    fn multi_level_headings_produce_h_tags() {
        let h = to_safe_html("# Title\n\n## Sub\n\n### H3");
        assert!(!h.contains("<h1"), "message h1 must be demoted, got {h:?}");
        assert!(!h.contains("<h2"), "message h2 must be demoted, got {h:?}");
        assert!(h.contains("<h3"), "got {h:?}");
        assert!(h.contains("<h4"), "got {h:?}");
        assert!(
            h.contains("Title") && h.contains("Sub") && h.contains("H3"),
            "got {h:?}"
        );
    }

    #[test]
    fn empty_or_whitespace_yields_empty() {
        assert!(to_safe_html("").is_empty());
        assert!(to_safe_html("   \n\t  ").is_empty());
    }

    #[test]
    fn table_parsed_and_kept() {
        let h = to_safe_html("|h1|h2|\n|---|---|\n|a|b|");
        assert!(h.contains("<table"));
        assert!(h.to_lowercase().contains("h1"));
    }

    /// 模型/导出常把表头与 GFM 分隔行写在同一行：`||` 后应为独立一行。
    #[test]
    fn normalize_splits_table_header_glued_to_delimiter_row() {
        let raw = "| 类别 | 能力 ||------|------|";
        let n = normalize_markdown_for_render(raw);
        assert!(
            n.contains("能力") && n.contains('\n') && n.contains("------"),
            "expected two lines, got {n:?}"
        );
        let h = to_safe_html(raw);
        assert!(
            h.contains("<table"),
            "glued header+delimiter should become a table, got {h:?}"
        );
    }

    #[test]
    fn chat_links_get_target_blank() {
        let h = to_safe_html("see [site](https://example.com/path)");
        assert!(h.contains("target=\"_blank\""));
        assert!(h.contains("rel=\"noopener noreferrer\""));
        assert!(h.contains("https://example.com/path"));
    }

    #[test]
    fn bare_https_url_becomes_link() {
        let h = to_safe_html("见 https://example.com/path 与尾");
        assert!(h.contains("href=\"https://example.com/path\""), "got {h:?}");
        assert!(h.contains("target=\"_blank\""), "got {h:?}");
        assert!(h.contains(">https://example.com/path<"), "got {h:?}");
    }

    #[test]
    fn bare_url_inside_code_stays_plain() {
        let h = to_safe_html("`https://example.com`");
        assert!(h.contains("<code>"), "got {h:?}");
        assert!(
            !h.contains("<a "),
            "inline code must not autolink, got {h:?}"
        );
    }

    #[test]
    fn bare_url_inside_fence_stays_plain() {
        let h = to_safe_html("```\nhttps://example.com\n```");
        assert!(h.contains("<pre") || h.contains("<code"), "got {h:?}");
        assert!(!h.contains("<a "), "fence must not autolink, got {h:?}");
    }

    #[test]
    fn javascript_scheme_not_autolinked() {
        let h = to_safe_html("go javascript:alert(1) now");
        assert!(!h.contains("<a "), "got {h:?}");
        assert!(h.contains("javascript:alert(1)"), "got {h:?}");
    }

    #[test]
    fn markdown_link_is_not_double_wrapped() {
        let h = to_safe_html("[site](https://example.com/path)");
        assert_eq!(h.matches("<a ").count(), 1, "got {h:?}");
        assert!(h.contains("href=\"https://example.com/path\""), "got {h:?}");
    }

    #[test]
    fn cjk_path_stays_inside_href() {
        let h = to_safe_html("见 https://example.com/文档 尾");
        assert!(h.contains("<a "), "got {h:?}");
        assert!(
            h.contains("https://example.com/文档") || h.contains("%E6%96%87%E6%A1%A3"),
            "CJK path missing from link, got {h:?}"
        );
    }

    #[test]
    fn markdown_off_does_not_autolink() {
        let h = plaintext_to_safe_html("见 https://example.com");
        assert!(!h.contains("<a "), "got {h:?}");
        assert!(h.contains("https://example.com"), "got {h:?}");
    }

    #[test]
    fn script_tag_stripped_by_ammonia() {
        let h = to_safe_html("Hello<script>alert(1)</script>");
        assert!(!h.to_lowercase().contains("<script"));
        assert!(h.contains("Hello"));
    }

    #[test]
    fn fenced_code_keeps_language_class() {
        let h = to_safe_html("```rust\nlet x = 1;\n```");
        assert!(h.contains("<pre") || h.contains("<code"), "got {h:?}");
        assert!(h.contains("language-rust"), "got {h:?}");
        let csharp = to_safe_html("```c#\nConsole.WriteLine(1);\n```");
        assert!(csharp.contains("language-c#"), "got {csharp:?}");
    }

    #[test]
    fn task_list_unchecked_keeps_disabled_checkbox() {
        let h = to_safe_html("- [ ] todo");
        let lower = h.to_lowercase();
        assert!(lower.contains("<input"), "got {h:?}");
        assert!(lower.contains("type=\"checkbox\""), "got {h:?}");
        assert!(lower.contains("disabled"), "got {h:?}");
        assert!(!lower.contains("checked"), "got {h:?}");
        assert!(h.contains("todo"), "got {h:?}");
    }

    #[test]
    fn task_list_checked_keeps_checked_disabled_checkbox() {
        let h = to_safe_html("- [x] done");
        let lower = h.to_lowercase();
        assert!(lower.contains("<input"), "got {h:?}");
        assert!(lower.contains("type=\"checkbox\""), "got {h:?}");
        assert!(lower.contains("disabled"), "got {h:?}");
        assert!(lower.contains("checked"), "got {h:?}");
        assert!(h.contains("done"), "got {h:?}");
    }

    #[test]
    fn loose_task_list_keeps_disabled_checkboxes_inside_paragraphs() {
        let h = to_safe_html("- [ ] a\n\n- [ ] b");
        let lower = h.to_lowercase();
        assert_eq!(lower.matches("<input").count(), 2, "got {h:?}");
        assert!(
            lower.contains("<p>") && lower.contains("<input"),
            "got {h:?}"
        );
        assert!(lower.contains("type=\"checkbox\""), "got {h:?}");
        assert!(lower.contains("disabled"), "got {h:?}");
        assert!(h.contains('a') && h.contains('b'), "got {h:?}");
    }

    #[test]
    fn normalize_splits_inline_fence_opener() {
        let raw = "`llm` 依赖：```rust\ncode()\n```\n\n|a|b|\n|---|---|\n|1|2|";
        let n = normalize_markdown_for_render(raw);
        assert!(
            n.contains("依赖：\n```rust"),
            "expected fence on its own line, got {n:?}"
        );
        let h = to_safe_html(raw);
        assert!(
            h.contains("<table"),
            "table after code fence should parse, got {h:?}"
        );
    }

    #[test]
    fn normalize_splits_glued_blockquote_after_fullwidth_period() {
        let raw = "结束。> 引用";
        let n = normalize_markdown_for_render(raw);
        assert!(
            n.contains("结束。\n\n> 引用"),
            "expected quote on its own block, got {n:?}"
        );
        let h = to_safe_html(raw);
        assert!(h.contains("<blockquote"), "got {h:?}");
    }

    #[test]
    fn normalize_inserts_space_after_list_markers() {
        assert_eq!(normalize_markdown_for_render("-规范"), "- 规范");
        assert_eq!(normalize_markdown_for_render("1.下一步"), "1. 下一步");
        assert_eq!(normalize_markdown_for_render("1.0"), "1.0");
        let h = to_safe_html("-规范\n");
        assert!(h.contains("<li") && h.contains("规范"), "got {h:?}");
    }

    #[test]
    fn normalize_inserts_space_after_atx_hashes() {
        let raw = "###规范与安全\n\n正文。";
        assert_eq!(
            normalize_markdown_for_render(raw),
            "### 规范与安全\n\n正文。"
        );
        let h = to_safe_html(raw);
        assert!(
            h.contains("<h3") && h.contains("规范与安全"),
            "expected h3 heading, got {h:?}"
        );
    }

    #[test]
    fn single_newline_in_paragraph_emits_line_break() {
        let h = to_safe_html("不调用任何工具\n用 JSON 回复");
        let lower = h.to_lowercase();
        assert!(
            lower.contains("<br") || lower.contains("br>"),
            "expected hard line break in HTML, got {h:?}"
        );
    }

    #[test]
    fn plaintext_escapes_and_line_breaks() {
        let h = plaintext_to_safe_html("a <b>\nc");
        assert!(h.contains("&lt;"));
        assert!(h.to_lowercase().contains("<br"));
        assert!(!h.contains("<b>"));
    }

    /// 模型把「运行结果」与围栏粘在一行时，先插换行再解析，避免整段吃进代码块或单 `<p>`。
    #[test]
    fn normalize_inserts_newline_before_fence_after_fullwidth_colon() {
        let raw = "小结：**文件**：`x`（C++17）**编译**：`y`**运行结果**：```out```";
        let n = normalize_markdown_for_render(raw);
        assert!(
            n.contains("运行结果**：\n```") || n.contains("运行结果**：\n\n```"),
            "expected newline before fence, got {n:?}"
        );
        let h = to_safe_html(raw);
        assert!(
            h.matches("<p").count() >= 2 || h.contains("<pre"),
            "expected multiple blocks or a code block, got {h:?}"
        );
    }

    #[test]
    fn fenced_code_gets_toolbar() {
        let h = to_safe_html("```rust\nlet x = 1;\n```");
        assert!(h.contains("md-code-block"), "got {h:?}");
        assert!(h.contains("data-md-copy-code"), "got {h:?}");
        assert!(h.contains(">rust</span>"), "got {h:?}");
    }

    #[test]
    fn gfm_alert_note_keeps_class() {
        let h = to_safe_html("> [!NOTE]\n> hello");
        assert!(h.contains("markdown-alert-note"), "got {h:?}");
        assert!(h.contains("hello"), "got {h:?}");
    }

    #[test]
    fn javascript_href_is_stripped() {
        let h = to_safe_html("[x](javascript:alert(1))");
        let lower = h.to_lowercase();
        assert!(
            !lower.contains("href=\"javascript:") && !lower.contains("href='javascript:"),
            "javascript href must not survive, got {h:?}"
        );
        assert!(
            !lower.contains("<a"),
            "stripped javascript link must not leave a clickable <a>, got {h:?}"
        );
        assert!(h.contains('x'), "got {h:?}");
    }

    #[test]
    fn data_image_src_is_stripped() {
        let h = to_safe_html("![x](data:text/html;base64,PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0Pg==)");
        let lower = h.to_lowercase();
        assert!(!lower.contains("data:"), "got {h:?}");
    }

    #[test]
    fn https_image_gets_referrerpolicy() {
        let h = to_safe_html("![x](https://example.com/p.png)");
        assert!(h.contains("<img"), "got {h:?}");
        assert!(h.contains("referrerpolicy=\"no-referrer\""), "got {h:?}");
        assert!(h.contains("https://example.com/p.png"), "got {h:?}");
    }

    #[test]
    fn relative_workspace_png_rewrites_to_raw_api() {
        let h = to_safe_html("![x](plots/a.png)");
        assert!(h.contains("<img"), "got {h:?}");
        assert!(h.contains("/workspace/file/raw?path="), "got {h:?}");
        assert!(
            !h.contains("plots/a.png\""),
            "raw dest should be query-encoded, got {h:?}"
        );
    }

    #[test]
    fn relative_svg_image_is_stripped() {
        let h = to_safe_html("![x](plots/a.svg)");
        let lower = h.to_lowercase();
        assert!(!lower.contains("<img"), "got {h:?}");
    }

    #[test]
    fn heading_attribute_syntax_does_not_emit_id() {
        let h = to_safe_html("# Title {#main}");
        let lower = h.to_lowercase();
        assert!(!lower.contains("id=\"main\""), "got {h:?}");
        assert!(!lower.contains("id='main'"), "got {h:?}");
        assert!(h.contains("Title"), "got {h:?}");
    }

    #[test]
    fn atx_h1_is_demoted_to_h3() {
        let h = to_safe_html("# Only");
        assert!(h.contains("<h3"), "got {h:?}");
        assert!(!h.contains("<h1"), "got {h:?}");
        assert!(h.contains("Only"), "got {h:?}");
    }
}

/// WASM 下由 `wasm-bindgen-test` 跑通「Markdown → 净化 HTML」链路（与 CSR 目标一致）。
#[cfg(all(test, target_arch = "wasm32"))]
mod wasm_bindgen_tests {
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::to_safe_html;

    #[wasm_bindgen_test]
    fn wasm_markdown_bold_and_sanitized() {
        let h = to_safe_html("**x**");
        assert!(
            h.contains("<strong>") || h.contains("<b>"),
            "expected bold tag, got {h:?}"
        );
    }

    #[wasm_bindgen_test]
    fn wasm_markdown_table() {
        let h = to_safe_html("|c|\n|-|\n|v|");
        assert!(h.contains("<table"), "expected table, got {h:?}");
    }
}
