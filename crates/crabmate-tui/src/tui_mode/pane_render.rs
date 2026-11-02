//! 两栏的渲染小事：正文预算换算 + 一列分隔线。
//!
//! 从 `render.rs` 拆出（issue #407）：那边贴着 920 行/文件的 `fn-nloc-ratchet` 门禁，
//! 而这两件事只依赖「版面 + 调色板」，与 transcript / composer 的排版无关。

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, Paragraph};

use super::layout::Panes;
use super::palette::Palette;
use super::pane_width::PaneLayout;
use super::render::BodyRow;
use super::state::UiState;

/// 计算 build_body_rows 使用的聊天区宽度（与 `draw` 里传给 `render_body` 的区域一致：
/// 聊天列内左右各留 1 列空隙，与侧栏 / composer 边框的对称留白对齐）。
///
/// 宽度必须**夹取后**再算：预算若按用户选择的原始列宽算，终端变窄时折行会比绘制区
/// 宽出好几列，行尾字符被右缘直接裁掉。夹取口径与 `layout::panes_with_lanes` 同一份
/// （幂等，重复夹不会给出第二个答案）。
pub(super) fn chat_body_width(layout: PaneLayout, terminal_width: u16) -> usize {
    let layout = layout.clamped(terminal_width);
    let col = if layout.visible {
        terminal_width.saturating_sub(layout.widths.left.saturating_add(layout.widths.right))
    } else {
        terminal_width
    };
    (col as usize).saturating_sub(2)
}

/// 两栏各画一列分隔线（左栏 `RIGHT`、右栏 `LEFT`），并贯穿上下两段。
///
/// 调用点必须在内容**之后**：落点是每列本来就空着的左右留白列，所以不影响内容排版
/// （`render_pane` 的「内容内宽 = 列宽 - 2」口径不变）；左栏的 lane 区与右栏的状态栈
/// 用同一列继续画，视觉上才是一条贯穿整列的线，而不是半截断掉。往后「拖动改宽」的
/// 把手也是这一列。
pub(super) fn paint_pane_dividers(frame: &mut Frame, p: &Panes, pal: &Palette) {
    if let Some(left) = p.left {
        paint_edge(frame, left, Borders::RIGHT, pal);
    }
    if let Some(lanes) = p.left_lanes {
        paint_edge(frame, lanes, Borders::RIGHT, pal);
    }
    if let Some(stack) = p.right_status {
        paint_edge(frame, stack, Borders::LEFT, pal);
    }
    if let Some(right) = p.right {
        paint_edge(frame, right, Borders::LEFT, pal);
    }
}

/// 画侧栏的一列边框（`borders` 决定画哪一侧）。
///
/// 用 `Block` 的边框而不是逐行拼 `│`：边框只写最外那一列的字符单元，不参与
/// `render_pane` 的截断宽度计算。颜色取 `Palette::border`（无色模式下退成 `Reset`，
/// 线仍在 —— 分隔靠位置而非颜色）。宽度不足 2 列时不画：那时已经没有留白列可占，
/// 画了就会盖住唯一的内容列。
fn paint_edge(frame: &mut Frame, area: Rect, borders: Borders, pal: &Palette) {
    if area.width < 2 || area.height == 0 {
        return;
    }
    let block = Block::default()
        .borders(borders)
        .border_style(Style::new().fg(pal.border));
    frame.render_widget(block, area);
}

/// 聊天列：上为主区（transcript 或只读文件视图），底部为 composer 输入区。
///
/// 从 `render.rs` 搬来（issue #418）：编辑器视图让这里多出一条分支，而那个文件贴着 920 行
/// 门禁。搬的是**布局**，主区内部与 composer 的画法仍在 `render.rs` —— 浮层与补全菜单的
/// 绘制顺序在那边，拆开两边都会重新长出各自的一份。
pub(super) fn render_chat_column(
    frame: &mut Frame,
    st: &mut UiState,
    area: Rect,
    prepared: &[BodyRow],
    cursor_allowed: bool,
    spin: Option<usize>,
    pal: &Palette,
) {
    // composer 含上下边框；至少给聊天区留 1 行，极小终端下不把 transcript 挤没。
    let rows = super::render::composer_outer_rows(st)
        .min(area.height.saturating_sub(1))
        .max(1);
    let parts = Layout::vertical([Constraint::Min(1), Constraint::Length(rows)]).split(area);
    let body_area = parts[0];
    // 编辑器视图只替换主区：composer 留着，`/chat` 与 `/editor <另一个文件>` 才能直接敲，
    // 该 lane 的在途回合也照常可见（切视图不打断回合）。
    if st.editor.open {
        frame.render_widget(Block::default().style(Style::new().bg(pal.bg)), body_area);
        let lines = super::editor_view::view_lines(
            &st.editor,
            st.workspace_path.as_deref(),
            body_area.width as usize,
            body_area.height as usize,
            pal,
        );
        frame.render_widget(Paragraph::new(lines), body_area);
    } else {
        super::render::render_body_and_overlays(frame, st, body_area, prepared, spin);
    }
    super::render::render_input(frame, st, parts[1], cursor_allowed);
}
