//! 全屏 TUI 渲染：状态行 / 左栏会话 / 主区 transcript / 底栏输入 /
//! 审批浮层。布局纯函数为主，便于单测。

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::md;
use super::state::{Focus, LineKind, UiState};

/// 低于此宽度隐藏左栏（等价 repl 布局）。
pub const SIDEBAR_MIN_WIDTH: u16 = 120;
/// 左栏宽度。
const SIDEBAR_WIDTH: u16 = 26;
const SIDEBAR_HINT: &str = "↑↓选 Enter用 n新建 r刷新";
/// composer 最多展开行数（多行输入）。
const MAX_COMPOSER_ROWS: usize = 4;
/// 审批浮层宽高上限。
const OVERLAY_MAX_WIDTH: u16 = 78;
const OVERLAY_HEIGHT: u16 = 8;

/// 区域色块（对齐 Desktop 面板观感：主体聊天深底 + 亮灰会话侧栏 +
/// 略亮输入条 + 底部蓝色状态栏）。
const STATUS_BG: Color = Color::Blue;
const STATUS_FG: Color = Color::White;
const SIDEBAR_BG: Color = Color::Indexed(240);
const CHAT_BG: Color = Color::Indexed(235);
const COMPOSER_BG: Color = Color::Indexed(237);
/// 顶栏背景（工作区名称；对齐 Desktop 顶部标题栏观感）。
const TOPBAR_BG: Color = Color::Indexed(238);

/// 状态行展示信息（mod 层组装好的显示值，含 override 标记）。
pub struct StatusInfo {
    pub api_base: String,
    /// 生效模型；本地 override 时带 `*` 后缀。
    pub model: Option<String>,
    pub role: Option<String>,
    pub mode: Option<String>,
    pub running: bool,
    pub cancel_sent: bool,
    /// transcript 回看行数（>0 时显示 ↑N）。
    pub view_offset: usize,
    /// 活跃搜索词（/find）。
    pub search_term: Option<String>,
    /// 搜索命中逻辑行数。
    pub search_total: usize,
}

/// 一条可渲染的 transcript 物理行（附所属逻辑行下标，供搜索锚点定位）。
pub struct BodyRow {
    pub log_index: usize,
    pub line: Line<'static>,
}

fn kind_style(kind: LineKind) -> (String, Style) {
    // 亮系前景（聊天区为深底色，保证可读）。
    match kind {
        LineKind::User => (
            "[你] ".to_string(),
            Style::new()
                .fg(Color::LightCyan)
                .add_modifier(Modifier::BOLD),
        ),
        LineKind::Assistant => ("[助手] ".to_string(), Style::new().fg(Color::LightGreen)),
        LineKind::Thinking => ("[思考] ".to_string(), Style::new().fg(Color::Gray)),
        LineKind::Tool => ("[工具] ".to_string(), Style::new().fg(Color::LightMagenta)),
        LineKind::System => ("[!] ".to_string(), Style::new().fg(Color::LightYellow)),
    }
}

/// 按显示宽度断行；`\n` 强制断行，控制字符丢弃。
fn wrap_physical(text: &str, width: usize) -> Vec<String> {
    let mut rows: Vec<String> = Vec::new();
    let mut row = String::new();
    let mut row_width = 0usize;
    for ch in text.chars() {
        if ch == '\n' {
            if !row.is_empty() {
                rows.push(std::mem::take(&mut row));
                row_width = 0;
            }
            continue;
        }
        if ch.is_control() {
            continue;
        }
        let cw = ch.width().unwrap_or(0);
        if !row.is_empty() && row_width + cw > width {
            rows.push(std::mem::take(&mut row));
            row_width = 0;
        }
        row.push(ch);
        row_width += cw;
    }
    if !row.is_empty() {
        rows.push(row);
    }
    if rows.is_empty() {
        rows.push(String::new());
    }
    rows
}

/// thinking 折叠行的单行预览：首行截断 + `…`（占位不超 content_width）。
fn fold_thinking(text: &str, content_width: usize) -> String {
    if content_width == 0 {
        return String::new();
    }
    let head = text.lines().next().unwrap_or("").trim();
    if head.is_empty() {
        return "…".to_string();
    }
    let budget = content_width.saturating_sub(2).max(1);
    let mut s = truncate_display(head, budget);
    s.push('…');
    s
}

/// 把 transcript 转成可渲染的物理行（含搜索高亮标记）；逻辑行首带前缀，
/// 仅作用于其第一物理行。thinking 折叠时只出一行预览。
fn body_rows(st: &UiState, width: usize) -> Vec<BodyRow> {
    let needle = st.search_term();
    let target = st.search_cursor;
    let mut out: Vec<BodyRow> = Vec::new();
    for (idx, log) in st.lines.iter().enumerate() {
        let (prefix, style) = kind_style(log.kind);
        let prefix_w = prefix.chars().count();
        let content_width = width.saturating_sub(prefix_w);
        let matched = needle.is_some_and(|n| log.text.to_lowercase().contains(n));
        let anchor_log = matched && target == Some(idx);
        let text = if log.kind == LineKind::Thinking && log.collapsed {
            fold_thinking(&log.text, content_width)
        } else {
            log.text.clone()
        };
        if log.kind == LineKind::Assistant {
            out.extend(assistant_body_rows(
                idx,
                &text,
                &prefix,
                style,
                content_width,
                matched,
                anchor_log,
            ));
        } else {
            out.extend(plain_body_rows(
                idx,
                &text,
                &prefix,
                style,
                content_width,
                matched,
                anchor_log,
            ));
        }
    }
    out
}

/// 非 Assistant 逻辑行 → 物理 BodyRow（thinking 折叠/用户/工具/系统行，纯文本）。
fn plain_body_rows(
    idx: usize,
    text: &str,
    prefix: &str,
    style: Style,
    content_width: usize,
    matched: bool,
    anchor_log: bool,
) -> Vec<BodyRow> {
    let mut rows = Vec::new();
    let mut first = true;
    for phys in wrap_physical(text, content_width) {
        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut row_style = style;
        if anchor_log && first {
            // 锚定命中行反色突出；其余命中行仅改前景。
            row_style = Style::new()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD);
        } else if matched {
            row_style = Style::new().fg(Color::Yellow);
        }
        if first {
            spans.push(Span::styled(prefix.to_string(), row_style));
        }
        spans.push(Span::styled(phys, row_style));
        rows.push(BodyRow {
            log_index: idx,
            line: Line::from(spans),
        });
        first = false;
    }
    rows
}

/// Assistant 逻辑行 → 全部物理 BodyRow（行内 markdown 样式 + 宽度折行 + 搜索高亮）。
fn assistant_body_rows(
    idx: usize,
    text: &str,
    prefix: &str,
    style: Style,
    content_width: usize,
    matched: bool,
    anchor_log: bool,
) -> Vec<BodyRow> {
    let styled = md::assistant_styled_text(text);
    let mut rows = Vec::new();
    for (row_no, row) in md::wrap_styled_chars(&styled, content_width)
        .into_iter()
        .enumerate()
    {
        let is_first = row_no == 0;
        let mut spans: Vec<Span<'static>> = Vec::new();
        if is_first {
            let pstyle =
                md::highlight_override(md::md_row_style(Style::new(), style), matched, anchor_log);
            spans.push(Span::styled(prefix.to_string(), pstyle));
        }
        spans.extend(md::styled_row_spans(
            &row,
            style,
            matched,
            anchor_log && is_first,
        ));
        rows.push(BodyRow {
            log_index: idx,
            line: Line::from(spans),
        });
    }
    rows
}

/// 测试辅助：兼容旧接口的纯行视图。
#[cfg(test)]
fn display_lines(st: &UiState, width: usize) -> Vec<Line<'static>> {
    body_rows(st, width).into_iter().map(|r| r.line).collect()
}

/// 按显示宽度截断（用于状态行），尾部补 `…`。
fn truncate_display(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut w = 0usize;
    for ch in text.chars() {
        let cw = ch.width().unwrap_or(0);
        if w + cw > width {
            out.push('…');
            break;
        }
        out.push(ch);
        w += cw;
    }
    out
}

/// 工作区展示名：路径 basename（根目录/空串回退原路径文本）。
fn workspace_basename(path: &str) -> String {
    let p = path.trim_end_matches('/');
    match p.rsplit('/').next().filter(|s| !s.is_empty()) {
        Some(base) => base.to_string(),
        None => path.to_string(),
    }
}

/// 顶栏内容：`工作区 <basename> · <完整路径>`（超宽截断，名称始终在左侧可见）。
fn top_text(st: &UiState, width: usize) -> String {
    let Some(path) = st
        .workspace_path
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    else {
        return truncate_display("工作区:（未获取）…", width);
    };
    let name = workspace_basename(path);
    truncate_display(&format!("工作区: {name} · {path}"), width)
}

/// 状态行内容（单行）。conv 为当前 conversation_id。
fn status_text(info: &StatusInfo, conv: Option<&str>, width: usize) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.push(format!("serve {}", info.api_base));
    if let Some(m) = info.model.as_deref() {
        parts.push(format!("model {m}"));
    }
    if let Some(r) = info.role.as_deref() {
        parts.push(format!("role {r}"));
    }
    if let Some(mode) = info.mode.as_deref() {
        parts.push(format!("mode {mode}"));
    }
    parts.push(match conv {
        Some(c) => format!("conv {c}"),
        None => "conv (new)".to_string(),
    });
    parts.push(match (info.running, info.cancel_sent) {
        (true, true) => "…取消中".to_string(),
        (true, false) => "● 运行中".to_string(),
        (false, _) => "○ 空闲".to_string(),
    });
    if let Some(term) = info.search_term.as_deref() {
        if info.search_total == 0 {
            parts.push(format!("find 「{term}」 无匹配"));
        } else {
            parts.push(format!("find 「{term}」 {}处", info.search_total));
        }
    }
    if info.view_offset > 0 {
        parts.push(format!("↑{}", info.view_offset));
    }
    truncate_display(&parts.join(" | "), width)
}

/// 左栏内容行：首行标题，其后每会话一行（`>` 当前，`*` serve 活跃，选中高亮）。
fn sidebar_rows(st: &UiState, width: usize) -> Vec<Line<'static>> {
    let mut rows: Vec<Line<'static>> = Vec::new();
    let title = format!("会话 [{}]", st.sessions.len());
    rows.push(Line::from(Span::styled(
        truncate_display(&title, width),
        Style::new().fg(Color::White).add_modifier(Modifier::BOLD),
    )));
    if st.sessions.is_empty() {
        rows.push(Line::from(Span::styled(
            truncate_display("（空）发一条消息后出现", width),
            Style::new().fg(Color::Gray),
        )));
        return rows;
    }
    for (i, row) in st.sessions.iter().enumerate() {
        let mark = if st.row_in_use(row) {
            ">"
        } else if st.active_session_id.as_deref() == Some(row.id.as_str()) {
            "*"
        } else {
            " "
        };
        let title = if row.title.trim().is_empty() {
            "(untitled)"
        } else {
            row.title.trim()
        };
        let text = truncate_display(&format!("{mark} {title}"), width);
        let style = match (st.focus, st.selected == i) {
            (Focus::Sidebar, true) => Style::new().add_modifier(Modifier::REVERSED),
            (Focus::Input, true) => Style::new().add_modifier(Modifier::BOLD),
            _ => Style::new(),
        };
        rows.push(Line::from(Span::styled(text, style)));
    }
    rows
}

/// 对左栏内容做窗口裁剪：标题固定，条目区尽量让选中行可见。
fn sidebar_view<'a>(rows: &[Line<'a>], height: usize, selected: usize) -> Vec<Line<'a>> {
    if height == 0 {
        return Vec::new();
    }
    if rows.len() <= height {
        return rows.to_vec();
    }
    let mut out = Vec::with_capacity(height);
    out.push(rows[0].clone());
    let item_max = height - 1;
    let items_len = rows.len() - 1;
    let sel = selected.min(items_len.saturating_sub(1));
    let start = if sel < item_max {
        0
    } else {
        sel + 1 - item_max
    };
    let end = (start + item_max).min(items_len);
    out.extend(rows[start + 1..end + 1].iter().cloned());
    out
}

/// 给一块区域整体铺背景色（先于文字绘制，杜绝段落内容不足时的底色缺口）。
fn paint_bg(frame: &mut Frame, rect: Rect, color: Color) {
    if rect.is_empty() {
        return;
    }
    frame.buffer_mut().set_style(rect, Style::new().bg(color));
}

/// 渲染一帧。对齐 Desktop：整屏 = 顶栏(工作区) + 主体 + 底部状态栏；主体内
/// 左会话列与右聊天列各自贯通，composer 只位于聊天列底部（会话列下方不出现输入框）。
pub fn draw(frame: &mut Frame, st: &UiState, info: &StatusInfo) {
    let area = frame.area();
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(area);
    let top_area = chunks[0];
    let body_area = chunks[1];
    let status_area = chunks[2];

    paint_bg(frame, top_area, TOPBAR_BG);
    let top = Paragraph::new(top_text(st, top_area.width as usize))
        .style(Style::new().fg(Color::White).add_modifier(Modifier::BOLD));
    frame.render_widget(top, top_area);

    if st.sidebar_visible {
        let cols = Layout::horizontal([Constraint::Length(SIDEBAR_WIDTH), Constraint::Min(0)])
            .split(body_area);
        paint_bg(frame, cols[0], SIDEBAR_BG);
        render_sidebar(frame, st, cols[0]);
        render_chat_column(frame, st, cols[1]);
    } else {
        render_chat_column(frame, st, body_area);
    }

    paint_bg(frame, status_area, STATUS_BG);
    let status = Paragraph::new(status_text(
        info,
        st.conversation_id.as_deref(),
        status_area.width as usize,
    ))
    .style(Style::new().fg(STATUS_FG).add_modifier(Modifier::BOLD));
    frame.render_widget(status, status_area);
}

/// 聊天列：上为主区消息（滚动 transcript），底部为 composer 输入区。
fn render_chat_column(frame: &mut Frame, st: &UiState, area: Rect) {
    let rows = composer_rows(st).min(area.height);
    let parts = Layout::vertical([Constraint::Min(1), Constraint::Length(rows)]).split(area);
    let body_area = parts[0];
    render_body(frame, st, body_area);
    if st.approval.is_some() {
        render_approval_overlay(frame, st, body_area);
    }
    render_input(frame, st, parts[1]);
}

/// composer 高度：随输入行数增长，最多 `MAX_COMPOSER_ROWS`（单行恒为 1）。
fn composer_rows(st: &UiState) -> u16 {
    st.input_line_count().clamp(1, MAX_COMPOSER_ROWS) as u16
}

fn render_sidebar(frame: &mut Frame, st: &UiState, area: Rect) {
    if area.height == 0 {
        return;
    }
    let cols = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(area);
    let list_area = cols[0];
    let rows = sidebar_rows(st, area.width.saturating_sub(1) as usize);
    let shown = sidebar_view(&rows, list_area.height as usize, st.selected);
    let list = Paragraph::new(shown).style(Style::new().fg(Color::White));
    frame.render_widget(list, list_area);
    // 提示行底色由整列 paint 保证；仅在聚焦左栏时显示按键提示。
    if cols[1].height > 0 && st.focus == Focus::Sidebar {
        let hint = Paragraph::new(Span::styled(SIDEBAR_HINT, Style::new().fg(Color::Gray)));
        frame.render_widget(hint, cols[1]);
    }
}

/// 空 transcript 时的快捷键引导。
const BODY_EMPTY_HINT: &str = "输入消息开始对话 · Alt+Enter 换行 · Ctrl+E 思考展开 · PgUp/PgDn 滚动 · /find 搜索 · Ctrl+C 退出 · /help";

fn render_body(frame: &mut Frame, st: &UiState, area: Rect) {
    paint_bg(frame, area, CHAT_BG);
    if st.lines.is_empty() {
        let hint = Paragraph::new(Span::styled(BODY_EMPTY_HINT, Style::new().fg(Color::White)));
        frame.render_widget(hint, area);
        return;
    }
    let rows = body_rows(st, (area.width as usize).saturating_sub(1));
    let rows_total = rows.len();
    let viewport = (area.height as usize).saturating_sub(1).max(1);
    let max_back = rows_total.saturating_sub(viewport);
    let back = if let Some(target) = st.search_cursor {
        // 搜索锚定：目标逻辑行置于视口上 1/3 处。
        match rows.iter().position(|r| r.log_index == target) {
            Some(pos) => {
                let want_start = pos.saturating_sub(viewport / 3);
                let end = (want_start + viewport).min(rows_total);
                rows_total.saturating_sub(end)
            }
            None => st.view_offset.min(max_back),
        }
    } else {
        st.view_offset.min(max_back)
    };
    let back = back.min(max_back);
    let end = rows_total.saturating_sub(back);
    let start = end.saturating_sub(viewport);
    let shown = rows
        .into_iter()
        .skip(start)
        .take(end.saturating_sub(start))
        .map(|r| r.line)
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(shown), area);
}

/// 审批浮层：命令预览 + 按键提示，置于聊天主体底部（composer 之上）。
fn render_approval_overlay(frame: &mut Frame, st: &UiState, area: Rect) {
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
    // 不能与底下聊天区残留文本/上一帧内容视觉重叠。
    frame.render_widget(Clear, rect);
    paint_bg(frame, rect, Color::Indexed(236));
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(Color::Yellow))
        .title(Span::styled(" 命令审批 ", Style::new().fg(Color::Yellow)));
    let inner = block.inner(rect);
    let preview = truncate_display(&ap.preview(), inner.width.saturating_sub(2) as usize);
    let mut lines: Vec<Line<'static>> = vec![Line::from(Span::styled(
        preview,
        Style::new().fg(Color::White).add_modifier(Modifier::BOLD),
    ))];
    if let Some(key) = ap
        .allowlist_key
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        lines.push(Line::from(Span::styled(
            format!("allowlist: {key}"),
            Style::new().fg(Color::Gray),
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "回合已暂停，等待你的决策",
        Style::new().fg(Color::Gray),
    )));
    lines.push(Line::from(Span::styled(
        "[Enter] 仅此一次  [a] 始终允许  [Esc/n] 拒绝  [Ctrl+C] 拒绝",
        Style::new().fg(Color::LightYellow),
    )));
    lines.push(Line::from(Span::styled(
        "Ctrl+C 只拒绝本命令；回合继续，取消需再按一次 Ctrl+C",
        Style::new().fg(Color::Gray),
    )));
    frame.render_widget(Paragraph::new(lines).block(block), rect);
}

/// composer 可视窗口：返回（可视各行、光标所在显示行、光标显示列、水平滚动起点）。
/// 仅显示光标附近的行；宽行按光标做水平滚动（与旧单行行为一致）。
fn composer_window(
    st: &UiState,
    width: usize,
    max_rows: usize,
) -> (Vec<String>, usize, usize, usize) {
    let lines = st.input_lines();
    let total = lines.len();
    let rows = total.min(max_rows).max(1);
    let (cursor_line, cursor_col) = st.cursor_line_col();
    // 光标行尽量保持在末行；行数溢出时向上滚动窗口。
    let top = (cursor_line + 1).saturating_sub(rows);
    let disp_row = cursor_line - top;
    let visible: Vec<String> = lines[top..top + rows].to_vec();
    let before: String = visible[disp_row].chars().take(cursor_col).collect();
    let cursor_cell = if st.focus == Focus::Input {
        UnicodeWidthStr::width(before.as_str())
    } else {
        0
    };
    let (hstart, shown_cursor) = horizontal_window(&visible[disp_row], cursor_cell, width);
    let shown: Vec<String> = visible
        .iter()
        .map(|l| cell_window(l, hstart, width))
        .collect();
    (shown, disp_row, shown_cursor, hstart)
}

/// 输入行水平窗口起点与光标在可见文本中的显示列。
fn horizontal_window(content: &str, cursor_cell: usize, width: usize) -> (usize, usize) {
    let total = UnicodeWidthStr::width(content);
    if total <= width || width == 0 {
        return (0, cursor_cell.min(total));
    }
    // 光标贴右缘前留 1 列：需要窗口起点 = cursor+1-width，再收尾 clamp 到 total-width。
    let start = cursor_cell
        .saturating_add(1)
        .saturating_sub(width)
        .min(total.saturating_sub(width));
    (start, cursor_cell.saturating_sub(start))
}

/// 从显示列 `start` 起取 `cells` 列的可见串；宽字符整体跳/取，不切半字。
fn cell_window(text: &str, start: usize, cells: usize) -> String {
    let mut out = String::new();
    let mut skipped = 0usize;
    let mut taken = 0usize;
    for ch in text.chars() {
        let cw = ch.width().unwrap_or(0);
        if skipped + cw <= start {
            skipped += cw;
            continue;
        }
        if taken + cw > cells {
            break;
        }
        out.push(ch);
        taken += cw;
    }
    out
}

fn render_input(frame: &mut Frame, st: &UiState, area: Rect) {
    if area.is_empty() {
        return;
    }
    paint_bg(frame, area, COMPOSER_BG);
    let visible_w = area.width.saturating_sub(1) as usize;
    let rows = (composer_rows(st) as usize).min(area.height as usize);
    let (shown, disp_row, shown_cursor, _hstart) = composer_window(st, visible_w, rows);
    for (i, text) in shown.iter().enumerate() {
        if i >= area.height as usize {
            break;
        }
        let row_area = Rect::new(area.x, area.y.saturating_add(i as u16), area.width, 1);
        let paragraph = Paragraph::new(Line::from(Span::raw(text.as_str())))
            .style(Style::new().fg(Color::White));
        frame.render_widget(paragraph, row_area);
    }
    if st.focus != Focus::Input {
        return;
    }
    let col = area
        .x
        .saturating_add(shown_cursor as u16)
        .min(area.x.saturating_add(area.width.saturating_sub(1)));
    let row = area
        .y
        .saturating_add(disp_row as u16)
        .min(area.y.saturating_add(area.height.saturating_sub(1)));
    frame.set_cursor_position((col, row));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crabmate_tui_core::SessionListItem;

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

    #[test]
    fn truncate_appends_ellipsis() {
        assert_eq!(truncate_display("abcdef", 4), "abcd…");
        assert_eq!(truncate_display("你好", 3), "你…");
        assert_eq!(truncate_display("", 4), "");
    }

    #[test]
    fn display_lines_prefix_only_first_physical() {
        let mut st = UiState::new();
        st.push_line(LineKind::Assistant, "一二三四五六七八九十");
        let lines = display_lines(&st, 4);
        assert!(lines.len() >= 2);
        assert_eq!(lines[0].spans.len(), 2);
        assert_eq!(lines[1].spans.len(), 1);
    }

    #[test]
    fn thinking_collapsed_shows_single_fold_row() {
        let mut st = UiState::new();
        st.stream_delta(LineKind::Thinking, "第一行思考\n第二行很长很长的思考内容");
        let rows = body_rows(&st, 60);
        assert_eq!(rows.len(), 1, "折叠后仅一行预览");
        assert!(rows[0].line.to_string().contains('…'));
    }

    #[test]
    fn thinking_expanded_wraps_multirow() {
        let mut st = UiState::new();
        st.stream_delta(LineKind::Thinking, "第一行思考\n第二行思考");
        st.toggle_thinking();
        let rows = body_rows(&st, 60);
        assert!(rows.len() >= 2, "展开后至少两行");
    }

    #[test]
    fn search_marks_and_anchors_rows() {
        let mut st = UiState::new();
        st.push_line(LineKind::Assistant, "hello world");
        st.push_line(LineKind::User, "bye");
        st.start_search("hello");
        let rows = body_rows(&st, 60);
        // 锚定行第一物理行以黄底反色突出
        assert!(
            rows[0]
                .line
                .spans
                .iter()
                .any(|s| s.style.bg == Some(Color::Yellow))
        );
        // 非命中行不带黄色前景
        assert!(
            rows[1]
                .line
                .spans
                .iter()
                .all(|s| s.style.fg != Some(Color::Yellow))
        );
    }

    #[test]
    fn tool_rows_carry_tool_kind_style() {
        let mut st = UiState::new();
        st.push_line(LineKind::Tool, "exec ✓");
        let rows = body_rows(&st, 60);
        assert!(rows[0].line.to_string().contains("[工具] exec ✓"));
    }

    #[test]
    fn workspace_basename_derives_name() {
        assert_eq!(workspace_basename("/data/proj"), "proj");
        assert_eq!(workspace_basename("/data/proj/"), "proj");
        assert_eq!(workspace_basename("/"), "/");
        assert_eq!(workspace_basename(""), "");
    }

    #[test]
    fn top_text_shows_name_and_full_path() {
        let mut st = UiState::new();
        st.workspace_path = Some("/data/proj".into());
        let s = top_text(&st, 80);
        assert!(s.contains("工作区: proj"));
        assert!(s.contains("/data/proj"));
    }

    #[test]
    fn status_line_lists_overrides() {
        let info = StatusInfo {
            api_base: "http://127.0.0.1:8080".into(),
            model: Some("gpt-x*".into()),
            role: Some("coder".into()),
            mode: Some("plan".into()),
            running: false,
            cancel_sent: false,
            view_offset: 0,
            search_term: None,
            search_total: 0,
        };
        let s = status_text(&info, Some("c1"), 200);
        assert!(s.contains("serve http://127.0.0.1:8080"));
        assert!(s.contains("model gpt-x*"));
        assert!(s.contains("role coder"));
        assert!(s.contains("mode plan"));
        assert!(s.contains("conv c1"));
        assert!(s.contains("○ 空闲"));
    }

    #[test]
    fn status_shows_running_state() {
        let info = StatusInfo {
            api_base: "http://x".into(),
            model: None,
            role: None,
            mode: None,
            running: true,
            cancel_sent: true,
            view_offset: 0,
            search_term: None,
            search_total: 0,
        };
        assert!(status_text(&info, None, 80).contains("…取消中"));
    }

    #[test]
    fn status_uses_serve_defaults_when_no_override() {
        let info = StatusInfo {
            api_base: "http://x".into(),
            model: Some("deepseek".into()),
            role: None,
            mode: None,
            running: false,
            cancel_sent: false,
            view_offset: 0,
            search_term: None,
            search_total: 0,
        };
        let s = status_text(&info, None, 200);
        assert!(s.contains("model deepseek"));
        assert!(!s.contains("mode "));
    }

    #[test]
    fn status_shows_search_and_view_back() {
        let info = StatusInfo {
            api_base: "http://x".into(),
            model: None,
            role: None,
            mode: None,
            running: false,
            cancel_sent: false,
            view_offset: 12,
            search_term: Some("grep".into()),
            search_total: 2,
        };
        let s = status_text(&info, None, 200);
        assert!(s.contains("find 「grep」"));
        assert!(s.contains("↑12"));
    }

    fn row(id: &str, title: &str, conv: Option<&str>) -> SessionListItem {
        SessionListItem {
            id: id.to_string(),
            title: title.to_string(),
            server_conversation_id: conv.map(str::to_string),
        }
    }

    #[test]
    fn sidebar_rows_marks_current_and_active() {
        let mut st = UiState::new();
        st.replace_sessions(vec![
            row("a", "Alpha", Some("c1")),
            row("b", "Beta", Some("c2")),
        ]);
        st.conversation_id = Some("c1".into());
        st.active_session_id = Some("b".into());
        let rows = sidebar_rows(&st, 20);
        assert_eq!(rows.len(), 3);
        assert!(rows[1].to_string().contains("> Alpha"));
        assert!(rows[2].to_string().contains("* Beta"));
    }

    #[test]
    fn sidebar_rows_untitled_and_truncate() {
        let mut st = UiState::new();
        st.replace_sessions(vec![row("a", "这是一个很长的会话标题标题", None)]);
        let rows = sidebar_rows(&st, 10);
        assert!(rows[1].to_string().contains("(untitled)") || rows[1].to_string().contains("…"));
    }

    #[test]
    fn sidebar_view_keeps_selected_visible() {
        let mut rows: Vec<Line<'static>> = vec![Line::from("header")];
        rows.extend((0..10).map(|i| Line::from(format!("item{i}"))));
        let v = sidebar_view(&rows, 4, 7);
        assert_eq!(v.len(), 4);
        assert!(v.last().unwrap().to_string().contains("item7"));
        assert!(v.first().unwrap().to_string().contains("header"));
    }

    #[test]
    fn sidebar_view_fits_without_window() {
        let mut rows: Vec<Line<'static>> = vec![Line::from("header")];
        rows.extend((0..3).map(|i| Line::from(format!("item{i}"))));
        let v = sidebar_view(&rows, 5, 1);
        assert_eq!(v.len(), 4);
    }

    #[test]
    fn cell_window_keeps_wide_char_boundary() {
        assert_eq!(cell_window("你好世界", 2, 4), "好世");
        assert_eq!(cell_window("你a好b", 3, 3), "好b");
    }

    #[test]
    fn visible_window_scrolls_to_cursor_on_right() {
        let (start, c) = horizontal_window("abcdefgh", 8, 4);
        assert_eq!(cell_window("abcdefgh", start, 4), "efgh");
        assert_eq!(c, 4);
    }

    #[test]
    fn visible_window_wide_char_cursor() {
        // 光标在末尾(14)，可视宽 6 → 滚到光标附近。
        let content = "crabmate> 你好";
        let (start, c) = horizontal_window(content, 14, 6);
        let shown = cell_window(content, start, 6);
        assert!(shown.contains('好'));
        assert_eq!(c, 6);
    }

    #[test]
    fn composer_window_multiline_follows_cursor_line() {
        let mut st = UiState::new();
        for ch in "a\nb\nc\nd\ne".chars() {
            st.insert_char(ch);
        }
        let (shown, disp_row, _, _) = composer_window(&st, 40, 4);
        assert_eq!(shown.len(), 4);
        // 光标在末行 e（行 4），窗口顶移 2，末行可见
        assert_eq!(shown.last().map(String::as_str), Some("e"));
        assert_eq!(disp_row, 3);
    }
}
