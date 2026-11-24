//! 写盘工具卡详情里的 diff 渲染：```` ```diff ```` 段 → 着色行 + 折叠（#323）。
//!
//! 行模型与切段在 `crabmate-tool-card`（纯逻辑、可单测）；本模块只把段映射成 HTML，
//! **所有文本一律经 [`plaintext_to_safe_html`] 转义后再拼接**，不引入未转义片段。
//!
//! 作用域是**任意工具的详情**：围栏内容要解析出增删 / hunk 行才着色（写盘工具的围栏由
//! crate 生成）；`run_command` 输出里自带的 ```` ```diff ```` 一样会着色，解析不出变化则原样不动。

use crate::i18n::{self, Locale};
use crate::markdown::plaintext_to_safe_html;
use crabmate_tool_card::{
    DIFF_COLLAPSE_MAX_LINES, DetailSegment, DiffLine, DiffLineKind, collapse_diff_lines,
    split_detail_segments,
};

fn diff_line_class(kind: DiffLineKind) -> &'static str {
    match kind {
        DiffLineKind::Add => "chat-tui-diff-line chat-tui-diff-add",
        DiffLineKind::Del => "chat-tui-diff-line chat-tui-diff-del",
        DiffLineKind::Hunk => "chat-tui-diff-line chat-tui-diff-hunk",
        DiffLineKind::Context => "chat-tui-diff-line chat-tui-diff-ctx",
        DiffLineKind::Meta => "chat-tui-diff-line chat-tui-diff-meta",
    }
}

fn diff_lines_html(lines: &[DiffLine]) -> String {
    let mut html = String::new();
    for line in lines {
        html.push_str(&format!(
            "<span class=\"{}\">{}</span>",
            diff_line_class(line.kind),
            plaintext_to_safe_html(&line.text)
        ));
    }
    html
}

/// diff 块 HTML：前 [`DIFF_COLLAPSE_MAX_LINES`] 行直出，其余收进原生 `<details>`
/// （免 JS 事件绑定；窄屏只在这块内部滚动/展开，不解锁 IDE 布局）。
fn diff_block_html(lines: &[DiffLine], locale: Locale) -> String {
    let (head, hidden) = collapse_diff_lines(lines, DIFF_COLLAPSE_MAX_LINES);
    let mut html = String::from("<div class=\"chat-tui-diff\">");
    html.push_str(&diff_lines_html(head));
    if hidden > 0 {
        html.push_str("<details class=\"chat-tui-diff-more\">");
        html.push_str("<summary class=\"chat-tui-diff-more-label\">");
        html.push_str(&plaintext_to_safe_html(&i18n::msg_diff_more_lines(
            locale, hidden,
        )));
        html.push_str("</summary>");
        html.push_str(&diff_lines_html(&lines[head.len()..]));
        html.push_str("</details>");
    }
    html.push_str("</div>");
    html
}

/// 详情正文 inner HTML：普通文本段照旧整段转义，```` ```diff ```` 段渲染成着色行。
pub(crate) fn detail_body_inner_html(detail: &str, locale: Locale) -> String {
    let mut html = String::new();
    for seg in split_detail_segments(detail) {
        match seg {
            DetailSegment::Text(text) => html.push_str(&plaintext_to_safe_html(&text)),
            DetailSegment::Diff(lines) => html.push_str(&diff_block_html(&lines, locale)),
        }
    }
    html
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StoredMessage;

    fn tool_msg(name: &str, detail: &str) -> StoredMessage {
        StoredMessage {
            id: "t1".into(),
            role: "system".into(),
            text: "✅ 写文件 完成".into(),
            reasoning_text: detail.into(),
            image_urls: vec![],
            state: None,
            is_tool: true,
            tool_call_id: Some("tc1".into()),
            tool_name: Some(name.into()),
            created_at: 0,
        }
    }

    #[test]
    fn diff_fence_renders_per_line_classes() {
        let detail = "变更预览（unified diff）\n\n`a.rs`\n```diff\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-old\n+new\n```";
        let html = detail_body_inner_html(detail, Locale::ZhHans);
        assert!(html.contains("chat-tui-diff-add"), "{html}");
        assert!(html.contains("chat-tui-diff-del"), "{html}");
        assert!(html.contains("chat-tui-diff-hunk"), "{html}");
        assert!(html.contains("chat-tui-diff-meta"), "{html}");
        assert!(html.contains("+new"), "{html}");
        assert!(!html.contains("```diff"), "fence 不应外泄: {html}");
    }

    #[test]
    fn large_diff_collapses_behind_details() {
        let mut body = String::from("```diff\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n");
        for i in 0..80 {
            body.push_str(&format!("+line {i}\n"));
        }
        body.push_str("```");
        let html = detail_body_inner_html(&body, Locale::ZhHans);
        assert!(html.contains("chat-tui-diff-more"), "{html}");
        assert!(html.contains("展开其余"), "{html}");
        // 前 N 行仍直出，不必展开就能看到。
        assert!(html.contains("+line 0"), "{html}");
    }

    #[test]
    fn plain_detail_is_escaped_without_diff_markup() {
        let html = detail_body_inner_html("a < b && c", Locale::ZhHans);
        assert!(!html.contains("chat-tui-diff"), "{html}");
        assert!(html.contains("&lt;"), "{html}");
        assert!(!html.contains("<b"), "{html}");
    }

    #[test]
    fn diff_line_text_is_escaped() {
        let detail = "```diff\n--- a/x\n+++ b/x\n+if a < b && c {}\n```";
        let html = detail_body_inner_html(detail, Locale::ZhHans);
        assert!(
            html.contains("&lt;") && html.contains("&amp;&amp;"),
            "{html}"
        );
        assert!(!html.contains("< b"), "diff 行未转义: {html}");
    }

    #[test]
    fn tool_body_uses_div_container_for_detail() {
        let m = tool_msg("apply_patch", "```diff\n--- a/x\n+++ b/x\n-a\n+b\n```");
        let html = super::super::tui_tool_process::tool_process_body_html(
            &m,
            Locale::ZhHans,
            None,
            None,
            None,
        );
        assert!(
            html.contains("<div class=\"chat-tui-tool-detail-body\">"),
            "{html}"
        );
        assert!(
            !html.contains("<pre class=\"chat-tui-tool-detail-body\">"),
            "{html}"
        );
        assert!(html.contains("chat-tui-diff-add"), "{html}");
    }
}
