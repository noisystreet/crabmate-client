//! 侧栏列表的可见窗口：条目区 `[start, end)` 的**唯一**计算来源。
//!
//! 渲染裁剪与鼠标命中检测都调这里，各算一遍就会在「列表够长且光标接近底部」时
//! 悄悄错开一行，症状是点到的行与反色高亮不同行，极难察觉。
//!
//! 窗口起点是**粘住**的：以 `prev_start` 为起点，只在光标越出窗口时做最小滚动。
//! 绝不能由光标反推（`sel + 1 - item_max`）—— 那样点一下窗口内任意行都会把该行
//! 钉到窗口底边、整列跳动，双击的第二下便落到别的行上，表现成「双击打开了错误的
//! 文件」（长树、滚到下方时最明显）。

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use super::palette::Palette;
use super::ws_sidebar::sidebar_view;

/// 侧栏列的内容区：去掉列内左留白 1 列与底部 1 行（按键提示）。
///
/// 渲染与鼠标命中检测共用；放在这里是因为它和 [`sidebar_window`] 一起定义
/// 「列表在屏幕上的位置与可见范围」这一件事。
pub(super) fn pane_list_area(area: Rect) -> Rect {
    if area.width <= 1 || area.height == 0 {
        return Rect::new(area.x, area.y, 0, 0);
    }
    let inner = Rect::new(area.x + 1, area.y, area.width - 1, area.height);
    ratatui::layout::Layout::vertical([
        ratatui::layout::Constraint::Min(1),
        ratatui::layout::Constraint::Length(1),
    ])
    .split(inner)[0]
}

/// 侧栏一帧的内容快照：行、光标与底部提示行。
///
/// 三件东西总是一起由 `sessions_content` / `workspace_content` 产出、一起交给
/// `render_pane`，打包成一个参数既省调用处的折行，也让 clippy 的形参上限有余量。
pub(super) struct SidebarRows {
    pub rows: Vec<Line<'static>>,
    pub cursor: usize,
    pub hint: Option<Line<'static>>,
    /// 内容宽度（列内左右各留 1 列后的值），与行内容是同一口径。
    pub width: usize,
}

/// 条目区（不含固定的标题行）的可见窗口 `[start, end)`。
pub(super) fn sidebar_window(
    rows_len: usize,
    height: usize,
    selected: usize,
    prev_start: usize,
) -> (usize, usize) {
    let items_len = rows_len.saturating_sub(1);
    if height == 0 || items_len == 0 {
        return (0, 0);
    }
    if rows_len <= height {
        return (0, items_len);
    }
    let item_max = height - 1;
    let sel = selected.min(items_len - 1);
    // 起点先夹进合法区间（内容变短 / 窗口变高后，旧起点可能越界）。
    let mut start = prev_start.min(items_len.saturating_sub(item_max));
    if sel < start {
        start = sel;
    } else if sel >= start.saturating_add(item_max) {
        start = (sel + 1).saturating_sub(item_max);
    }
    (start, (start + item_max).min(items_len))
}

/// 通用固定列渲染：内容窗口裁剪（含提示行）。内容行/光标/提示由 [`super::ws_sidebar`] 提供。
/// 列内左右各留 1 列空隙（对齐聊天区观感），行宽按 `width - 2` 截断。
pub(super) fn render_pane(
    frame: &mut Frame,
    area: Rect,
    content: SidebarRows,
    pal: &Palette,
    list_start: &mut usize,
) {
    if area.width <= 1 || area.height == 0 {
        return;
    }
    let SidebarRows {
        rows,
        cursor,
        hint,
        width,
    } = content;
    // 内容区与提示行矩形同样跑鼠标命中检测用的那份计算。
    let list_area = pane_list_area(area);
    // 窗口起点**粘住**：只在光标越出窗口时最小滚动，结果写回状态供命中检测读。
    let (start, end) = sidebar_window(rows.len(), list_area.height as usize, cursor, *list_start);
    *list_start = start;
    let shown = sidebar_view(&rows, start, end, width, pal);
    let list = Paragraph::new(shown).style(Style::new().fg(pal.fg));
    frame.render_widget(list, list_area);
    // 提示行占列内最后一行（高度不足 2 行时没有提示位）。内容自带样式：会话栏是静态
    // 键位，工作区栏在光标行被截断时会改成完整相对路径。
    if area.height >= 2
        && let Some(text) = hint
    {
        let hint_area = Rect::new(area.x + 1, area.y + area.height - 1, area.width - 1, 1);
        frame.render_widget(Paragraph::new(text), hint_area);
    }
}

#[cfg(test)]
mod tests {
    use super::sidebar_window;

    /// 窗口起点**粘住**：光标在窗口内移动时不动。
    ///
    /// 「双击打开错误文件」的根因回归：过去起点由光标反推，点一下窗口最上面
    /// 那行就会把整列往上顶，双击的第二下在新位置上重算 → 落到别的行。
    #[test]
    fn start_sticks_when_cursor_stays_inside() {
        // 101 行（标题 + 100 条），窗口 5 行 = 标题 + 4 条
        let (rows_len, height) = (101, 5);
        // 光标在底部：起点让第 40 条成为最后一条可见项
        let (start, _) = sidebar_window(rows_len, height, 40, 0);
        assert_eq!(start, 37, "40 + 1 - 4");

        // 点可视第 1 行（条目 37）与最后一行（条目 40）：起点都必须不变
        assert_eq!(sidebar_window(rows_len, height, start, start).0, start);
        assert_eq!(sidebar_window(rows_len, height, 40, start).0, start);
    }

    /// 只有光标真的越出窗口时才最小滚动。
    #[test]
    fn start_scrolls_minimally_on_overflow() {
        let (rows_len, height, start) = (101, 5, 37);
        // 向下越界一格 → 只滚一行
        assert_eq!(sidebar_window(rows_len, height, 41, start).0, 38);
        // 向上越界一格 → 光标变成新起点
        assert_eq!(sidebar_window(rows_len, height, 36, start).0, 36);
    }

    /// 内容变短 / 窗口变高后，旧起点越界要被夹回去，而不是留下空白。
    #[test]
    fn start_clamps_when_content_shrinks() {
        // 只有 3 条，窗口 5 行 → 全部可见
        assert_eq!(sidebar_window(4, 5, 0, 37), (0, 3));
        // 仍比窗口长但没这么长：光标取 8（sel=0 本就要求露出来，起点会归 0）
        assert_eq!(sidebar_window(11, 5, 8, 37).0, 6, "10 条 - 4 = 6");
    }
}
