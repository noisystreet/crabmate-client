//! [`super`]（md.rs）的行内样式 / 折行 / 围栏单测。
//!
//! 独立成文件：与 settings_panel / render 同理，避免 lizard 对超大测试文件的
//! 函数体合并跨度超过 fn-nloc 门禁。

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
fn glued_mid_line_fence_splits_and_code_stays_plain() {
    // `依赖：```rust` 粘连围栏：normalize 拆成独立围栏行后，in_fence 状态机正确接管
    let text = "依赖：```rust\nx **y** `t`\n```";
    let chars = assistant_styled_text(text);
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
