//! 左栏下半的 lane 列表渲染：每条连接一行整行宽的底色条，自上而下堆叠。
//!
//! 单独成文件有两个理由：`render.rs` 贴着 920 行门禁；而这块逻辑与「怎么画一个
//! pane」无关，只依赖调色板与截断工具，拆开后两边都不必为对方让路。
//!
//! 版面口径全部来自 `layout`（`lane_shown_len` / `lane_index_at`）：
//! 一行对应哪条 lane 与鼠标命中必须是同一份算法，否则会出现「点第 2 行切到第 3 条」。

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use super::layout::lane_shown_len;
use super::palette::Palette;
use super::text::truncate_display;

/// 画出整块 lane 区：标题行 + 每条 lane 一行底色条，行与行之间不留空行。
///
/// 色条左右各内缩 1 列（与左右侧栏的内容区同口径）；条数少于可用行时下方留空
/// （不拉伸），多于可用行时按行截断并在标题补 `↓n`。
pub(super) fn render_lane_list(
    frame: &mut Frame,
    area: Rect,
    lanes: &[(String, bool)],
    palette: &Palette,
) {
    let inner = Rect::new(
        area.x.saturating_add(1),
        area.y,
        area.width.saturating_sub(2),
        area.height,
    );
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let rows = lane_list_rows(lanes, inner.width as usize, inner.height, palette);
    frame.render_widget(Paragraph::new(rows), inner);
}

/// lane 区内容行：首行标题，其后每条 lane 一行。
///
/// `lanes` 是外壳层算好的快照 `(名字, 是否当前)` —— lane 的状态各自住在自己的
/// `LaneApp` 里，渲染不该为了画一行去借遍所有 lane（那会把借用冲突引到渲染路径上）。
fn lane_list_rows(
    lanes: &[(String, bool)],
    width: usize,
    height: u16,
    palette: &Palette,
) -> Vec<Line<'static>> {
    if width == 0 || height == 0 {
        return Vec::new();
    }
    // 标题固定 1 行，其余每行一条 lane（与 `layout::lane_index_at` 同一口径）。
    let avail = height.saturating_sub(1);
    let shown = lane_shown_len(avail, lanes.len());
    let mut rows = Vec::with_capacity(height as usize);
    rows.push(Line::from(Span::styled(
        truncate_display(&lane_title(lanes.len(), shown), width),
        Style::new().fg(palette.fg).add_modifier(Modifier::BOLD),
    )));
    for (name, active) in lanes.iter().take(shown) {
        rows.push(lane_row(name, *active, width, palette));
    }
    rows
}

/// 标题：`连接 [总数]`，装不下时补 `↓n`（被截掉的条数）。
fn lane_title(total: usize, shown: usize) -> String {
    if shown < total {
        format!("连接 [{total}] ↓{}", total - shown)
    } else {
        format!("连接 [{total}]")
    }
}

/// 一条 lane 的一行：整行宽的底色条，文字贴左，当前 lane 多一个 `▎`。
///
/// 恒一行而不是「均分可用行」的色块：两行块在真终端上读起来像一个按钮，文字贴首行
/// 之后第二行只剩一条没有信息的色带，还会把切换 lane 的文字基线撑得跟上面的会话
/// 列表对不齐。一行一条与左侧会话列表同一行距，才读得出是「列表项」。
///
/// 整行补空格到 `width` —— 只给文字上底色的话，条会缩成「文字那么宽」，看起来仍是
/// 一行高亮文字，而不是列表里的一行。
fn lane_row(name: &str, active: bool, width: usize, palette: &Palette) -> Line<'static> {
    let mark = if active { "▎" } else { " " };
    let text = pad_display(&truncate_display(&format!("{mark} {name}"), width), width);
    Line::from(Span::styled(text, row_style(active, palette)))
}

/// 底色条样式：当前 lane 走 `palette.selection`（无色模式自动退回反色），其余用
/// 「比条带底色亮一档」的 `overlay_bg`，这样条与 lane 区底色仍分得开。
fn row_style(active: bool, palette: &Palette) -> Style {
    if active {
        palette
            .selection(palette.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(palette.fg).bg(palette.overlay_bg)
    }
}

/// 补空格到 `width` 个显示列（CJK 按 2 列计）；已够宽则原样返回。
fn pad_display(text: &str, width: usize) -> String {
    let used = UnicodeWidthStr::width(text);
    let mut out = String::with_capacity(text.len() + width.saturating_sub(used));
    out.push_str(text);
    out.push_str(&" ".repeat(width.saturating_sub(used)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pal() -> Palette {
        Palette::default()
    }

    fn lanes(names: &[&str]) -> Vec<(String, bool)> {
        names
            .iter()
            .enumerate()
            .map(|(i, n)| ((*n).to_string(), i == 0))
            .collect()
    }

    /// 单 lane：标题 1 行 + 1 行底色条，其余行留空（版面高度固定，不随条数拉伸）。
    #[test]
    fn one_lane_takes_one_row_under_the_title() {
        let rows = lane_list_rows(&lanes(&["local"]), 20, 20, &pal());
        // 1（标题）+ 1（那条 lane）= 2 行有内容，其余交给调用方的底色。
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].spans[0].content.as_ref(), "连接 [1]");
        // 唯一那条就是当前 lane，故整行都是选中底色。
        assert_eq!(rows[1].spans[0].style.bg, Some(pal().selection_bg));
        assert!(rows[1].spans[0].content.contains("local"));
    }

    /// 当前 lane 用选中底色 + 竖条标记；其它 lane 用一档抬升底色，每条各占一行。
    #[test]
    fn active_lane_is_marked_and_tinted() {
        let rows = lane_list_rows(&lanes(&["a", "b"]), 20, 20, &pal());
        // 一条一行：a = rows[1]，b = rows[2]。
        let a = &rows[1];
        let b = &rows[2];
        assert!(a.spans[0].content.starts_with('▎'));
        assert_eq!(a.spans[0].style.bg, Some(pal().selection_bg));
        assert!(!b.spans[0].content.starts_with('▎'));
        assert_eq!(b.spans[0].style.bg, Some(pal().overlay_bg));
    }

    /// 每行宽度恒等于内容区宽度：色块必须是矩形，不是「文字那么宽」的底色。
    #[test]
    fn every_row_fills_the_width() {
        let rows = lane_list_rows(&lanes(&["local", "很长很长的连接名字"]), 16, 20, &pal());
        for r in &rows {
            let w: usize = r
                .spans
                .iter()
                .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
                .sum();
            assert!(w <= 16, "行宽 {w} 超出 16 列");
        }
        // 色块行必须**正好**铺满（不足部分由 pad_display 补空格）。
        for r in &rows[1..] {
            let w: usize = r
                .spans
                .iter()
                .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
                .sum();
            assert_eq!(w, 16);
        }
    }

    /// 装不下时标题行给出被截掉的条数，而不是静默丢弃。
    #[test]
    fn overflow_is_reported_in_the_title() {
        let names: Vec<String> = (0..30).map(|i| format!("lane{i}")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let rows = lane_list_rows(&lanes(&refs), 20, 10, &pal());
        // 可用 9 行、块高 1 → 画 9 条，标题报 ↓21。
        assert_eq!(rows[0].spans[0].content.as_ref(), "连接 [30] ↓21");
        assert_eq!(rows.len(), 10);
    }

    /// 只有标题行放得下时（高度 1）不 panic，且标题如实报出「全部被截掉」。
    #[test]
    fn one_row_only_shows_the_title() {
        let rows = lane_list_rows(&lanes(&["a", "b"]), 20, 1, &pal());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].spans[0].content.as_ref(), "连接 [2] ↓2");
    }

    #[test]
    fn zero_height_or_width_draws_nothing() {
        assert!(lane_list_rows(&lanes(&["a"]), 0, 10, &pal()).is_empty());
        assert!(lane_list_rows(&lanes(&["a"]), 10, 0, &pal()).is_empty());
    }
}
