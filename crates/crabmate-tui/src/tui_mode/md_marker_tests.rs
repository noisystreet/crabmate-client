//! [`super`]（md.rs）的行首标记（标题/列表/引用）与共享 normalize 对齐单测。
//! 独立成文件理由同 md_tests.rs。

use super::*;

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
    // 与 desktop 共用同一 normalize 入口：`###规范`/`-规范`/`1.下一步` 补空格，`>正文` 原样
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
    // 渲染文本与共享 normalize 输出逐字一致（补的空格原样呈现、不吞字符）
    for input in ["###规范", "-规范", "1.下一步", ">正文"] {
        let chars = assistant_styled_text(input);
        let joined: String = chars.iter().map(|(_, ch)| *ch).collect();
        let expected = normalize_markdown_for_render(input);
        assert_eq!(joined, expected, "应与共享 normalize 输出一致: {input}");
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
