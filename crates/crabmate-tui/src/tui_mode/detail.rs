//! `/detail` 浮层：把某次工具调用的输出全文铺在主区，可滚动、只读（issue #428 第 3 条）。
//!
//! 与 `/fold`（`state_fold`）的分工是刻意的：那一个改「哪些行可见」（折叠态、可逆开合），
//! 这一个只读一行现成的文本 —— 不改 transcript、不新增第二套折叠状态机、`sticky` 也与它
//! 无关。关闭浮层是**无痕**的：屏幕上回到打开之前的样子。
//!
//! 数据来自 `UiState::tool_details`（行号 → 全文），在结果帧到达时由
//! `state_tools::tool_end` 写入。**历史回放**的 tool 行没有结构化输出（那是由会话正文
//! 重建的瘦行），因此这里找不到条目时不能假装找到了。
//!
//! 几何（[`detail_layout`]）与按键夹取（[`detail_scroll_bounds`]）共用同一份折行：
//! 两处各算一遍就会出现「`End` 停在渲染时才被截掉的越界值上，之后按 `↑` 像失灵」。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use super::LaneApp;
use super::render::overlay::centered;
use super::render::{paint_bg, truncate_display};
use super::state::{DetailOverlay, LineKind, UiState};
use super::text::wrap_display;

/// 浮层四周留白（与审批展开档同一档）。
const DETAIL_MARGIN: u16 = 4;
/// 正文宽度上限：再宽反而要横着扫，读工具输出不如窄一点折行。
const DETAIL_MAX_WIDTH: u16 = 120;
/// 低于此高度不如不铺：边框 2 + 正文 1 + 尾行 2 都放不下时，浮层只会挤掉正文。
const DETAIL_MIN_HEIGHT: u16 = 7;
/// 尾部固定行数（空行 + 位置/按键提示）。
const DETAIL_TAIL_ROWS: usize = 2;
/// `PgUp` / `PgDn` 一步滚几行。
const DETAIL_PAGE_STEP: usize = 8;
/// 标题里工具行文本最多占多少列（余下留给「工具详情 · …」的固定部分）。
const DETAIL_TITLE_COLS: usize = 48;

/// 浮层几何：矩形 + 正文折行后的总行数。
struct DetailLayout {
    rect: Rect,
    /// `is_empty()` 表示这块地方画不下（终端太窄太矮）。
    total: usize,
}

impl DetailLayout {
    /// 正文折行宽度：扣掉边框左右各 1 列，再各留 1 列内缩（与 `text()` 同一口径）。
    fn text_width(&self) -> usize {
        self.rect.width.saturating_sub(4).max(1) as usize
    }

    fn is_empty(&self) -> bool {
        self.rect.width == 0 || self.rect.height == 0
    }
}

/// 按可用区域与正文算出浮层矩形与总行数。
fn detail_layout(area: Rect, text: &str) -> DetailLayout {
    let width = area
        .width
        .saturating_sub(DETAIL_MARGIN)
        .min(DETAIL_MAX_WIDTH);
    let height = area.height.saturating_sub(DETAIL_MARGIN);
    if width == 0 || height < DETAIL_MIN_HEIGHT {
        return DetailLayout {
            rect: Rect::new(0, 0, 0, 0),
            total: 0,
        };
    }
    let mut layout = DetailLayout {
        rect: centered(area, width, height),
        total: 0,
    };
    layout.total = wrap_display(text, layout.text_width()).len();
    layout
}

/// 正文区的（总行数, 可显示行数）：**按键侧**用它夹住滚动上界。
///
/// `End` 必须能停在最后一页、`↑` 要能从那里往回走 —— 只靠渲染时截断做不到
/// （`scroll` 会停在越界值上，看起来像按键失灵）。
pub(super) fn detail_scroll_bounds(area: Rect, st: &UiState) -> (usize, usize) {
    let Some(open) = st.detail else {
        return (0, 0);
    };
    let Some(detail) = st.tool_details.get(&open.line) else {
        return (0, 0);
    };
    let layout = detail_layout(area, &detail.text);
    if layout.is_empty() {
        return (0, 0);
    }
    let rows = layout.rect.height.saturating_sub(2) as usize;
    let body = rows.saturating_sub(DETAIL_TAIL_ROWS).max(1);
    (layout.total, body)
}

/// 浮层画在聊天主体上：`Clear` 清残留 → 铺浮层底色 → 边框 + 标题 → 正文 → 尾行。
pub(super) fn render_detail_overlay(frame: &mut Frame, st: &UiState, area: Rect) {
    let Some(open) = st.detail else {
        return;
    };
    // 侧表里没有这一行（被总量预算淘汰 / 或本来就是回放行）：不画空壳，
    // 让用户看到的仍是 transcript —— 「打开了却一片空白」比什么都不做更困惑。
    let Some(detail) = st.tool_details.get(&open.line) else {
        return;
    };
    let layout = detail_layout(area, &detail.text);
    if layout.is_empty() {
        return;
    }
    let pal = &st.palette;
    frame.render_widget(Clear, layout.rect);
    paint_bg(frame, layout.rect, pal.overlay_bg);

    let block = detail_block(pal, st, open.line);
    let inner = block.inner(layout.rect);
    frame.render_widget(block, layout.rect);

    let body_h = (inner.height as usize)
        .saturating_sub(DETAIL_TAIL_ROWS)
        .max(1);
    let scroll = open.scroll.min(layout.total.saturating_sub(body_h));
    let body = wrap_display(&detail.text, layout.text_width())
        .into_iter()
        .skip(scroll)
        .take(body_h)
        .map(|text| Line::from(Span::styled(text, Style::new().fg(pal.fg))))
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(body),
        Rect::new(inner.x, inner.y, inner.width, body_h as u16),
    );

    frame.render_widget(
        Paragraph::new(detail_tail_lines(
            st,
            &layout,
            body_h,
            scroll,
            detail.truncated,
        )),
        Rect::new(
            inner.x,
            inner.y.saturating_add(body_h as u16),
            inner.width,
            (inner.height as usize).saturating_sub(body_h) as u16,
        ),
    );
}

/// 边框与标题：标题里带**工具行文本**，否则同时开过两条之后就看不出这是哪一次调用。
fn detail_block(pal: &super::palette::Palette, st: &UiState, line: usize) -> Block<'static> {
    let name = st
        .lines
        .get(line)
        .map(|l| truncate_display(l.text(), DETAIL_TITLE_COLS))
        .unwrap_or_default();
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(pal.border))
        .title(Span::styled(
            format!(" 工具详情 · {name} "),
            Style::new().fg(pal.accent).add_modifier(Modifier::BOLD),
        ))
}

/// 尾行：位置读数 + 按键；被 TUI 上限截断时明说，别让「终端里的上限」读成
/// 「工具只输出了这些」。
fn detail_tail_lines(
    st: &UiState,
    layout: &DetailLayout,
    body_h: usize,
    scroll: usize,
    truncated: bool,
) -> Vec<Line<'static>> {
    let pal = &st.palette;
    let end = (scroll + body_h).min(layout.total);
    let position = if layout.total == 0 {
        "（没有正文）".to_string()
    } else {
        format!("第 {}-{} / {} 行", scroll + 1, end, layout.total)
    };
    let cut = if truncated {
        " · 已按 TUI 上限截断（完整内容见 serve 侧）"
    } else {
        ""
    };
    vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("{position}{cut} · ↑↓/PgUp/PgDn/Home/End 滚动 · Esc 关闭"),
            Style::new().fg(pal.muted),
        )),
    ]
}

/// `/detail [n]` 的参数：`n` 从底部数（1 起，缺省最近一条）。
fn parse_detail_arg(arg: Option<&str>) -> Result<usize, String> {
    const USAGE: &str = "用法：/detail [n] —— n 从底部数（1 起），缺省 = 最近一条带详情的工具";
    let Some(raw) = arg.map(str::trim).filter(|a| !a.is_empty()) else {
        return Ok(1);
    };
    match raw.parse::<usize>() {
        Ok(n) if n >= 1 => Ok(n),
        _ => Err(USAGE.to_string()),
    }
}

impl UiState {
    /// `/detail [n]`：打开从底部数第 `n` 个**带详情**的工具行。
    ///
    /// 三种「打不开」各有独立文案（没有详情 / 越界 / 参数写错），共同的理由是：
    /// 命令已经从输入框里消失，静默不做等于让用户怀疑自己按错了。
    pub fn open_detail_command(&mut self, arg: Option<&str>) {
        let n = match parse_detail_arg(arg) {
            Ok(n) => n,
            Err(usage) => {
                self.push_line(LineKind::System, &usage);
                return;
            }
        };
        if self.tool_details.is_empty() {
            self.push_line(
                LineKind::System,
                "没有可展开的工具详情：只有本进程跑过的工具带结构化输出（历史回放行没有）",
            );
            return;
        }
        let total = self.tool_details.len();
        if n == 0 || n > total {
            let msg = format!("只有 {total} 条工具详情（n 从底部数，1 起）");
            self.push_line(LineKind::System, &msg);
            return;
        }
        // `BTreeMap` 的迭代顺序就是行号顺序，`rev().nth(n-1)` 即「从底部数第 n 条」。
        let Some((&line, _)) = self.tool_details.iter().rev().nth(n - 1) else {
            return;
        };
        self.detail = Some(DetailOverlay { line, scroll: 0 });
        // 搜索锚点让位：浮层开着时 ↑↓ 是滚动，画面不该还被 `/find` 钉在命中行上。
        self.search_cursor = None;
    }
}

/// 该键是否关闭浮层：`Esc`，或**裸** `q`（带 `Ctrl` 的 `q` 留给进程级退出语义，
/// 别在浮层里截走它）。
fn detail_key_closes(key: &KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc => true,
        KeyCode::Char('q') => !key.modifiers.contains(KeyModifiers::CONTROL),
        _ => false,
    }
}

/// 一个滚动键 → 新 `scroll`（`max_scroll` 由上界夹取给出）。
///
/// 带走 `Ctrl` 的键不算：`Ctrl+E`（thinking）/ `Ctrl+G`（折叠导航）这类组合不该被
/// 当成 `k` / `j` 的单键语义。
fn step_scroll(scroll: usize, key: &KeyEvent, max_scroll: usize) -> usize {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return scroll;
    }
    let next = match key.code {
        KeyCode::Up | KeyCode::Char('k') => scroll.saturating_sub(1),
        KeyCode::Down | KeyCode::Char('j') => scroll.saturating_add(1),
        KeyCode::PageUp => scroll.saturating_sub(DETAIL_PAGE_STEP),
        KeyCode::PageDown => scroll.saturating_add(DETAIL_PAGE_STEP),
        KeyCode::Home => 0,
        KeyCode::End => max_scroll,
        _ => return scroll,
    };
    next.min(max_scroll)
}

// 显式生命周期（不用 `impl LaneApp<'_>`）：与 `approve.rs` 同一个理由 —— lizard 会把
// `'_` 当成未闭合的字符字面量，把后面所有函数体吞进同一个函数（fn-nloc 会误报）。
impl<'a> LaneApp<'a> {
    /// 详情浮层开着时接管按键；返回 `true` = 这个键归浮层。
    ///
    /// 单独一层是为了 `on_key` 的 CCN（那条 if 链贴着门禁），也与 `consume_clarify_key`
    /// 同一手法。
    pub(super) fn consume_detail_key(&mut self, key: &KeyEvent) -> bool {
        if self.st.detail.is_none() {
            return false;
        }
        self.on_detail_key(key);
        true
    }

    /// 浮层按键：`Esc` / `q` 关闭，其余只认滚动键 —— **一律吃掉**。
    ///
    /// 吃掉而不是交回输入框：浮层盖住整块主区，用户看不见输入框，字符漏进去就是
    /// 「关掉浮层之后发现多了几句莫名其妙的话」。审批浮层同此口径。
    fn on_detail_key(&mut self, key: &KeyEvent) {
        if detail_key_closes(key) {
            self.st.detail = None;
            return;
        }
        let (total, body) = self.detail_view_bounds();
        let max_scroll = total.saturating_sub(body);
        let Some(open) = self.st.detail.as_mut() else {
            return;
        };
        open.scroll = step_scroll(open.scroll, key, max_scroll);
    }

    /// 正文区的（总行数, 可显示行数）：优先用渲染层回写的正文矩形，没画过时按整屏估。
    fn detail_view_bounds(&self) -> (usize, usize) {
        let area = self.st.body_area.unwrap_or_else(|| {
            // 与 `approve` 的估算同向：估大的方向只让滚动上界略宽，渲染侧还会再夹一次。
            let (w, h) = self.last_size;
            Rect::new(0, 0, w, h.saturating_sub(6))
        });
        detail_scroll_bounds(area, &self.st)
    }
}

#[cfg(test)]
#[path = "detail_tests.rs"]
mod detail_tests;
