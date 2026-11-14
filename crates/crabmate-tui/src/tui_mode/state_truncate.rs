//! transcript 的「截掉最后一轮」原语（issue #422）。
//!
//! `/conv retry` / `/conv edit` 的**本地半步**：把 transcript 退回到最后一轮用户消息之前，
//! 好让这一轮重发出来的内容落在同一位置。服务端那半步是 `POST /chat/branch`
//! （见 `serve::branch`）—— 两边必须成对使用，只截本地不改服务端会重复落一条用户消息。
//!
//! **为什么不是「回合段数据结构」**：本 issue 只需要「最近一个回合」，从尾部回扫第一条
//! `LineKind::User` 就够。引入回合段会让 render / 滚动 / `/find` / 折叠同时改口径，而那四处
//! 的键全是**行下标**；截断只会删尾部，所以保留行的下标不变，需要重算的只有「尾部指向」
//! 的那几份状态（`reset_transcript` 已经在那几份上走过一遍，这里复用同一组清理）。
//!
//! 单独成文件是 `state.rs` 贴着 920 行门禁（与 `state_cursor.rs` / `state_history.rs` 同一手法）。

use super::{LineKind, UiState};

impl UiState {
    /// 最后一条用户消息的（行下标，原文）。找不到（空 transcript / 只有系统提示）则 `None`。
    // issue #422：命令接线在下一个提交（/conv retry）；此前只在单测里用到。
    #[allow(dead_code)]
    #[must_use]
    pub fn last_user_turn(&self) -> Option<(usize, String)> {
        let idx = self
            .lines
            .iter()
            .rposition(|l| l.kind() == LineKind::User)?;
        Some((idx, self.lines[idx].text().to_string()))
    }

    /// 截掉最后一轮，返回被截掉的那条用户消息原文（调用方拿它重发）。
    ///
    /// 幂等性说明：连续调用会**一轮一轮往回退**（第二次调用截掉的是上上轮），
    /// 这正是「重试也没成，再退一轮重来」的语义，因此不做「只能截一次」的限制。
    // issue #422：命令接线在下一个提交（/conv retry）；此前只在单测里用到。
    #[allow(dead_code)]
    pub fn truncate_to_last_user_turn(&mut self) -> Option<String> {
        let (idx, text) = self.last_user_turn()?;
        self.lines.truncate(idx);
        self.after_truncate(idx);
        Some(text)
    }

    /// 行数变少之后要重算的那几份状态。
    ///
    /// 与 [`Self::reset_transcript`] 同一组字段，区别是这里**保留**前缀 —— 所以不能整表清空
    /// 的两处（`tool_pending` 的「工具 id → 行号」、搜索词本身）按「留下的行」收窄：
    ///
    /// - `tool_pending` 的值是行号：只丢指向被删行的那些，否则后续 `TOOL_CALL_RESULT`
    ///   会去改写一个已经不存在的位置（表现是「工具结果消失」）；
    /// - 详情侧表（`tool_details` / `detail_chars` / 浮层）整表作废：它是进程内只读旁路，
    ///   逐条减配额只多一个记账偏移的机会；
    /// - 折叠光标 / 行命中表 / 链接热区都指向具体行，一并作废（下一帧渲染会重建）；
    /// - 搜索词保留、命中整表重扫：`/find` 的锚点不能指到已经不存在的行上；
    /// - `history_page` **不动**：截断只动尾部，头部历史页边界仍然有效，`/older` 还能用。
    fn after_truncate(&mut self, idx: usize) {
        self.tool_pending.retain(|_, line| *line < idx);
        self.tool_details.clear();
        self.detail_chars = 0;
        self.detail = None;
        self.view_offset = 0;
        self.body_at_top = false;
        self.fold_cursor = None;
        self.body_area = None;
        self.body_rows_log.clear();
        self.body_links.clear();
        if self.search.is_some() {
            self.rescan_search();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_with_turn() -> UiState {
        let mut s = UiState::new();
        s.push_line(LineKind::User, "第一问");
        s.push_line(LineKind::Assistant, "第一答");
        s.push_line(LineKind::User, "第二问");
        s.push_line(LineKind::Tool, "✓ read_file a.rs");
        s.push_line(LineKind::Assistant, "第二答");
        s
    }

    /// 截断点就是最后一条用户行：它自己与它之后的行全部丢掉，返回它的原文。
    #[test]
    fn truncates_at_the_last_user_line() {
        let mut s = state_with_turn();
        assert_eq!(s.truncate_to_last_user_turn().as_deref(), Some("第二问"));
        assert_eq!(s.lines.len(), 2, "只剩第一轮");
        assert_eq!(s.lines[1].text(), "第一答");
    }

    /// 没有用户行（新建会话 / 只有系统提示）时什么都不动。
    #[test]
    fn no_user_line_is_a_no_op() {
        let mut s = UiState::new();
        s.push_line(LineKind::System, "已连接");
        assert_eq!(s.truncate_to_last_user_turn(), None);
        assert_eq!(s.lines.len(), 1, "不许把系统提示也截掉");
    }

    /// 再按一次往回退一轮：这也是「上一轮重试也没成，再退一轮」的语义。
    #[test]
    fn a_second_call_walks_back_another_turn() {
        let mut s = state_with_turn();
        s.truncate_to_last_user_turn();
        assert_eq!(s.truncate_to_last_user_turn().as_deref(), Some("第一问"));
        assert!(s.lines.is_empty());
        assert_eq!(s.truncate_to_last_user_turn(), None, "退到底就停");
    }

    /// 搜索命中是行下标：截断后不能留下指向已删行的锚点。
    #[test]
    fn search_hits_never_point_past_the_end() {
        let mut s = state_with_turn();
        s.push_line(LineKind::Assistant, "第二答里有 read_file 这个词");
        // 命中落在将被截掉的尾部。
        assert!(s.start_search("read_file") > 0);
        s.truncate_to_last_user_turn();
        assert_eq!(s.lines.len(), 2);
        assert!(
            s.search_hits.iter().all(|&i| i < s.lines.len()),
            "命中集合必须整体落在现存行内：{:?}",
            s.search_hits
        );
        assert_eq!(s.search_total, s.search_hits.len());
    }

    /// 折叠光标指向具体行：截断后一律作废，否则 `↑/↓` 从一行不存在的地方开始走。
    #[test]
    fn fold_cursor_is_dropped() {
        let mut s = state_with_turn();
        s.fold_cursor = Some(3);
        s.truncate_to_last_user_turn();
        assert_eq!(s.fold_cursor, None);
    }

    /// 头部历史页边界不受影响：`/older` 仍然可用（截断只动尾部）。
    #[test]
    fn history_page_cursor_survives() {
        let mut s = state_with_turn();
        s.history_page = Some(super::super::HistoryPage {
            window_start: 40,
            has_older: true,
            loading: false,
        });
        s.truncate_to_last_user_turn();
        assert_eq!(
            s.history_page.map(|p| p.window_start),
            Some(40),
            "截尾不动头部游标"
        );
    }
}
