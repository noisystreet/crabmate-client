//! 工作区侧栏可视行构建：标题 / 目录树行 / 占位与反色高亮；左侧会话/工作区共列分发。
//!
//! 拆出本模块以控制 `render.rs` 单文件行数（fn-nloc 门禁 ≤ 920）；数据侧见
//! [`super::workspace_tree`]。

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::render::{SIDEBAR_HINT, sidebar_rows, truncate_display, workspace_basename};
use super::state::{Focus, UiState};
use super::workspace_tree::WsRow;

/// 工作区树聚焦时的按键提示（占位性质，超宽即裁）。
const WORKSPACE_HINT: &str = "↑↓选 Enter/→展开 ◀收起 r刷新 w会话";

/// 工作区侧栏标题：`工作区 <basename>`（未获取时占位）。
fn workspace_title(st: &UiState, width: usize) -> String {
    let name = st
        .workspace_path
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(workspace_basename)
        .filter(|n| !n.is_empty());
    match name {
        Some(n) => truncate_display(&format!("工作区: {n}"), width),
        None => truncate_display("工作区: (未获取)", width),
    }
}

/// 单棵树行文本：缩进 + 目录 `▾/▸`（文件占位空格对齐）+ 名称。
fn ws_row_text(row: &WsRow, width: usize) -> String {
    let pad = "  ".repeat(usize::from(row.indent));
    let glyph = if row.is_dir {
        if row.expanded { "▾ " } else { "▸ " }
    } else {
        "  "
    };
    truncate_display(&format!("{pad}{glyph}{}", row.name), width)
}

/// 追加一条左栏行（可选反色高亮）。
fn push_sidebar_line(rows: &mut Vec<Line<'static>>, text: String, fg: Color, highlight: bool) {
    let base = Style::new().fg(fg);
    let style = if highlight {
        base.add_modifier(Modifier::REVERSED)
    } else {
        base
    };
    rows.push(Line::from(Span::styled(text, style)));
}

/// 工作区侧栏内容行：首行标题，其后为扁平树（加载中/错误/空目录占位）。
fn workspace_sidebar_rows(st: &UiState, width: usize) -> Vec<Line<'static>> {
    let mut rows: Vec<Line<'static>> = Vec::new();
    let title_style = Style::new().fg(Color::White).add_modifier(Modifier::BOLD);
    rows.push(Line::from(Span::styled(
        workspace_title(st, width),
        title_style,
    )));
    let highlight_first = st.focus == Focus::Workspace && st.ws_cursor == 0;
    if !st.ws_ready {
        let (text, fg) = match st.ws_root_err.as_deref() {
            Some(e) => (
                truncate_display(&format!("（读取失败：{e}）"), width),
                Color::LightRed,
            ),
            None => (truncate_display("（加载中…）", width), Color::Gray),
        };
        push_sidebar_line(&mut rows, text, fg, highlight_first);
        return rows;
    }
    if st.ws_rows.is_empty() {
        push_sidebar_line(
            &mut rows,
            truncate_display("（空目录）", width),
            Color::Gray,
            highlight_first,
        );
        return rows;
    }
    let highlight = st.focus == Focus::Workspace;
    for (i, row) in st.ws_rows.iter().enumerate() {
        let fg = if row.loading {
            Color::Gray
        } else if row.is_dir {
            Color::LightCyan
        } else {
            Color::White
        };
        push_sidebar_line(
            &mut rows,
            ws_row_text(row, width),
            fg,
            highlight && i == st.ws_cursor,
        );
    }
    rows
}

/// 对左栏内容做窗口裁剪：标题固定，条目区尽量让选中行可见。
pub(super) fn sidebar_view<'a>(rows: &[Line<'a>], height: usize, selected: usize) -> Vec<Line<'a>> {
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

/// 左栏可视内容（会话 / 工作区树共列分发）：返回（行、光标、聚焦时提示文案）。
pub(super) fn sidebar_content(
    st: &UiState,
    width: usize,
) -> (Vec<Line<'static>>, usize, Option<&'static str>) {
    if st.focus == Focus::Workspace {
        (
            workspace_sidebar_rows(st, width),
            st.ws_cursor,
            Some(WORKSPACE_HINT),
        )
    } else {
        let hint = (st.focus == Focus::Sidebar).then_some(SIDEBAR_HINT);
        (sidebar_rows(st, width), st.selected, hint)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws_row(indent: u8, rel: &str, name: &str, is_dir: bool, expanded: bool) -> WsRow {
        WsRow {
            indent,
            rel: rel.to_string(),
            name: name.to_string(),
            is_dir,
            expanded,
            loading: false,
        }
    }

    fn ready_ws_state() -> UiState {
        let mut st = UiState::new();
        st.workspace_path = Some("/data/proj".to_string());
        st.ws_ready = true;
        st.ws_cursor = 2;
        st.ws_rows = vec![
            ws_row(0, "src", "src", true, false),
            ws_row(1, "src/lib.rs", "lib.rs", false, false),
            ws_row(0, "README.md", "README.md", false, false),
        ];
        st
    }

    #[test]
    fn workspace_title_shows_basename_or_placeholder() {
        let mut st = ready_ws_state();
        let rows = workspace_sidebar_rows(&st, 60);
        assert!(rows[0].to_string().contains("工作区: proj"));
        st.workspace_path = None;
        let rows = workspace_sidebar_rows(&st, 60);
        assert!(rows[0].to_string().contains("(未获取)"));
    }

    #[test]
    fn workspace_sidebar_rows_render_tree_with_glyph_and_indent() {
        let mut st = ready_ws_state();
        st.focus = Focus::Workspace;
        let rows = workspace_sidebar_rows(&st, 60);
        assert_eq!(rows.len(), 4, "标题 + 3 树行");
        assert!(rows[1].to_string().contains("▸ src"), "折叠目录带 ▸");
        assert!(rows[2].to_string().contains("lib.rs"), "缩进子行");
        assert!(!rows[2].to_string().contains("▸"));
        // 光标行（README.md，cursor=2 → 行 3）反色高亮
        assert!(
            rows[3].spans[0]
                .style
                .add_modifier
                .contains(Modifier::REVERSED),
            "工作区聚焦时光标行应反色"
        );
        assert!(
            !rows[1].spans[0]
                .style
                .add_modifier
                .contains(Modifier::REVERSED)
        );
    }

    #[test]
    fn workspace_sidebar_rows_loading_error_empty() {
        let mut st = UiState::new();
        st.focus = Focus::Workspace;
        st.workspace_path = None;
        let rows = workspace_sidebar_rows(&st, 60);
        assert!(rows[1].to_string().contains("加载中"));
        st.ws_root_err = Some("连接断开".to_string());
        let rows = workspace_sidebar_rows(&st, 60);
        assert!(rows[1].to_string().contains("读取失败"));
        assert!(rows[1].to_string().contains("连接断开"));
        st.ws_ready = true;
        let rows = workspace_sidebar_rows(&st, 60);
        assert!(rows[1].to_string().contains("空目录"));
    }

    #[test]
    fn workspace_row_highlight_requires_workspace_focus() {
        let st = ready_ws_state(); // focus 默认 Input
        let rows = workspace_sidebar_rows(&st, 60);
        assert!(
            !rows[3].spans[0]
                .style
                .add_modifier
                .contains(Modifier::REVERSED),
            "非 Workspace 焦点不反色"
        );
    }

    #[test]
    fn workspace_loading_row_is_dimmed() {
        let mut st = ready_ws_state();
        st.focus = Focus::Workspace;
        st.ws_cursor = 0; // 占位行不被反色，便于检查灰色前景
        st.ws_rows[1].loading = true;
        st.ws_rows[1].name = "\u{22ef}".to_string();
        let rows = workspace_sidebar_rows(&st, 60);
        assert_eq!(
            rows[2].spans[0].style.fg,
            Some(Color::Gray),
            "在途占位行应为灰色"
        );
    }

    #[test]
    fn sidebar_content_dispatches_sessions_vs_workspace() {
        let mut st = ready_ws_state();
        st.focus = Focus::Workspace;
        let (rows, cursor, hint) = sidebar_content(&st, 60);
        assert_eq!(cursor, st.ws_cursor);
        assert_eq!(rows.len(), 4);
        assert!(hint.is_some());
        st.focus = Focus::Sidebar;
        let (rows, _, hint) = sidebar_content(&st, 60);
        assert!(rows[0].to_string().contains("会话"));
        assert!(hint.is_some());
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
}
