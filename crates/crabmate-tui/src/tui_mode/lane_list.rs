//! 左栏下半的 lane 列表渲染。
//!
//! 单独成文件有两个理由：`render.rs` 贴着 920 行门禁；而这块逻辑与「怎么画一个
//! pane」无关，只依赖调色板与截断工具，拆开后两边都不必为对方让路。

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::palette::Palette;
use super::text::truncate_display;

/// 左栏下半的 lane 列表行：首行标题，其后每条 lane 一行（`>` 当前）。
///
/// `lanes` 是外壳层算好的快照 `(名字, 是否当前)` —— lane 的状态各自住在自己的
/// `LaneApp` 里，渲染不该为了画一行去借遍所有 lane（那会把借用冲突引到渲染路径上）。
pub(super) fn lane_list_rows(
    lanes: &[(String, bool)],
    width: usize,
    palette: &Palette,
) -> Vec<Line<'static>> {
    let title = format!("连接 [{}]", lanes.len());
    let mut rows = Vec::with_capacity(lanes.len() + 1);
    rows.push(Line::from(Span::styled(
        truncate_display(&title, width),
        Style::new().fg(palette.fg).add_modifier(Modifier::BOLD),
    )));
    for (name, active) in lanes {
        let mark = if *active { ">" } else { " " };
        let text = truncate_display(&format!("{mark} {name}"), width);
        let style = if *active {
            Style::new().fg(palette.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::new()
        };
        rows.push(Line::from(Span::styled(text, style)));
    }
    rows
}
