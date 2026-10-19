//! [`super`]（md.rs）的行首标记（标题/列表/引用）与共享 normalize 对齐单测。
//! 独立成文件理由同 md_tests.rs。

use super::*;

/// 测试用默认调色板（深色主题）。
fn pal() -> super::Palette {
    super::Palette::default()
}

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
fn heading_hides_marker_and_adds_bar() {
    let chars = assistant_styled_text("# 标题", &pal(), 0);
    assert!(!chars.iter().any(|(_, ch)| *ch == '#'), "标题标记不该上屏");
    assert!(chars.iter().any(|(_, ch)| *ch == '▌'), "H1 应加左侧竖条");
    assert!(has_bold(&chars, '标'));
    assert!(has_bold(&chars, '题'));
}

/// 层级靠缩进区分：H2 +2、H3 +4、H4+ 一律 +6。
#[test]
fn heading_levels_are_indented() {
    let render = |t: &str| -> String {
        assistant_styled_text(t, &pal(), 0)
            .iter()
            .map(|(_, c)| *c)
            .collect()
    };
    assert_eq!(render("## 二级"), "  二级");
    assert_eq!(render("### 三级"), "    三级");
    assert_eq!(render("###### 六级"), "      六级");
}

#[test]
fn list_and_quote_markers_render() {
    // 无序列表：`-` 换圆点，正文不加粗
    let ul = assistant_styled_text("- 项目", &pal(), 0);
    let ul_text: String = ul.iter().map(|(_, ch)| *ch).collect();
    assert_eq!(ul_text, "• 项目");
    assert_eq!(ul[0].0.fg, Some(pal().muted), "圆点用 muted");
    assert!(!has_bold(&ul, '项'));
    // 有序列表：编号保留（数字携带顺序信息）
    let ol = assistant_styled_text("1. 有序", &pal(), 0);
    assert!(has_gray(&ol, '1'));
    assert!(has_gray(&ol, '.'));
    // 引用：`>` 保留（多行引用靠它连成范围）
    let quote = assistant_styled_text("> 引用", &pal(), 0);
    assert!(has_gray(&quote, '>'));
    let joined: String = quote.iter().map(|(_, ch)| *ch).collect();
    assert_eq!(joined, "> 引用");
}

/// 嵌套层级靠符号区分：一级 `•`、缩进 2 及以上 `◦`。
#[test]
fn nested_list_bullets_change_with_indent() {
    let render = |t: &str| -> String {
        assistant_styled_text(t, &pal(), 0)
            .iter()
            .map(|(_, c)| *c)
            .collect()
    };
    assert_eq!(render("- a"), "• a");
    assert_eq!(render("  - b"), "  ◦ b");
    assert_eq!(render("   - c"), "   ◦ c");
}

/// 4 空格缩进是 CommonMark 的缩进代码块，不该被当成列表项（保持原样）。
#[test]
fn four_space_indent_is_not_a_list() {
    let text: String = assistant_styled_text("    - d", &pal(), 0)
        .iter()
        .map(|(_, c)| *c)
        .collect();
    assert_eq!(text, "    - d");
}

/// 任务列表与无序列表的标记都占 2 列，混排时正文起始列一致。
#[test]
fn task_and_bullet_markers_share_indent() {
    let bullet: String = assistant_styled_text("- 项", &pal(), 0)
        .iter()
        .map(|(_, c)| *c)
        .collect();
    let task: String = assistant_styled_text("- [ ] 项", &pal(), 0)
        .iter()
        .map(|(_, c)| *c)
        .collect();
    assert_eq!(bullet, "• 项");
    assert_eq!(task, "☐ 项");
}

#[test]
fn heading_keeps_inline_code_and_no_marker_loss() {
    let chars = assistant_styled_text("# 看 `x` 用", &pal(), 0);
    assert!(chars.iter().any(|(s, ch)| {
        *ch == 'x' && s.bg == Some(pal().code_bg) && s.add_modifier.contains(Modifier::BOLD)
    }));
    let joined: String = chars.iter().map(|(_, ch)| *ch).collect();
    assert_eq!(joined, "▌ 看 x 用");
}

#[test]
fn spaced_asterisks_mid_line_not_treated_as_list() {
    let chars = assistant_styled_text("a * b * c", &pal(), 0);
    let joined: String = chars.iter().map(|(_, ch)| *ch).collect();
    assert_eq!(joined, "a * b * c");
    assert!(!has_bold(&chars, 'b') && !chars.iter().any(|(s, _)| s.fg == Some(Color::Gray)));
}

#[test]
fn glued_markers_match_desktop_normalize_semantics() {
    // 与 desktop 共用同一 normalize 入口：`###规范`/`-规范`/`1.下一步` 补空格，`>正文` 原样
    let h = assistant_styled_text("###规范", &pal(), 0);
    let h_text: String = h.iter().map(|(_, ch)| *ch).collect();
    assert!(!h_text.contains('#'), "标题标记不该上屏：{h_text:?}");
    assert!(h_text.contains("规范"));
    assert!(has_bold(&h, '规'));
    // 粘连列表仍识别，但标记已换成圆点，不再与 normalize 输出逐字一致
    let ul = assistant_styled_text("-规范", &pal(), 0);
    let ul_text: String = ul.iter().map(|(_, ch)| *ch).collect();
    assert_eq!(ul_text, "• 规范");
    let ol = assistant_styled_text("1.下一步", &pal(), 0);
    assert!(has_gray(&ol, '1'));
    assert!(has_gray(&ol, '.'));
    let q = assistant_styled_text(">正文", &pal(), 0);
    assert!(has_gray(&q, '>'));
    // 渲染文本与共享 normalize 输出逐字一致（补的空格原样呈现、不吞字符）
    // 标题与无序列表的标记已被替换，不再逐字一致；有序列表与引用仍原样保留。
    for input in ["1.下一步", ">正文"] {
        let chars = assistant_styled_text(input, &pal(), 0);
        let joined: String = chars.iter().map(|(_, ch)| *ch).collect();
        let expected = normalize_markdown_for_render(input);
        assert_eq!(joined, expected, "应与共享 normalize 输出一致: {input}");
    }
}

#[test]
fn ascii_flags_and_asterisk_emphasis_stay_untouched() {
    // normalize 对 ASCII 粘连不补空格：`-rf` 不是列表
    let rf = assistant_styled_text("-rf", &pal(), 0);
    assert!(!rf.iter().any(|(s, _)| s.fg == Some(Color::Gray)));
    // 行首 `*强调*` 走斜体而非列表
    let em = assistant_styled_text("*强调*", &pal(), 0);
    assert!(
        em.iter()
            .any(|(s, ch)| *ch == '强' && s.add_modifier.contains(Modifier::ITALIC))
    );
    assert!(!em.iter().any(|(s, _)| s.fg == Some(Color::Gray)));
}

#[test]
fn heading_tab_separator_accepted() {
    let chars = assistant_styled_text("#\t标题", &pal(), 0);
    assert!(!chars.iter().any(|(_, ch)| *ch == '#'), "标记不该上屏");
    assert!(chars.iter().any(|(_, ch)| *ch == '▌'));
    assert!(has_bold(&chars, '标'));
}

/// 分隔线：`---` 变成一条铺满宽度的 `─`，短横线本身不上屏。
#[test]
fn rule_renders_as_full_width_line() {
    let chars = assistant_styled_text("---", &pal(), 20);
    let text: String = chars.iter().map(|(_, c)| *c).collect();
    assert_eq!(text, "─".repeat(20));
    assert!(!chars.iter().any(|(_, c)| *c == '-'));
    assert_eq!(chars[0].0.fg, Some(pal().muted), "分隔线用 muted");
}

/// 分隔线在宽度未知（纯文本场景）时退化成最短可见长度，不是零长。
#[test]
fn rule_has_minimum_length_without_width() {
    let chars = assistant_styled_text("***", &pal(), 0);
    assert_eq!(chars.iter().filter(|(_, c)| *c == '─').count(), RULE_MIN);
}

/// 任务列表：`[ ]` / `[x]` 换成方框，勾选态用 accent。
#[test]
fn task_list_uses_checkboxes() {
    let todo: String = assistant_styled_text("- [ ] 待办", &pal(), 0)
        .iter()
        .map(|(_, c)| *c)
        .collect();
    assert_eq!(todo, "☐ 待办");
    let done = assistant_styled_text("- [x] 完成", &pal(), 0);
    let done_text: String = done.iter().map(|(_, c)| *c).collect();
    assert_eq!(done_text, "☑ 完成");
    assert_eq!(done[0].0.fg, Some(pal().accent), "勾选项用 accent");
}

/// 围栏里的 `---` 与 `- [ ]` 是代码内容，不能被当成结构标记。
#[test]
fn rule_and_task_inside_fence_stay_literal() {
    let chars = assistant_styled_text("```\n---\n- [ ] x\n```", &pal(), 10);
    let text: String = chars.iter().map(|(_, c)| *c).collect();
    assert!(text.contains("---"), "围栏内的 --- 应保持字面：{text:?}");
    assert!(text.contains("[ ]"), "围栏内的任务标记应保持字面：{text:?}");
}
