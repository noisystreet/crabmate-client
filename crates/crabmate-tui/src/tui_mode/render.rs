//! 全屏 TUI 渲染：状态行 / 主区 transcript / 底栏输入。纯函数为主，便于单测。

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::state::{LineKind, UiState};

/// 状态行展示所需信息（从 main 层配置派生）。
#[derive(Debug, Clone, Copy)]
pub struct StatusInfo<'a> {
    pub api_base: &'a str,
    pub model: Option<&'a str>,
    pub role: Option<&'a str>,
    pub mode: Option<&'a str>,
    pub running: bool,
    pub cancel_sent: bool,
}

const INPUT_PROMPT: &str = "crabmate> ";

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

/// 状态行内容（单行）。
fn status_text(info: &StatusInfo, conv: Option<&str>, width: usize) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.push(format!("serve {}", info.api_base));
    if let Some(m) = info.model {
        parts.push(format!("model {m}"));
    }
    if let Some(r) = info.role {
        parts.push(format!("role {r}"));
    }
    if let Some(mode) = info.mode {
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

/// 渲染一帧。光标位置限制在输入区右侧，超出时收尾。
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

    if st.lines.is_empty() {
        let hint = Paragraph::new("（空）输入消息开始对话 · Ctrl+C 退出");
        frame.render_widget(hint, body_area);
    } else {
        let mut rows = display_lines(st, (body_area.width as usize).saturating_sub(1));
        let rows_total = rows.len();
        let viewport = (body_area.height as usize).saturating_sub(1);
        let back = st.view_offset.min(rows_total.saturating_sub(viewport));
        let end = rows_total.saturating_sub(back);
        let start = end.saturating_sub(viewport);
        let shown = rows.split_off(start);
        frame.render_widget(Paragraph::new(shown), body_area);
    }

    let input = st.current_input();
    let before = st.input_before_cursor();
    let full = format!("{INPUT_PROMPT}{input}");
    let visible_w = input_area.width.saturating_sub(1) as usize;
    // 光标用显示列（CJK=2 列），不是字符数。
    let cursor_cell =
        UnicodeWidthStr::width(INPUT_PROMPT) + UnicodeWidthStr::width(before.as_str());
    let (shown, shown_cursor) = visible_window(&full, cursor_cell, visible_w);
    let paragraph = Paragraph::new(Line::from(Span::raw(shown)));
    frame.render_widget(paragraph, input_area);
    let col = input_area.x.saturating_add(shown_cursor as u16).min(
        input_area
            .x
            .saturating_add(input_area.width.saturating_sub(1)),
    );
    frame.set_cursor_position((col, input_area.y));
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
        // 前缀只在首物理行：首行 2 个 span，续行 1 个 span
        assert!(lines.len() >= 2);
        assert_eq!(lines[0].spans.len(), 2);
        assert_eq!(lines[1].spans.len(), 1);
    }

    #[test]
    fn status_line_lists_overrides() {
        let info = StatusInfo {
            api_base: "http://127.0.0.1:8080",
            model: Some("gpt-x"),
            role: Some("coder"),
            mode: Some("plan"),
            running: false,
            cancel_sent: false,
        };
        let s = status_text(&info, Some("c1"), 200);
        assert!(s.contains("serve http://127.0.0.1:8080"));
        assert!(s.contains("model gpt-x"));
        assert!(s.contains("role coder"));
        assert!(s.contains("mode plan"));
        assert!(s.contains("conv c1"));
        assert!(s.contains("○ 空闲"));
    }

    #[test]
    fn status_shows_running_state() {
        let info = StatusInfo {
            api_base: "http://x",
            model: None,
            role: None,
            mode: None,
            running: true,
            cancel_sent: true,
        };
        assert!(status_text(&info, None, 80).contains("…取消中"));
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
        // 内容宽 8 > 宽 4；光标在末尾：窗口滚到 content 尾部，光标贴右缘。
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
}
