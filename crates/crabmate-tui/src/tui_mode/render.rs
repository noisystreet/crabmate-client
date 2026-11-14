//! 全屏 TUI 渲染：状态行 / 左栏会话 / 右栏工作区目录树（仿 Desktop 默认显示）/
//! 主区 transcript / 底栏输入 / 审批浮层。布局纯函数为主，便于单测。

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use unicode_width::UnicodeWidthStr;

use super::layout::panes_with_lanes;
use super::links::RowLink;
use super::md;
use super::palette::{Palette, with_row_style};
use super::settings_panel::PanelContent;
use super::state::{Focus, LineKind, LogLine, ToolState, UiState};
use super::status::{SPINNER_FRAMES, status_line};
use super::status_stack;
use super::ws_sidebar::{pane_list_area, sessions_content, sidebar_view, workspace_content};

// 浮层（审批 / 设置面板）拆到 `overlay.rs`：它们自成一套模板，也把本文件的行数
// 从 920 门禁线附近拉开，给后续 B4 / B5 的改动留余量。
#[path = "overlay.rs"]
pub(super) mod overlay;
use overlay::render_approval_overlay;
// 面板内容必须按弹层内宽生成（否则居中后错位），所以把同一个矩形计算暴露给
// `mod.rs` 的 `settings_content(...)` 调用点。
pub(crate) use overlay::settings_panel_content_width;

/// 低于此宽度隐藏左右栏（等价 repl 布局）。
pub const SIDEBAR_MIN_WIDTH: u16 = 120;
// 正文预算换算与两栏分隔线拆到 `pane_render`（本文件贴着 920 行/文件的 `fn-nloc-ratchet`
// 门禁）。这里 re-export，调用点（`mod.rs` 与各测试）仍写 `render::chat_body_width`。
pub(super) use super::pane_render::{chat_body_width, paint_pane_dividers};
/// 左栏会话栏聚焦时的按键提示。宽度必须 ≤ 侧栏内宽（`SIDEBAR_WIDTH` - 1 = 25 列），
/// 否则 `render_pane` 会直接裁掉尾部的键；中文按 2 列计。
pub(super) const SIDEBAR_HINT: &str = "↑↓ Enter用 Del删 n新 r刷";
/// 过滤态下的侧栏提示：清过滤只能走命令（v1 没有「左栏内退格清空」这个入口）。
pub(super) const SIDEBAR_FILTER_HINT: &str = "↑↓ Enter用 · /conv filter off 清过滤";
/// 删除确认态的提示（`Delete` 已按一次）：y / Enter 删，Esc / n 取消。
///
/// 与 Desktop 的确认框同语义，但借用侧栏底部提示行，不抢焦点、不开浮层。
pub(super) const SIDEBAR_DELETE_HINT: &str = "[y] 删除 [Esc] 取消";
/// composer 最多展开行数（多行输入）。
const MAX_COMPOSER_ROWS: usize = 4;
// 配色见 [`Palette`]：状态栏 / 侧栏 / composer 用 `chrome_bg`，聊天区用 `bg`；正文 / 次要 / 强调 = `fg` / `muted` / `accent`（随 Desktop 主题）。

// `StatusInfo`（状态栏 / 状态栈的显示快照）单独成文件：它是纯数据 + 口径文档，与「怎么画」
// 无关；而 `render.rs` 正贴着 920 行的文件门禁 —— `ws` 段（#425）加一个字段就撞线。
// `pub use` 保持 `render::StatusInfo` 这条既有路径不变，调用点零改动。
#[path = "status_view.rs"]
mod status_view;
pub use status_view::StatusInfo;

// `BodyRow`（物理行 + 链接热区）单独成文件：`render.rs` 贴着 920 行文件门禁，
// 而它只是数据结构与命中查询，与「怎么画」关系松散（与下面的 `prefix` 同一手法）。
#[path = "body_row.rs"]
mod body_row;
pub use body_row::BodyRow;

// 前缀（文本 / 显示列数 / 配色）单独成文件：前缀文本同时是折行预算的一部分，三件事
// 必须一起改。与 `overlay` 同一布局（`#[path]` 子模块），`mod.rs` 不必为此多一行 ——
// 它已贴着 920 行文件门禁。
#[path = "prefix.rs"]
mod prefix;
pub(crate) use prefix::{TOOL_PROGRESS_MARK, prefix_for, prefix_style, prefix_width};
// 这两个只被 `render_tests` 直接引用（生产路径经 `prefix_for` 间接用到），故只在测试
// 构建里重导出：无条件 `pub(crate) use` 会让非测试构建报 unused import。
#[cfg(test)]
pub(crate) use prefix::{TOOL_PREFIX_PROGRESS, kind_prefix};

/// 内容基色（工具行按结果另算，见 [`tool_content_style`]）。
fn kind_style(kind: LineKind, pal: &Palette) -> Style {
    match kind {
        LineKind::User => Style::new().fg(pal.accent).add_modifier(Modifier::BOLD),
        LineKind::Assistant => Style::new().fg(pal.fg),
        LineKind::Thinking => Style::new().fg(pal.muted),
        LineKind::Tool => Style::new().fg(pal.muted),
        // 中性档：这一档混有真失败与「已加载 3 条」类通报，不能按结果判色（详见 `prefix::kind_prefix`）。
        LineKind::System => Style::new().fg(pal.muted),
        // 过程摘要行：与工具行同档色、不抢注意力（它只说「这里折了几步」）。
        LineKind::Process => Style::new().fg(pal.info),
        // 技能行：正文按常规正文色（名称要读得清），识别度交给 accent 前缀 `✦ `。
        LineKind::Skill => Style::new().fg(pal.fg),
    }
}

/// thinking 折叠行的单行预览：`▸` 标记 + 首行截断 + `…`（占位不超 content_width）。
///
/// 只产出**静态** `▸`：生成中的动画由渲染层 [`with_progress_mark`] 替换标记，
/// 这样 body memo 不会因为换帧而失效（否则停顿期每 120ms 就要全量重建一次）。
fn fold_thinking(text: &str, content_width: usize) -> String {
    const MARK: &str = "▸ ";
    // 按显示列算预算，不用字节数：`▸` 是 3 字节但只有 2 列。
    let mark_w = UnicodeWidthStr::width(MARK);
    // 放不下「标记 + 至少 1 列正文 + 省略号」时只留标记，避免渲染出超宽行。
    // `truncate_display` 现在保证结果 ≤ 预算（`…` 已算在预算内），故这里只需给结尾那个
    // `…` 留 1 列：MARK + shown + `…` ≤ mark_w + (content_width - mark_w - 1) + 1。
    // （旧实现要留 2 列，因为截断结果可能比预算宽 1 列；修掉后正文多出 1 列可用。）
    let body_budget = content_width.saturating_sub(mark_w + 1);
    if body_budget == 0 {
        return MARK.trim_end().to_string();
    }
    let head = text.lines().next().unwrap_or("").trim();
    let shown = truncate_display(head, body_budget);
    format!("{MARK}{shown}…")
}

/// 末行「活跃进度」的 spinner 帧：仅在「生成中 + 未取消 + 末行是折叠 Thinking
/// 或进行中的工具行」时给出。展开态（Ctrl+E）正文本身在变，不叠加动画。
fn progress_mark_frame(st: &UiState, info: &StatusInfo) -> Option<usize> {
    let live = st.lines.last().is_some_and(|l| match l.kind() {
        LineKind::Thinking => l.is_collapsed(),
        // 结束的行（含成败未知）不再是「进行中」，不加 spinner。
        LineKind::Tool => l.tool_state().is_running(),
        _ => false,
    });
    (info.running && !info.cancel_sent && live).then_some(info.spinner)
}

/// 把末行的静态进度标记换成 spinner 帧字符（保留原 span 样式）。
///
/// 两种落点：
/// - 折叠思考行：标记在**内容** span 开头（`▸ `），故只动 `spans[1]`，避免误改
///   正文里恰好出现的 `▸`；极窄宽度下 [`fold_thinking`] 只输出单个 `▸`（无空格），
///   此时 `strip_prefix` 不命中，自然保留标记而不是渲染出超宽行。
/// - 进行中工具行：进度位在**消息前缀**末尾（`工具 ⠿ `），故只处理有成对
///   `(前缀, 内容)` span 的第一物理行，不会碰到正文。
///
/// 两种标记都是「1 列字形 + 空格」，与 spinner 帧等宽，换帧不改变行宽。
fn with_progress_mark(line: Line<'static>, frame: usize) -> Line<'static> {
    const THINK_MARK: &str = "▸ ";
    let glyph = SPINNER_FRAMES[frame % SPINNER_FRAMES.len()];
    let mut spans: Vec<Span<'static>> = line.spans;
    if let Some(span) = spans.get_mut(1)
        && let Some(rest) = span.content.as_ref().strip_prefix(THINK_MARK)
    {
        *span = Span::styled(format!("{glyph} {rest}"), span.style);
        return Line::from(spans);
    }
    if spans.len() >= 2
        && let Some(span) = spans.first_mut()
        && let Some(head) = span.content.as_ref().strip_suffix(TOOL_PROGRESS_MARK)
    {
        *span = Span::styled(format!("{head}{glyph} "), span.style);
    }
    Line::from(spans)
}
/// 单条逻辑行的渲染上下文（前缀/基色/宽度/搜索态/配色），把 10 来个参数收成一个
/// 结构体：`plain` / `assistant` 两条渲染路径共用，也避免超长参数表。
struct RowCtx<'a> {
    idx: usize,
    kind: LineKind,
    tool_state: ToolState,
    text: &'a str,
    prefix: &'static str,
    prefix_style: Style,
    style: Style,
    content_width: usize,
    matched: bool,
    anchor_log: bool,
    pal: &'a Palette,
}

/// 把 transcript 转成可渲染的物理行（含搜索高亮标记）；逻辑行首带前缀，
/// 仅作用于其第一物理行。thinking 折叠时只出一行预览。
///
/// 全量重建（**仅测试**的参照实现）：生产路径走 `body_cache` 的行级缓存，
/// 这里保留是为了给缓存写「结果必须与全量重建一致」的等价性断言。
#[cfg(test)]
pub(crate) fn build_body_rows(st: &UiState, width: usize) -> Vec<BodyRow> {
    let needle = st.search_term();
    let target = st.search_cursor;
    let pal = &st.palette;
    let mut out: Vec<BodyRow> = Vec::new();
    // 顶部分页提示（`/older`）：与 `body_cache` 同源，保证「缓存 vs 全量重建」逐行一致。
    if let Some(row) = super::body_cache::history_banner_row(st, width) {
        out.push(row);
    }
    // 块间分隔：kind 变化即为块边界，画极暗满宽细线（对齐 Desktop 块间距观感），
    // 同类相邻（连续工具行）保持紧凑；`prev` 存 (类别, 前缀宽度) 供分隔线缩进。
    let mut prev: Option<(LineKind, usize)> = None;
    for (idx, log) in st.lines.iter().enumerate() {
        // 被折进过程摘要的行整行不画（issue #369）。`prev` 不更新 —— 分隔线的缩进要按
        // 最后一条**可见**行算，否则隐藏段后面会多出一条孤立分隔线。
        if log.is_hidden() {
            continue;
        }
        let prefix_w = prefix_width(prefix_for(log));
        let matched = needle.is_some_and(|n| log.text().to_lowercase().contains(n));
        if let Some((_, prev_indent)) = prev.filter(|(k, _)| *k != log.kind()) {
            out.push(BodyRow::separator(width, prev_indent, pal));
        }
        out.extend(line_rows(
            log,
            idx,
            width,
            matched,
            target == Some(idx),
            pal,
        ));
        prev = Some((log.kind(), prefix_w));
    }
    out
}

/// 单条逻辑行 → 物理行（**不含**块间分隔线）。
///
/// 这是 `build_body_rows` 的循环体，单独拆出来是给 [`super::body_cache`] 按行复用：
/// 流式期只有最后一行会变，不必每帧把整个 transcript 的 markdown 解析与折行重做一遍。
///
/// `matched` / `anchor` 由调用方给出而不是内部现算：缓存层已维护好命中集合
/// （见 `state_search`），再按行 `to_lowercase()` 一遍等于把省下的开销又赔回去。
pub(crate) fn line_rows(
    log: &LogLine,
    idx: usize,
    width: usize,
    matched: bool,
    anchor: bool,
    pal: &Palette,
) -> Vec<BodyRow> {
    // 被折进过程摘要的行：一个物理行都不出（issue #369）。折叠 / 展开只改 `hidden`，
    // 缓存靠 `rev` 失效那一行，其余行不受影响。
    if log.is_hidden() {
        return Vec::new();
    }
    let prefix = prefix_for(log);
    let prefix_w = prefix_width(prefix);
    let content_width = width.saturating_sub(prefix_w);
    let text = if log.kind() == LineKind::Thinking && log.is_collapsed() {
        fold_thinking(log.text(), content_width)
    } else {
        log.text().to_string()
    };
    let cx = RowCtx {
        idx,
        kind: log.kind(),
        tool_state: log.tool_state(),
        text: &text,
        prefix,
        prefix_style: prefix_style(log.kind(), pal),
        style: kind_style(log.kind(), pal),
        content_width,
        matched,
        anchor_log: matched && anchor,
        pal,
    };
    if log.kind() == LineKind::Assistant {
        assistant_body_rows(&cx)
    } else {
        plain_body_rows(&cx)
    }
}

/// 工具行内容色：成功走 `ok`、失败 `err` + BOLD（无色模式下 BOLD 仍可辨），
/// 其余（进行中 / 成败未知 / 非工具行）沿用基色。
fn tool_content_style(cx: &RowCtx<'_>, base: Style) -> Style {
    match (cx.kind, cx.tool_state.ok()) {
        (LineKind::Tool, Some(true)) => Style::new().fg(cx.pal.ok),
        (LineKind::Tool, Some(false)) => Style::new().fg(cx.pal.err).add_modifier(Modifier::BOLD),
        _ => base,
    }
}

/// 非 Assistant 逻辑行 → 物理 BodyRow（thinking 折叠/用户/工具/系统行，纯文本）。
fn plain_body_rows(cx: &RowCtx<'_>) -> Vec<BodyRow> {
    let base = tool_content_style(cx, cx.style);
    let mut rows = Vec::new();
    let mut first = true;
    for phys in md::wrap_physical(cx.text, cx.content_width) {
        let mut spans: Vec<Span<'static>> = Vec::new();
        let anchor = cx.anchor_log && first;
        let row_style = md::highlight_override(base, cx.matched, anchor, cx.pal);
        if first {
            spans.push(Span::styled(
                cx.prefix.to_string(),
                md::highlight_override(cx.prefix_style, cx.matched, anchor, cx.pal),
            ));
        }
        spans.push(Span::styled(phys, row_style));
        // 纯文本行（用户 / 工具 / 系统 / 折叠后的思考）不做行内 markdown 解析，因此
        // 这里没有链接热区：**已知差异** —— 这些行里的 URL 仍不可点（终端自己的
        // 链接处理器也收不到点击，本程序开着鼠标上报）。要给它们热区，得在
        // `wrap_physical` 之后另算一次列，那是独立的一件事。
        rows.push(BodyRow::content(cx.idx, Line::from(spans), Vec::new()));
        first = false;
    }
    rows
}

/// Assistant 逻辑行 → 全部物理 BodyRow（行内 markdown 样式 + 宽度折行 + 搜索高亮）。
fn assistant_body_rows(cx: &RowCtx<'_>) -> Vec<BodyRow> {
    // 代码块要铺成等宽矩形，得知道可用宽度；首行的 `prefix` 会占几列，先扣掉。
    let code_width = cx.content_width.saturating_sub(prefix_width(cx.prefix));
    let styled = md::assistant_styled(cx.text, cx.pal, code_width);
    let mut rows = Vec::new();
    for (row_no, wrapped) in md::wrap_styled_rows(&styled, cx.content_width)
        .into_iter()
        .enumerate()
    {
        let is_first = row_no == 0;
        let mut spans: Vec<Span<'static>> = Vec::new();
        if is_first {
            let pstyle = md::highlight_override(
                md::md_row_style(Style::new(), cx.prefix_style),
                cx.matched,
                cx.anchor_log,
                cx.pal,
            );
            spans.push(Span::styled(cx.prefix.to_string(), pstyle));
        }
        spans.extend(md::styled_row_spans(
            &wrapped.chars,
            cx.style,
            cx.matched,
            cx.anchor_log && is_first,
            cx.pal,
        ));
        // 链接列与 `wrapped.chars` 出自同一次折行；首行前面还站着前缀，整行右移。
        let links = shift_links(&wrapped.links, is_first, prefix_width(cx.prefix));
        rows.push(BodyRow::content(cx.idx, Line::from(spans), links));
    }
    rows
}

/// 行内链接列 → 正文区坐标：首行前面还有前缀（`You ` / `tool ` 之类），整段右移。
fn shift_links(links: &[(u16, u16, String)], first: bool, prefix_w: usize) -> Vec<RowLink> {
    let shift = u16::try_from(if first { prefix_w } else { 0 }).unwrap_or(u16::MAX);
    links
        .iter()
        .map(|(start, end, url)| RowLink {
            start_col: start + shift,
            end_col: end + shift,
            url: url.clone(),
        })
        .collect()
}

/// 测试便捷名（仅测试引用 build_body_rows）。
#[cfg(test)]
fn body_rows(st: &UiState, width: usize) -> Vec<BodyRow> {
    build_body_rows(st, width)
}

/// 测试辅助：兼容旧接口的纯行视图。
#[cfg(test)]
fn display_lines(st: &UiState, width: usize) -> Vec<Line<'static>> {
    body_rows(st, width).into_iter().map(|r| r.line).collect()
}

// 截断规则已下沉到 [`super::text`]：`render.rs` 贴着 920 行门禁，且状态栏 / 左右侧栏 /
// 设置面板共用同一条「显示宽 ≤ width」的不变式。这里保留再导出，历史调用方不必搬家。
pub(super) use super::text::truncate_display;
// 输入行水平窗口（起点、光标显示列）与按显示列切片的规则同样住在 [`super::text`]：本文件
// 自用（composer），设置面板自己 `use super::text::…`，不再经这里绕一层。
use super::text::{cell_window, horizontal_window};
// 审批浮层的「命令区总共多少行 / 能画多少行」要按键侧也用：滚动上界必须与渲染同源，
// 否则 `End` 会停在一个渲染时才被截掉的越界值上。
pub(super) use overlay::approval_scroll_bounds;

/// 工作区展示名：路径 basename（根目录/空串回退原路径文本）。
pub(super) fn workspace_basename(path: &str) -> String {
    let p = path.trim_end_matches('/');
    match p.rsplit('/').next().filter(|s| !s.is_empty()) {
        Some(base) => base.to_string(),
        None => path.to_string(),
    }
}

/// 左栏内容行：首行标题，其后每会话一行（`>` 当前，`*` serve 活跃，选中高亮）。
pub(super) fn sidebar_rows(st: &UiState, width: usize) -> Vec<Line<'static>> {
    let mut rows: Vec<Line<'static>> = Vec::new();
    let visible = st.visible_indices();
    // 过滤态把条数写成 `[可见/总数]`：只报总数的话，「怎么只剩两条」会像 bug。
    let title = if st.session_filter_active() {
        format!("会话 [{}/{}]", visible.len(), st.sessions.len())
    } else {
        format!("会话 [{}]", st.sessions.len())
    };
    // 过滤词挂在**标题行**而不是另起一行：`sidebar_window` 假定「首行标题、其余都是条目」，
    // 多插一行会让滚动窗口与鼠标命中一起偏一行。
    let title = match st.filter_term() {
        Some(term) => format!("{title} · filter: {term}"),
        None => title,
    };
    rows.push(Line::from(Span::styled(
        truncate_display(&title, width),
        Style::new().fg(st.palette.fg).add_modifier(Modifier::BOLD),
    )));
    if st.sessions.is_empty() {
        rows.push(Line::from(Span::styled(
            truncate_display("（空）发一条消息后出现", width),
            Style::new().fg(st.palette.muted),
        )));
        return rows;
    }
    if visible.is_empty() {
        rows.push(Line::from(Span::styled(
            truncate_display("（无匹配）", width),
            Style::new().fg(st.palette.muted),
        )));
        return rows;
    }
    for &i in &visible {
        let row = &st.sessions[i];
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
        let selected = st.selected == i;
        // 选中只换底色、不占额外列：文本预算就是整份 `width`（不再为竖条扣一列）。
        let text = truncate_display(&format!("{mark} {title}"), width);
        rows.push(selected_row(st, text, selected));
    }
    rows
}

/// 选中行：只换**底色**（无色模式降级为反色），焦点不在本栏时再补 `BOLD`。
///
/// 不画左缘竖条：竖条要占一列，那一列会让文字在选中 / 未选中之间整体横移一格，还得从
/// 行宽预算里扣一列（每行都与鼠标命中共用同一份预算）。「哪条被选中」由底色回答；
/// 焦点是否在本栏靠 `BOLD` —— 底色在失焦时本来就淡一档。
///
/// 未选中行保持裸文本（不设前景色，跟随终端默认），避免整列被 palette 接管。
fn selected_row(st: &UiState, text: String, selected: bool) -> Line<'static> {
    if !selected {
        return Line::from(Span::styled(text, Style::new()));
    }
    let mut body = st.palette.selection(st.palette.fg);
    if st.focus != Focus::Sidebar {
        body = body.add_modifier(Modifier::BOLD);
    }
    Line::from(Span::styled(text, body))
}

/// 给一块区域整体铺背景色（先于文字绘制，杜绝段落内容不足时的底色缺口）。
pub(super) fn paint_bg(frame: &mut Frame, rect: Rect, color: Color) {
    if rect.is_empty() {
        return;
    }
    frame.buffer_mut().set_style(rect, Style::new().bg(color));
}

/// 格子背景：把仍是「主题底色」的单元格换成采样色，前景文字与显式底色都不动。
///
/// 只替换 `pal.bg` / `pal.chrome_bg`：选中行、错误色、搜索高亮等都有各自的底色，
/// 被图色覆盖的话高亮就整片消失。先判断再改，等于「只给没有个性的格子铺图」。
fn paint_bg_cells(frame: &mut Frame, st: &UiState) {
    let Some(cells) = st.bg_cells.as_ref() else {
        return;
    };
    let area = frame.area();
    let flat = [st.palette.bg, st.palette.chrome_bg];
    let buf = frame.buffer_mut();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if !flat.contains(&buf[(x, y)].bg) {
                continue;
            }
            if let Some(c) = cells.get(x - area.left(), y - area.top(), area.width, area.height) {
                buf[(x, y)].bg = c;
            }
        }
    }
}

/// 渲染一帧。整屏 = 主体 + 底部状态栏；宽屏主体为左会话列 | 聊天列 | 右工作区目录树
/// （右栏默认显示），composer 只位于聊天列底部。
/// `prepared` 为事件循环按（宽度/内容指纹/搜索）memo 好的 transcript 物理行；
/// `panel` 为设置面板内容（`Some` 时最后绘制全屏浮层并隐藏 composer 光标）。
pub fn draw(
    frame: &mut Frame,
    st: &mut UiState,
    info: &StatusInfo,
    prepared: &[BodyRow],
    panel: Option<&PanelContent>,
    // 外壳层算好的 lane 快照（名字 / 状态 / 是否当前）；`len == 0` 时不占版面。
    lanes: &[super::lane_list::LaneEntry],
    // 待就绪的 lane（`/lane new` 已写声明、ctl 还没起好）：排在真连接之后，
    // 不属于 `lanes` —— 它们还不是连接，也没有点击目标。
    pending: &[super::pending_lane::PendingLane],
) {
    let area = frame.area();
    // 右栏底部状态栈的行数（与 `status_stack::stack_lines` 同源，否则底部会空行 / 末段被裁）。
    // 状态栈在右栏：内宽取右栏（列宽 - 2，列内左右各留 1 列）。列宽先按本帧终端夹取 ——
    // 夹取与 `panes_with_lanes` 同一份（幂等），否则「按 34 列算行数、按 20 列画栈」
    // 会让栈里多出或少掉一行，而鼠标点段用的是行数，一漂就点到别的段。
    let layout = st.pane_layout().clamped(area.width);
    let stack_rows = status_stack::stack_rows(
        info,
        layout.widths.right.saturating_sub(2) as usize,
        st.status_stack_collapsed,
    );
    // lane 列表**单 lane 也占位**：否则用户在界面上完全看不出自己连的是哪台 serve、
    // 有没有别的连接。高度是左栏的一半（`layout::lane_pane_height`），与条数无关，
    // 条数只影响区内画得出几条（放不下的在标题补 `↓n`）；鼠标命中用的是同一份口径。
    // 主体三列 / 底栏 / 状态栈 / 左栏 lane 区的矩形与鼠标命中检测共用 `layout`。
    // 宽度已在上面夹过一次；`panes_with_lanes` 内部再夹一次（幂等），所以两处一致。
    let p = panes_with_lanes(area, layout, stack_rows, lanes.len());

    let spin = progress_mark_frame(st, info);
    let pal = st.palette; // 拷贝：下面要 `&mut st` 回写 `body_at_top`，底栏/浮层仍要调色板

    // 宽屏三列；窄屏 `left`/`right` 为 None，聊天列即整个主体。
    if let (Some(left), Some(right)) = (p.left, p.right) {
        // 列内左右各留 1 列：内容宽度（`- 2`）在「构建行」与「标题溢出计数截断」两处
        // 必须是同一口径，否则 `↑n`/`↓n` 会把标题挤出列外。
        let left_w = left.width.saturating_sub(2) as usize;
        let (rows, cursor, hint) = sessions_content(st, left_w);
        paint_bg(frame, left, pal.chrome_bg);
        render_pane(frame, left, rows, cursor, hint, &pal, left_w);
        let right_w = right.width.saturating_sub(2) as usize;
        let (rows, cursor, hint) = workspace_content(st, right_w);
        paint_bg(frame, right, pal.chrome_bg);
        render_pane(frame, right, rows, cursor, hint, &pal, right_w);
    }
    // 左栏下半：lane 列表。固定占左栏一半高度，与左栏同宽同底色，视觉上是同一列的两段。
    // 每条 lane 一行整行宽的底色条（当前 lane 用选中底色），自上而下堆叠，条数少时下方留空。
    if let Some(lane_pane) = p.left_lanes {
        paint_bg(frame, lane_pane, pal.chrome_bg);
        super::lane_list::render_lane_list(frame, lane_pane, lanes, pending, &pal);
    }
    if let Some(rs) = p.right_status {
        render_status_stack(frame, rs, st, info, &pal);
    }
    // 两栏各一条分隔线：画在内容之后，落点是列内本来就空着的留白列，不影响排版。
    // 具体理由与「上下两段贯穿」的口径见 `pane_render::paint_pane_dividers`。
    paint_pane_dividers(frame, &p, &pal);
    super::pane_render::render_chat_column(
        frame,
        st,
        p.chat,
        prepared,
        panel.is_none(),
        spin,
        &pal,
    );

    // 底栏**只在窄屏存在**：宽屏的状态已进右栏底部，那一行还给正文。
    if let Some(status_area) = p.status {
        paint_bg(frame, status_area, pal.chrome_bg);
        // 分段样式已在 `status_line` 里定好（label 淡色 / 值正文色 / 分隔极暗），此处不能再套
        // 整行 `fg + BOLD`：base style 的修饰位会盖到 label 上，淡化标签就白做了。
        let status = status_line(info, status_area.width as usize, &pal);
        frame.render_widget(Paragraph::new(status), status_area);
    }

    // 顶层：设置面板（整屏覆盖）或「格子背景 + 权限菜单」，顺序约束见 `render_top_layer`。
    overlay::render_top_layer(frame, area, st, panel, &pal);
}

/// 右栏底部状态栈：铺条带色后逐行画（每段一行，值已按列宽截断）。
///
/// 与 desktop 的 `.side-status-stack` 同位置、同形态（纵向、贴右栏底部）；
/// 窄屏没有右栏时不会走到这里（`panes` 的 `right_status` 为 `None`）。
fn render_status_stack(
    frame: &mut Frame,
    area: Rect,
    st: &UiState,
    info: &StatusInfo,
    pal: &Palette,
) {
    paint_bg(frame, area, pal.chrome_bg);
    let width = area.width.saturating_sub(2) as usize;
    let lines = status_stack::stack_lines(info, width, pal, st.status_stack_collapsed);
    // 列内左右各留 1 列，与左右两栏的内容对齐（`render_pane` 同一口径）。
    let inner = Rect::new(area.x.saturating_add(1), area.y, width as u16, area.height);
    frame.render_widget(Paragraph::new(lines), inner);
}

/// 主区（transcript + 审批浮层 + `/` 补全菜单）的绘制入口，供 `pane_render` 调用。
///
/// 三者共用同一矩形且顺序有约束（浮层压 transcript、菜单让位给浮层），所以留在一起，
/// 不让调用方各画各的。
pub(super) fn render_body_and_overlays(
    frame: &mut Frame,
    st: &mut UiState,
    body_area: Rect,
    prepared: &[BodyRow],
    spin: Option<usize>,
) {
    render_body(frame, st, body_area, prepared, spin);
    // 浮层压 transcript，`/` 菜单让位给浮层：三者互斥，写成一条链而不是嵌套 if。
    // 问卷排在审批之后：两者都是模态，但审批是流内阻塞门，先解决它。
    if st.approval.is_some() {
        render_approval_overlay(frame, st, body_area);
    } else if st.clarify_open && st.clarify.is_some() {
        super::clarify::render_clarify_overlay(frame, st, body_area);
    } else if st.detail.is_some() {
        // 工具详情（`/detail`）：只读浏览，压住 transcript、也让 `/` 菜单让位 ——
        // 浮层开着时按键全归它（见 `consume_detail_key`），菜单画出来也选不动。
        super::detail::render_detail_overlay(frame, st, body_area);
    } else {
        super::slash_menu::render_slash_menu(frame, st, body_area);
    }
}

/// composer 内容行数：随输入行数增长，最多 `MAX_COMPOSER_ROWS`（单行恒为 1）。
fn composer_rows(st: &UiState) -> u16 {
    st.input_line_count().clamp(1, MAX_COMPOSER_ROWS) as u16
}

/// composer 占用总行数：内容行 + 上下边框。
pub(super) fn composer_outer_rows(st: &UiState) -> u16 {
    composer_rows(st).saturating_add(2)
}

/// 终端太窄/太矮时放不下边框，退回无边框（否则内宽可能为 0）。
fn use_composer_border(area: Rect) -> bool {
    area.width >= 4 && area.height >= 3
}

/// composer 内容宽度：有边框时左右各占 1 列；无边框时沿用旧的左内缩 1 列。
fn composer_content_width(area: Rect) -> usize {
    let w = if use_composer_border(area) {
        area.width.saturating_sub(2)
    } else {
        area.width.saturating_sub(1)
    };
    w as usize
}

/// 空 composer 的占位提示（仅显示，不写输入缓冲）。
const COMPOSER_PLACEHOLDER: &str = "输入消息 · Alt+Enter 换行 · /help 查看命令";

/// 打开 composer 占位提示的开关：`1` / `on` / `true` / `yes` / `enable`。
///
/// 默认**不显示**：空会话时 [`BODY_EMPTY_HINT`] 已经把同一批快捷键说过一遍，
/// 输入框里再挂一条等于同一句话在屏幕上出现两次；那行灰字还长期占着首行，
/// 容易和真正输入的内容混淆。
const COMPOSER_HINT_ENV: &str = "CM_TUI_COMPOSER_HINT";

/// 是否画 composer 占位提示：默认关，[`COMPOSER_HINT_ENV`] 显式开启时才画。
fn composer_hint_enabled() -> bool {
    composer_hint_enabled_with(&|k| std::env::var(k).ok())
}

/// [`composer_hint_enabled`] 的可注入版本（单测不依赖真实环境变量）。
fn composer_hint_enabled_with(env: &dyn Fn(&str) -> Option<String>) -> bool {
    match env(COMPOSER_HINT_ENV) {
        Some(v) => matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "on" | "true" | "yes" | "enable"
        ),
        None => false,
    }
}

/// 通用固定列渲染：内容窗口裁剪（含提示行）。内容行/光标/提示由 [`super::ws_sidebar`] 提供。
/// 列内左右各留 1 列空隙（对齐聊天区观感），行宽按 `width - 2` 截断。
/// `width` 与构建行时用的是同一个值（`列宽 - 2`），标题溢出计数也按它截断。
fn render_pane(
    frame: &mut Frame,
    area: Rect,
    rows: Vec<Line<'static>>,
    cursor: usize,
    hint: Option<Line<'static>>,
    pal: &Palette,
    width: usize,
) {
    if area.width <= 1 || area.height == 0 {
        return;
    }
    // 内容区与提示行矩形同样跑鼠标命中检测用的那份计算。
    let list_area = pane_list_area(area);
    let shown = sidebar_view(&rows, list_area.height as usize, cursor, width, pal);
    let list = Paragraph::new(shown).style(Style::new().fg(pal.fg));
    frame.render_widget(list, list_area);
    // 提示行占列内最后一行（高度不足 2 行时没有提示位）。内容自带样式：会话栏是静态
    // 键位，工作区栏在光标行被截断时会改成完整相对路径。
    if area.height >= 2
        && let Some(text) = hint
    {
        let hint_area = Rect::new(area.x + 1, area.y + area.height - 1, area.width - 1, 1);
        frame.render_widget(Paragraph::new(text), hint_area);
    }
}

/// 空 transcript 时的快捷键引导。
const BODY_EMPTY_HINT: &str = "输入消息开始对话 · Alt+Enter 换行 · Ctrl+E 思考 · Ctrl+W 工作区树 · PgUp/PgDn 滚动 · /find 搜索 · Ctrl+C 退出 · /help";

fn render_body(
    frame: &mut Frame,
    st: &mut UiState,
    area: Rect,
    prepared: &[BodyRow],
    spin: Option<usize>,
) {
    paint_bg(frame, area, st.palette.bg);
    // 正文区左右各让 1 列，故宽度 ≤ 2 时已无内容可画（与下面的 −2 预算同步）。
    if area.width <= 2 || area.height == 0 {
        return;
    }
    // 左右各留 1 列间距（与左侧对称；宽度须与 `chat_body_width` 完全一致，
    // 否则折行预算大于绘制区，行尾字符会被区域右缘裁掉）。
    let text_area = Rect::new(
        area.x.saturating_add(1),
        area.y,
        area.width.saturating_sub(2),
        area.height,
    );
    if st.lines.is_empty() {
        // 空 transcript 画不出锚点：留着它下一帧会按已失效的下标定位一次。
        st.reanchor_log = None;
        let hint = Paragraph::new(Span::styled(
            BODY_EMPTY_HINT,
            Style::new().fg(st.palette.muted),
        ));
        frame.render_widget(hint, text_area);
        return;
    }
    let rows = prepared;
    let rows_total = rows.len();
    let viewport = (text_area.height as usize).saturating_sub(1).max(1);
    // 折叠 / 展开刚刚改变了物理行数：把原本在视口顶端的那条逻辑行钉回顶端，
    // 否则用户正在回看的内容会突然上跳（`view_offset` 是「从底部回看的行数」）。
    if let Some(target) = st.reanchor_log.take()
        && let Some(pos) = rows.iter().position(|r| r.log_index == Some(target))
    {
        st.view_offset = rows_total.saturating_sub(pos + viewport);
    }
    // 搜索命中行优先（`/find` 跳转要把它停在视口上 1/3 处）；折叠导航开着时改用它 ——
    // 刚按过 ↑/↓ 的人要的正是「把这一条带到眼前」。窗口计算与 clamp 都在
    // `body_cache::scroll_window` 里，那里有单测。
    let anchor_pos = st
        .search_cursor
        .or(st.fold_cursor)
        .and_then(|target| rows.iter().position(|r| r.log_index == Some(target)));
    let win = super::body_cache::scroll_window(rows_total, viewport, st.view_offset, anchor_pos);
    // 在顶由渲染层回写：只有这里知道 `max_back`；前插保持贴顶、PgUp 续翻都读它。
    st.body_at_top = win.at_top;
    // 视口顶端那条逻辑行：折叠 / 展开改变物理行数时，靠它把视线钉回原处（issue #369）。
    st.body_top_log = rows[win.start..win.end].iter().find_map(|r| r.log_index);
    // 折叠思考行的 spinner 在此替换标记：body memo 不含帧号，故不会触发重建。
    // 只命中最后一条逻辑行（`prepared` 里可能有多条历史折叠行）。
    let spin_target = spin.map(|_| st.lines.len().saturating_sub(1));
    let fold_cursor = st.fold_cursor;
    let sel_style = st.palette.selection(st.palette.fg);
    let shown = rows[win.start..win.end]
        .iter()
        .map(|r| {
            let mut line = r.line.clone();
            if let (Some(f), Some(idx)) = (spin, spin_target)
                && r.log_index == Some(idx)
            {
                line = with_progress_mark(line, f);
            }
            if fold_cursor.is_some() && r.log_index == fold_cursor {
                line = with_row_style(line, sel_style);
            }
            line
        })
        .collect::<Vec<_>>();
    // 鼠标要的是「物理行 → 逻辑行」，而这份对应只在这里存在：连窗口一起回写，与
    // `body_top_log` 同一条规矩 —— 在 `mouse.rs` 里重算一遍排版，点下去一定错行。
    st.body_area = Some(text_area);
    st.body_rows_log = rows[win.start..win.end]
        .iter()
        .map(|r| r.log_index)
        .collect();
    // 链接热区与 `body_rows_log` 共用同一份窗口与坐标原点：鼠标侧只做减法，
    // 绝不重算折行（重算就会点偏）。
    st.body_links = rows[win.start..win.end]
        .iter()
        .map(|r| r.links.clone())
        .collect();
    frame.render_widget(Paragraph::new(shown), text_area);
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

/// 画 composer 边框（或无边框退化）并返回可用内区；内区为空时返回 `None`。
fn composer_inner(frame: &mut Frame, area: Rect, focused: bool, pal: &Palette) -> Option<Rect> {
    let inner = if use_composer_border(area) {
        let border = if focused { pal.accent } else { pal.muted };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::new().fg(border));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        inner
    } else {
        // 极窄/极矮终端放不下边框：退回旧的「左内缩 1 列」画法。
        Rect::new(
            area.x.saturating_add(1),
            area.y,
            area.width.saturating_sub(1),
            area.height,
        )
    };
    (!inner.is_empty()).then_some(inner)
}

/// 空 composer 的占位提示（只画不写缓冲：输入第一个字符就消失）。
fn render_composer_placeholder(frame: &mut Frame, inner: Rect, pal: &Palette) {
    let hint = Paragraph::new(Span::styled(
        COMPOSER_PLACEHOLDER,
        Style::new().fg(pal.muted).add_modifier(Modifier::DIM),
    ));
    frame.render_widget(hint, Rect::new(inner.x, inner.y, inner.width, 1));
}

/// 逐行画 composer 内容（已按宽度窗口裁剪）。
fn render_composer_rows(frame: &mut Frame, inner: Rect, rows: &[String], pal: &Palette) {
    for (i, text) in rows.iter().enumerate() {
        if i >= inner.height as usize {
            break;
        }
        let row_area = Rect::new(inner.x, inner.y.saturating_add(i as u16), inner.width, 1);
        let paragraph =
            Paragraph::new(Line::from(Span::raw(text.as_str()))).style(Style::new().fg(pal.fg));
        frame.render_widget(paragraph, row_area);
    }
}

/// composer 硬件光标位置（clamp 在内区内，避免落到边框或浮层上）。
fn composer_cursor_pos(inner: Rect, shown_cursor: usize, disp_row: usize) -> (u16, u16) {
    let col = inner
        .x
        .saturating_add(shown_cursor as u16)
        .min(inner.x.saturating_add(inner.width.saturating_sub(1)));
    let row = inner
        .y
        .saturating_add(disp_row as u16)
        .min(inner.y.saturating_add(inner.height.saturating_sub(1)));
    (col, row)
}

pub(super) fn render_input(frame: &mut Frame, st: &UiState, area: Rect, cursor_allowed: bool) {
    if area.width <= 1 || area.height == 0 {
        return;
    }
    let pal = &st.palette;
    paint_bg(frame, area, pal.chrome_bg);
    // 焦点在输入框且没被浮层占键盘时边框用 accent；聚焦在侧栏/工作区或浮层打开时 muted。
    let focused = cursor_allowed && st.focus == Focus::Input;
    let Some(inner) = composer_inner(frame, area, focused, pal) else {
        return;
    };
    if composer_hint_enabled() && st.current_input().trim().is_empty() {
        render_composer_placeholder(frame, inner, pal);
    }
    let rows = (composer_rows(st) as usize).min(inner.height as usize);
    let (shown, disp_row, shown_cursor, _hstart) =
        composer_window(st, composer_content_width(area), rows);
    render_composer_rows(frame, inner, &shown, pal);
    // 全屏浮层（设置面板）打开时不需要 composer 硬件光标，避免光标漏到浮层外。
    if focused {
        frame.set_cursor_position(composer_cursor_pos(inner, shown_cursor, disp_row));
    }
}

#[cfg(test)]
#[path = "render_tests.rs"]
mod tests;

// 颜色级断言单独成文件：`render_tests.rs` 已 859 行，且按关注点分开
// （结构与宽度的说不该和 buffer 颜色的混在一处）。
#[cfg(test)]
#[path = "render_color_tests.rs"]
mod color_tests;

// 系统行的前缀 / 配色断言：`render_tests.rs` 叠上 #410 的链接热区测试后已到 912 行，
// 再放这三支就顶过 920 行门禁，故按关注点再拆一个文件（与 `color_tests` 同一手法）。
#[cfg(test)]
#[path = "render_system_tests.rs"]
mod system_tests;

// 左栏**过滤态**的断言（issue #423）：标题条数 / 过滤词 / 无匹配占位行 / 提示行切换。
// `render_tests.rs` 早已贴着门禁，故与上面两支同一手法再拆一个文件。
#[cfg(test)]
#[path = "render_filter_tests.rs"]
mod filter_tests;
