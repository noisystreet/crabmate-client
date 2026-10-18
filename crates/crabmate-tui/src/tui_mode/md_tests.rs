//! [`super`]（md.rs）的行内样式 / 折行 / 围栏单测。
//!
//! 独立成文件：与 settings_panel / render 同理，避免 lizard 对超大测试文件的
//! 函数体合并跨度超过 fn-nloc 门禁。

use super::*;

/// 测试用默认调色板（深色主题）。
fn pal() -> super::Palette {
    super::Palette::default()
}

#[test]
fn bold_code_and_link_get_expected_styles() {
    let chars = inline_styled_chars("a **b** `c` [d](https://e)", &pal());
    assert!(
        chars
            .iter()
            .any(|(s, ch)| { s.add_modifier.contains(Modifier::BOLD) && *ch == 'b' })
    );
    assert!(
        chars
            .iter()
            .any(|(s, ch)| s.bg == Some(pal().code_bg) && *ch == 'c')
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
    let chars = inline_styled_chars("尾 **未闭合", &pal());
    assert!(
        chars
            .iter()
            .all(|(s, _)| !s.add_modifier.contains(Modifier::BOLD))
    );
    let rows = wrap_styled_chars(&chars, 200);
    let spans = styled_row_spans(&rows[0], Style::new(), false, false, &pal());
    let joined: String = spans.iter().map(|s| s.content.as_ref()).collect();
    assert!(joined.contains("**未闭合"));
}

#[test]
fn matched_highlight_keeps_bold_but_turns_accent() {
    let chars = inline_styled_chars("a **b** c", &pal());
    let rows = wrap_styled_chars(&chars, 200);
    let spans = styled_row_spans(
        &rows[0],
        Style::new().fg(Color::LightGreen),
        true,
        false,
        &pal(),
    );
    assert!(spans.iter().any(|s| s.style.fg == Some(pal().hit)
        && s.style.add_modifier.contains(Modifier::BOLD)
        && s.to_string().contains('b')));
}

#[test]
fn anchor_row_overrides_whole_row() {
    let chars = inline_styled_chars("**b**", &pal());
    let rows = wrap_styled_chars(&chars, 200);
    let spans = styled_row_spans(&rows[0], Style::new(), false, true, &pal());
    assert!(
        spans
            .iter()
            .all(|s| s.style.bg == Some(pal().hit) && s.style.fg == Some(pal().bg))
    );
}

#[test]
fn code_fence_content_is_plain_and_literal() {
    let text = "前 **A**\n```\nls *.rs **x** `t`\n```\n后 **B**";
    let chars = assistant_styled_text(text, &pal());
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
fn glued_mid_line_fence_splits_and_code_stays_plain() {
    // `依赖：```rust` 粘连围栏：normalize 拆成独立围栏行后，in_fence 状态机正确接管
    let text = "依赖：```rust\nx **y** `t`\n```";
    let chars = assistant_styled_text(text, &pal());
    let joined: String = chars.iter().map(|(_, ch)| *ch).collect();
    assert_eq!(joined, "依赖：\n```rust\nx **y** `t`\n```");
    let styled = |c: char| {
        chars
            .iter()
            .filter(|(_, ch)| *ch == c)
            .any(|(s, _)| !s.add_modifier.is_empty() || s.fg.is_some())
    };
    // 围栏内整行纯文本：星号/反引号原样保留、无样式
    assert!(!styled('y'));
    assert!(!styled('`'));
    assert!(!styled('*'));
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

/// 物理行 → 纯文本（断言空行 / 段落间距用）。
fn row_text(row: &[(Style, char)]) -> String {
    row.iter().map(|(_, ch)| *ch).collect()
}

/// 段落间距：围栏外的空行落成一行空白（对齐 desktop 的 `.msg-md-prose p` 间距）。
#[test]
fn paragraph_gap_becomes_blank_row() {
    let rows = wrap_styled_chars(&assistant_styled_text("甲\n\n乙", &pal()), 200);
    let text: Vec<String> = rows.iter().map(|r| row_text(r)).collect();
    assert_eq!(text, vec!["甲", "", "乙"], "段落之间应有一个空行");
}

/// 多个连续空行折叠为一个：CommonMark 里它们只算一个段落分隔，
/// desktop 的 `<p>` 之间也只有一个 margin。
#[test]
fn consecutive_blank_lines_collapse_to_one_gap() {
    let rows = wrap_styled_chars(&assistant_styled_text("甲\n\n\n\n乙", &pal()), 200);
    let text: Vec<String> = rows.iter().map(|r| row_text(r)).collect();
    assert_eq!(text, vec!["甲", "", "乙"]);
}

/// 首尾空行不产生间距（同 CommonMark：文档首尾空行不算段落）。
#[test]
fn leading_and_trailing_blank_lines_add_no_gap() {
    let rows = wrap_styled_chars(&assistant_styled_text("\n\n甲\n\n", &pal()), 200);
    let text: Vec<String> = rows.iter().map(|r| row_text(r)).collect();
    assert_eq!(text, vec!["甲"]);
}

/// 围栏代码块内的空行原样保留（desktop 的 `<pre>` 同样保留）。
#[test]
fn blank_line_inside_fence_is_preserved() {
    let rows = wrap_styled_chars(&assistant_styled_text("```\na\n\nb\n```", &pal()), 200);
    let text: Vec<String> = rows.iter().map(|r| row_text(r)).collect();
    assert_eq!(text, vec!["```", "a", "", "b", "```"]);
}

/// 未闭合围栏（流式输出中途）的结尾空行也要保留：它属于代码内容，
/// 围栏闭合后仍在，提前裁掉会让空行「先消失、再出现」。
#[test]
fn trailing_blank_inside_unclosed_fence_is_kept() {
    let rows = wrap_styled_chars(&assistant_styled_text("```\na\n\n", &pal()), 200);
    let text: Vec<String> = rows.iter().map(|r| row_text(r)).collect();
    assert_eq!(text, vec!["```", "a", ""]);
}
