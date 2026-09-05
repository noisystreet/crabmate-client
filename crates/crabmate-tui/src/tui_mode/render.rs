//! 全屏 TUI 渲染：状态行 / 左栏会话 / 右栏工作区目录树（仿 Desktop 默认显示）/
//! 主区 transcript / 底栏输入 / 审批浮层。布局纯函数为主，便于单测。

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::md;
use super::settings_panel::PanelContent;
use super::state::{Focus, LineKind, UiState};
use super::ws_sidebar::{sessions_content, sidebar_view, workspace_content};

/// 低于此宽度隐藏左右栏（等价 repl 布局）。
pub const SIDEBAR_MIN_WIDTH: u16 = 120;
/// 左会话列 / 右工作区列宽度。
pub(crate) const SIDEBAR_WIDTH: u16 = 26;

/// 计算 build_body_rows 使用的聊天区宽度（与 draw 里传给 render_body 的区域一致：
/// 宽屏时左右各占一列）。
pub(crate) fn chat_body_width(sidebar_visible: bool, terminal_width: u16) -> usize {
    let col = if sidebar_visible {
        terminal_width.saturating_sub(SIDEBAR_WIDTH * 2)
    } else {
        terminal_width
    };
    (col as usize).saturating_sub(1)
}
pub(super) const SIDEBAR_HINT: &str = "↑↓选 Enter用 n新建 r刷新";
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
#[derive(Clone)]
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
/// 供事件循环做帧间 memo（内容/宽度/搜索未变时复用），渲染层拿结果直接裁剪。
pub(crate) fn build_body_rows(st: &UiState, width: usize) -> Vec<BodyRow> {
    let needle = st.search_term();
    let target = st.search_cursor;
    let mut out: Vec<BodyRow> = Vec::new();
    // 相邻的两个消息气泡（用户/助手块）之间插入一个空行；中间夹 thinking/工具/
    // 系统行时属于同回合，不打断（只比较相邻日志行，避免在思考/工具与正文间空行）。
    let mut prev_bubble = false;
    for (idx, log) in st.lines.iter().enumerate() {
        let (prefix, style) = kind_style(log.kind);
        let prefix_w = prefix.chars().count();
        let content_width = width.saturating_sub(prefix_w);
        let matched = needle.is_some_and(|n| log.text.to_lowercase().contains(n));
        let anchor_log = matched && target == Some(idx);
        let is_bubble = matches!(log.kind, LineKind::User | LineKind::Assistant);
        if is_bubble && prev_bubble {
            out.push(BodyRow {
                log_index: idx,
                line: Line::from(""),
            });
        }
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
        prev_bubble = is_bubble;
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
    for phys in md::wrap_physical(text, content_width) {
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

/// 按显示宽度截断（用于状态行），尾部补 `…`。
pub(super) fn truncate_display(text: &str, width: usize) -> String {
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
pub(super) fn workspace_basename(path: &str) -> String {
    let p = path.trim_end_matches('/');
    match p.rsplit('/').next().filter(|s| !s.is_empty()) {
        Some(base) => base.to_string(),
        None => path.to_string(),
    }
}

/// 顶栏内容：仅工作区名（`basename`，对齐 Desktop 标题栏：不显示“工作区: ”前缀与
/// 绝对路径；未获取时占位）。
fn top_text(st: &UiState, width: usize) -> String {
    let Some(path) = st
        .workspace_path
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    else {
        return truncate_display("（未获取）…", width);
    };
    let name = workspace_basename(path);
    truncate_display(&name, width)
}

/// 状态行左侧内容：serve / model / role / mode / conv 与搜索、回看标记
/// （不含运行态——运行态在整行右缘右对齐，仿 Desktop 状态栏）。
fn status_left_text(info: &StatusInfo, conv: Option<&str>) -> String {
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
    parts.join(" | ")
}

/// 状态行右侧运行态指示（空闲 / 生成中 / 取消中）。
fn status_running_text(info: &StatusInfo) -> &'static str {
    match (info.running, info.cancel_sent) {
        (true, true) => "… 取消中",
        (true, false) => "● 生成中",
        (false, _) => "○ 空闲",
    }
}

/// 把 `right` 显示列宽感知地对齐到整行右缘（`left` 先截断，避免挤压右缘指示）。
fn pad_right(left: &str, right: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let right_w = UnicodeWidthStr::width(right);
    let cut = truncate_display(left, width.saturating_sub(right_w));
    let pad = width.saturating_sub(UnicodeWidthStr::width(cut.as_str()) + right_w);
    format!("{cut}{}{right}", " ".repeat(pad))
}

/// 状态行内容（单行）：左侧上下文 + 右对齐的空闲/生成中指示（仿 Desktop）。
fn status_text(info: &StatusInfo, conv: Option<&str>, width: usize) -> String {
    pad_right(
        &status_left_text(info, conv),
        status_running_text(info),
        width,
    )
}

/// 左栏内容行：首行标题，其后每会话一行（`>` 当前，`*` serve 活跃，选中高亮）。
pub(super) fn sidebar_rows(st: &UiState, width: usize) -> Vec<Line<'static>> {
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
    // 顶栏工作区名称居中（对齐 Desktop 标题栏观感）。
    let top = Paragraph::new(top_text(st, top_area.width as usize))
        .alignment(Alignment::Center)
        .style(Style::new().fg(Color::White).add_modifier(Modifier::BOLD));
    frame.render_widget(top, top_area);

    if st.sidebar_visible {
        let cols = Layout::horizontal([
            Constraint::Length(SIDEBAR_WIDTH),
            Constraint::Min(0),
            Constraint::Length(SIDEBAR_WIDTH),
        ])
        .split(body_area);
        let (rows, cursor, hint) = sessions_content(st, cols[0].width.saturating_sub(2) as usize);
        paint_bg(frame, cols[0], SIDEBAR_BG);
        render_pane(frame, cols[0], rows, cursor, hint);
        let (rows, cursor, hint) = workspace_content(st, cols[2].width.saturating_sub(2) as usize);
        paint_bg(frame, cols[2], SIDEBAR_BG);
        render_pane(frame, cols[2], rows, cursor, hint);
        render_chat_column(frame, st, cols[1], prepared, panel.is_none());
    } else {
        render_chat_column(frame, st, body_area, prepared, panel.is_none());
    }

    paint_bg(frame, status_area, STATUS_BG);
    let status = Paragraph::new(status_text(
        info,
        st.conversation_id.as_deref(),
        status_area.width as usize,
    ))
    .style(Style::new().fg(STATUS_FG).add_modifier(Modifier::BOLD));
    frame.render_widget(status, status_area);

    // 设置面板浮层：整屏覆盖（含顶栏/状态栏），最后绘制压住全部内容。
    if let Some(content) = panel {
        render_settings_overlay(frame, area, content);
    }
}

/// 聊天列：上为主区消息（滚动 transcript），底部为 composer 输入区。
fn render_chat_column(
    frame: &mut Frame,
    st: &UiState,
    area: Rect,
    prepared: &[BodyRow],
    cursor_allowed: bool,
) {
    let rows = composer_rows(st).min(area.height);
    let parts = Layout::vertical([Constraint::Min(1), Constraint::Length(rows)]).split(area);
    let body_area = parts[0];
    render_body(frame, st, body_area, prepared);
    if st.approval.is_some() {
        render_approval_overlay(frame, st, body_area);
    }
    render_input(frame, st, parts[1], cursor_allowed);
}

/// composer 高度：随输入行数增长，最多 `MAX_COMPOSER_ROWS`（单行恒为 1）。
fn composer_rows(st: &UiState) -> u16 {
    st.input_line_count().clamp(1, MAX_COMPOSER_ROWS) as u16
}

/// 通用固定列渲染：内容窗口裁剪（含提示行）。内容行/光标/提示由 [`super::ws_sidebar`] 提供。
/// 列内左右各留 1 列空隙（对齐聊天区观感），行宽按 `width - 2` 截断。
fn render_pane(
    frame: &mut Frame,
    area: Rect,
    rows: Vec<Line<'static>>,
    cursor: usize,
    hint: Option<&'static str>,
) {
    if area.width <= 1 || area.height == 0 {
        return;
    }
    let inner = Rect::new(area.x + 1, area.y, area.width - 1, area.height);
    let cols = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(inner);
    let shown = sidebar_view(&rows, cols[0].height as usize, cursor);
    let list = Paragraph::new(shown).style(Style::new().fg(Color::White));
    frame.render_widget(list, cols[0]);
    if cols[1].height > 0
        && let Some(text) = hint
    {
        let hint_p = Paragraph::new(Span::styled(text, Style::new().fg(Color::Gray)));
        frame.render_widget(hint_p, cols[1]);
    }
}

/// 空 transcript 时的快捷键引导。
const BODY_EMPTY_HINT: &str = "输入消息开始对话 · Alt+Enter 换行 · Ctrl+E 思考 · Ctrl+W 工作区树 · PgUp/PgDn 滚动 · /find 搜索 · Ctrl+C 退出 · /help";

fn render_body(frame: &mut Frame, st: &UiState, area: Rect, prepared: &[BodyRow]) {
    paint_bg(frame, area, CHAT_BG);
    if area.width <= 1 || area.height == 0 {
        return;
    }
    // 与区域左缘留 1 列间距（右缘贴边；折行/memo 宽度不变）。
    let text_area = Rect::new(
        area.x.saturating_add(1),
        area.y,
        area.width.saturating_sub(1),
        area.height,
    );
    if st.lines.is_empty() {
        let hint = Paragraph::new(Span::styled(BODY_EMPTY_HINT, Style::new().fg(Color::White)));
        frame.render_widget(hint, text_area);
        return;
    }
    let rows = prepared;
    let rows_total = rows.len();
    let viewport = (text_area.height as usize).saturating_sub(1).max(1);
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
    let shown = rows[start..end.min(rows_total)]
        .iter()
        .map(|r| r.line.clone())
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(shown), text_area);
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

/// 设置面板浮层：整屏 Clear + 独立色块 + 边框块（模板同审批浮层），
/// 内容行由 [`super::settings_panel`] 构建（已按内宽截断），编辑时给出硬件光标位。
fn render_settings_overlay(frame: &mut Frame, area: Rect, content: &PanelContent) {
    let bg = Color::Indexed(234);
    frame.render_widget(Clear, area);
    paint_bg(frame, area, bg);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(Color::Yellow))
        .title(Span::styled(
            " 设置（模型 / 会话） ",
            Style::new().fg(Color::Yellow),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let shown: Vec<Line<'static>> = content
        .lines
        .iter()
        .take(inner.height as usize)
        .cloned()
        .collect();
    frame.render_widget(Paragraph::new(shown), inner);
    // 文本编辑光标：内容行下标 + cell 列（仅在浮层内可见区域落位）。
    if let Some((row, col)) = content.cursor
        && (row as u16) < inner.height
        && (col as u16) < inner.width
    {
        frame.set_cursor_position((inner.x + col as u16, inner.y + row as u16));
    }
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

fn render_input(frame: &mut Frame, st: &UiState, area: Rect, cursor_allowed: bool) {
    if area.width <= 1 || area.height == 0 {
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
        let row_area = Rect::new(
            area.x.saturating_add(1),
            area.y.saturating_add(i as u16),
            area.width.saturating_sub(1),
            1,
        );
        let paragraph = Paragraph::new(Line::from(Span::raw(text.as_str())))
            .style(Style::new().fg(Color::White));
        frame.render_widget(paragraph, row_area);
    }
    // 全屏浮层（设置面板）打开时不需要 composer 硬件光标，避免光标漏到浮层外。
    if st.focus != Focus::Input || !cursor_allowed {
        return;
    }
    let col = area
        .x
        .saturating_add(1)
        .saturating_add(shown_cursor as u16)
        .min(area.x.saturating_add(area.width.saturating_sub(1)));
    let row = area
        .y
        .saturating_add(disp_row as u16)
        .min(area.y.saturating_add(area.height.saturating_sub(1)));
    frame.set_cursor_position((col, row));
}

#[cfg(test)]
#[path = "render_tests.rs"]
mod tests;
