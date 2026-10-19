//! 浮层渲染（审批 / 设置面板），从 `render.rs` 拆出以腾出行数门禁余量。
//!
//! 两者是同一套模板：`Clear` 清掉底下的残留字形 → `paint_bg` 铺主题条带色 →
//! `Borders::ALL` 边框 + 标题。差别只在尺寸与内容来源，所以放在同一文件里，
//! 也方便后续把「浮层居中的退化分支」一并改动（见 `tui_visual_polish.md` 的 B4）。

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

// 本文件是 `render` 的子模块，`super::` 即 `render`：直接取它已引入的类型与私有
// 工具函数（`paint_bg` 铺底、`truncate_display` 截断），不必再走 `super::super::…`。
use super::{Palette, PanelContent, UiState, paint_bg, truncate_display};

/// 审批浮层宽高上限。
const OVERLAY_MAX_WIDTH: u16 = 78;
const OVERLAY_HEIGHT: u16 = 8;

/// 审批浮层：命令预览 + 按键提示，置于聊天主体底部（composer 之上）。
pub(super) fn render_approval_overlay(frame: &mut Frame, st: &UiState, area: Rect) {
    let Some(ap) = &st.approval else {
        return;
    };
    let width = area.width.saturating_sub(4).min(OVERLAY_MAX_WIDTH);
    if width == 0 || area.height < OVERLAY_HEIGHT {
        return;
    }
    let x = area.x.saturating_add(area.width.saturating_sub(width) / 2);
    let y = area
        .y
        .saturating_add(area.height.saturating_sub(OVERLAY_HEIGHT));
    let rect = Rect::new(x, y, width, OVERLAY_HEIGHT);
    // 先清空矩形内字形，再整体铺底色：浮层必须是独立色块，
    // 不能与底下聊天区残留文本/上一帧内容视觉重叠。底色跟主题走——
    // 固定深底在浅色主题下会让正文色（深字）不可读。
    let pal = &st.palette;
    frame.render_widget(Clear, rect);
    paint_bg(frame, rect, pal.chrome_bg);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(pal.accent))
        .title(Span::styled(" 命令审批 ", Style::new().fg(pal.accent)));
    let inner = block.inner(rect);
    let preview = truncate_display(&ap.preview(), inner.width.saturating_sub(2) as usize);
    let mut lines: Vec<Line<'static>> = vec![Line::from(Span::styled(
        preview,
        Style::new().fg(pal.fg).add_modifier(Modifier::BOLD),
    ))];
    if let Some(key) = ap
        .allowlist_key
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        lines.push(Line::from(Span::styled(
            format!("allowlist: {key}"),
            Style::new().fg(pal.muted),
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "回合已暂停，等待你的决策",
        Style::new().fg(pal.muted),
    )));
    lines.push(Line::from(Span::styled(
        "[Enter] 仅此一次  [a] 始终允许  [Esc/n] 拒绝  [Ctrl+C] 拒绝",
        Style::new().fg(pal.accent),
    )));
    lines.push(Line::from(Span::styled(
        "Ctrl+C 只拒绝本命令；回合继续，取消需再按一次 Ctrl+C",
        Style::new().fg(pal.muted),
    )));
    frame.render_widget(Paragraph::new(lines).block(block), rect);
}

/// 设置面板浮层：整屏 Clear + 独立色块 + 边框块（模板同审批浮层），
/// 内容行由 [`super::settings_panel`] 构建（已按内宽截断），编辑时给出硬件光标位。
pub(super) fn render_settings_overlay(
    frame: &mut Frame,
    area: Rect,
    content: &PanelContent,
    pal: &Palette,
) {
    frame.render_widget(Clear, area);
    paint_bg(frame, area, pal.chrome_bg);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(pal.accent))
        .title(Span::styled(
            " 设置（模型 / 会话） ",
            Style::new().fg(pal.accent),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let shown: Vec<Line<'static>> = content
        .lines
        .iter()
        .take(inner.height as usize)
        .cloned()
        .collect();
    frame.render_widget(Paragraph::new(shown), inner);
    // 文本编辑光标：内容行下标 + cell 列（仅在浮层内可见区域落位）。
    if let Some((row, col)) = content.cursor
        && (row as u16) < inner.height
        && (col as u16) < inner.width
    {
        frame.set_cursor_position((inner.x + col as u16, inner.y + row as u16));
    }
}
