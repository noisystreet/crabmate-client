//! [`super`]（md.rs）的行内样式 / 折行 / 围栏单测。
//!
//! 独立成文件：与 settings_panel / render 同理，避免 lizard 对超大测试文件的
//! 函数体合并跨度超过 fn-nloc 门禁。

use ratatui::style::Color;

use super::md_links::link_style;
use super::*;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// 测试用固定深色调色板（不随默认主题与终端能力漂移）。
fn pal() -> super::Palette {
    super::Palette::from_theme(Some("shadcn-dark"))
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

/// 超过目标宽度的行不截断：内容换成多行，且**每行都补齐到块宽**
/// （否则最后一段右侧短一截，`code_bg` 矩形的右边界会缺角）。
#[test]
fn overlong_code_line_wraps_into_full_width_rows() {
    let text = "```\nxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\n```"; // 35 个 x
    let rows = wrap_styled_chars(&assistant_styled_text(text, &pal(), 20), 200);
    let texts: Vec<String> = rows.iter().map(|r| row_text(r)).collect();
    let total: usize = texts.iter().map(|t| t.matches('x').count()).sum();
    assert_eq!(total, 35, "内容不该被裁：{texts:?}");
    assert!(texts.len() >= 2, "35 列应折成 2 行以上：{texts:?}");
    for t in &texts {
        assert_eq!(t.width(), 20, "每行都该补齐到块宽：{t:?}");
    }
}

// ---------- 表格（极简无竖线：表头加粗 + `─` 分隔 + 列间两空格） ----------

fn table_rows_of(text: &str) -> Vec<String> {
    wrap_styled_chars(&assistant_styled_text(text, &pal(), 0), 200)
        .iter()
        .map(|r| row_text(r))
        .collect()
}

/// 基本形态：表头加粗、一条 `─` 分隔、列按内容对齐、**不出现 `|`**。
#[test]
fn table_renders_without_pipes() {
    let text = "| 名称 | 值 |\n| --- | --- |\n| a | 1 |";
    let rows = table_rows_of(text);
    assert_eq!(rows, vec!["名称  值", "────  ──", "a     1 "]);
    // 竖线一律不上屏
    assert!(
        rows.iter().all(|r| !r.contains('|')),
        "表格不该显示 `|`：{rows:?}"
    );
}

/// 表头单元格带加粗修饰（不是靠颜色）。
#[test]
fn table_header_is_bold() {
    let chars = assistant_styled_text("| H |\n| - |\n| v |", &pal(), 0);
    let h = chars.iter().find(|(_, ch)| *ch == 'H').expect("表头应存在");
    assert!(
        h.0.add_modifier.contains(ratatui::style::Modifier::BOLD),
        "表头单元格应加粗"
    );
    let v = chars.iter().find(|(_, ch)| *ch == 'v').expect("数据应存在");
    assert!(
        !v.0.add_modifier.contains(ratatui::style::Modifier::BOLD),
        "数据单元格不该加粗"
    );
}

/// 分隔行里的冒号决定对齐：`:--:` 居中、`--:` 右对齐。
#[test]
fn table_alignment_follows_align_row() {
    let text = "| l | c | r |\n| :-- | :-: | --: |\n| x | y | z |";
    let rows = table_rows_of(text);
    // 三列内容宽都是 1（列宽取表头 1），对齐在单宽列上表现不出差异；
    // 这里验证分隔行本身按列宽铺满、列间两空格。
    assert_eq!(rows[1], "─  ─  ─");
    assert_eq!(rows.len(), 3);
}

/// CJK 单元格按**显示宽度**对齐（中文占 2 列），不是按字符数。
#[test]
fn table_column_width_uses_display_width() {
    let text = "| 名字 | v |\n| --- | --- |\n| 张三 | 1 |\n| ab | 22 |";
    let rows = table_rows_of(text);
    // 第一列内容宽 = 4（两个汉字），第二列 = 2
    assert_eq!(rows[2], "张三  1 ");
    assert_eq!(rows[3], "ab    22");
    // 两条数据行等宽
    assert_eq!(rows[2].width(), rows[3].width());
}

// ---------- 表格单元格里的链接（issue #428 第 2 条） ----------

/// 取一行里 `[start, end)` 列区间覆盖到的可见文字。
fn covered_text(row: &WrappedRow, start: u16, end: u16) -> String {
    let mut out = String::new();
    let mut col = 0usize;
    for (_, ch) in &row.chars {
        let cw = ch.width().unwrap_or(0);
        if col >= usize::from(start) && col < usize::from(end) {
            out.push(*ch);
        }
        col += cw;
    }
    out
}

/// 找到携带 `url` 热区的那一行，连同它的列区间。
fn hotspot<'a>(rows: &'a [WrappedRow], url: &str) -> Option<(&'a WrappedRow, u16, u16)> {
    rows.iter().find_map(|row| {
        row.links
            .iter()
            .find(|(_, _, u)| u == url)
            .map(|(start, end, _)| (row, *start, *end))
    })
}

/// 单元格里的链接要留下热区，且热区正好压在链接文字那几列上。
#[test]
fn table_cell_link_gets_a_hotspot_on_its_columns() {
    let text = "| [指南](https://example.com/a) | v |\n| --- | --- |\n| a | 1 |";
    let rows = wrap_styled_rows(&assistant_styled(text, &pal(), 0), 200);
    let (row, start, end) =
        hotspot(&rows, "https://example.com/a").expect("表格单元格里的链接应登记热区");
    assert_eq!(covered_text(row, start, end), "指南");
}

/// 单元格被列宽压到必须截断时，热区要跟着收口到 `…` 之前 ——
/// 留一段越过 `…` 的热区，点下去打开的就是屏幕上没画出来的那截链接。
#[test]
fn truncated_table_cell_clips_the_link() {
    let text = "| [非常长的链接文字](https://example.com/a) |\n| --- |\n| x |";
    let rows = wrap_styled_rows(&assistant_styled(text, &pal(), 8), 200);
    let (row, start, end) = hotspot(&rows, "https://example.com/a")
        .expect("被截断的单元格前几个字仍在屏上，热区应收口而不是整条消失");
    let shown = row_text(&row.chars);
    // 结构性断言（不绑死具体几个字）：热区右端 == 可见文字去掉结尾 `…` 之后的显示宽度。
    let visible = shown.strip_suffix('…').unwrap_or(&shown);
    assert_eq!(usize::from(start), 0, "热区应从可见文字的开头起：{shown:?}");
    assert_eq!(
        usize::from(end),
        visible.width(),
        "热区应止于 `…` 之前：{shown:?}"
    );
    assert!(!covered_text(row, start, end).is_empty());
}

/// 右对齐单元格的链接区间要跟着左填充一起右移，否则热区整体偏左
/// （把 `pad_cell` 里的 `absorb` 写回 `extend` 就会踩到）。
#[test]
fn right_aligned_cell_link_shifts_with_the_padding() {
    // 表头比内容宽，右对齐才真的产生左填充。
    let text = "| 名称 | 链接列 |\n| --- | ---: |\n| a | [文档](https://example.com/b) |";
    let rows = wrap_styled_rows(&assistant_styled(text, &pal(), 0), 200);
    let (row, start, end) =
        hotspot(&rows, "https://example.com/b").expect("右对齐单元格里的链接应登记热区");
    assert_eq!(covered_text(row, start, end), "文档");
}

/// 只有表头 + 分隔行也算表（流式输出到一半时就该以表格形态出现）。
#[test]
fn table_without_body_rows_still_renders() {
    let rows = table_rows_of("| a | b |\n| --- | --- |");
    assert_eq!(rows, vec!["a  b", "─  ─"]);
}

/// 没有分隔行就不是表格 —— 普通正文里的竖线保持原样。
#[test]
fn pipe_text_without_align_row_is_not_a_table() {
    let rows = table_rows_of("a | b");
    assert_eq!(rows, vec!["a | b"]);
}

/// 围栏里的 `|` 是代码内容，不能被当成表格。
#[test]
fn pipes_inside_fence_are_not_a_table() {
    let text = "```\n| a | b |\n| --- | --- |\n```";
    let rows = table_rows_of(text);
    assert_eq!(rows, vec!["| a | b | ", "| --- | --- | "]);
}

/// 超宽时压缩并截断单元格（以 `…` 收尾），不会把表格撑出屏幕宽度。
#[test]
fn table_shrinks_and_truncates_when_too_wide() {
    let text = "| 很长的列名 | 另一个长列名 |\n| --- | --- |\n| 值 | 值 |";
    let rows = wrap_styled_chars(&assistant_styled_text(text, &pal(), 16), 200)
        .iter()
        .map(|r| row_text(r))
        .collect::<Vec<_>>();
    assert!(
        rows.iter().any(|r| r.contains('…')),
        "超宽表格应截断并加省略号：{rows:?}"
    );
    for r in &rows {
        assert!(
            r.width() <= 16,
            "表格不应超出预算宽度：{r:?} 宽 {}",
            r.width()
        );
    }
}

/// 单元格里的行内 markdown 仍然生效（`**粗**` 去掉标记且加粗）。
#[test]
fn table_cell_supports_inline_markdown() {
    let rows = table_rows_of("| 状态 |\n| --- |\n| **ok** |");
    // 列宽取表头（4 列），`ok` 补到 4 列；`**` 标记不上屏
    assert_eq!(rows[2], "ok  ");
    let chars = assistant_styled_text("| 状态 |\n| --- |\n| **ok** |", &pal(), 0);
    let o = chars.iter().find(|(_, ch)| *ch == 'o').expect("应有省略");
    let _ = o;
}

// ---------- 行内图片（降级成 `[图] alt`） ----------

fn render_inline(text: &str) -> String {
    assistant_styled_text(text, &pal(), 0)
        .iter()
        .map(|(_, c)| *c)
        .collect()
}

/// 基本形态：`![alt](url)` → `[图] alt`，URL 不上屏，感叹号不残留。
#[test]
fn image_degrades_to_alt_text() {
    assert_eq!(render_inline("![架构图](https://x/a.png)"), "[图] 架构图");
    // 没有 alt 就只留标记
    assert_eq!(render_inline("![](https://x/a.png)"), "[图]");
    // 夹在正文里
    assert_eq!(render_inline("见图 ![x](u) 结束"), "见图 [图] x 结束");
}

/// `[图]` 用 accent —— 让人一眼看出「这里原本是张图」，而不是一段普通文字。
#[test]
fn image_tag_is_accented() {
    let chars = assistant_styled_text("![a](u)", &pal(), 0);
    let open = chars.iter().find(|(_, c)| *c == '[').expect("应有 [图]");
    assert_eq!(open.0.fg, Some(pal().accent), "[图] 用 accent");
    // alt 本身不是 accent（它是正文）
    let alt = chars.iter().find(|(_, c)| *c == 'a').expect("应有 alt");
    assert_ne!(alt.0.fg, Some(pal().accent));
}

/// alt 里的行内 markdown 仍然生效。
#[test]
fn image_alt_keeps_inline_markdown() {
    let chars = assistant_styled_text("![**粗**](u)", &pal(), 0);
    let text: String = chars.iter().map(|(_, c)| *c).collect();
    assert_eq!(text, "[图] 粗", "`**` 标记不该上屏");
    let c = chars.iter().find(|(_, ch)| *ch == '粗').expect("应有 alt");
    assert!(c.0.add_modifier.contains(Modifier::BOLD));
}

/// 语法不完整（缺 `)` 或缺 `](`）时不算图片，原样保留 —— 不吞掉残句。
#[test]
fn incomplete_image_syntax_is_left_alone() {
    assert_eq!(render_inline("![x](u"), "![x](u");
    assert_eq!(render_inline("![x] 没有 url"), "![x] 没有 url");
    assert_eq!(render_inline("a ! b"), "a ! b");
}

/// 普通链接不受影响，仍是 accent + 下划线。
#[test]
fn plain_link_still_works() {
    let chars = assistant_styled_text("[文档](https://x)", &pal(), 0);
    let text: String = chars.iter().map(|(_, c)| *c).collect();
    assert_eq!(text, "文档");
    let d = chars
        .iter()
        .find(|(_, c)| *c == '文')
        .expect("应有链接文字");
    assert_eq!(d.0.fg, Some(pal().accent));
    assert!(d.0.add_modifier.contains(Modifier::UNDERLINED));
}

/// 围栏里的 `![x](u)` 是代码内容，保持字面。
#[test]
fn image_inside_fence_stays_literal() {
    let chars = assistant_styled_text("```\n![x](u)\n```", &pal(), 10);
    let text: String = chars.iter().map(|(_, c)| *c).collect();
    assert!(text.contains("![x](u)"), "围栏内应保持字面：{text:?}");
}

/// 表格单元格里的图片同样降级（单元格走的是同一个行内渲染入口）。
#[test]
fn image_inside_table_cell_degrades() {
    let rows = wrap_styled_chars(
        &assistant_styled_text("| 图 |\n| --- |\n| ![a](u) |", &pal(), 0),
        200,
    );
    let text: String = rows[2].iter().map(|(_, c)| *c).collect();
    assert_eq!(text, "[图] a");
}

// ---------- 折行：续行的悬挂缩进 / 竖条延续 ----------

fn wrapped_rows(text: &str, width: usize) -> Vec<String> {
    wrap_styled_chars(&assistant_styled_text(text, &pal(), width), width)
        .iter()
        .map(|r| row_text(r))
        .collect()
}

/// 引用折行后，**每一行**都以竖条开头 —— 一整个引用块在屏幕上是连续的竖线。
#[test]
fn wrapped_quote_keeps_bar_on_every_row() {
    let rows = wrapped_rows("> 这是一段很长很长很长很长很长的引用内容", 20);
    assert!(rows.len() >= 2, "应折成多行：{rows:?}");
    for r in &rows {
        assert!(r.starts_with("▎ "), "续行也该带竖条：{r:?}");
        assert!(r.width() <= 20, "不该超宽：{r:?}");
    }
}

/// 嵌套引用的续行带上**同样数量的**竖条。
#[test]
fn wrapped_nested_quote_keeps_all_bars() {
    let rows = wrapped_rows("> > 一二三四五六七八九十一二三四五六七八九十", 20);
    assert!(rows.len() >= 2, "应折成多行：{rows:?}");
    for r in &rows {
        assert!(r.starts_with("▎ ▎ "), "续行应保留两级竖条：{r:?}");
    }
}

/// 无序列表折行后，续行缩进到**正文起始列**（悬挂缩进），而不是顶到第 0 列。
#[test]
fn wrapped_list_item_hangs_under_content() {
    let rows = wrapped_rows("- 一二三四五六七八九十一二三四五六七八九十", 20);
    assert!(rows.len() >= 2, "应折成多行：{rows:?}");
    assert!(rows[0].starts_with("• "), "首行是圆点：{rows:?}");
    for r in &rows[1..] {
        assert!(r.starts_with("  "), "续行应悬挂缩进 2 列：{r:?}");
        assert!(!r.starts_with("•"), "续行不该重复圆点：{r:?}");
    }
}

/// 任务列表同理。
#[test]
fn wrapped_task_item_hangs_under_content() {
    let rows = wrapped_rows("- [x] 一二三四五六七八九十一二三四五六七八九十", 20);
    assert!(rows.len() >= 2, "应折成多行：{rows:?}");
    assert!(rows[0].starts_with("☑ "), "首行是勾选框：{rows:?}");
    for r in &rows[1..] {
        assert!(r.starts_with("  "), "续行应悬挂缩进：{r:?}");
    }
}

/// 标题折行后续行与标题正文对齐（H1 的 `▌ ` 占 2 列）。
#[test]
fn wrapped_heading_aligns_with_its_own_body() {
    let rows = wrapped_rows("# 一二三四五六七八九十一二三四五六七八九十", 20);
    assert!(rows.len() >= 2, "应折成多行：{rows:?}");
    assert!(rows[0].starts_with("▌ "), "H1 首行有竖条：{rows:?}");
    for r in &rows[1..] {
        assert!(r.starts_with("  "), "续行应缩进 2 列：{r:?}");
    }
}

/// 有序列表折行后续行对齐到编号之后（`1. ` 占 3 列）。
#[test]
fn wrapped_ordered_item_hangs_after_number() {
    let rows = wrapped_rows("1. 一二三四五六七八九十一二三四五六七八九十", 20);
    assert!(rows.len() >= 2, "应折成多行：{rows:?}");
    for r in &rows[1..] {
        assert!(r.starts_with("   "), "续行应缩进 3 列：{r:?}");
        assert!(!r.starts_with("1."), "续行不该重复编号：{r:?}");
    }
}

/// 把「显示列区间」映射回字符，用来验证热区真的盖在链接文字上。
fn col_slice(chars: &[(Style, char)], start: u16, end: u16) -> String {
    let mut out = String::new();
    let mut col = 0u16;
    for &(_, ch) in chars {
        let cw = u16::try_from(ch.to_string().width()).unwrap_or(0);
        if col >= start && col + cw <= end {
            out.push(ch);
        }
        col += cw;
    }
    out
}

/// 链接的屏幕列与折行同源：报出来的区间正盖在那几个字符上。
#[test]
fn link_columns_land_on_the_link_characters() {
    let styled = assistant_styled("[文档](https://x) 后面", &pal(), 0);
    let rows = wrap_styled_rows(&styled, 40);
    assert_eq!(rows.len(), 1);
    let (start, end, url) = rows[0].links[0].clone();
    assert_eq!(url, "https://x");
    // 展示文本只有「文档」（两个汉字 = 4 列），URL 不上屏
    assert_eq!((start, end), (0, 4));
    assert_eq!(col_slice(&rows[0].chars, start, end), "文档");
}

/// 链接被折行切开：每行各得一段，两段指向同一个 URL —— 屏幕上分成几块，
/// 每一块都得能点。
#[test]
fn wrapped_link_reports_one_range_per_row() {
    let styled = assistant_styled("[abcd](https://x)", &pal(), 0);
    let rows = wrap_styled_rows(&styled, 2);
    assert_eq!(rows.len(), 2, "宽 2 应折成 ab / cd");
    for row in &rows {
        assert_eq!(row.links, vec![(0, 2, "https://x".to_string())]);
    }
}

/// 两条链接紧贴（中间没有间隔字符）：必须分成两段，否则点右边那条会打开左边那条。
#[test]
fn adjacent_links_do_not_merge() {
    let styled = assistant_styled("[a](https://one)[b](https://two)", &pal(), 0);
    let rows = wrap_styled_rows(&styled, 40);
    let links = &rows[0].links;
    assert_eq!(links.len(), 2, "两段不能并成一段");
    assert_eq!((links[0].0, links[0].1), (0, 1));
    assert_eq!(links[0].2, "https://one");
    assert_eq!((links[1].0, links[1].1), (1, 2));
    assert_eq!(links[1].2, "https://two");
}

/// 没有链接的行不该有任何热区（否则点空白也会拉起浏览器）。
#[test]
fn plain_text_rows_have_no_links() {
    let styled = assistant_styled("没有链接的正文", &pal(), 0);
    let rows = wrap_styled_rows(&styled, 40);
    assert!(rows.iter().all(|r| r.links.is_empty()));
}

/// 列表项正文里的链接也要能点：`wrap_body` 插续行前缀时区间必须跟着右移。
#[test]
fn list_item_link_survives_the_hanging_prefix() {
    let styled = assistant_styled("- [文档](https://x)", &pal(), 0);
    let rows = wrap_styled_rows(&styled, 40);
    let links = &rows[0].links;
    assert_eq!(links.len(), 1, "列表项的链接不能丢");
    let (start, end, url) = links[0].clone();
    assert_eq!(url, "https://x");
    // `• ` 占 2 列，所以「文档」从第 2 列开始
    assert_eq!((start, end), (2, 6));
    assert_eq!(col_slice(&rows[0].chars, start, end), "文档");
}

// ---- 裸 URL 也按 `[url](url)` 渲染 ----
//
// 背景：共享 parser 只认 `[文字](url)`，而模型最常直接吐裸地址。那些字在屏幕上由
// 终端自己画了下划线，但本程序开着鼠标上报（`shell.rs` 的 `MOUSE_ON`），点击被送进
// 本进程、终端自己的链接处理器收不到 —— 于是「点下去毫无反应」。这一组用例钉住
// 「裸 URL 有热区」以及几条不该被改写的边界。

/// 裸 URL 变成链接，展示文本就是地址本身（屏幕上与改写前逐字一致）。
#[test]
fn bare_url_becomes_a_link_showing_the_address() {
    let styled = assistant_styled("见 https://example.com/a?b=1 末尾", &pal(), 0);
    let rows = wrap_styled_rows(&styled, 60);
    assert_eq!(rows.len(), 1);
    let (start, end, url) = rows[0].links[0].clone();
    assert_eq!(url, "https://example.com/a?b=1");
    assert_eq!(
        col_slice(&rows[0].chars, start, end),
        "https://example.com/a?b=1",
        "热区必须正好盖在地址本身上"
    );
}

/// 句中裸 URL 前后的文字仍是普通文本（不能把整行都变成链接）。
#[test]
fn bare_url_leaves_surrounding_text_plain() {
    let styled = assistant_styled("见 https://x.com 末尾", &pal(), 0);
    let rows = wrap_styled_rows(&styled, 60);
    assert_eq!(rows[0].links.len(), 1);
    let link = link_style(&pal());
    let underlined: String = rows[0]
        .chars
        .iter()
        .filter(|(s, _)| s.add_modifier.contains(Modifier::UNDERLINED))
        .map(|(_, ch)| *ch)
        .collect();
    assert_eq!(underlined, "https://x.com", "只有地址带链接样式");
    assert!(link.add_modifier.contains(Modifier::UNDERLINED));
}

/// 尾部标点属于句子，不属于地址：全角逗号、半角句号、外层的右括号都要剥掉。
#[test]
fn bare_url_drops_trailing_punctuation() {
    for (text, want) in [
        ("见 https://x.com，然后呢", "https://x.com"),
        ("见 https://x.com。", "https://x.com"),
        ("(见 https://x.com)", "https://x.com"),
        ("见 https://x.com.", "https://x.com"),
    ] {
        let styled = assistant_styled(text, &pal(), 0);
        let rows = wrap_styled_rows(&styled, 60);
        assert_eq!(rows[0].links[0].2, want, "输入：{text}");
    }
}

/// 地址内部成对的括号要保留（维基式条目名），不能一见到 `)` 就截断。
#[test]
fn bare_url_keeps_a_balanced_parenthesis() {
    let styled = assistant_styled("见 https://x.com/Rust_(lang) 末尾", &pal(), 0);
    let rows = wrap_styled_rows(&styled, 60);
    assert_eq!(rows[0].links[0].2, "https://x.com/Rust_(lang)");
}

/// 行内代码里的地址是人写给人看的字面量，不该变成可点目标。
#[test]
fn url_inside_inline_code_stays_plain() {
    let styled = assistant_styled("用 `https://x.com` 就够了", &pal(), 0);
    let rows = wrap_styled_rows(&styled, 60);
    assert!(rows.iter().all(|r| r.links.is_empty()));
}

/// 代码块同理：`code_line_chars` 不过行内解析。
#[test]
fn url_inside_code_fence_stays_plain() {
    let styled = assistant_styled("```\ncurl https://x.com\n```", &pal(), 40);
    let rows = wrap_styled_rows(&styled, 60);
    assert!(rows.iter().all(|r| r.links.is_empty()));
}

/// 已经是 `[url](url)` 的不能再被裸 URL 扫描二次改写：仍是一条，指向自己。
#[test]
fn markdown_link_whose_text_is_a_url_is_not_rewritten() {
    let styled = assistant_styled("[https://x.com](https://x.com)", &pal(), 0);
    let rows = wrap_styled_rows(&styled, 60);
    assert_eq!(rows[0].links.len(), 1, "重复登记会变成两个叠在一起的热区");
    assert_eq!(rows[0].links[0].2, "https://x.com");
}

/// 只有 scheme 没有正文（`https://` 后面直接断开）按普通文本处理：
/// 打开一个空地址没有意义。
#[test]
fn bare_scheme_without_a_body_is_not_a_link() {
    let styled = assistant_styled("格式是 https:// 开头", &pal(), 0);
    let rows = wrap_styled_rows(&styled, 60);
    assert!(rows.iter().all(|r| r.links.is_empty()));
}
