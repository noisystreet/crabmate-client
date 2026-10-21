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
use unicode_width::UnicodeWidthStr;

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

/// 设置页左侧导航轨宽度。
///
/// 对齐 desktop `.settings-nav { grid-template-columns: 220px }` 的终端比例版：
/// TUI 只有「模型 / 会话」两个分区，14 列足够放下选中标记与中文标签。
const SETTINGS_NAV_WIDTH: u16 = 14;
/// 设置页顶栏高度：只占标题这一行，区域靠底色块与主体区分（不画分隔线）。
const SETTINGS_HEADER_ROWS: u16 = 1;
/// 设置页底栏高度：状态提示 + 按键说明，同样靠底色块区分。
///
/// 与 `settings_panel_draw::footer_lines()` 恒返回 2 行是隐式契约：那边加行要同步调这里，
/// 否则多出来的行会被 `take` 静默截断。
const SETTINGS_FOOTER_ROWS: u16 = 2;
/// 内容区左右内缩（desktop `.settings-content { padding: 0 14px }` 的终端缩略版）。
const SETTINGS_CONTENT_PAD: u16 = 2;
/// 低于此宽度不画左导航轨：两栏挤在一起还不如单列铺满。
const SETTINGS_NAV_MIN_WIDTH: u16 = 44;

/// 设置页四区矩形（顶栏 / 左导航 / 内容区 / 底栏），对齐 desktop 设置页骨架。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SettingsPanelLayout {
    /// 顶栏：标题 + 状态徽标，整行卡片色（不画分隔线）。
    pub header: Rect,
    /// 左导航轨；`width == 0` 表示终端太窄，不画导航（改用 Tab 切分区）。
    pub nav: Rect,
    /// 右侧内容区（字段行按它的内宽排版）。
    pub content: Rect,
    /// 贴底底栏：状态提示 + 按键说明，整行卡片色。
    pub footer: Rect,
}

/// 计算设置页四区矩形。
///
/// 目标是与 desktop 的设置**页面**同构（顶栏 / 左导航 + 内容 / 贴底底栏），
/// 而不是居中浮层：分区已多到需要导航轨，浮层里既塞不下也看不全。
///
/// **极小终端**（`width < 20` 或 `height < 8`）退化为「整屏内容区、无顶栏底栏」：
/// 那种尺寸下顶栏 / 底栏这两条常驻区域都在抢本就不够用的行。
///
/// 内容区宽度与 [`settings_panel_content_width`] 必须同源 —— 字段行是按宽度排版的
/// （值列对齐、超长截断都在 `SettingsPanel::content(ctx, width)` 里做），
/// 两边各算一套就会出现「画 60 列、按 64 列生成」这种错位。
pub(super) fn settings_panel_layout(area: Rect) -> SettingsPanelLayout {
    if area.width < 20 || area.height < 8 {
        return SettingsPanelLayout {
            header: Rect::new(area.x, area.y, area.width, 0),
            nav: Rect::new(area.x, area.y, 0, 0),
            content: area,
            footer: Rect::new(area.x, area.y, area.width, 0),
        };
    }
    let header = Rect::new(area.x, area.y, area.width, SETTINGS_HEADER_ROWS);
    let footer_y = area.y + area.height - SETTINGS_FOOTER_ROWS;
    let footer = Rect::new(area.x, footer_y, area.width, SETTINGS_FOOTER_ROWS);
    let body_y = header.y + header.height;
    let body_h = footer_y.saturating_sub(body_y);
    // 导航轨最多占 1/3 宽，避免窄屏上把字段挤成一条缝。
    let nav_w = if area.width >= SETTINGS_NAV_MIN_WIDTH {
        SETTINGS_NAV_WIDTH.min(area.width / 3)
    } else {
        0
    };
    SettingsPanelLayout {
        header,
        nav: Rect::new(area.x, body_y, nav_w, body_h),
        content: Rect::new(
            area.x.saturating_add(nav_w),
            body_y,
            area.width.saturating_sub(nav_w),
            body_h,
        ),
        footer,
    }
}

/// 内容区扣掉左右内缩后的可绘矩形。
///
/// 绘制（[`render_settings_body`]）与宽度计算（[`settings_panel_content_width`]）共用这一份，
/// 从根上避免「画 60 列、按 64 列生成」这类错位 —— 包括极窄终端下内缩需要减半的情况。
fn content_inner_rect(rect: Rect) -> Rect {
    // 内缩取一半宽（窄终端下两边各 1 列），避免把内容挤没。
    let pad = SETTINGS_CONTENT_PAD.min(rect.width / 2);
    Rect::new(
        rect.x.saturating_add(pad),
        rect.y,
        rect.width.saturating_sub(pad.saturating_mul(2)),
        rect.height,
    )
}

/// 面板内容可用宽度：与 [`settings_panel_layout`] 同源（扣掉内容区左右内缩）。
pub(crate) fn settings_panel_content_width(area: Rect) -> usize {
    content_inner_rect(settings_panel_layout(area).content).width as usize
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

/// 设置页（全屏）：顶栏 + 左侧分区导航 + 内容区 + 贴底底栏。
///
/// 骨架对齐 desktop 的 `.settings-page`（顶栏 / `settings-layout` 两栏 / 贴底操作区），
/// 不再是居中浮层。字段行由 [`super::settings_panel`] 按
/// [`settings_panel_content_width`] 排版，编辑时给出硬件光标位。
pub(super) fn render_settings_overlay(
    frame: &mut Frame,
    area: Rect,
    content: &PanelContent,
    pal: &Palette,
) {
    let layout = settings_panel_layout(area);
    // 整屏清底：设置是「页面」而非浮层，底下不留对话残影。
    frame.render_widget(Clear, area);
    paint_bg(frame, area, pal.bg);

    render_settings_header(frame, layout.header, content, pal);
    if layout.nav.width > 0 {
        render_settings_nav(frame, layout.nav, content, pal);
    }
    let inner = render_settings_body(frame, layout.content, content);
    render_settings_footer(frame, layout.footer, content, pal);

    // 文本编辑光标：内容行下标 + cell 列（仅在内容可见区内落位）。
    if let Some((row, col)) = content.cursor
        && inner.width > 0
        && (row as u16) < inner.height
        && (col as u16) < inner.width
    {
        frame.set_cursor_position((inner.x + col as u16, inner.y + row as u16));
    }
}

/// 顶栏：`设置 · <分区>` + 状态徽标（未保存 / 保存中 / 只读）。
fn render_settings_header(frame: &mut Frame, rect: Rect, content: &PanelContent, pal: &Palette) {
    if rect.height == 0 || rect.width == 0 {
        return;
    }
    // 高度由 `SETTINGS_HEADER_ROWS` 决定，这里直接用 rect 而不是写死 1 行：
    // 常量若调大，新增的行也一并清底上色，不会留下一条页面底色的缝。
    frame.render_widget(Clear, rect);
    paint_bg(frame, rect, pal.overlay_bg);
    let mut spans = vec![
        Span::styled(
            " 设置",
            Style::new().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" · ", Style::new().fg(pal.muted)),
        Span::styled(
            content.title.clone(),
            Style::new().fg(pal.fg).add_modifier(Modifier::BOLD),
        ),
    ];
    if let Some(badge) = &content.badge {
        // 徽标走 `err` 色：未保存 / 只读都是「需要你先处理」的状态，与正常标题区分。
        spans.push(Span::styled("  ", Style::new()));
        spans.push(Span::styled(
            format!("[{badge}]"),
            Style::new().fg(pal.err).add_modifier(Modifier::BOLD),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), rect);
}

/// 左侧分区导航轨：竖排、选中项反色底块。
fn render_settings_nav(frame: &mut Frame, rect: Rect, content: &PanelContent, pal: &Palette) {
    // 整块铺卡片色，与内容区的页面底色天然分开 —— 不画竖分隔线。
    paint_bg(frame, rect, pal.overlay_bg);
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let lines: Vec<Line<'static>> = content
        .nav
        .iter()
        .take(rect.height as usize)
        .map(|(label, active)| {
            let style = if *active {
                // 无色模式（`pal.mono`）下 `bg` / `accent` 全是 `Reset`，色块会整个消失，
                // 此时退回反色 + 加粗 —— palette.rs 的约定就是「无区分只能靠 DIM/BOLD/REVERSED」。
                if pal.mono {
                    Style::new().add_modifier(Modifier::BOLD | Modifier::REVERSED)
                } else {
                    Style::new()
                        .fg(pal.bg)
                        .bg(pal.accent)
                        .add_modifier(Modifier::BOLD)
                }
            } else {
                Style::new().fg(pal.muted)
            };
            let text = if *active {
                format!(" {label} ")
            } else {
                format!(" {label}")
            };
            Line::from(Span::styled(fill_cells(&text, rect.width as usize), style))
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), rect);
}

/// 内容区：字段行铺在顶栏与底栏之间，左右留内缩；返回可绘区（供光标定位）。
fn render_settings_body(frame: &mut Frame, rect: Rect, content: &PanelContent) -> Rect {
    let inner = content_inner_rect(rect);
    if inner.width == 0 || inner.height == 0 {
        return inner;
    }
    let shown: Vec<Line<'static>> = content
        .lines
        .iter()
        .take(inner.height as usize)
        .cloned()
        .collect();
    frame.render_widget(Paragraph::new(shown), inner);
    inner
}

/// 底栏：状态提示 + 按键说明，贴底固定（对齐 desktop 页面底部操作区）。
fn render_settings_footer(frame: &mut Frame, rect: Rect, content: &PanelContent, pal: &Palette) {
    if rect.height == 0 || rect.width == 0 {
        return;
    }
    // 同顶栏：整块卡片色本身就是一条「底栏」，不再画上方分隔线。
    paint_bg(frame, rect, pal.overlay_bg);
    let lines: Vec<Line<'static>> = content
        .footer
        .iter()
        .take(rect.height as usize)
        .cloned()
        .collect();
    frame.render_widget(Paragraph::new(lines), rect);
}

/// 把文本**填满**指定 cell 宽（CJK 按 2 列算）：不足补空格、超出截断。
///
/// 用于反色 / 反色块导航项铺满整行。注意与 `settings_panel::pad_cells` 语义不同 ——
/// 那个是「补齐到至少 `cells`、超出原样返回」（字段标签列对齐用），这里必须截断，
/// 否则超长标签会画出导航轨边界。两者刻意不复用同一个名字。
fn fill_cells(text: &str, width: usize) -> String {
    let used = UnicodeWidthStr::width(text);
    if used >= width {
        return truncate_display(text, width);
    }
    let mut out = String::with_capacity(text.len() + width - used);
    out.push_str(text);
    out.push_str(&" ".repeat(width - used));
    out
}

// 测试与实现分开文件（同 render / md / settings_panel 惯例），避免本文件顶到行数门禁。
#[cfg(test)]
#[path = "overlay_tests.rs"]
mod tests;
