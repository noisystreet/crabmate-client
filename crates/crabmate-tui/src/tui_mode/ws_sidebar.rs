//! 侧栏可视行构建：左栏会话列表 + 右栏工作区目录树（仿 Desktop：工作区在右侧，
//! 宽屏默认显示），含标题/树行/占位与反色高亮。
//!
//! 拆出本模块以控制 `render.rs` 单文件行数（fn-nloc 门禁 ≤ 920）；数据侧见
//! [`super::workspace_tree`]。

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use super::palette::Palette;
use super::render::{
    SIDEBAR_DELETE_HINT, SIDEBAR_FILTER_HINT, SIDEBAR_HINT, selected_row_in, truncate_display,
    workspace_basename,
};
use super::state::{Focus, UiState};
// `WsRow` 只给下面两个测试模块的 fixture 用（渲染路径只收 `&WsRow`，从不构造它）。
#[cfg(test)]
use super::workspace_tree::WsRow;
use super::ws_file_kind;
use super::ws_row::{GlyphPolicy, ws_row_clipped, ws_row_text};
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
    let basename = st
        .workspace_path
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(workspace_basename)
        .filter(|n| !n.is_empty());
    // git 括注只跟着**真工作区名**走：还没拿到工作区时不该显示分支 —— 那会读成
    // 「这个空壳在 main 上」。拼接规则与窄屏 `ws` 段同一份（[`super::status::workspace_with_git`]）。
    let defaults = st.serve_defaults.as_ref();
    let name = match basename {
        Some(base) => super::status::workspace_with_git(
            &base,
            defaults.and_then(|d| d.git_branch.as_deref()),
            defaults.and_then(|d| d.git_dirty.as_deref()),
        ),
        None => "（未获取）".to_string(),
    };
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

/// 追加一条侧栏行。
///
/// 选中只换**样式**（[`Palette::selection`]：底色，无色模式退回反色），不画左缘竖条：
/// 竖条要占一列，那一列会让文字在选中 / 未选中之间整体横移一格（每移一次光标视线就跳
/// 一下），还得从这一行的宽度预算里再扣一列 —— 而这一栏每行都与鼠标命中共用同一份预算。
///
/// `text` 由调用方按 `width` 截断（[`ws_row_text`] / [`truncate_display`]），这里不再重复截。
fn push_sidebar_line(
    rows: &mut Vec<Line<'static>>,
    text: String,
    fg: Color,
    highlight: bool,
    pal: &Palette,
) {
    let style = if highlight {
        pal.selection(fg)
    } else {
        Style::new().fg(fg)
    };
    rows.push(Line::from(Span::styled(text, style)));
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
        push_sidebar_line(&mut rows, text, fg, highlight_first, pal);
        return rows;
    }
    if st.ws_rows.is_empty() {
        push_sidebar_line(
            &mut rows,
            truncate_display("（空目录）", width),
            pal.muted,
            highlight_first,
            pal,
        );
        return rows;
    }
    let highlight = st.focus == Focus::Workspace;
    // 每帧取一次：策略是进程级的，逐行再问一遍既无用又多一次 `OnceLock` 命中。
    let policy = GlyphPolicy::from_env();
    for (i, row) in st.ws_rows.iter().enumerate() {
        // 目录 / 文件 / 占位三档颜色都取 palette：写死的 `LightCyan` / `White` 在浅色主题下
        // 会变成「浅底白字」，等于看不见。
        let fg = if row.placeholder {
            pal.muted
        } else {
            // 目录 / 代码 / 配置 / 文档 / 图片 / 其它：类型 → 槽的映射在 `Palette` 里，
            // 这里只说「这行是什么」。自己挑槽的话，同一类型迟早会在这里和那里两个颜色。
            pal.ws_entry_color(ws_file_kind::file_kind(&row.name, row.is_dir))
        };
        push_sidebar_line(
            &mut rows,
            ws_row_text(row, width, policy).0,
            fg,
            highlight && i == st.ws_cursor,
            pal,
        );
    }
    rows
}

/// 对左栏内容做窗口裁剪：标题固定，条目区尽量让选中行可见。
///
/// 溢出时在标题行尾补 `↑n` / `↓n`（被藏起来的条数）。放标题行而不是上下沿：条目区一行都
/// 不能浪费，标题行本来就在，且计数与裁剪共用 [`sidebar_window`] 的同一份窗口。
///
/// **收 owned 的行**：调用方（`render_pane`）拿到的是本帧刚构建的临时量，窗口覆盖全部时
/// 直接原样交还即可。旧签名收 `&[Line]`，那条路径只能 `to_vec()` —— 每帧把整列每个
/// `Span` 的 `String` 再复制一遍，而屏幕上只画得下二十来行。
pub(super) fn sidebar_view(
    rows: Vec<Line<'static>>,
    start: usize,
    end: usize,
    width: usize,
    pal: &Palette,
) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return Vec::new();
    }
    let items_len = rows.len() - 1;
    if start == 0 && end >= items_len {
        return rows;
    }
    let mut items = rows.into_iter();
    // 标题单独取出：它要按窗口补 `↑n` / `↓n`，而 `title_with_overflow` 返回的是**借用**
    // 标题 span 的 `Line`，标题一旦在函数末尾 drop 就借不动了 —— 所以这里把它转成 owned
    // （每帧只有一行，代价可忽略）。
    let Some(title) = items.next() else {
        return Vec::new();
    };
    let shown = title_with_overflow(&title, start, items_len.saturating_sub(end), width, pal);
    let mut out = Vec::with_capacity(end.saturating_sub(start) + 1);
    out.push(into_owned_line(shown));
    // 条目区 `[start, end)`：逐个**移动**（零克隆）。
    out.extend(items.skip(start).take(end.saturating_sub(start)));
    out
}

/// 把借用来的 `Line` 转成 `'static`（`Span` 内容 `Cow` → owned）。
///
/// 只用在窗口裁剪后的标题行上：它是本模块唯一需要「借用后存活到返回」的行。
fn into_owned_line(line: Line<'_>) -> Line<'static> {
    Line::from(
        line.spans
            .into_iter()
            .map(|span| Span::styled(span.content.into_owned(), span.style))
            .collect::<Vec<_>>(),
    )
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

/// 列内容区与可见窗口见 [`super::sidebar_window`]（渲染与命中共用同一份）。
pub(super) use super::sidebar_window::{pane_list_area, sidebar_window};

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
        );
    } else if let Some(e) = pick.error.as_deref() {
        push_sidebar_line(
            &mut rows,
            truncate_display(&format!("（失败：{e}）"), width),
            pal.err,
            highlight_first,
            pal,
        );
    } else if !pick.enabled {
        push_sidebar_line(
            &mut rows,
            truncate_display("（项目池未启用：服务端配置后可用）", width),
            pal.muted,
            highlight_first,
            pal,
        );
    } else if pick.projects.is_empty() {
        push_sidebar_line(
            &mut rows,
            truncate_display("（无可用项目）", width),
            pal.muted,
            highlight_first,
            pal,
        );
    } else {
        for (i, name) in pick.projects.iter().enumerate() {
            push_sidebar_line(
                &mut rows,
                truncate_display(name, width),
                pal.fg,
                st.focus == Focus::Workspace && i == st.ws_cursor,
                pal,
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
    st: &mut UiState,
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
    // 可见集合**只算一次**：行文本与光标位置吃同一份。
    let visible = st.visible_indices();
    // 光标是「可见列表里的位置」：过滤后行号已经不是会话下标了。
    let cursor = st.selected_visible_pos_in(&visible).unwrap_or(0);
    // 再把「不依赖缓存」的几样一次取完：`sync` 会可变借出 `st.session_rows`，那之后
    // 连只读借用 `st` 都不行（见 `render::selected_row_in` 的说明）。
    let pal = st.palette;
    let focus_sidebar = st.focus == Focus::Sidebar;
    let selected = st.selected;
    let mut rows: Vec<Line<'static>> = vec![session_title_line(st, width, &visible)];
    let placeholder = if st.sessions.is_empty() {
        Some("（空）发一条消息后出现")
    } else if visible.is_empty() {
        Some("（无匹配）")
    } else {
        None
    };
    if let Some(text) = placeholder {
        rows.push(placeholder_line(text, width, &pal));
        return (rows, cursor, hint);
    }
    // 行文本走缓存：每帧只比对源字段（零分配），只有变了的行才重新拼字符串
    // （见 `sidebar_window::SessionRows`）。屏幕只画得下二十来行，而这里是 n 行。
    let texts = st.session_rows.sync(
        &st.sessions,
        st.conversation_id.as_deref(),
        st.active_session_id.as_deref(),
        width,
        &visible,
    );
    for (pos, text) in texts.iter().enumerate() {
        rows.push(selected_row_in(
            &pal,
            focus_sidebar,
            text.clone(),
            selected == visible[pos],
        ));
    }
    (rows, cursor, hint)
}

/// 会话栏标题行：`会话 [n]`（过滤态写成 `[可见/总数]`），过滤词挂在同一行尾部。
///
/// 过滤词放标题行而不是另起一行：`sidebar_window` 假定「首行标题、其余都是条目」，
/// 多插一行会让滚动窗口与鼠标命中一起偏一行。
pub(super) fn session_title_line(st: &UiState, width: usize, visible: &[usize]) -> Line<'static> {
    let title = if st.session_filter_active() {
        format!("会话 [{}/{}]", visible.len(), st.sessions.len())
    } else {
        format!("会话 [{}]", st.sessions.len())
    };
    let title = match st.filter_term() {
        Some(term) => format!("{title} · filter: {term}"),
        None => title,
    };
    Line::from(Span::styled(
        truncate_display(&title, width),
        Style::new().fg(st.palette.fg).add_modifier(Modifier::BOLD),
    ))
}

/// 占位行（`（空）…` / `（无匹配）`）：`muted` 前景，按显示宽截断。
pub(super) fn placeholder_line(text: &str, width: usize, pal: &Palette) -> Line<'static> {
    Line::from(Span::styled(
        truncate_display(text, width),
        Style::new().fg(pal.muted),
    ))
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
/// 而提示行比行内多出缩进与字形两段预算（缩进 2 列/层 + `▸ ` 2 列），
/// 是这一列里唯一还能多放十几个字符的地方。键位本身仍可由 `/help` 与空会话首屏
/// 提示获得，所以这里优先让人认清「当前选的是哪个文件」。
fn workspace_hint_line(st: &UiState, width: usize) -> Line<'static> {
    let pal = &st.palette;
    if let Some(row) = st.ws_rows.get(st.ws_cursor)
        && !row.placeholder
        && ws_row_clipped(row, width, GlyphPolicy::from_env())
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
#[path = "ws_sidebar_file_kind_tests.rs"]
mod file_kind_tests;

#[cfg(test)]
#[path = "ws_sidebar_tests.rs"]
mod tests;
