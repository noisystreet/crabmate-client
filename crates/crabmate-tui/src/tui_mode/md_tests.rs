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
    let chars = assistant_styled_text(text, &pal(), 0);
    // 只看「加了修饰位」：代码行现在自带 code_bg 与正文字色，不能再按「有 fg 就算样式」判。
    let emphasised = |c: char| {
        chars
            .iter()
            .filter(|(_, ch)| *ch == c)
            .any(|(s, _)| !s.add_modifier.is_empty())
    };
    // 围栏外强调生效
    assert!(emphasised('A'));
    assert!(emphasised('B'));
    // 围栏内整行纯文本：星号/反引号原样保留、不带 markdown 修饰
    assert!(!emphasised('x'));
    assert!(!emphasised('`'));
    assert!(!emphasised('*'));
    let joined: String = chars.iter().map(|(_, ch)| *ch).collect();
    assert!(joined.contains("ls *.rs **x** `t`"), "got {joined:?}");
    // 围栏标记本身不上屏
    assert!(
        !joined.contains("```"),
        "代码围栏记号不该出现在正文里：{joined:?}"
    );
}

#[test]
fn code_fence_renders_a_language_label_instead_of_the_marker() {
    let rows = wrap_styled_chars(
        &assistant_styled_text("```rust\nfn main() {}\n```", &pal(), 0),
        200,
    );
    let text: Vec<String> = rows.iter().map(|r| row_text(r)).collect();
    assert_eq!(text, vec!["rust", "fn main() {} "]);
    // 收尾括号：闭合围栏不额外占一行
    assert_eq!(text.len(), 2, "闭合围栏不应产生第三行");
}

#[test]
fn code_lines_carry_the_code_background() {
    let palette = pal();
    let chars = assistant_styled_text("```\nlet x = 1;\n```", &palette, 0);
    let code_chars: Vec<&(Style, char)> = chars.iter().filter(|(_, ch)| *ch == 'l').collect();
    assert!(!code_chars.is_empty(), "代码内容应在字符流里");
    assert_eq!(
        code_chars[0].0.bg,
        Some(palette.code_bg),
        "代码行应铺 code_bg"
    );
    // 代码从最左列开始，不留内边距（那会看起来像代码没对齐色块左边界）
    let head = chars
        .iter()
        .find(|(_, ch)| *ch != '\n')
        .expect("应有可见内容");
    assert_eq!(head.1, 'l', "代码应贴着左边界开始");
    assert_eq!(head.0.bg, Some(palette.code_bg), "行首也是代码底");
}

#[test]
fn tabs_in_code_expand_to_four_columns() {
    let rows = wrap_styled_chars(&assistant_styled_text("```\n\ta\n```", &pal(), 0), 200);
    let code_row = row_text(&rows[0]); // 无语言：没有标签行
    // `▏` + 1 空格 + 4 空格(tab) + `a` + 末尾 1 空格
    assert_eq!(code_row, "    a ", "tab 应展开成 4 列空格");
}

#[test]
fn glued_mid_line_fence_splits_and_code_stays_plain() {
    // `依赖：```rust` 粘连围栏：normalize 拆成独立围栏行后，in_fence 状态机正确接管
    let text = "依赖：```rust\nx **y** `t`\n```";
    let chars = assistant_styled_text(text, &pal(), 0);
    let joined: String = chars.iter().map(|(_, ch)| *ch).collect();
    // 开围栏折成语言标签；代码行前缀竖条 + 末尾 padding；闭合行不出现
    assert_eq!(joined, "依赖：\nrust\nx **y** `t` ");
    let emphasised = |c: char| {
        chars
            .iter()
            .filter(|(_, ch)| *ch == c)
            .any(|(s, _)| !s.add_modifier.is_empty())
    };
    // 围栏内不带 markdown 修饰
    assert!(!emphasised('y'));
    assert!(!emphasised('*'));
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
    let rows = wrap_styled_chars(&assistant_styled_text("甲\n\n乙", &pal(), 0), 200);
    let text: Vec<String> = rows.iter().map(|r| row_text(r)).collect();
    assert_eq!(text, vec!["甲", "", "乙"], "段落之间应有一个空行");
}

/// 多个连续空行折叠为一个：CommonMark 里它们只算一个段落分隔，
/// desktop 的 `<p>` 之间也只有一个 margin。
#[test]
fn consecutive_blank_lines_collapse_to_one_gap() {
    let rows = wrap_styled_chars(&assistant_styled_text("甲\n\n\n\n乙", &pal(), 0), 200);
    let text: Vec<String> = rows.iter().map(|r| row_text(r)).collect();
    assert_eq!(text, vec!["甲", "", "乙"]);
}

/// 首尾空行不产生间距（同 CommonMark：文档首尾空行不算段落）。
#[test]
fn leading_and_trailing_blank_lines_add_no_gap() {
    let rows = wrap_styled_chars(&assistant_styled_text("\n\n甲\n\n", &pal(), 0), 200);
    let text: Vec<String> = rows.iter().map(|r| row_text(r)).collect();
    assert_eq!(text, vec!["甲"]);
}

/// 围栏代码块内的空行原样保留（desktop 的 `<pre>` 同样保留）。
#[test]
fn blank_line_inside_fence_is_preserved() {
    let rows = wrap_styled_chars(&assistant_styled_text("```\na\n\nb\n```", &pal(), 0), 200);
    let text: Vec<String> = rows.iter().map(|r| row_text(r)).collect();
    // 开围栏 → 语言标签行；空代码行只剩竖条 + 两侧 padding
    assert_eq!(text, vec!["a ", " ", "b "]);
}

/// 未闭合围栏（流式输出中途）的结尾空行也要保留：它属于代码内容，
/// 围栏闭合后仍在，提前裁掉会让空行「先消失、再出现」。
#[test]
fn trailing_blank_inside_unclosed_fence_is_kept() {
    let rows = wrap_styled_chars(&assistant_styled_text("```\na\n\n", &pal(), 0), 200);
    let text: Vec<String> = rows.iter().map(|r| row_text(r)).collect();
    // 尾部两个空 part 各落一行代码行：流式续写时它们原地变内容，
    // 不会「先不显示、下一帧突然多一行」。
    assert_eq!(text, vec!["a ", " ", " "]);
}

/// 代码块要铺成**等宽矩形**：标签行、每行代码都补到同一个目标宽度。
///
/// 之前只有「内容有多宽就铺多宽」，短行色块窄、长行色块宽，视觉上是若干横条
/// 而不是一个块。
#[test]
fn code_block_lines_share_one_width() {
    let text = "```rust\nfn main() {}\nlet x = 1;\n```";
    let rows = wrap_styled_chars(&assistant_styled_text(text, &pal(), 40), 200);
    let texts: Vec<String> = rows.iter().map(|r| row_text(r)).collect();
    assert!(texts.len() >= 3, "应有标签行 + 2 行代码，实际 {texts:?}");
    let widths: Vec<usize> = texts.iter().map(|t| t.chars().count()).collect();
    assert!(
        widths.iter().all(|&w| w == 40),
        "每行都应补到 40 列（等宽矩形），实际 {widths:?}"
    );
    // 内容没被裁掉
    assert!(texts[0].contains("rust"), "got {:?}", texts[0]);
    assert!(texts[1].contains("fn main() {}"), "got {:?}", texts[1]);
}

/// 超过目标宽度的行不裁也不补：交给折行，块在那种情况下自然变宽。
#[test]
fn overlong_code_line_is_not_truncated_to_the_block_width() {
    let text = "```\nxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\n```";
    let rows = wrap_styled_chars(&assistant_styled_text(text, &pal(), 20), 200);
    let code = row_text(&rows[0]); // 无语言：没有标签行，第一行就是代码
    assert!(
        code.contains(&"x".repeat(40)),
        "超宽代码行不该被裁到块宽：{code:?}"
    );
}
