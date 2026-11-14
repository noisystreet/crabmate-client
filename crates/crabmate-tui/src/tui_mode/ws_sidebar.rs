//! 侧栏可视行构建：左栏会话列表 + 右栏工作区目录树（仿 Desktop：工作区在右侧，
//! 宽屏默认显示），含标题/树行/占位与反色高亮。
//!
//! 拆出本模块以控制 `render.rs` 单文件行数（fn-nloc 门禁 ≤ 920）；数据侧见
//! [`super::workspace_tree`]。

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use super::palette::Palette;
use super::render::{
    SIDEBAR_DELETE_HINT, SIDEBAR_FILTER_HINT, SIDEBAR_HINT, sidebar_rows, truncate_display,
    workspace_basename,
};
use super::state::{Focus, UiState};
use super::workspace_tree::WsRow;
use super::ws_truncate::middle_ellipsis;

/// 工作区树（目录行）聚焦时的按键提示。宽度必须 ≤ 侧栏内宽
/// （`SIDEBAR_WIDTH` - 1 = 25 列），否则 `render_pane` 会直接裁掉尾部按键；中文按 2 列计。
const WORKSPACE_HINT: &str = "Enter展开 ←收 r刷 p池 w栏";
/// 文件行的提示：Enter 把相对路径插入输入框。
const WORKSPACE_FILE_HINT: &str = "Enter插路径 ←父 r刷 w栏";
/// 编辑器视图下文件行的提示：Enter（或鼠标双击）打开这个文件。
///
/// 视图开着时 `ws_activate` 的调用方改走 `open_editor`，插路径的含义已经不成立 ——
/// 提示还写「插路径」就是骗人。同样 ≤ 25 列（本行 24 列）。
const WORKSPACE_FILE_OPEN_HINT: &str = "Enter/双击开 ←父 r刷 w栏";
/// 工作区未设置（根路径为空）时的按键提示。
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
        Style::new().fg(st.palette.fg).add_modifier(Modifier::BOLD),
    )];
    if !counter.is_empty() {
        let pad = width
            .saturating_sub(UnicodeWidthStr::width(left.as_str()))
            .saturating_sub(counter_w);
        spans.push(Span::styled(" ".repeat(pad), Style::new()));
        spans.push(Span::styled(counter, Style::new().fg(st.palette.muted)));
    }
    Line::from(spans)
}

/// 单棵树行文本 + 名称是否被省略（供提示行判断要不要补全名）。
///
/// 缩进 + 目录 `▾/▸`（文件用空格对齐）+ 名称；名称超宽时中段省略，见
/// [`middle_ellipsis`]：开头与扩展名都保留，避免同目录长名被压成一串 `…xx.rs`。
fn ws_row_text(row: &WsRow, width: usize) -> (String, bool) {
    let pad = "  ".repeat(usize::from(row.indent));
    let glyph = if row.is_dir {
        if row.expanded { "▾ " } else { "▸ " }
    } else {
        "  "
    };
    let prefix = format!("{pad}{glyph}");
    let prefix_w = UnicodeWidthStr::width(prefix.as_str());
    if prefix_w >= width {
        return (truncate_display(&prefix, width), true);
    }
    let room = width - prefix_w;
    let clipped = UnicodeWidthStr::width(row.name.as_str()) > room;
    (
        format!("{prefix}{}", middle_ellipsis(&row.name, room)),
        clipped,
    )
}

/// 该行在给定内宽下是否画不全。
///
/// 高亮行左缘多一个 `▎`（`push_sidebar_line` 按 `width - 1` 再截一次），所以
/// 「名称本身没超」也可能被那一列挤掉——两种都算画不全，提示行才需要补全名。
fn ws_row_clipped(row: &WsRow, width: usize) -> bool {
    let (text, name_clipped) = ws_row_text(row, width);
    name_clipped || UnicodeWidthStr::width(text.as_str()) > width.saturating_sub(1)
}

/// 追加一条侧栏行；`highlight` 时左缘加 `▎` 竖条 + 选中底色。
///
/// 竖条占 1 列，故高亮行按 `width - 1` 再截断一次：调用方按 `width` 截断过，直接拼上去
/// 会让末列被 `Paragraph` 裁掉（与 `fae763b` 的「预算必须恒等于绘制区宽度」同一类问题）。
fn push_sidebar_line(
    rows: &mut Vec<Line<'static>>,
    text: String,
    fg: Color,
    highlight: bool,
    pal: &Palette,
    width: usize,
) {
    if !highlight {
        rows.push(Line::from(Span::styled(text, Style::new().fg(fg))));
        return;
    }
    let body = truncate_display(&text, width.saturating_sub(1));
    rows.push(Line::from(vec![
        Span::styled("▎", Style::new().fg(pal.accent)),
        Span::styled(body, pal.selection(fg)),
    ]));
}

/// 工作区侧栏内容行：首行标题，其后为扁平树（加载中/错误/空目录占位）。
fn workspace_sidebar_rows(st: &UiState, width: usize) -> Vec<Line<'static>> {
    let mut rows: Vec<Line<'static>> = Vec::new();
    let pal = &st.palette;
    rows.push(workspace_title(st, width));
    let highlight_first = st.focus == Focus::Workspace && st.ws_cursor == 0;
    if !st.ws_ready {
        let (text, fg) = match st.ws_root_err.as_deref() {
            Some(e) => (
                truncate_display(&format!("（读取失败：{e}）"), width),
                pal.err,
            ),
            None => (truncate_display("（加载中…）", width), pal.muted),
        };
        push_sidebar_line(&mut rows, text, fg, highlight_first, pal, width);
        return rows;
    }
    if st.ws_rows.is_empty() {
        push_sidebar_line(
            &mut rows,
            truncate_display("（空目录）", width),
            pal.muted,
            highlight_first,
            pal,
            width,
        );
        return rows;
    }
    let highlight = st.focus == Focus::Workspace;
    for (i, row) in st.ws_rows.iter().enumerate() {
        // 目录 / 文件 / 占位三档颜色都取 palette：写死的 `LightCyan` / `White` 在浅色主题下
        // 会变成「浅底白字」，等于看不见。
        let fg = if row.placeholder {
            pal.muted
        } else if row.is_dir {
            pal.accent
        } else {
            pal.fg
        };
        push_sidebar_line(
            &mut rows,
            ws_row_text(row, width).0,
            fg,
            highlight && i == st.ws_cursor,
            pal,
            width,
        );
    }
    rows
}

/// 对左栏内容做窗口裁剪：标题固定，条目区尽量让选中行可见。
///
/// 溢出时在标题行尾补 `↑n` / `↓n`（被藏起来的条数）。放标题行而不是上下沿：条目区一行都
/// 不能浪费，标题行本来就在，且计数与裁剪共用 [`sidebar_window`] 的同一份窗口。
pub(super) fn sidebar_view<'a>(
    rows: &[Line<'a>],
    height: usize,
    selected: usize,
    width: usize,
    pal: &Palette,
) -> Vec<Line<'a>> {
    if height == 0 {
        return Vec::new();
    }
    if rows.len() <= height {
        return rows.to_vec();
    }
    let (start, end) = sidebar_window(rows.len(), height, selected);
    let mut out = Vec::with_capacity(height);
    // `end` 是条目区右开区间，标题行占 `rows[0]`，故条目总数为 `rows.len() - 1`。
    out.push(title_with_overflow(
        &rows[0],
        start,
        (rows.len() - 1).saturating_sub(end),
        width,
        pal,
    ));
    out.extend(rows[start + 1..end + 1].iter().cloned());
    out
}

/// 标题行 + 溢出计数（上下都没有隐藏时原样返回，仅多一次 clone）。
///
/// 逐 span 搬运而不是 `to_string()` 后重涂：标题可能是多 span 的（工作区名 `fg+BOLD`
/// + 右侧 `光标/总数` 的 `muted`），压平会让计数器在「树一长」时突然变色。
///
/// 预算分配顺序是「非空白内容优先」：先算出名称与计数器要占多少，剩下的才给右对齐填充。
/// 否则填充会刚好吃掉整份预算（`↓4` 只有 3 列，留白却有一大截），计数器反而被挤没。
fn title_with_overflow<'a>(
    title: &Line<'a>,
    above: usize,
    below: usize,
    width: usize,
    pal: &Palette,
) -> Line<'a> {
    if above == 0 && below == 0 {
        return title.clone();
    }
    let mut hint = String::new();
    if above > 0 {
        hint.push_str(&format!("↑{above}"));
    }
    if below > 0 {
        hint.push_str(&format!("↓{below}"));
    }
    // 标题原本按 `width` 截断过：这里要给它让出 `空格 + 计数` 的位置。
    let hint_w = UnicodeWidthStr::width(hint.as_str()) + 1;
    let mut room = width.saturating_sub(hint_w);
    // 非空白 span（名称、计数器）先占位，填充只能吃剩下的。
    let content_need: usize = title
        .spans
        .iter()
        .filter(|s| !s.content.trim().is_empty())
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
        .sum();
    let mut pad_room = room.saturating_sub(content_need);
    let mut spans: Vec<Span<'a>> = Vec::new();
    for span in &title.spans {
        if room == 0 {
            break;
        }
        let text = span.content.as_ref();
        let span_w = UnicodeWidthStr::width(text);
        if text.trim().is_empty() {
            let keep = span_w.min(pad_room);
            pad_room -= keep;
            room -= keep;
            if keep > 0 {
                spans.push(Span::styled(" ".repeat(keep), span.style));
            }
            continue;
        }
        if span_w <= room {
            room -= span_w;
            spans.push(span.clone());
            continue;
        }
        // 极端窄：内容也放不下，截断后收工（保留它自己的样式）。
        spans.push(Span::styled(truncate_display(text, room), span.style));
        room = 0;
    }
    spans.push(Span::styled(format!(" {hint}"), Style::new().fg(pal.muted)));
    Line::from(spans)
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
    let pal = &st.palette;
    rows.push(Line::from(Span::styled(
        truncate_display("项目池（选择/切换工作区）", width),
        Style::new().fg(pal.fg).add_modifier(Modifier::BOLD),
    )));
    let Some(pick) = st.ws_pick.as_ref() else {
        return rows;
    };
    let highlight_first = st.focus == Focus::Workspace && st.ws_cursor == 0;
    if pick.loading {
        push_sidebar_line(
            &mut rows,
            truncate_display("（加载中…）", width),
            pal.muted,
            highlight_first,
            pal,
            width,
        );
    } else if let Some(e) = pick.error.as_deref() {
        push_sidebar_line(
            &mut rows,
            truncate_display(&format!("（失败：{e}）"), width),
            pal.err,
            highlight_first,
            pal,
            width,
        );
    } else if !pick.enabled {
        push_sidebar_line(
            &mut rows,
            truncate_display("（项目池未启用：服务端配置后可用）", width),
            pal.muted,
            highlight_first,
            pal,
            width,
        );
    } else if pick.projects.is_empty() {
        push_sidebar_line(
            &mut rows,
            truncate_display("（无可用项目）", width),
            pal.muted,
            highlight_first,
            pal,
            width,
        );
    } else {
        for (i, name) in pick.projects.iter().enumerate() {
            push_sidebar_line(
                &mut rows,
                truncate_display(name, width),
                pal.fg,
                st.focus == Focus::Workspace && i == st.ws_cursor,
                pal,
                width,
            );
        }
    }
    rows
}

/// 左栏会话可视内容：返回（行、光标、键盘聚焦时提示文案）。
///
/// 提示文案只在 [`UiState::pane_hint_visible`] 成立时给（焦点是键盘带来的）：鼠标
/// 点进来的焦点不画那一行。
///
/// 删除确认态（`Delete` 已按一次、还没确认）时提示行换成确认文案：不弹浮层，
/// 但必须让人一眼看出「现在按 y 会真的删掉」。
pub(super) fn sessions_content(
    st: &UiState,
    width: usize,
) -> (Vec<Line<'static>>, usize, Option<Line<'static>>) {
    let hint = st.pane_hint_visible(Focus::Sidebar).then(|| {
        let text = if st.confirm_delete.is_some() {
            SIDEBAR_DELETE_HINT
        } else if st.session_filter_active() {
            // 过滤态要告诉人「怎么清」：v1 的清空入口是命令，左栏里退格没有含义。
            SIDEBAR_FILTER_HINT
        } else {
            SIDEBAR_HINT
        };
        hint_line(text, &st.palette)
    });
    (
        sidebar_rows(st, width),
        // 光标是「可见列表里的位置」：过滤后行号已经不是会话下标了。
        st.selected_visible_pos().unwrap_or(0),
        hint,
    )
}

/// 右栏工作区可视内容：返回（行、光标、键盘聚焦时提示行）。
/// 右栏宽屏默认显示；光标反色与按键提示仅在 `Focus::Workspace` 聚焦时出现
/// （提示还要是键盘带来的焦点，见 [`UiState::pane_hint_visible`]）。
pub(super) fn workspace_content(
    st: &UiState,
    width: usize,
) -> (Vec<Line<'static>>, usize, Option<Line<'static>>) {
    if st.ws_pick.is_some() {
        let hint = st
            .pane_hint_visible(Focus::Workspace)
            .then(|| hint_line(WORKSPACE_PICK_HINT, &st.palette));
        (workspace_pick_rows(st, width), st.ws_cursor, hint)
    } else {
        let hint = st
            .pane_hint_visible(Focus::Workspace)
            .then(|| workspace_hint_line(st, width));
        (workspace_sidebar_rows(st, width), st.ws_cursor, hint)
    }
}

/// 提示行统一走 `Line`：工作区栏在光标行画不全时要换成完整相对路径，不再只有静态文案
/// （`render_pane` 的 `hint` 因此收 `Line`；颜色一律 `muted`，与原有的提示行一致）。
fn hint_line(text: &'static str, pal: &Palette) -> Line<'static> {
    Line::from(Span::styled(text, Style::new().fg(pal.muted)))
}

/// 工作区栏的提示行：光标行被截断时改成「完整相对路径」，否则照旧给键位。
///
/// 终端没有 hover/tooltip 这类第二展示位，长文件名一旦被省略就再也看不到全名；
/// 而提示行比行内多出缩进与字形两段预算（缩进 2 列/层 + `▸ ` 2 列 + 高亮竖条 1 列），
/// 是这一列里唯一还能多放十几个字符的地方。键位本身仍可由 `/help` 与空会话首屏
/// 提示获得，所以这里优先让人认清「当前选的是哪个文件」。
fn workspace_hint_line(st: &UiState, width: usize) -> Line<'static> {
    let pal = &st.palette;
    if let Some(row) = st.ws_rows.get(st.ws_cursor)
        && !row.placeholder
        && ws_row_clipped(row, width)
    {
        return Line::from(Span::styled(
            middle_ellipsis(&row.rel, width),
            Style::new().fg(pal.muted),
        ));
    }
    hint_line(workspace_hint(st), pal)
}

/// 按光标所在行选择提示文案（文件行强调 Enter 插入路径；编辑器视图下强调打开）。
fn workspace_hint(st: &UiState) -> &'static str {
    if st.workspace_path.is_none() {
        return WORKSPACE_UNSET_HINT;
    }
    match st.ws_rows.get(st.ws_cursor) {
        Some(row) if !row.placeholder && !row.is_dir => {
            if st.editor.open {
                WORKSPACE_FILE_OPEN_HINT
            } else {
                WORKSPACE_FILE_HINT
            }
        }
        _ => WORKSPACE_HINT,
    }
}

#[cfg(test)]
mod tests {
    use super::super::pane_width::SIDEBAR_WIDTH;
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
        // 显式落深色配色：`UiState::new()` 走 `Palette::resolve`（会读 `NO_COLOR` 等环境），
        // 测试不能依赖运行环境是不是无色终端。
        st.set_palette(Palette::from_theme(Some("shadcn-dark")));
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
        // 光标行（README.md，cursor=2 → 行 3）：左缘竖条 + 选中底色（不再是反色）
        assert_eq!(rows[3].spans[0].content, "▎", "选中行左缘竖条");
        assert_eq!(
            rows[3].spans[1].style.bg,
            Some(st.palette.selection_bg),
            "选中行应为 selection_bg 底色"
        );
        assert_eq!(rows[1].spans.len(), 1, "未选中行不带竖条");
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
        assert_eq!(rows[3].spans.len(), 1, "非 Workspace 焦点不加竖条");
    }

    #[test]
    fn mono_palette_falls_back_to_reverse() {
        let mut st = ready_ws_state();
        st.set_palette(Palette::mono());
        st.focus = Focus::Workspace;
        let rows = workspace_sidebar_rows(&st, 60);
        assert!(
            rows[3].spans[1]
                .style
                .add_modifier
                .contains(Modifier::REVERSED),
            "无色模式下选中态必须退回反色（selection_bg 已被抹成 Reset）"
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
            Some(st.palette.muted),
            "在途占位行应为淡色"
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

    /// 鼠标点出来的焦点不画底部按键提示（点击后那行只会挤掉一行内容）。
    #[test]
    fn pointer_focus_hides_pane_hints() {
        let mut st = ready_ws_state();
        st.focus = Focus::Workspace;
        st.focus_by_pointer = true;
        let (_, _, hint) = workspace_content(&st, 60);
        assert!(hint.is_none(), "鼠标点进工作区时不显示按键提示");

        st.focus = Focus::Sidebar;
        let (_, _, hint) = sessions_content(&st, 60);
        assert!(hint.is_none(), "鼠标点进会话栏时不显示按键提示");

        // 键盘接管后（`on_key` 清掉这一位）提示照旧回来。
        st.focus_by_pointer = false;
        let (_, _, hint) = sessions_content(&st, 60);
        assert!(hint.is_some(), "键盘带来的焦点要显示提示");
    }

    /// 删除确认态：提示行必须换成确认文案，不能让「下一键就删」悄悄发生。
    #[test]
    fn sessions_hint_switches_to_confirm_copy_while_deleting() {
        let mut st = ready_ws_state();
        st.focus = Focus::Sidebar;
        let (_, _, hint) = sessions_content(&st, 60);
        let text = hint.expect("键盘聚焦要给提示").to_string();
        assert_eq!(text, SIDEBAR_HINT, "平时是按键提示");
        assert!(text.contains("Del"), "平时要提示删除键");

        st.confirm_delete = Some("s1".to_string());
        let (_, _, hint) = sessions_content(&st, 60);
        let text = hint.expect("确认态也要给提示").to_string();
        assert_eq!(text, SIDEBAR_DELETE_HINT, "确认态换确认文案");
        assert!(!text.contains("Del删"), "确认态不再重复按键说明");
    }

    #[test]
    fn sidebar_view_keeps_selected_visible() {
        let pal = Palette::from_theme(Some("shadcn-dark"));
        let mut rows: Vec<Line<'static>> = vec![Line::from("header")];
        rows.extend((0..10).map(|i| Line::from(format!("item{i}"))));
        let v = sidebar_view(&rows, 4, 7, 20, &pal);
        assert_eq!(v.len(), 4);
        assert!(v.last().unwrap().to_string().contains("item7"));
        assert!(v.first().unwrap().to_string().contains("header"));
    }

    #[test]
    fn sidebar_view_reports_hidden_rows_in_title() {
        let pal = Palette::from_theme(Some("shadcn-dark"));
        let mut rows: Vec<Line<'static>> = vec![Line::from("header")];
        rows.extend((0..10).map(|i| Line::from(format!("item{i}"))));
        // 高 4 行 = 标题 + 3 条；选中第 7 条 → 上方藏 5 条、下方藏 2 条。
        let v = sidebar_view(&rows, 4, 7, 20, &pal);
        let title = v[0].to_string();
        assert!(title.contains("↑5"), "上方隐藏计数：{title}");
        assert!(title.contains("↓2"), "下方隐藏计数：{title}");
        assert_eq!(v.len(), 4);
    }

    #[test]
    fn sidebar_view_fits_without_window() {
        let pal = Palette::from_theme(Some("shadcn-dark"));
        let mut rows: Vec<Line<'static>> = vec![Line::from("header")];
        rows.extend((0..3).map(|i| Line::from(format!("item{i}"))));
        let v = sidebar_view(&rows, 5, 1, 20, &pal);
        assert_eq!(v.len(), 4);
        assert!(!v[0].to_string().contains('↑'), "没有隐藏行时不该出现计数");
    }

    /// 带溢出计数时标题仍要保住多 span 样式（工作区名 BOLD / 计数器 muted）。
    #[test]
    fn overflow_hint_keeps_counter_style_and_drops_padding() {
        let pal = Palette::from_theme(Some("shadcn-dark"));
        // 模拟 `workspace_title`：[名称(BOLD)] [右对齐填充] [计数器(Gray)]
        let mut rows = vec![Line::from(vec![
            Span::styled(
                "proj".to_string(),
                Style::new().fg(Color::White).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ".repeat(13), Style::new()),
            Span::styled("2/3".to_string(), Style::new().fg(Color::Gray)),
        ])];
        rows.extend((0..5).map(|i| Line::from(format!("item{i}"))));
        // 高 2 行 = 标题 + 1 条；选中第 0 条 → 下方藏 4 条。
        let v = sidebar_view(&rows, 2, 0, 20, &pal);
        let head = &v[0];
        let text = head.to_string();
        assert!(text.contains("2/3"), "计数器该留下：{text}");
        assert!(text.contains("↓4"), "应带溢出计数：{text}");
        assert!(
            UnicodeWidthStr::width(text.as_str()) <= 20,
            "标题不得超宽：{text}"
        );
        let counter = head
            .spans
            .iter()
            .find(|s| s.content.contains("2/3"))
            .expect("计数器 span");
        assert_eq!(
            counter.style.fg,
            Some(Color::Gray),
            "计数器样式不能被压平成标题样式"
        );
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
        assert_eq!(rows[1].spans.len(), 1, "非光标行不带竖条");
        assert_eq!(rows[2].spans[0].content, "▎", "光标行带竖条");
        assert_eq!(
            rows[2].spans[1].style.bg,
            Some(st.palette.selection_bg),
            "光标行应为 selection_bg 底色"
        );
    }

    #[test]
    fn workspace_content_unset_and_pick_hints() {
        let mut st = UiState::new();
        st.focus = Focus::Workspace;
        let (_, _, hint) = workspace_content(&st, 60);
        assert!(
            hint.unwrap().to_string().contains("未设置"),
            "路径为空给未设置提示"
        );
        st.workspace_path = Some("/data/proj".to_string());
        st.ws_ready = true;
        let (_, _, hint) = workspace_content(&st, 60);
        let hint = hint.unwrap().to_string();
        assert!(hint.contains("Enter展开"), "正常树给树提示");
        assert!(hint.contains('p'), "提示 p 切项目");
        assert!(hint.contains('w'), "提示 w 回会话栏");
        st.ws_pick = Some(super::super::workspace_tree::WsPickState::default());
        let (rows, _, hint) = workspace_content(&st, 60);
        assert!(rows[0].to_string().contains("项目池"));
        assert!(hint.unwrap().to_string().contains("Enter切换"));
    }

    #[test]
    fn hints_fit_sidebar_width() {
        // 提示占满侧栏内宽（`SIDEBAR_WIDTH` - 1），超出会被 `render_pane` 直接裁掉尾按键。
        for hint in [
            WORKSPACE_HINT,
            WORKSPACE_FILE_HINT,
            WORKSPACE_FILE_OPEN_HINT,
            WORKSPACE_UNSET_HINT,
            WORKSPACE_PICK_HINT,
            SIDEBAR_HINT,
            SIDEBAR_DELETE_HINT,
        ] {
            assert!(
                UnicodeWidthStr::width(hint) < SIDEBAR_WIDTH as usize,
                "提示超宽会被裁切：{hint}"
            );
        }
    }

    #[test]
    fn file_row_hint_says_open_while_the_editor_view_is_open() {
        // `ready_ws_state` 的光标停在 README.md（文件行）
        let mut st = ready_ws_state();
        st.focus = Focus::Workspace;
        assert_eq!(workspace_hint(&st), WORKSPACE_FILE_HINT);
        st.editor.open_file("README.md", 0);
        assert_eq!(
            workspace_hint(&st),
            WORKSPACE_FILE_OPEN_HINT,
            "编辑器视图下 Enter 是打开，提示不能还写「插路径」"
        );
        // 目录行仍是展开提示：视图开着也只是文件行的语义变了
        st.ws_cursor = 0;
        assert_eq!(workspace_hint(&st), WORKSPACE_HINT);
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
        let (text, clipped) = ws_row_text(&row, 14);
        assert!(UnicodeWidthStr::width(text.as_str()) <= 14);
        assert!(text.ends_with("name.rs"), "保留尾部文件名：{text}");
        assert!(text.contains('…'), "中段省略：{text}");
        assert!(clipped, "名称超宽要报告给提示行");
    }

    /// 中段省略的核心收益：窄栏下开头与扩展名同时在，同目录长名才区分得开。
    #[test]
    fn narrow_row_keeps_name_head_and_extension() {
        let row = ws_row(
            2,
            "src/api/crabmate_super_long_module_name.rs",
            "crabmate_super_long_module_name.rs",
            false,
            false,
        );
        let (text, clipped) = ws_row_text(&row, 20);
        assert!(UnicodeWidthStr::width(text.as_str()) <= 20, "{text}");
        assert!(clipped);
        assert!(!text.contains("crabmate_super_long"), "整体已超宽：{text}");
        assert!(text.contains("…"), "{text}");
        // 缩进 + glyph 之后名称只剩 14 列，头尾都要有内容。
        assert!(text.contains("crab"), "保留名称开头：{text}");
        assert!(text.ends_with(".rs"), "保留扩展名：{text}");
    }

    /// 光标行画不全时，提示行改显示完整相对路径（终端没有 hover，这是唯一的第二展示位）。
    #[test]
    fn clipped_cursor_row_puts_full_rel_on_hint_line() {
        let mut st = ready_ws_state();
        st.focus = Focus::Workspace;
        st.ws_cursor = 2;
        st.ws_rows[2].rel = "src/deep/nested/very_long_module_name.rs".to_string();
        st.ws_rows[2].name = "very_long_module_name.rs".to_string();
        st.ws_rows[2].indent = 3;
        let (_, _, hint) = workspace_content(&st, SIDEBAR_WIDTH as usize - 2);
        let hint = hint.expect("聚焦要提示").to_string();
        // 提示行比行内多出缩进 + 字形 + 高亮竖条，所以放下的是「路径」而不是裸文件名；
        // 仍超过列宽时继续省略，但首尾都给出来（这里放得下 `src/deep/` 前缀）。
        assert!(hint.starts_with("src/deep/"), "提示行给出相对路径：{hint}");
        assert!(hint.ends_with("module_name.rs"), "{hint}");
        assert_ne!(hint, WORKSPACE_FILE_HINT, "被截断时不再让位给键位提示");
    }

    /// 名字放得下时提示行保持键位，别把提示位长期占掉。
    #[test]
    fn fitting_cursor_row_keeps_key_hint() {
        let mut st = ready_ws_state();
        st.focus = Focus::Workspace;
        st.ws_cursor = 2; // README.md，宽度充裕
        let (_, _, hint) = workspace_content(&st, SIDEBAR_WIDTH as usize - 2);
        let hint = hint.expect("聚焦要提示").to_string();
        assert_eq!(hint, WORKSPACE_FILE_HINT, "{hint}");
    }

    #[test]
    fn file_row_hint_mentions_insert() {
        let mut st = ready_ws_state();
        st.focus = Focus::Workspace;
        st.ws_cursor = 2; // README.md
        let (_, _, hint) = workspace_content(&st, 60);
        assert!(hint.unwrap().to_string().contains("插路径"));
    }
}
