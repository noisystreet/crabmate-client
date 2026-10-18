//! 侧栏可视行构建：左栏会话列表 + 右栏工作区目录树（仿 Desktop：工作区在右侧，
//! 宽屏默认显示），含标题/树行/占位与反色高亮。
//!
//! 拆出本模块以控制 `render.rs` 单文件行数（fn-nloc 门禁 ≤ 920）；数据侧见
//! [`super::workspace_tree`]。

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::render::{SIDEBAR_HINT, sidebar_rows, truncate_display, workspace_basename};
use super::state::{Focus, UiState};
use super::workspace_tree::WsRow;

/// 工作区树（目录行）聚焦时的按键提示。宽度必须 ≤ 侧栏内宽
/// （`SIDEBAR_WIDTH` - 1 = 25 列），否则 `render_pane` 会直接裁掉尾部按键；中文按 2 列计。
const WORKSPACE_HINT: &str = "Enter展开 ←收 r刷 p池 w栏";
/// 文件行的提示：Enter 把相对路径插入输入框。
const WORKSPACE_FILE_HINT: &str = "Enter插路径 ←父 r刷 w栏";
/// 工作区未设置（顶栏路径为空）时的按键提示。
const WORKSPACE_UNSET_HINT: &str = "未设置：p 选项目池 · r刷";
/// 项目池「选择/切换工作区」子视图的按键提示。
const WORKSPACE_PICK_HINT: &str = "↑↓ Enter切换 r刷 Esc返回";

/// 工作区侧栏标题：`basename` 左对齐，`光标/总数` 右对齐（未就绪/空树时不显示计数）。
fn workspace_title(st: &UiState, width: usize) -> Line<'static> {
    let name = st
        .workspace_path
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(workspace_basename)
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "（未获取）".to_string());
    let total = st.ws_rows.iter().filter(|r| !r.placeholder).count();
    let counter = if st.ws_ready && total > 0 {
        let pos = st
            .ws_rows
            .iter()
            .take(st.ws_cursor.saturating_add(1))
            .filter(|r| !r.placeholder)
            .count();
        format!("{pos}/{total}")
    } else {
        String::new()
    };
    let counter_w = UnicodeWidthStr::width(counter.as_str());
    let name_w = if counter.is_empty() {
        width
    } else {
        width.saturating_sub(counter_w + 1)
    };
    let left = truncate_display(&name, name_w);
    let mut spans = vec![Span::styled(
        left.clone(),
        Style::new().fg(Color::White).add_modifier(Modifier::BOLD),
    )];
    if !counter.is_empty() {
        let pad = width
            .saturating_sub(UnicodeWidthStr::width(left.as_str()))
            .saturating_sub(counter_w);
        spans.push(Span::styled(" ".repeat(pad), Style::new()));
        spans.push(Span::styled(counter, Style::new().fg(Color::Gray)));
    }
    Line::from(spans)
}

/// 名称按显示宽度做尾部截断：保留开头 `…` 与末尾字符，深层目录仍能看清文件名。
fn truncate_name_tail(name: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(name) <= width {
        return name.to_string();
    }
    if width == 1 {
        return "…".to_string();
    }
    let budget = width - 1;
    let mut tail: Vec<char> = Vec::new();
    let mut used = 0usize;
    for ch in name.chars().rev() {
        let cw = ch.width().unwrap_or(0);
        if used + cw > budget {
            break;
        }
        tail.push(ch);
        used += cw;
    }
    let mut out = String::from("…");
    out.extend(tail.into_iter().rev());
    out
}

/// 单棵树行文本：缩进 + 目录 `▾/▸`（文件占位空格对齐）+ 名称（超宽保留尾部）。
fn ws_row_text(row: &WsRow, width: usize) -> String {
    let pad = "  ".repeat(usize::from(row.indent));
    let glyph = if row.is_dir {
        if row.expanded { "▾ " } else { "▸ " }
    } else {
        "  "
    };
    let prefix = format!("{pad}{glyph}");
    let prefix_w = UnicodeWidthStr::width(prefix.as_str());
    if prefix_w >= width {
        return truncate_display(&prefix, width);
    }
    format!(
        "{prefix}{}",
        truncate_name_tail(&row.name, width - prefix_w)
    )
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
    rows.push(workspace_title(st, width));
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
        let fg = if row.placeholder {
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
    let (start, end) = sidebar_window(rows.len(), height, selected);
    out.extend(rows[start + 1..end + 1].iter().cloned());
    out
}

/// 条目区（不含固定的标题行）的可见窗口 `[start, end)`。
///
/// 渲染裁剪与鼠标命中检测必须共用这一份：各算一遍就会在「列表够长且光标接近底部」时
/// 悄悄错开一行，而症状是点到的行和反色高亮不同行，极难察觉。
pub(super) fn sidebar_window(rows_len: usize, height: usize, selected: usize) -> (usize, usize) {
    let items_len = rows_len.saturating_sub(1);
    if height == 0 || items_len == 0 {
        return (0, 0);
    }
    if rows_len <= height {
        return (0, items_len);
    }
    let item_max = height - 1;
    let sel = selected.min(items_len - 1);
    let start = if sel < item_max {
        0
    } else {
        sel + 1 - item_max
    };
    (start, (start + item_max).min(items_len))
}

/// 侧栏列的内容区：去掉列内左留白 1 列与底部 1 行（按键提示）。
/// 渲染与鼠标命中检测共用。
pub(super) fn pane_list_area(area: Rect) -> Rect {
    if area.width <= 1 || area.height == 0 {
        return Rect::new(area.x, area.y, 0, 0);
    }
    let inner = Rect::new(area.x + 1, area.y, area.width - 1, area.height);
    Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(inner)[0]
}

/// 项目池「选择/切换工作区」内容行：首行标题，其后为候选项目（加载中/未启用/错误占位）。
fn workspace_pick_rows(st: &UiState, width: usize) -> Vec<Line<'static>> {
    let mut rows: Vec<Line<'static>> = Vec::new();
    rows.push(Line::from(Span::styled(
        truncate_display("项目池（选择/切换工作区）", width),
        Style::new().fg(Color::White).add_modifier(Modifier::BOLD),
    )));
    let Some(pick) = st.ws_pick.as_ref() else {
        return rows;
    };
    let highlight_first = st.focus == Focus::Workspace && st.ws_cursor == 0;
    if pick.loading {
        push_sidebar_line(
            &mut rows,
            truncate_display("（加载中…）", width),
            Color::Gray,
            highlight_first,
        );
    } else if let Some(e) = pick.error.as_deref() {
        push_sidebar_line(
            &mut rows,
            truncate_display(&format!("（失败：{e}）"), width),
            Color::LightRed,
            highlight_first,
        );
    } else if !pick.enabled {
        push_sidebar_line(
            &mut rows,
            truncate_display("（项目池未启用：服务端配置后可用）", width),
            Color::Gray,
            highlight_first,
        );
    } else if pick.projects.is_empty() {
        push_sidebar_line(
            &mut rows,
            truncate_display("（无可用项目）", width),
            Color::Gray,
            highlight_first,
        );
    } else {
        for (i, name) in pick.projects.iter().enumerate() {
            push_sidebar_line(
                &mut rows,
                truncate_display(name, width),
                Color::White,
                st.focus == Focus::Workspace && i == st.ws_cursor,
            );
        }
    }
    rows
}

/// 左栏会话可视内容：返回（行、光标、聚焦时提示文案）。
pub(super) fn sessions_content(
    st: &UiState,
    width: usize,
) -> (Vec<Line<'static>>, usize, Option<&'static str>) {
    let hint = (st.focus == Focus::Sidebar).then_some(SIDEBAR_HINT);
    (sidebar_rows(st, width), st.selected, hint)
}

/// 右栏工作区可视内容：返回（行、光标、聚焦时提示文案）。
/// 右栏宽屏默认显示；光标反色与按键提示仅在 `Focus::Workspace` 聚焦时出现。
pub(super) fn workspace_content(
    st: &UiState,
    width: usize,
) -> (Vec<Line<'static>>, usize, Option<&'static str>) {
    if st.ws_pick.is_some() {
        (
            workspace_pick_rows(st, width),
            st.ws_cursor,
            (st.focus == Focus::Workspace).then_some(WORKSPACE_PICK_HINT),
        )
    } else {
        let hint = (st.focus == Focus::Workspace).then(|| workspace_hint(st));
        (workspace_sidebar_rows(st, width), st.ws_cursor, hint)
    }
}

/// 按光标所在行选择提示文案（文件行强调 Enter 插入路径）。
fn workspace_hint(st: &UiState) -> &'static str {
    if st.workspace_path.is_none() {
        return WORKSPACE_UNSET_HINT;
    }
    match st.ws_rows.get(st.ws_cursor) {
        Some(row) if !row.placeholder && !row.is_dir => WORKSPACE_FILE_HINT,
        _ => WORKSPACE_HINT,
    }
}

#[cfg(test)]
mod tests {
    use super::super::render::SIDEBAR_WIDTH;
    use super::*;

    fn ws_row(indent: u8, rel: &str, name: &str, is_dir: bool, expanded: bool) -> WsRow {
        WsRow {
            indent,
            rel: rel.to_string(),
            name: name.to_string(),
            is_dir,
            expanded,
            loading: false,
            placeholder: false,
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
        assert!(rows[0].to_string().contains("proj"));
        assert!(!rows[0].to_string().contains("工作区:"), "标题不带前缀");
        st.workspace_path = None;
        let rows = workspace_sidebar_rows(&st, 60);
        assert!(rows[0].to_string().contains("未获取"));
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
        st.ws_rows[1].placeholder = true;
        st.ws_rows[1].name = "\u{22ef}".to_string();
        let rows = workspace_sidebar_rows(&st, 60);
        assert_eq!(
            rows[2].spans[0].style.fg,
            Some(Color::Gray),
            "在途占位行应为灰色"
        );
    }

    #[test]
    fn panes_content_workspace_always_rows_sessions_only_left() {
        let mut st = ready_ws_state();
        st.focus = Focus::Workspace;
        let (rows, cursor, hint) = workspace_content(&st, 60);
        assert_eq!(cursor, st.ws_cursor);
        assert_eq!(rows.len(), 4);
        assert!(hint.is_some());
        // 默认（Input）右栏同样渲染树行，只是不给按键提示
        st.focus = Focus::Input;
        let (rows, _, hint) = workspace_content(&st, 60);
        assert_eq!(rows.len(), 4, "工作区树默认可见");
        assert!(hint.is_none());
        st.focus = Focus::Sidebar;
        let (rows, _, hint) = sessions_content(&st, 60);
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

    #[test]
    fn pick_rows_loading_error_disabled() {
        let mut st = UiState::new();
        st.focus = Focus::Workspace;
        assert_eq!(workspace_pick_rows(&st, 60).len(), 1, "无 pick 态仅标题");
        st.ws_pick = Some(super::super::workspace_tree::WsPickState {
            loading: true,
            ..Default::default()
        });
        assert!(
            workspace_pick_rows(&st, 60)[1]
                .to_string()
                .contains("加载中")
        );
        st.ws_pick = Some(super::super::workspace_tree::WsPickState::default());
        assert!(
            workspace_pick_rows(&st, 60)[1]
                .to_string()
                .contains("项目池未启用")
        );
        st.ws_pick = Some(super::super::workspace_tree::WsPickState {
            enabled: true,
            error: Some("e".to_string()),
            ..Default::default()
        });
        assert!(workspace_pick_rows(&st, 60)[1].to_string().contains("失败"));
    }

    #[test]
    fn pick_rows_highlight_cursor() {
        let mut st = UiState::new();
        st.focus = Focus::Workspace;
        st.ws_cursor = 1;
        st.ws_pick = Some(super::super::workspace_tree::WsPickState {
            enabled: true,
            projects: vec!["proj-a".to_string(), "proj-b".to_string()],
            ..Default::default()
        });
        let rows = workspace_pick_rows(&st, 60);
        assert_eq!(rows.len(), 3);
        assert!(
            !rows[1].spans[0]
                .style
                .add_modifier
                .contains(Modifier::REVERSED),
            "非光标行不反色"
        );
        assert!(
            rows[2].spans[0]
                .style
                .add_modifier
                .contains(Modifier::REVERSED),
            "光标行反色"
        );
    }

    #[test]
    fn workspace_content_unset_and_pick_hints() {
        let mut st = UiState::new();
        st.focus = Focus::Workspace;
        let (_, _, hint) = workspace_content(&st, 60);
        assert!(hint.unwrap().contains("未设置"), "路径为空给未设置提示");
        st.workspace_path = Some("/data/proj".to_string());
        st.ws_ready = true;
        let (_, _, hint) = workspace_content(&st, 60);
        let hint = hint.unwrap();
        assert!(hint.contains("Enter展开"), "正常树给树提示");
        assert!(hint.contains('p'), "提示 p 切项目");
        assert!(hint.contains('w'), "提示 w 回会话栏");
        st.ws_pick = Some(super::super::workspace_tree::WsPickState::default());
        let (rows, _, hint) = workspace_content(&st, 60);
        assert!(rows[0].to_string().contains("项目池"));
        assert!(hint.unwrap().contains("Enter切换"));
    }

    #[test]
    fn hints_fit_sidebar_width() {
        // 提示占满侧栏内宽（`SIDEBAR_WIDTH` - 1），超出会被 `render_pane` 直接裁掉尾按键。
        for hint in [
            WORKSPACE_HINT,
            WORKSPACE_FILE_HINT,
            WORKSPACE_UNSET_HINT,
            WORKSPACE_PICK_HINT,
        ] {
            assert!(
                UnicodeWidthStr::width(hint) < SIDEBAR_WIDTH as usize,
                "提示超宽会被裁切：{hint}"
            );
        }
    }

    #[test]
    fn workspace_title_right_aligns_counter() {
        let mut st = ready_ws_state();
        st.focus = Focus::Workspace;
        st.ws_cursor = 1;
        let title = workspace_title(&st, 24).to_string();
        assert!(title.starts_with("proj"), "左对齐 basename：{title}");
        assert!(title.ends_with("2/3"), "右对齐光标/总数：{title}");
        assert_eq!(UnicodeWidthStr::width(title.as_str()), 24, "补足内宽");
    }

    #[test]
    fn deep_row_keeps_filename_tail() {
        let row = ws_row(2, "a/b/c/module_name.rs", "module_name.rs", false, false);
        let text = ws_row_text(&row, 14);
        assert!(UnicodeWidthStr::width(text.as_str()) <= 14);
        assert!(text.ends_with("name.rs"), "保留尾部文件名：{text}");
        assert!(text.contains('…'), "中段省略：{text}");
    }

    #[test]
    fn file_row_hint_mentions_insert() {
        let mut st = ready_ws_state();
        st.focus = Focus::Workspace;
        st.ws_cursor = 2; // README.md
        let (_, _, hint) = workspace_content(&st, 60);
        assert!(hint.unwrap().contains("插路径"));
    }
}
