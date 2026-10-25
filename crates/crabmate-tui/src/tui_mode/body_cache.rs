//! transcript body 的**行级**缓存。
//!
//! 原先 `prepare_body` 用一个帧间 memo 键（内容代数 + 宽度 + 搜索）挡住「没变就不重建」，
//! 但流式期每个增量都会让代数 +1 —— 于是每一帧都把**整个** transcript 重新做一遍
//! markdown 解析与折行。一轮长回答就是「累计字符数 × 增量次数」，输出越久越卡。
//!
//! 这里按**逻辑行**缓存：只有文本/折叠位/工具状态变了的行才重新解析，其余行复用。
//! 组装扁平结果时只做搬运，并把每个逻辑行在扁平数组里的起点记下来，
//! 于是「只有最后一行变了」这种最常见的情形只需要重做一行。
//!
//! 失效维度：
//! - 宽度或搜索词变化 → 整表重建（每行折行点、高亮状态都变）
//! - 行数变少 → 整表重建（`reset_transcript` / 回放替换）
//! - 单行 `rev` / 折叠位 / 命中态 / 锚点态变化 → 从该行起重建到末尾
//!
//! 「从该行起重建到末尾」而不是只 patch 该行，是因为物理行数不固定：中间插入一行
//! 会让后面所有行的起点平移。流式主路径改的总是最后一行（尾部重建 = 一行），
//! 而工具结果回填、搜索跳转这类事件频率低，重建尾部可以接受。

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::palette::Palette;
use super::render::{BodyRow, line_rows, prefix_for, prefix_width};
use super::state::{HistoryPage, LineKind, LogLine, UiState};
use super::text::truncate_display;

/// 单条逻辑行的缓存项。
struct CachedLine {
    kind: LineKind,
    /// 内容代数（`LogLine::rev`）：文本追加/替换、工具成败标记变化都会 +1。
    rev: u64,
    collapsed: bool,
    matched: bool,
    anchor: bool,
}

impl CachedLine {
    fn still_valid(&self, log: &LogLine, matched: bool, anchor: bool) -> bool {
        self.kind == log.kind()
            && self.rev == log.rev()
            && self.collapsed == log.is_collapsed()
            && self.matched == matched
            && self.anchor == anchor
    }
}

/// 见模块头注释。
#[derive(Default)]
pub(super) struct BodyCache {
    init: bool,
    width: usize,
    /// 建立缓存时的搜索词（变则整表失效：高亮位置全变）。
    needle: Option<String>,
    /// 建立缓存时的配色板。主题也是渲染输入（前缀/正文/高亮色都取自此），
    /// 变了必须整表重建 —— 否则设置面板里换主题后聊天区会停在旧配色。
    palette: Palette,
    /// 顶部分页提示（`history_page` 的投影）：它变了要整表重建，
    /// 否则「还有更早」出现 / 消失时聊天区不会有任何变化。
    banner: Option<&'static str>,
    lines: Vec<CachedLine>,
    /// 组装好的扁平物理行（含块间分隔行）。
    flat: Vec<BodyRow>,
    /// `starts[i]` = 第 i 条逻辑行在 `flat` 中的起点（含它的前置分隔行）。
    starts: Vec<usize>,
    /// 测试用：累计重建的逻辑行数。用来断言流式期不是全量重建。
    #[cfg(test)]
    rebuilt: usize,
}

impl BodyCache {
    /// 保证缓存与 `st` 一致，然后供渲染层取用。
    pub(super) fn ensure(&mut self, st: &UiState, width: usize) {
        let needle = st.search_term();
        let banner = history_banner(st.history_page);
        let structural = self.needs_full_rebuild(width, needle, st.palette, st.lines.len(), banner);
        if structural {
            self.init = true;
            self.width = width;
            self.banner = banner;
            self.needle = needle.map(str::to_owned);
            self.palette = st.palette;
            self.lines.clear();
            self.starts.clear();
            self.flat.clear();
        }
        let anchor = st.search_cursor;
        let mut dirty = usize::MAX;
        for (i, log) in st.lines.iter().enumerate() {
            let matched = st.search_hits.binary_search(&i).is_ok();
            match self.lines.get(i) {
                Some(c) if c.still_valid(log, matched, anchor == Some(i)) => {}
                _ => {
                    dirty = i;
                    break;
                }
            }
        }
        if dirty == usize::MAX {
            return;
        }
        self.rebuild_from(st, dirty, width, anchor);
    }

    /// 是否必须整表重建（宽度 / 搜索词 / 配色 / 行数收缩 / 分页提示变化）。
    ///
    /// 抽成独立函数：`ensure` 的圈复杂度贴着门禁（≤10），再往它的 `||` 链上加一项就会破线。
    fn needs_full_rebuild(
        &self,
        width: usize,
        needle: Option<&str>,
        palette: Palette,
        line_count: usize,
        banner: Option<&'static str>,
    ) -> bool {
        !self.init
            || self.width != width
            || self.banner != banner
            || self.needle.as_deref() != needle
            || palette != self.palette
            || self.lines.len() > line_count
    }

    /// 组装后的扁平行；须在 [`Self::ensure`] 之后调用。
    pub(super) fn as_slice(&self) -> &[BodyRow] {
        &self.flat
    }

    fn rebuild_from(&mut self, st: &UiState, from: usize, width: usize, anchor: Option<usize>) {
        let cut = self.starts.get(from).copied().unwrap_or(self.flat.len());
        self.flat.truncate(cut);
        self.starts.truncate(from);
        self.lines.truncate(from);
        // 顶部分页提示排在最前：它不属于任何逻辑行（`log_index: None`），
        // 于是搜索锚点、spinner 目标这些「按逻辑行定位」的逻辑会自然跳过它。
        if from == 0
            && let Some(row) = history_banner_row(st, width)
        {
            self.flat.push(row);
        }
        #[cfg(test)]
        {
            self.rebuilt += st.lines.len() - from;
        }

        let pal = &st.palette;
        // 分隔线缩进取**上一条**逻辑行的前缀宽度，所以要从 `from - 1` 取。
        let mut prev = from
            .checked_sub(1)
            .and_then(|i| st.lines.get(i))
            .map(|l| (l.kind(), prefix_width(prefix_for(l))));
        for i in from..st.lines.len() {
            let log = &st.lines[i];
            let matched = st.search_hits.binary_search(&i).is_ok();
            let is_anchor = anchor == Some(i);
            let start = self.flat.len();
            if let Some((_, prev_indent)) = prev.filter(|(k, _)| *k != log.kind()) {
                self.flat.push(BodyRow::separator(width, prev_indent, pal));
            }
            let rows = line_rows(log, i, width, matched, is_anchor, pal);
            self.flat.extend(rows);
            let prefix_w = prefix_width(prefix_for(log));
            self.lines.push(CachedLine {
                kind: log.kind(),
                rev: log.rev(),
                collapsed: log.is_collapsed(),
                matched,
                anchor: is_anchor,
            });
            self.starts.push(start);
            prev = Some((log.kind(), prefix_w));
        }
    }
}

/// 顶部分页提示文案（`history_page` 的投影；`None` = 不显示）。
///
/// 放在 body 的**第一条物理行**而不是 sticky 在视口顶：Desktop 的「加载更早」也在
/// 对话顶部，滚到顶才看得见，语义一致。
fn history_banner(page: Option<HistoryPage>) -> Option<&'static str> {
    let page = page?;
    if page.loading {
        return Some("↑ 正在加载更早的消息…");
    }
    page.has_older.then_some("↑ 还有更早的消息：/older 加载")
}

/// 提示行本体：左对齐、`muted` 色、按显示宽截断（与其它物理行同宽预算）。
///
/// `pub(super)` 给 `render::build_body_rows` 复用 —— 缓存与「无缓存全量重建」必须
/// 产出逐行一致的结果，否则 `assert_same_as_full_rebuild` 会红。
pub(super) fn history_banner_row(st: &UiState, width: usize) -> Option<BodyRow> {
    let text = history_banner(st.history_page)?;
    Some(BodyRow {
        log_index: None,
        line: Line::from(Span::styled(
            truncate_display(text, width),
            Style::new()
                .fg(st.palette.muted)
                .add_modifier(Modifier::DIM),
        )),
    })
}

/// 正文视口的滚动窗口（纯计算，便于单测；渲染层只负责把 `start..end` 画出来）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ScrollWindow {
    /// 要画的物理行区间（半开）。
    pub start: usize,
    pub end: usize,
    /// 视口是否已贴到内容顶（内容不足一屏时恒 `false`：没有可滚的余量）。
    pub at_top: bool,
}

/// 算正文视口：`anchor_pos` 是搜索命中行的**物理**行号（`/find` 跳转时优先它）。
///
/// 两条路径都过同一处 `.min(max_back)` 兜底。搜索锚定的 `end` 理论上不会小于
/// `min(viewport, rows_total)`，但那是要推导才成立的不变量 —— 不值得让渲染层依赖它，
/// 也不值得让下一个人猜「为什么这里不用 clamp」。
pub(super) fn scroll_window(
    rows_total: usize,
    viewport: usize,
    view_offset: usize,
    anchor_pos: Option<usize>,
) -> ScrollWindow {
    let max_back = rows_total.saturating_sub(viewport);
    let back = match anchor_pos {
        // 搜索锚定：把目标行放到视口上 1/3 处。
        Some(pos) => {
            let want_start = pos.saturating_sub(viewport / 3);
            rows_total.saturating_sub((want_start + viewport).min(rows_total))
        }
        None => view_offset,
    }
    .min(max_back);
    let end = rows_total.saturating_sub(back);
    ScrollWindow {
        start: end.saturating_sub(viewport),
        end,
        at_top: rows_total > viewport && back >= max_back,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui_mode::render::build_body_rows;

    /// 缓存结果必须与「无缓存的全量重建」逐行一致 —— 这是这个模块唯一重要的事。
    ///
    /// 比较用 `Debug` 而不是 `to_string()`：后者只比文本，样式（配色、搜索高亮、
    /// 工具行成败色）全被忽略 —— 主题失效这类 bug 就会从测试里漏过去。
    fn assert_same_as_full_rebuild(st: &UiState, cache: &mut BodyCache, width: usize) {
        cache.ensure(st, width);
        let got: Vec<(Option<usize>, String)> = cache
            .as_slice()
            .iter()
            .map(|r| (r.log_index, format!("{:?}", r.line)))
            .collect();
        let want: Vec<(Option<usize>, String)> = build_body_rows(st, width)
            .iter()
            .map(|r| (r.log_index, format!("{:?}", r.line)))
            .collect();
        assert_eq!(got, want, "缓存与全量重建不一致（width={width}）");
    }

    /// 把整批行换成另一批等长的行（不经过 `reset_transcript`）：行版本是全局唯一的，
    /// 所以「长度相同、kind 相同、只有文本不同」的替换也必须被逐行发现。
    #[test]
    fn wholesale_replacement_without_reset_is_detected() {
        let mut st = UiState::new();
        st.push_line(LineKind::User, "旧的一行");
        st.push_line(LineKind::Assistant, "旧的二行");
        let mut cache = BodyCache::default();
        assert_same_as_full_rebuild(&st, &mut cache, 80);

        st.lines[0] = LogLine::replayed(LineKind::User, "新的一行", false, None);
        st.lines[1] = LogLine::replayed(LineKind::Assistant, "新的二行", false, None);
        assert_same_as_full_rebuild(&st, &mut cache, 80);
    }

    /// 视口窗口的三种态：内容不足一屏时不算「在顶」；`back == max_back` 才算；中间不算。
    /// 这条判定决定「前插是否继续贴顶」和「PgUp 要不要续翻」，改错会静默失效。
    #[test]
    fn scroll_window_marks_top_only_when_pinned_and_scrollable() {
        // 不足一屏：没有可滚的余量，谈不上「在顶」。
        let short = scroll_window(5, 10, crate::tui_mode::state::SCROLL_TOP, None);
        assert!(!short.at_top, "内容不足一屏不该判在顶");
        assert_eq!((short.start, short.end), (0, 5));

        // 贴顶哨兵：clamp 到 max_back，视口顶对齐第一行。
        let top = scroll_window(100, 10, crate::tui_mode::state::SCROLL_TOP, None);
        assert!(top.at_top);
        assert_eq!((top.start, top.end), (0, 10), "贴顶时画前 10 行");

        // 回到底部：跟随最新。
        let bottom = scroll_window(100, 10, 0, None);
        assert!(!bottom.at_top);
        assert_eq!((bottom.start, bottom.end), (90, 100));

        // 恰好停在顶上（PgUp 累积到 max_back，不是哨兵）同样算在顶。
        let exactly = scroll_window(100, 10, 90, None);
        assert!(exactly.at_top, "back == max_back 就是贴顶");
    }

    /// 搜索锚定分支一样受 `max_back` 兜底：命中首行不越界（且等于贴顶），命中末行贴着下沿。
    #[test]
    fn scroll_window_clamps_the_search_anchor() {
        let first = scroll_window(100, 10, 0, Some(0));
        assert_eq!((first.start, first.end), (0, 10));
        assert!(first.at_top, "锚定到首行等于贴顶");

        let last = scroll_window(100, 10, 0, Some(99));
        assert_eq!((last.start, last.end), (90, 100), "末行落在视口下沿");
        assert!(!last.at_top);
    }

    /// 顶部分页提示（`/older`）也走「缓存 vs 全量重建」同一条路：两路必须产出同一行，
    /// 且 `loading` 态切换时要整表重建（否则提示不会变成「正在加载」）。
    #[test]
    fn history_banner_row_matches_full_rebuild() {
        let mut st = UiState::new();
        st.push_line(LineKind::User, "已加载的最新页");
        st.set_history_page(40, true);
        let mut cache = BodyCache::default();

        assert_same_as_full_rebuild(&st, &mut cache, 80);
        assert!(
            first_row_text(&cache).contains("更早"),
            "首行应是分页提示：{}",
            first_row_text(&cache)
        );

        st.set_older_loading(true);
        assert_same_as_full_rebuild(&st, &mut cache, 80);
        assert!(
            first_row_text(&cache).contains("正在加载"),
            "loading 态要换文案：{}",
            first_row_text(&cache)
        );
    }

    fn first_row_text(cache: &BodyCache) -> String {
        cache.as_slice()[0]
            .line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    /// 主题切换必须整表重建：颜色是渲染输入，漏掉它聊天区会停在旧配色。
    #[test]
    fn palette_change_rebuilds_styles() {
        let mut st = UiState::new();
        st.push_line(LineKind::User, "已渲染的内容");
        st.start_search("内容");
        let mut cache = BodyCache::default();
        assert_same_as_full_rebuild(&st, &mut cache, 80);

        st.set_palette(Palette::from_theme(Some("shadcn-dark")));
        assert_same_as_full_rebuild(&st, &mut cache, 80);
        st.set_palette(Palette::mono());
        assert_same_as_full_rebuild(&st, &mut cache, 80);
    }

    #[test]
    fn cache_matches_full_rebuild_while_streaming() {
        let mut st = UiState::new();
        st.push_line(LineKind::User, "问题");
        st.stream_delta(LineKind::Assistant, "# 标题\n\n正文 **粗** 与 `code`");
        let mut cache = BodyCache::default();
        assert_same_as_full_rebuild(&st, &mut cache, 80);
        // 流式续接（最常见的路径）
        for delta in ["\n\n- 项目一", "\n- 项目二", "\n\n```sh\necho hi\n```"] {
            st.stream_delta(LineKind::Assistant, delta);
            assert_same_as_full_rebuild(&st, &mut cache, 80);
        }
    }

    #[test]
    fn cache_matches_full_rebuild_across_invalidations() {
        let mut st = UiState::new();
        st.push_line(LineKind::User, "hi");
        st.push_line(LineKind::Thinking, "想一想");
        st.push_line(LineKind::Tool, "…");
        let mut cache = BodyCache::default();
        assert_same_as_full_rebuild(&st, &mut cache, 80);

        // 宽度变化
        assert_same_as_full_rebuild(&st, &mut cache, 40);
        // thinking 折叠位变化
        st.toggle_thinking();
        assert_same_as_full_rebuild(&st, &mut cache, 40);
        // 搜索激活 → 高亮 + 锚点
        st.start_search("hi");
        assert_same_as_full_rebuild(&st, &mut cache, 40);
        // 锚点跳转（只影响锚定行）
        st.next_search_hit();
        assert_same_as_full_rebuild(&st, &mut cache, 40);
        // 清空搜索
        st.clear_search();
        assert_same_as_full_rebuild(&st, &mut cache, 40);
    }

    /// 工具结果回填会改**中间行**，且文本可能等长——必须靠 `rev` 而不是长度发现。
    #[test]
    fn middle_line_replacement_is_detected() {
        let mut st = UiState::new();
        st.tool_start("id", "read_file", Some("a.txt"));
        st.push_line(LineKind::User, "tail");
        let mut cache = BodyCache::default();
        assert_same_as_full_rebuild(&st, &mut cache, 80);

        st.tool_end("id", "read_file", Some(true), Some("ok"));
        assert_same_as_full_rebuild(&st, &mut cache, 80);
    }

    /// 流式期只重建被改动的那一行 —— 这是整个模块存在的理由。
    /// 全量重建的旧实现下，100 次增量 × 50 行 = 5000 次行重建；这里应是 100 次。
    #[test]
    fn streaming_rebuilds_only_the_changed_line() {
        let mut st = UiState::new();
        st.push_line(LineKind::User, "seed");
        for i in 0..50 {
            st.push_line(LineKind::Assistant, &format!("历史 {i}"));
        }
        let mut cache = BodyCache::default();
        cache.ensure(&st, 80);
        let baseline = cache.rebuilt;

        for _ in 0..100 {
            st.stream_delta(LineKind::Assistant, "x");
            cache.ensure(&st, 80);
        }
        assert_eq!(
            cache.rebuilt - baseline,
            100,
            "每次流式增量只该重建最后一行"
        );
    }

    /// transcript 清空后缓存不能残留（否则会显示上一个会话的尾部）。
    #[test]
    fn reset_transcript_clears_cache() {
        let mut st = UiState::new();
        st.push_line(LineKind::User, "旧内容");
        let mut cache = BodyCache::default();
        cache.ensure(&st, 80);
        assert!(!cache.as_slice().is_empty());

        st.reset_transcript();
        assert_same_as_full_rebuild(&st, &mut cache, 80);
        assert!(cache.as_slice().is_empty());
    }
}
