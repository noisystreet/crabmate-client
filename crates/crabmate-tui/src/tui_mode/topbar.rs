//! 顶栏：左品牌 + 居中工作区名。
//!
//! 从 `render.rs` 拆出（该文件贴着 920 行门禁）：这里是一组纯函数，输入
//! `UiState` + 宽度，输出单行，便于单测。
//!
//! **为什么不是「单串 + `Alignment::Center`」**：左边一旦有品牌段，整行居中就会让
//! 中段跟着品牌宽度漂。现在品牌先占位、工作区名在剩余列里自己补空格居中，位置
//! 可预测，且**总宽恒等于传入宽度**（`Paragraph` 不折行、不补宽，行尾全靠这里算准）。
//!
//! **模式与键位提示不在这里**：顶栏只回答「我在哪个工作区」。模式（`ask` / `plan` /
//! `act`）由状态栏的 `mode` 段承担——那里带优先级淘汰，窄终端会自己收窄；键位提示
//! 则不值得常驻一行，只留在顶栏会让这行从「位置指示」变成「说明书」。

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use super::{UiState, truncate_display, workspace_basename};

/// 左段品牌（`muted` + `BOLD`）：比中段压一档，不抢工作区名。
const BRAND: &str = "CrabMate";
/// 中段至少要有这么多列，才值得为左段让位；否则退回「只有一个居中的工作区名」。
const MIN_NAME_W: usize = 4;

/// 追加一段空白（宽 0 时不产生空 span，别让 `Line` 里堆无意义片段）。
fn push_pad(spans: &mut Vec<Span<'static>>, n: usize) {
    if n > 0 {
        spans.push(Span::raw(" ".repeat(n)));
    }
}

/// 顶栏一行。
///
/// 两档降级：放得下品牌就画品牌，放不下就整段让位、只剩居中的工作区名。
/// 工作区名在任何宽度下都保留：它是顶栏唯一不可省的项。
pub(super) fn top_line(st: &UiState, width: usize) -> Line<'static> {
    let pal = &st.palette;
    let name = workspace_name(st);
    let name_w = UnicodeWidthStr::width(name.as_str());
    let name_style = Style::new().fg(pal.fg).add_modifier(Modifier::BOLD);

    // 品牌宽度含尾部 1 列，那一列是与中段的固定间隔。
    let brand_w = UnicodeWidthStr::width(BRAND) + 1;
    // 名字本身不到 4 列时按名字算：短名字不值得为「留够 4 列」白占位。
    let name_min = name_w.min(MIN_NAME_W);
    let brand = name_min + brand_w <= width;
    let avail = width.saturating_sub(if brand { brand_w } else { 0 });
    // 连名字自己都放不下：只剩一个截断后居中的名字（等价改造前的观感）。
    if avail < name_min {
        return centered(&name, width, name_style);
    }

    let name = truncate_display(&name, avail);
    let name_w = UnicodeWidthStr::width(name.as_str());
    let pad = avail.saturating_sub(name_w) / 2;
    let mut spans: Vec<Span<'static>> = Vec::with_capacity(4);
    if brand {
        spans.push(Span::styled(
            format!("{BRAND} "),
            Style::new().fg(pal.muted).add_modifier(Modifier::BOLD),
        ));
    }
    push_pad(&mut spans, pad);
    spans.push(Span::styled(name, name_style));
    push_pad(&mut spans, avail.saturating_sub(pad + name_w));
    Line::from(spans)
}

/// 中段文本：工作区路径的 basename（对齐 Desktop 标题栏，不带「工作区: 」前缀、
/// 不显示绝对路径）；未获取时占位。
fn workspace_name(st: &UiState) -> String {
    match st
        .workspace_path
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        Some(path) => workspace_basename(path),
        None => "（未获取）…".to_string(),
    }
}

/// 单段文本按显示宽居中。
///
/// 带左右段时不能用 `Paragraph::alignment`：它是对**整行**居中，会把左段一起推走。
fn centered(text: &str, width: usize, style: Style) -> Line<'static> {
    let text = truncate_display(text, width);
    let w = UnicodeWidthStr::width(text.as_str());
    let pad = width.saturating_sub(w) / 2;
    let mut spans: Vec<Span<'static>> = Vec::with_capacity(2);
    if pad > 0 {
        spans.push(Span::raw(" ".repeat(pad)));
    }
    spans.push(Span::styled(text, style));
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui_mode::render::Palette;

    fn state(path: Option<&str>) -> UiState {
        let mut st = UiState::new();
        // 固定调色板：断言不能随终端环境（`NO_COLOR` / `COLORTERM`）漂移。
        st.set_palette(Palette::from_theme(Some("shadcn-dark")));
        st.workspace_path = path.map(str::to_string);
        st
    }

    fn width_of(line: &Line<'_>) -> usize {
        UnicodeWidthStr::width(line.to_string().as_str())
    }

    #[test]
    fn top_has_brand_and_workspace_name_only() {
        let st = state(Some("/data/proj"));
        let line = top_line(&st, 80);
        let text = line.to_string();
        assert!(text.starts_with("CrabMate "), "左段品牌：{text:?}");
        assert!(text.contains("proj"), "中段工作区名：{text:?}");
        assert!(
            !text.contains("/data"),
            "只显示 basename，不带绝对路径：{text:?}"
        );
        assert_eq!(width_of(&line), 80, "行宽必须铺满窗口：{text:?}");
        // 模式由状态栏的 `mode` 段显示；键位提示不值得常驻顶栏。
        assert!(
            !text.contains("ASK") && !text.contains("ACT"),
            "顶栏不该再出现模式徽标：{text:?}"
        );
        assert!(
            !text.contains("Ctrl+E") && !text.contains("F2"),
            "顶栏不该再出现键位提示：{text:?}"
        );
    }

    #[test]
    fn top_line_never_exceeds_width() {
        for width in [3usize, 4, 6, 8, 12, 13, 18, 40, 80, 200] {
            let st = state(Some("/data/proj"));
            let line = top_line(&st, width);
            let w = width_of(&line);
            assert!(
                w <= width,
                "width={width} 超宽：{w} 列 / {:?}",
                line.to_string()
            );
        }
    }

    #[test]
    fn narrow_top_drops_the_brand() {
        let st = state(Some("/data/proj"));
        // 品牌占 9 列（含尾部间隔），名字最少留 4 列：13 列是分界。
        let at12 = top_line(&st, 12).to_string();
        assert!(
            !at12.contains("CrabMate"),
            "12 列放不下品牌该让位：{at12:?}"
        );
        let at13 = top_line(&st, 13).to_string();
        assert!(at13.contains("CrabMate"), "13 列刚好放得下：{at13:?}");
        assert!(at13.contains("proj"), "名字始终保留：{at13:?}");
    }

    #[test]
    fn short_name_keeps_the_brand_when_it_fits() {
        // 2 列名字 + 9 列品牌 = 11 列。够就不该按「中段留 4 列」把品牌白丢掉。
        let st = state(Some("/data/ab"));
        let line = top_line(&st, 11);
        let text = line.to_string();
        assert!(text.contains("CrabMate"), "短名字时品牌仍该在：{text:?}");
        assert!(text.trim_end().ends_with("ab"), "{text:?}");
        assert_eq!(width_of(&line), 11);
    }

    #[test]
    fn very_narrow_top_keeps_only_the_centered_name() {
        let st = state(Some("/data/proj"));
        let line = top_line(&st, 8);
        let text = line.to_string();
        assert_eq!(text.trim(), "proj", "只剩工作区名：{text:?}");
        assert!(!text.contains("CrabMate"), "品牌该先让位：{text:?}");
        assert_eq!(width_of(&line), 8);
    }

    #[test]
    fn missing_workspace_still_shows_a_placeholder() {
        let st = state(None);
        let text = top_line(&st, 80).to_string();
        assert!(text.contains("未获取"), "未获取工作区应有占位：{text:?}");
        assert!(!text.contains("工作区:"), "顶栏不带“工作区: ”前缀");
    }
}
