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
// 选项枚举在 `approve`（与按键→决策的逻辑同处）。
use super::super::approve::ApprovalChoice;

/// 审批浮层宽高上限。
const OVERLAY_MAX_WIDTH: u16 = 78;
/// 边框 2 行 + preview + （可选）allowlist + 空行 + 状态 + 选项行 + 两行提示。
const OVERLAY_HEIGHT: u16 = 9;

/// 在 `area` 内居中放一个 `width × height` 的矩形；装不下时贴左上，绝不越界。
fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let x = area.x.saturating_add(area.width.saturating_sub(width) / 2);
    let y = area
        .y
        .saturating_add(area.height.saturating_sub(height) / 2);
    Rect::new(x, y, width, height)
}

/// 设置面板弹层矩形（含边框）：居中、宽度有上限、高度随内容自适应。
///
/// 为什么不是整屏：整屏覆盖把底下的对话完全盖住，看起来像「切换了页面」而不是
/// 「打开了一个浮层」；居中留边距后，底下的会话仍在视野里，与桌面端的模态观感一致。
///
/// **极小终端退化为整屏**（与拆出前的行为一致）：可用区放不下「边框 + 一行内容」时，
/// 居中只会挤掉本就不多的行，整屏反而最稳。
///
/// 宽度与 [`settings_panel_content_width`] 必须同源 —— 内容行是按宽度排版的
/// （值列对齐、超长截断都在 `SettingsPanel::content(ctx, width)` 里做），
/// 两边各算一套就会出现「弹层 76 列、内容按 78 列生成」这种错位。
pub(super) fn settings_panel_rect(area: Rect, content_lines: usize) -> Rect {
    if area.width < 8 || area.height < 4 {
        return area;
    }
    let width = area.width.saturating_sub(4).clamp(4, OVERLAY_MAX_WIDTH);
    let want = (content_lines as u16).saturating_add(2); // 上下边框
    let height = want.min(area.height.saturating_sub(2)).max(3);
    centered(area, width, height)
}

/// 面板内容可用宽度：与 [`settings_panel_rect`] 同源（扣掉左右边框）。
///
/// 绘制侧用 `Block::inner`，这里用同样的减法，保证「按多宽生成」= 「画多宽」。
pub(crate) fn settings_panel_content_width(area: Rect) -> usize {
    settings_panel_rect(area, 0).width.saturating_sub(2) as usize
}

/// 审批浮层：命令预览 + 按键提示，居中浮在聊天主体上。
pub(super) fn render_approval_overlay(frame: &mut Frame, st: &UiState, area: Rect) {
    let Some(ap) = &st.approval else {
        return;
    };
    let width = area.width.saturating_sub(4).min(OVERLAY_MAX_WIDTH);
    if width == 0 || area.height < OVERLAY_HEIGHT {
        return;
    }
    // 纵向居中而不是贴底：贴底时它和状态栏、composer 提示挤在一起，容易被当成
    // 一行普通输出扫过去；居中之后才像「卡住等你决策」的模态。方块大小不变，
    // 所以 8 行内容仍完整可见。
    let rect = centered(area, width, OVERLAY_HEIGHT);
    // 先清空矩形内字形，再整体铺底色：浮层必须是独立色块，
    // 不能与底下聊天区残留文本/上一帧内容视觉重叠。底色跟主题走——
    // 固定深底在浅色主题下会让正文色（深字）不可读。
    let pal = &st.palette;
    frame.render_widget(Clear, rect);
    paint_bg(frame, rect, pal.overlay_bg);
    // 边框 / 标题用 `err`：这是「回合被阻塞、等你决策」的状态，与正常的 accent 边框
    // 区分开；`mono` 下 `err` 已退化为 `Reset`，仍有边框与标题可辨。
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(pal.err))
        .title(Span::styled(
            " 命令审批（等待决策） ",
            Style::new().fg(pal.err).add_modifier(Modifier::BOLD),
        ));
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
    // 三个选项：`←` / `→` 移动，选中项画成色块（`err` 底 + 聊天区前景），
    // 未选中只是 muted 文字。用色块而不是 `[ ]` 括起来，是因为在深色终端里
    // 反色块比括号更容易一眼扫到当前会提交哪一项。
    lines.push(choice_line(st.approval_choice, pal));
    lines.push(Line::from(Span::styled(
        "[←/→] 选择   [Enter] 确认   [Esc] 拒绝   [Ctrl+C] 拒绝",
        Style::new().fg(pal.accent),
    )));
    lines.push(Line::from(Span::styled(
        "Ctrl+C 只拒绝本命令；回合继续，取消需再按一次 Ctrl+C",
        Style::new().fg(pal.muted),
    )));
    frame.render_widget(Paragraph::new(lines).block(block), rect);
}

/// 三个动作选项排成一行；选中项加 `err` 底色块。
fn choice_line(selected: ApprovalChoice, pal: &Palette) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (i, choice) in ApprovalChoice::ALL.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("   "));
        }
        let style = if *choice == selected {
            Style::new()
                .fg(pal.bg)
                .bg(pal.err)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(pal.muted)
        };
        spans.push(Span::styled(format!(" {} ", choice.label()), style));
    }
    Line::from(spans)
}

/// 设置面板浮层：居中弹层（`Clear` + 独立色块 + 边框块，模板同审批浮层），
/// 内容行由 [`super::settings_panel`] 构建（按 [`settings_panel_content_width`] 截断），
/// 编辑时给出硬件光标位。极小终端下 [`settings_panel_rect`] 会退化为整屏。
pub(super) fn render_settings_overlay(
    frame: &mut Frame,
    area: Rect,
    content: &PanelContent,
    pal: &Palette,
) {
    let rect = settings_panel_rect(area, content.lines.len());
    frame.render_widget(Clear, rect);
    paint_bg(frame, rect, pal.overlay_bg);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(pal.accent))
        .title(Span::styled(
            " 设置（模型 / 会话） ",
            Style::new().fg(pal.accent),
        ));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
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

// 测试与实现分开文件（同 render / md / settings_panel 惯例），避免本文件顶到行数门禁。
#[cfg(test)]
#[path = "overlay_tests.rs"]
mod tests;
