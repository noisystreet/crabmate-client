//! 全屏 TUI 渲染：状态行 / 左栏会话 / 右栏工作区目录树（仿 Desktop 默认显示）/
//! 主区 transcript / 底栏输入 / 审批浮层。布局纯函数为主，便于单测。

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::layout::panes;
use super::md;
use super::palette::Palette;
use super::settings_panel::PanelContent;
use super::state::{Focus, LineKind, LogLine, UiState};
use super::status::{SPINNER_FRAMES, status_line};
use super::ws_sidebar::{pane_list_area, sessions_content, sidebar_view, workspace_content};

// 浮层（审批 / 设置面板）拆到 `overlay.rs`：它们自成一套模板，也把本文件的行数
// 从 920 门禁线附近拉开，给后续 B4 / B5 的改动留余量。
#[path = "overlay.rs"]
mod overlay;
use overlay::{render_approval_overlay, render_settings_overlay};
// 面板内容必须按弹层内宽生成（否则居中后错位），所以把同一个矩形计算暴露给
// `mod.rs` 的 `settings_content(...)` 调用点。
pub(crate) use overlay::settings_panel_content_width;

// 顶栏三段式（B5）拆到 `topbar.rs`：纯函数 + 自算列，别让本文件再贴 920 行门禁。
#[path = "topbar.rs"]
mod topbar;
use topbar::top_line;

/// 低于此宽度隐藏左右栏（等价 repl 布局）。
pub const SIDEBAR_MIN_WIDTH: u16 = 120;
/// 左会话列 / 右工作区列宽度。
pub(crate) const SIDEBAR_WIDTH: u16 = 26;

/// 计算 build_body_rows 使用的聊天区宽度（与 draw 里传给 render_body 的区域一致：
/// 聊天列内左右各留 1 列空隙，与侧栏 / composer 边框的对称留白对齐）。
pub(crate) fn chat_body_width(sidebar_visible: bool, terminal_width: u16) -> usize {
    let col = if sidebar_visible {
        terminal_width.saturating_sub(SIDEBAR_WIDTH * 2)
    } else {
        terminal_width
    };
    (col as usize).saturating_sub(2)
}
/// 左栏会话栏聚焦时的按键提示。宽度必须 ≤ 侧栏内宽（`SIDEBAR_WIDTH` - 1 = 25 列），
/// 否则 `render_pane` 会直接裁掉尾部的键；中文按 2 列计。
pub(super) const SIDEBAR_HINT: &str = "↑↓ Enter用 Del删 n新 r刷";
/// 删除确认态的提示（`Delete` 已按一次）：y / Enter 删，Esc / n 取消。
///
/// 与 Desktop 的确认框同语义，但借用侧栏底部提示行，不抢焦点、不开浮层。
pub(super) const SIDEBAR_DELETE_HINT: &str = "[y] 删除 [Esc] 取消";
/// composer 最多展开行数（多行输入）。
const MAX_COMPOSER_ROWS: usize = 4;
// 配色见 [`Palette`]：顶栏 / 状态栏 / 侧栏 / composer 用 `chrome_bg`，聊天区用 `bg`，
// 正文 / 次要 / 强调分别为 `fg` / `muted` / `accent`（随 Desktop 主题）。

/// 状态行展示信息（mod 层组装好的显示值，含 override 标记）。
#[derive(Clone)]
pub struct StatusInfo {
    pub api_base: String,
    /// 生效模型；本地 override 时带 `*` 后缀。
    pub model: Option<String>,
    pub role: Option<String>,
    pub mode: Option<String>,
    pub running: bool,
    pub cancel_sent: bool,
    /// 生成中 spinner 帧号（事件循环按时间推进；空闲 / 取消中不显示）。
    pub spinner: usize,
    /// transcript 回看行数（>0 时显示 ↑N）。
    pub view_offset: usize,
    /// 活跃搜索词（/find）。
    pub search_term: Option<String>,
    /// 搜索命中逻辑行数。
    pub search_total: usize,
}

/// 一条可渲染的 transcript 物理行。
pub struct BodyRow {
    /// 所属逻辑行下标（供搜索锚点定位）；块间分隔行不属于任何逻辑行，为 `None`。
    /// 用 `Option` 而非哨兵下标，是为了让"分隔行不该被锚点命中"成为类型级事实——
    /// 锚点查找按 `Some(target)` 匹配，无需依赖“首匹配/末匹配”之类的取行顺序。
    pub log_index: Option<usize>,
    pub line: Line<'static>,
}

impl BodyRow {
    /// 内容行：逻辑行 `idx` 的一个物理行。
    fn content(idx: usize, line: Line<'static>) -> Self {
        Self {
            log_index: Some(idx),
            line,
        }
    }

    /// 块间分隔行：极暗的满宽细线，不归属任何逻辑行（搜索锚点据此跳过）。
    /// `indent` 取上一条逻辑行的前缀宽度，让线与上方块内容左对齐；
    /// `width` 小于 `indent` 或极窄时 `rule` 退化为 0，此时等同空行，不会 panic。
    fn separator(width: usize, indent: usize, pal: &Palette) -> Self {
        let indent = indent.min(width);
        let rule = width.saturating_sub(indent);
        let mut text = String::with_capacity(width);
        text.push_str(&" ".repeat(indent));
        text.push_str(&"─".repeat(rule));
        // 无色模式没有可靠的“极暗”色，退回 DIM；`─` 本身已能表达分隔。
        let style = if pal.mono {
            Style::new().add_modifier(Modifier::DIM)
        } else {
            Style::new().fg(pal.border)
        };
        Self {
            log_index: None,
            line: Line::from(Span::styled(text, style)),
        }
    }
}

/// 消息前缀（宽 4–7 列；宽度参与折行计算，故与样式分开）。
fn kind_prefix(kind: LineKind) -> &'static str {
    match kind {
        LineKind::User => "[你] ",
        LineKind::Assistant => "[助手] ",
        LineKind::Thinking => "[思考] ",
        LineKind::Tool => "[工具] ",
        LineKind::System => "[!] ",
    }
}

/// 进行中工具行的进度位占位（`⠿` 与 spinner 帧同为 1 列宽，换帧不改折行点）。
const TOOL_PROGRESS_MARK: &str = "⠿ ";

/// 进行中工具行的完整前缀：`[工具] ` + 进度位（即 [`TOOL_PROGRESS_MARK`]）。
/// 结束行不加进度位——它的文本自带 `✓ / ✗`，不重复。
const TOOL_PREFIX_PROGRESS: &str = "[工具] ⠿ ";

/// 逻辑行前缀：进行中的工具行多一个进度位。
/// 进度位是**静态**占位，动画由渲染层换字符，因此不进入 body memo。
fn prefix_for(log: &LogLine) -> &'static str {
    // 回放的历史工具行没有「进行中」这个概念，不带进度位占位。
    match (log.kind, log.tool_ok, log.replayed) {
        (LineKind::Tool, None, false) => TOOL_PREFIX_PROGRESS,
        _ => kind_prefix(log.kind),
    }
}
/// 逻辑行前缀占用的**显示列数**（折行预算必须按列算，不能按字符数）：
/// `[助手] ` 只有 5 个字符却占 7 列，按字符数留白会让每行首物理行超出正文区
/// 1–2 列，末字被区域右缘裁掉——观感就是「行尾被切、没有折回来」。
fn prefix_width(prefix: &str) -> usize {
    UnicodeWidthStr::width(prefix)
}

/// 前缀样式：一律 muted + DIM，把亮度让给正文；系统告警要抢眼，保持 err + BOLD。
fn prefix_style(kind: LineKind, pal: &Palette) -> Style {
    if kind == LineKind::System {
        return Style::new().fg(pal.err).add_modifier(Modifier::BOLD);
    }
    Style::new().fg(pal.muted).add_modifier(Modifier::DIM)
}

/// 内容基色（工具行按结果另算，见 [`tool_content_style`]）。
fn kind_style(kind: LineKind, pal: &Palette) -> Style {
    match kind {
        LineKind::User => Style::new().fg(pal.accent).add_modifier(Modifier::BOLD),
        LineKind::Assistant => Style::new().fg(pal.fg),
        LineKind::Thinking => Style::new().fg(pal.muted),
        LineKind::Tool => Style::new().fg(pal.muted),
        LineKind::System => Style::new().fg(pal.err),
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
    let live = st.lines.last().is_some_and(|l| match l.kind {
        LineKind::Thinking => l.collapsed,
        // 回放的历史行不是「进行中」，别给它加 spinner。
        LineKind::Tool => l.tool_ok.is_none() && !l.replayed,
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
/// - 进行中工具行：进度位在**消息前缀**末尾（`[工具] ⠿ `），故只处理有成对
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
    tool_ok: Option<bool>,
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
/// 供事件循环做帧间 memo（内容/宽度/搜索/主题未变时复用），
/// 渲染层拿结果直接裁剪；生成中的 spinner 标记由渲染层替换，不进 memo。
pub(crate) fn build_body_rows(st: &UiState, width: usize) -> Vec<BodyRow> {
    let needle = st.search_term();
    let target = st.search_cursor;
    let pal = &st.palette;
    let mut out: Vec<BodyRow> = Vec::new();
    // 块间分隔：相邻两条日志行 kind 不同即为块边界（用户 / 思考 / 正文 / 工具 /
    // 系统各自成块），画一条极暗满宽细线代替空行——间距不变，但块边界更清晰
    // （对齐 Desktop `.chat-tui-transcript` 的块间距观感）；同类相邻
    // （如连续工具行）保持紧凑，对应 desktop 的 tool-process 块内多行。
    // 上一条逻辑行的 (类别, 前缀宽度)：分隔线缩进取上一条逻辑行的前缀宽度，
    // 因此要存宽度而不只是类别——进行中的工具行前缀比结束行宽 2 列。
    let mut prev: Option<(LineKind, usize)> = None;
    for (idx, log) in st.lines.iter().enumerate() {
        let prefix = prefix_for(log);
        let prefix_w = prefix_width(prefix);
        let content_width = width.saturating_sub(prefix_w);
        let matched = needle.is_some_and(|n| log.text.to_lowercase().contains(n));
        if let Some((_, prev_indent)) = prev.filter(|(k, _)| *k != log.kind) {
            out.push(BodyRow::separator(width, prev_indent, pal));
        }
        let text = if log.kind == LineKind::Thinking && log.collapsed {
            fold_thinking(&log.text, content_width)
        } else {
            log.text.clone()
        };
        let cx = RowCtx {
            idx,
            kind: log.kind,
            tool_ok: log.tool_ok,
            text: &text,
            prefix,
            prefix_style: prefix_style(log.kind, pal),
            style: kind_style(log.kind, pal),
            content_width,
            matched,
            anchor_log: matched && target == Some(idx),
            pal,
        };
        if log.kind == LineKind::Assistant {
            out.extend(assistant_body_rows(&cx));
        } else {
            out.extend(plain_body_rows(&cx));
        }
        prev = Some((log.kind, prefix_w));
    }
    out
}

/// 工具行内容色：成功走 `ok`、失败 `err` + BOLD（无色模式下 BOLD 仍可辨），
/// 其余（进行中 / 非工具行）沿用基色。
fn tool_content_style(cx: &RowCtx<'_>, base: Style) -> Style {
    match (cx.kind, cx.tool_ok) {
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
        rows.push(BodyRow::content(cx.idx, Line::from(spans)));
        first = false;
    }
    rows
}

/// Assistant 逻辑行 → 全部物理 BodyRow（行内 markdown 样式 + 宽度折行 + 搜索高亮）。
fn assistant_body_rows(cx: &RowCtx<'_>) -> Vec<BodyRow> {
    // 代码块要铺成等宽矩形，得知道可用宽度；首行的 `prefix` 会占几列，先扣掉。
    let code_width = cx.content_width.saturating_sub(prefix_width(cx.prefix));
    let styled = md::assistant_styled_text(cx.text, cx.pal, code_width);
    let mut rows = Vec::new();
    for (row_no, row) in md::wrap_styled_chars(&styled, cx.content_width)
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
            &row,
            cx.style,
            cx.matched,
            cx.anchor_log && is_first,
            cx.pal,
        ));
        rows.push(BodyRow::content(cx.idx, Line::from(spans)));
    }
    rows
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
    let title = format!("会话 [{}]", st.sessions.len());
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
        let selected = st.selected == i;
        // 竖条占 1 列：选中行的文本预算要减 1，否则末列会被 `Paragraph` 直接裁掉。
        let budget = width.saturating_sub(usize::from(selected));
        let text = truncate_display(&format!("{mark} {title}"), budget);
        rows.push(selected_row(st, text, selected));
    }
    rows
}

/// 选中行：左缘 `▎` 竖条（聚焦本栏时 `accent`，否则 `muted`）+ 选中底色。
///
/// 未选中行保持裸文本（不设前景色，跟随终端默认），避免整列被 palette 接管。
/// 底色在无色模式下降级为反色（见 [`Palette::selection`]）。
fn selected_row(st: &UiState, text: String, selected: bool) -> Line<'static> {
    if !selected {
        return Line::from(Span::styled(text, Style::new()));
    }
    let focused = st.focus == Focus::Sidebar;
    let bar_fg = if focused {
        st.palette.accent
    } else {
        st.palette.muted
    };
    // 焦点不在本栏时底色本来就更淡，再补 BOLD 让「当前是哪个会话」仍能一眼扫到。
    let mut body = st.palette.selection(st.palette.fg);
    if !focused {
        body = body.add_modifier(Modifier::BOLD);
    }
    Line::from(vec![
        Span::styled("▎", Style::new().fg(bar_fg)),
        Span::styled(text, body),
    ])
}

/// 给一块区域整体铺背景色（先于文字绘制，杜绝段落内容不足时的底色缺口）。
fn paint_bg(frame: &mut Frame, rect: Rect, color: Color) {
    if rect.is_empty() {
        return;
    }
    frame.buffer_mut().set_style(rect, Style::new().bg(color));
}

/// 渲染一帧。对齐 Desktop：整屏 = 顶栏(工作区) + 主体 + 底部状态栏；宽屏主体为
/// 左会话列 | 聊天列 | 右工作区目录树（右栏默认显示），composer 只位于聊天列底部。
/// `prepared` 为事件循环按（宽度/内容指纹/搜索）memo 好的 transcript 物理行；
/// `panel` 为设置面板内容（`Some` 时最后绘制全屏浮层并隐藏 composer 光标）。
pub fn draw(
    frame: &mut Frame,
    st: &UiState,
    info: &StatusInfo,
    prepared: &[BodyRow],
    panel: Option<&PanelContent>,
) {
    let area = frame.area();
    // 顶栏 / 主体三列 / 状态栏的矩形与鼠标命中检测共用 `layout::panes`。
    let p = panes(area, st.sidebar_visible);

    let spin = progress_mark_frame(st, info);
    let pal = &st.palette;
    paint_bg(frame, p.top, pal.chrome_bg);
    // 顶栏：左品牌 + 居中工作区名。位置由 `top_line` 自算——
    // 两段混排不能用整行居中，否则中段会随左段宽度漂。
    // 模式不在这里（由状态栏 `mode` 段显示），也不放键位提示。
    frame.render_widget(Paragraph::new(top_line(st, p.top.width as usize)), p.top);

    // 宽屏三列；窄屏 `left`/`right` 为 None，聊天列即整个主体。
    if let (Some(left), Some(right)) = (p.left, p.right) {
        // 列内左右各留 1 列：内容宽度（`- 2`）在「构建行」与「标题溢出计数截断」两处
        // 必须是同一口径，否则 `↑n`/`↓n` 会把标题挤出列外。
        let left_w = left.width.saturating_sub(2) as usize;
        let (rows, cursor, hint) = sessions_content(st, left_w);
        paint_bg(frame, left, pal.chrome_bg);
        render_pane(frame, left, rows, cursor, hint, pal, left_w);
        let right_w = right.width.saturating_sub(2) as usize;
        let (rows, cursor, hint) = workspace_content(st, right_w);
        paint_bg(frame, right, pal.chrome_bg);
        render_pane(frame, right, rows, cursor, hint, pal, right_w);
    }
    render_chat_column(frame, st, p.chat, prepared, panel.is_none(), spin);

    paint_bg(frame, p.status, pal.chrome_bg);
    // 分段样式已在 `status_line` 里定好（label 淡色 / 值正文色 / 分隔极暗），此处不能再套
    // 整行 `fg + BOLD`：base style 的修饰位会盖到 label 上，淡化标签就白做了。
    let status = status_line(
        info,
        st.conversation_id.as_deref(),
        p.status.width as usize,
        pal,
    );
    frame.render_widget(Paragraph::new(status), p.status);

    // 设置面板浮层：整屏覆盖（含顶栏/状态栏），最后绘制压住全部内容。
    if let Some(content) = panel {
        render_settings_overlay(frame, area, content, pal);
    }
}

/// 聊天列：上为主区消息（滚动 transcript），底部为 composer 输入区。
fn render_chat_column(
    frame: &mut Frame,
    st: &UiState,
    area: Rect,
    prepared: &[BodyRow],
    cursor_allowed: bool,
    spin: Option<usize>,
) {
    // composer 含上下边框；至少给聊天区留 1 行，极小终端下不把 transcript 挤没。
    let rows = composer_outer_rows(st)
        .min(area.height.saturating_sub(1))
        .max(1);
    let parts = Layout::vertical([Constraint::Min(1), Constraint::Length(rows)]).split(area);
    let body_area = parts[0];
    render_body(frame, st, body_area, prepared, spin);
    if st.approval.is_some() {
        render_approval_overlay(frame, st, body_area);
    }
    // `/` 补全菜单贴在 composer 正上方；审批浮层（回合被阻塞）优先，此时不画。
    if st.approval.is_none() {
        super::slash_menu::render_slash_menu(frame, st, body_area);
    }
    render_input(frame, st, parts[1], cursor_allowed);
}

/// composer 内容行数：随输入行数增长，最多 `MAX_COMPOSER_ROWS`（单行恒为 1）。
fn composer_rows(st: &UiState) -> u16 {
    st.input_line_count().clamp(1, MAX_COMPOSER_ROWS) as u16
}

/// composer 占用总行数：内容行 + 上下边框。
fn composer_outer_rows(st: &UiState) -> u16 {
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

/// 通用固定列渲染：内容窗口裁剪（含提示行）。内容行/光标/提示由 [`super::ws_sidebar`] 提供。
/// 列内左右各留 1 列空隙（对齐聊天区观感），行宽按 `width - 2` 截断。
/// `width` 与构建行时用的是同一个值（`列宽 - 2`），标题溢出计数也按它截断。
fn render_pane(
    frame: &mut Frame,
    area: Rect,
    rows: Vec<Line<'static>>,
    cursor: usize,
    hint: Option<&'static str>,
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
    // 提示行占列内最后一行（高度不足 2 行时没有提示位）。
    if area.height >= 2
        && let Some(text) = hint
    {
        let hint_area = Rect::new(area.x + 1, area.y + area.height - 1, area.width - 1, 1);
        let hint_p = Paragraph::new(Span::styled(text, Style::new().fg(pal.muted)));
        frame.render_widget(hint_p, hint_area);
    }
}

/// 空 transcript 时的快捷键引导。
const BODY_EMPTY_HINT: &str = "输入消息开始对话 · Alt+Enter 换行 · Ctrl+E 思考 · Ctrl+W 工作区树 · PgUp/PgDn 滚动 · /find 搜索 · Ctrl+C 退出 · /help";

fn render_body(
    frame: &mut Frame,
    st: &UiState,
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
    let max_back = rows_total.saturating_sub(viewport);
    let back = if let Some(target) = st.search_cursor {
        // 搜索锚定：目标逻辑行置于视口上 1/3 处。
        match rows.iter().position(|r| r.log_index == Some(target)) {
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
    // 折叠思考行的 spinner 在此替换标记：body memo 不含帧号，故不会触发重建。
    // 只命中最后一条逻辑行（`prepared` 里可能有多条历史折叠行）。
    let spin_target = spin.map(|_| st.lines.len().saturating_sub(1));
    let shown = rows[start..end.min(rows_total)]
        .iter()
        .map(|r| {
            let line = r.line.clone();
            match (spin, spin_target) {
                (Some(f), Some(idx)) if r.log_index == Some(idx) => with_progress_mark(line, f),
                _ => line,
            }
        })
        .collect::<Vec<_>>();
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

/// 输入行水平窗口起点与光标在可见文本中的显示列（设置面板文本编辑复用）。
pub(super) fn horizontal_window(content: &str, cursor_cell: usize, width: usize) -> (usize, usize) {
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
pub(super) fn cell_window(text: &str, start: usize, cells: usize) -> String {
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

fn render_input(frame: &mut Frame, st: &UiState, area: Rect, cursor_allowed: bool) {
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
    if st.current_input().trim().is_empty() {
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
