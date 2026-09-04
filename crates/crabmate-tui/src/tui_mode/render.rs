//! 全屏 TUI 渲染：状态行 / 左栏会话 / 主区 transcript / 底栏输入。
//! 布局纯函数为主，便于单测。

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::state::{Focus, LineKind, UiState};

/// 低于此宽度隐藏左栏（等价 repl 布局）。
pub const SIDEBAR_MIN_WIDTH: u16 = 120;
/// 左栏宽度。
const SIDEBAR_WIDTH: u16 = 26;
const INPUT_PROMPT: &str = "crabmate> ";
const SIDEBAR_HINT: &str = "↑↓选 Enter用 n新建 r刷新";

/// 状态行展示信息（mod 层组装好的显示值，含 override 标记）。
pub struct StatusInfo {
    pub api_base: String,
    /// 生效模型；本地 override 时带 `*` 后缀。
    pub model: Option<String>,
    pub role: Option<String>,
    pub mode: Option<String>,
    pub running: bool,
    pub cancel_sent: bool,
}

fn kind_style(kind: LineKind) -> (String, Style) {
    match kind {
        LineKind::User => (
            "[你] ".to_string(),
            Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ),
        LineKind::Assistant => ("[助手] ".to_string(), Style::new().fg(Color::Green)),
        LineKind::Thinking => ("[思考] ".to_string(), Style::new().fg(Color::DarkGray)),
        LineKind::System => ("[!] ".to_string(), Style::new().fg(Color::Yellow)),
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

/// 把 transcript 转成可渲染的物理行；逻辑行首带前缀，仅作用于其第一物理行。
fn display_lines(st: &UiState, width: usize) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    for log in &st.lines {
        let (prefix, style) = kind_style(log.kind);
        let content_width = width.saturating_sub(prefix.chars().count());
        let mut first = true;
        for phys in wrap_physical(&log.text, content_width) {
            let mut spans: Vec<Span<'static>> = Vec::new();
            if first {
                spans.push(Span::styled(prefix.clone(), style));
            }
            spans.push(Span::styled(phys, style));
            out.push(Line::from(spans));
            first = false;
        }
    }
    out
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
    truncate_display(&parts.join(" | "), width)
}

/// 左栏内容行：首行标题，其后每会话一行（`>` 当前，`*` serve 活跃，选中高亮）。
fn sidebar_rows(st: &UiState, width: usize) -> Vec<Line<'static>> {
    let mut rows: Vec<Line<'static>> = Vec::new();
    let title = format!("会话 [{}]", st.sessions.len());
    rows.push(Line::from(Span::styled(
        truncate_display(&title, width),
        Style::new().fg(Color::Blue).add_modifier(Modifier::BOLD),
    )));
    if st.sessions.is_empty() {
        rows.push(Line::from(Span::styled(
            truncate_display("（空）发一条消息后出现", width),
            Style::new().fg(Color::DarkGray),
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

/// 渲染一帧。左栏显隐由事件循环按 `SIDEBAR_MIN_WIDTH` 写入 `st.sidebar_visible`。
pub fn draw(frame: &mut Frame, st: &UiState, info: &StatusInfo) {
    let area = frame.area();
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(area);
    let status_area = chunks[0];
    let body_area = chunks[1];
    let input_area = chunks[2];

    let status = Paragraph::new(status_text(
        info,
        st.conversation_id.as_deref(),
        status_area.width as usize,
    ));
    frame.render_widget(status, status_area);

    if st.sidebar_visible {
        let cols = Layout::horizontal([Constraint::Length(SIDEBAR_WIDTH), Constraint::Min(0)])
            .split(body_area);
        render_sidebar(frame, st, cols[0]);
        render_body(frame, st, cols[1]);
    } else {
        render_body(frame, st, body_area);
    }

    render_input(frame, st, input_area);
}

fn render_sidebar(frame: &mut Frame, st: &UiState, area: ratatui::layout::Rect) {
    if area.height == 0 {
        return;
    }
    let cols = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(area);
    let list_area = cols[0];
    let rows = sidebar_rows(st, area.width.saturating_sub(1) as usize);
    let shown = sidebar_view(&rows, list_area.height as usize, st.selected);
    frame.render_widget(Paragraph::new(shown), list_area);
    if cols[1].height > 0 && st.focus == Focus::Sidebar {
        let hint = Paragraph::new(Span::styled(SIDEBAR_HINT, Style::new().fg(Color::DarkGray)));
        frame.render_widget(hint, cols[1]);
    }
}

fn render_body(frame: &mut Frame, st: &UiState, area: ratatui::layout::Rect) {
    if st.lines.is_empty() {
        let hint = Paragraph::new("（空）输入消息开始对话 · Ctrl+C 退出 · Tab 切到会话列表");
        frame.render_widget(hint, area);
        return;
    }
    let mut rows = display_lines(st, (area.width as usize).saturating_sub(1));
    let rows_total = rows.len();
    let viewport = (area.height as usize).saturating_sub(1);
    let back = st.view_offset.min(rows_total.saturating_sub(viewport));
    let end = rows_total.saturating_sub(back);
    let start = end.saturating_sub(viewport);
    let shown = rows.split_off(start);
    frame.render_widget(Paragraph::new(shown), area);
}

/// 输入区可视内容与光标列：Input 聚焦时光标跟随（超宽水平滚动），
/// 否则显示内容开头（无光标）。
fn input_window(st: &UiState, width: usize) -> (String, usize) {
    let before = st.input_before_cursor();
    let full = format!("{INPUT_PROMPT}{}", st.current_input());
    let cursor_cell = if st.focus == Focus::Input {
        UnicodeWidthStr::width(INPUT_PROMPT) + UnicodeWidthStr::width(before.as_str())
    } else {
        0
    };
    visible_window(&full, cursor_cell, width)
}

fn render_input(frame: &mut Frame, st: &UiState, area: ratatui::layout::Rect) {
    let visible_w = area.width.saturating_sub(1) as usize;
    let (shown, shown_cursor) = input_window(st, visible_w);
    frame.render_widget(Paragraph::new(Line::from(Span::raw(shown))), area);
    if st.focus != Focus::Input {
        return;
    }
    let col = area
        .x
        .saturating_add(shown_cursor as u16)
        .min(area.x.saturating_add(area.width.saturating_sub(1)));
    frame.set_cursor_position((col, area.y));
}

/// 输入行水平窗口：内容不超宽原样返回；超宽时滚动窗口使光标保持可见。
/// 返回（可见文本，光标在可见文本中的显示列）。
fn visible_window(content: &str, cursor_cell: usize, width: usize) -> (String, usize) {
    let total = UnicodeWidthStr::width(content);
    if total <= width || width == 0 {
        return (content.to_string(), cursor_cell.min(total));
    }
    // 光标贴右缘前留 1 列：需要窗口起点 = cursor+1-width，再收尾 clamp 到 total-width。
    let start = cursor_cell
        .saturating_add(1)
        .saturating_sub(width)
        .min(total.saturating_sub(width));
    (
        cell_window(content, start, width),
        cursor_cell.saturating_sub(start),
    )
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
    fn status_line_lists_overrides() {
        let info = StatusInfo {
            api_base: "http://127.0.0.1:8080".into(),
            model: Some("gpt-x*".into()),
            role: Some("coder".into()),
            mode: Some("plan".into()),
            running: false,
            cancel_sent: false,
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
        };
        let s = status_text(&info, None, 200);
        assert!(s.contains("model deepseek"));
        assert!(!s.contains("mode "));
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
    fn visible_window_keeps_fit_content() {
        let (s, c) = visible_window("ab", 2, 10);
        assert_eq!(s, "ab");
        assert_eq!(c, 2);
    }

    #[test]
    fn visible_window_scrolls_to_cursor_on_right() {
        let (s, c) = visible_window("abcdefgh", 8, 4);
        assert_eq!(s, "efgh");
        assert_eq!(c, 4);
    }

    #[test]
    fn visible_window_wide_char_cursor() {
        // "crabmate> " =10 列 + 你好 =4 列，总宽 14；光标在末尾(14)，可视宽 6 → 滚到光标附近。
        let (s, c) = visible_window("crabmate> 你好", 14, 6);
        assert!(s.contains('好'));
        assert_eq!(c, 6);
    }

    #[test]
    fn input_window_follows_cursor_when_wide() {
        let mut st = UiState::new();
        for ch in "你好world".chars() {
            st.insert_char(ch);
        }
        let (shown, cursor) = input_window(&st, 6);
        // 超宽时窗口滚到光标附近：不再显示开头，光标列在可视区右缘。
        assert!(
            !shown.starts_with("crab"),
            "window should scroll to the cursor"
        );
        assert!(!shown.is_empty());
        assert_eq!(cursor, 6);
    }

    #[test]
    fn input_window_shows_head_when_not_focused() {
        let mut st = UiState::new();
        for ch in "你好world".chars() {
            st.insert_char(ch);
        }
        st.focus = Focus::Sidebar;
        let (shown, cursor) = input_window(&st, 6);
        assert!(shown.starts_with("crab"));
        assert_eq!(cursor, 0);
    }
}
