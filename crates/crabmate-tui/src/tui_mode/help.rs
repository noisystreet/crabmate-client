//! `/help`：命令来自共享条目表，TUI 专属的键位与区域操作在这里补；产出是一层**可滚动浮层**
//! （旧版往 transcript 灌三十条 200~600 字符的长句，把对话顶走且事后无法回看）。
//!
//! 三条分工（与 `crabmate_client_api::help` 的模块文档同一口径）：
//!
//! - **命令 + usage + 摘要**：`crabmate_client_api::help`，与 Desktop 同一份，按
//!   [`Surface::Tui`] 过滤 —— 各端能敲哪些命令由 `slash_caps` 决定，这里不另立名单；
//! - **键位**：`hotkey_docs`（TUI 的 `F6` / `Ctrl+T` 在 Web 上不存在，不共享）；
//! - **区域操作**：会话栏 / 工作区树的键盘导航，Web 是鼠标，没有共同形态。

use crabmate_client_api::help as api_help;
use crabmate_client_api::slash_caps::Surface;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use super::LaneApp;
use super::hotkeys::{self, HotkeyGroup};
use super::state::UiState;

/// 左列宽度：最长 usage 是 `/copy [assistant|user|path|code]`（32 列）。
pub(super) const USAGE_COL: usize = 34;

/// 键位分组的小标题前缀（`hotkey_docs` 的分组名接在后面）。
const KEY_GROUP_PREFIX: &str = "";

/// 判定散在别的模块、没有热键条目的键（快捷键分组之外的那些）。
const MISC_KEYS_LINE: &str =
    "其它键：Ctrl+G 折叠导航 · PgUp/PgDn 翻页 · Ctrl+End 回底部 · 回合中 Enter 排队发送";

/// 分区域操作（会话栏 / 连接 / 工作区 / 审批 / 澄清）。只写「在哪按什么」，长尾规则进 docs。
const AREA_LINES: &[(&str, &str)] = &[
    (
        "会话栏",
        "↑↓ 选 · Enter 用 · n 新建 · r 刷新 · Del 删除（再确认）",
    ),
    (
        "连接",
        "左栏下半列出全部 serve 连接（单条也显示，> 为当前）",
    ),
    (
        "工作区树",
        "↑↓ 选 · Enter/→ 展开 · ← 收起/回父 · r 刷新 · Tab 回输入",
    ),
    (
        "项目切换",
        "p 从项目池选择/切换 · ↑↓ 选 · Enter 切换 · r 刷新",
    ),
    ("审批浮层", "Enter 一次 · a 始终 · Esc/n 拒绝"),
    (
        "澄清问卷",
        "Tab/↑↓ 换题 · Enter 提交 · Esc 收起（等于没答）· F7 重开",
    ),
];

/// 浮层的非正文行数：上下边框 + 标题 / 页脚提示。
const OVERLAY_CHROME_ROWS: u16 = 4;

/// `/help` 浮层的滚动状态（`total` 在打开时算一次，按键时不必重建整张表）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HelpOverlay {
    pub(super) scroll: usize,
    pub(super) total: usize,
}

/// 帮助正文的一行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelpRow {
    /// 分组标题（渲染成 accent 色）。
    Group(&'static str),
    /// 左列 + 右列（命令 / 摘要、区域 / 按键、键组 / 键位）。
    Pair { left: String, right: String },
    /// 整行说明（无左列）。
    Note(String),
}

/// 浮层可见的正文行数：渲染与按键共用这一处算式（两处各算一次就会「滚不到底」）。
pub(super) fn help_view_rows(screen_h: u16) -> usize {
    usize::from(screen_h.saturating_sub(OVERLAY_CHROME_ROWS)).max(1)
}

/// 帮助正文：命令 → 快捷键 → 区域操作。
///
/// 每次打开重建（约 35 行、无 IO）。`Simple` 是刻意的：帮助是**静态表**，不值得为它缓存。
#[must_use]
pub fn help_rows() -> Vec<HelpRow> {
    let mut rows = Vec::new();
    rows.push(HelpRow::Group("命令 / commands"));
    for e in api_help::entries(Surface::Tui) {
        rows.push(HelpRow::Pair {
            left: e.usage.to_string(),
            right: e.summary.to_string(),
        });
    }
    rows.push(HelpRow::Group("快捷键 / keys"));
    for group in HotkeyGroup::ALL {
        rows.push(HelpRow::Pair {
            left: format!("{KEY_GROUP_PREFIX}{}", group.label()),
            right: hotkeys::group_line(group).to_string(),
        });
    }
    rows.push(HelpRow::Note(MISC_KEYS_LINE.to_string()));
    rows.push(HelpRow::Group("区域操作 / panes"));
    for (area, keys) in AREA_LINES {
        rows.push(HelpRow::Pair {
            left: (*area).to_string(),
            right: (*keys).to_string(),
        });
    }
    rows
}

impl UiState {
    /// `/help`：打开浮层。**不再**往 transcript 灌行 —— 那正是「看一次顶走三十行、想再看
    /// 又要再顶一次」的来源。
    pub(super) fn show_help(&mut self) {
        // 与 `/detail` 互斥：两个都是占主区的只读浮层，叠着画只会互相盖住。
        self.detail = None;
        self.help = Some(HelpOverlay {
            scroll: 0,
            total: help_rows().len(),
        });
    }
}

/// 绘制帮助浮层（`render.rs` 的 body 分支调用）。
///
/// 占满正文区：`/help` 是「查一眼就走」的模态，切成小框会让本就不宽的行更挤。滚动窗口
/// 与 [`help_view_rows`] 同一口径 —— 两处各算一遍就是「滚不到底」的来源。
pub(super) fn render_help_overlay(frame: &mut Frame, st: &UiState, area: Rect) {
    let Some(overlay) = st.help.as_ref() else {
        return;
    };
    let pal = &st.palette;
    let rows = help_rows();
    let view = help_view_rows(area.height);
    let max = rows.len().saturating_sub(view);
    let scroll = overlay.scroll.min(max);
    // 先清字形再铺主题底色：浅色主题下否则会和底下正文视觉重叠（与审批 / 澄清浮层同处理）。
    frame.render_widget(Clear, area);
    super::render::paint_bg(frame, area, pal.overlay_bg);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(pal.border))
        .title(Span::styled(
            " 帮助 · Esc 关闭 · ↑↓/PgUp/PgDn 滚动 ",
            Style::new().fg(pal.accent).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let lines: Vec<Line<'static>> = rows
        .into_iter()
        .skip(scroll)
        .take(view)
        .map(|row| match row {
            HelpRow::Group(title) => Line::from(Span::styled(
                title.to_string(),
                Style::new().fg(pal.accent).add_modifier(Modifier::BOLD),
            )),
            HelpRow::Pair { left, right } => Line::from(vec![
                // 左列淡色、右列正文色：与状态行「label / value」同一套层级，扫读时才分得开。
                Span::styled(format!("  {left:<USAGE_COL$}"), Style::new().fg(pal.muted)),
                Span::styled(right, Style::new().fg(pal.fg)),
            ]),
            HelpRow::Note(text) => Line::from(Span::styled(
                format!("  {text}"),
                Style::new().fg(pal.muted),
            )),
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

// 显式生命周期（不用 `impl LaneApp<'_>`）：与 `detail.rs` / `approve.rs` 同一理由 ——
// lizard 的 C 词法器会把 `'_` 当成未闭合的字符字面量，把后面所有函数体吞进同一个函数。
impl<'a> LaneApp<'a> {
    /// 帮助浮层开着时接管按键；返回 `true` = 这个键归浮层。
    ///
    /// 与 [`Self::consume_detail_key`] 同一条口径：模态期间**一律吃掉**其它键 —— 浮层盖住
    /// 主区，字符漏进输入框就是「关掉之后发现多了几句莫名其妙的话」。
    pub(super) fn consume_help_key(&mut self, key: &KeyEvent) -> bool {
        if self.st.help.is_none() {
            return false;
        }
        if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
            self.st.help = None;
            return true;
        }
        // 可见行数与渲染共用 `help_view_rows`：各算一遍就会出现「滚不到底 / 滚过头」。
        let view = help_view_rows(self.last_size.1);
        let Some(overlay) = self.st.help.as_mut() else {
            return true;
        };
        let max = overlay.total.saturating_sub(view);
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                overlay.scroll = overlay.scroll.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                overlay.scroll = overlay.scroll.saturating_add(1);
            }
            KeyCode::PageUp => overlay.scroll = overlay.scroll.saturating_sub(view),
            KeyCode::PageDown => overlay.scroll = overlay.scroll.saturating_add(view),
            KeyCode::Home => overlay.scroll = 0,
            KeyCode::End => overlay.scroll = max,
            _ => {}
        }
        overlay.scroll = overlay.scroll.min(max);
        true
    }
}

#[cfg(test)]
mod tests {
    use crabmate_client_api::slash_caps::{Surface, supported};

    use super::{AREA_LINES, HelpRow, USAGE_COL, help_rows};

    /// `/help` 的命令清单必须覆盖本端**每一个**能敲的共享 head。
    ///
    /// 清单来自共享条目表（不再手写字面量），本用例钉的是「按 Surface::Tui 过滤后仍覆盖
    /// 全部 head」—— 加 head 忘了补条目时，共享层与这里都会红。
    #[test]
    fn help_lists_every_tui_head() {
        let lefts: Vec<String> = help_rows()
            .into_iter()
            .filter_map(|r| match r {
                HelpRow::Pair { left, .. } => Some(left),
                _ => None,
            })
            .collect();
        for head in supported(Surface::Tui) {
            let needle = format!("/{}", head.as_str());
            assert!(
                lefts.iter().any(|l| l.starts_with(&needle)),
                "/help 命令清单漏了 {needle}"
            );
        }
    }

    /// 左列宽度要容得下最长的那条 usage / 区域名，否则右列被顶歪 —— 对齐一失效就退回「文字墙」。
    #[test]
    fn usage_column_fits_the_longest_entry() {
        for e in crabmate_client_api::help::entries(Surface::Tui) {
            assert!(
                e.usage.chars().count() < USAGE_COL,
                "{} 超出左列宽度 {USAGE_COL}，右列会错位",
                e.usage
            );
        }
        for (area, _) in AREA_LINES {
            assert!(
                area.chars().count() < USAGE_COL,
                "区域名 {area} 超出左列宽度"
            );
        }
    }
}
