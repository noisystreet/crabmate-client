//! 历史分页（`/older`）与回放行落地的状态逻辑。
//!
//! 从 `state.rs` 拆出的两个理由：那里贴着 920 行门禁；更重要的是「前插」要**同时**
//! 处理三处行号状态（工具行映射与搜索锚点整体平移、命中集合整表重扫），这组不变式
//! 必须写在同一处 —— 漏掉任何一处，后续事件就会落到错误的行上（工具结果把历史消息
//! 改写成摘要、`/find` 跳到隔壁行）。

use super::{LineKind, LogLine, UiState};

/// 已回放历史的分页游标（`None` = 当前 transcript 不能往前翻）。
///
/// `window_start` 直接当下次请求的 `before_index`；`has_older` 决定顶部提示行与
/// `/older` 是否还该发请求；`loading` 既挡重复触发，也是「这次回包是不是我这次
/// 翻页」的判据（在途时不可能再发第二次）。
#[derive(Debug, Clone, Copy)]
pub struct HistoryPage {
    pub window_start: u32,
    pub has_older: bool,
    pub loading: bool,
}

impl UiState {
    /// 回放一行历史（`GET /conversation/messages`）。
    ///
    /// 历史行一律是**已结束**的：回放只转写落盘的结果，不存在「还在跑」。
    /// `tool_ok` 是信封里的成败；服务端没给就是 `None`（标记显示 `?`，不猜）。
    pub fn push_replayed_line(&mut self, kind: LineKind, text: &str, tool_ok: Option<bool>) {
        let collapsed = kind == LineKind::Thinking && !self.thinking_visible();
        self.lines
            .push(LogLine::replayed(kind, text, collapsed, tool_ok));
        self.search_on_line_added();
    }

    /// 会话切换 / 新建：清空 transcript 回最新视图。
    ///
    /// 分页游标一起清：新 transcript 的历史页边界与旧会话无关，留着会让 `/older`
    /// 拿着旧会话的 `window_start` 去请求别的会话。
    pub fn reset_transcript(&mut self) {
        self.lines.clear();
        self.tool_pending.clear();
        self.view_offset = 0;
        self.body_at_top = false;
        self.history_page = None;
        self.search = None;
        self.search_cursor = None;
        self.search_total = 0;
        self.search_hits.clear();
    }

    /// 把一页更早的历史前插到 transcript 头部。
    ///
    /// `rows` 是「旧 → 新」顺序（与服务端分页同向），前插后仍排在已加载内容之前。
    ///
    /// 视口：`view_offset` 是「从底部回看的物理行数」，前插只动头部、不动底部，所以
    /// 默认**不改**它 —— 用户看到的仍是同一段内容（视线不动）。唯一例外是「已经贴顶」：
    /// 那时贴顶等同于「正看着最早那条」，必须继续贴顶，否则刚加载的一页落在视口上方
    /// 看不见、还要再按一次 Ctrl+Home（`body_at_top` 由 `render_body` 每帧回写）。
    pub fn prepend_replayed(&mut self, rows: Vec<(LineKind, String, Option<bool>)>) {
        let n = rows.len();
        if n == 0 {
            return;
        }
        let collapsed_thinking = !self.thinking_visible();
        let mut head: Vec<LogLine> = rows
            .into_iter()
            .map(|(kind, text, tool_ok)| {
                let collapsed = kind == LineKind::Thinking && collapsed_thinking;
                LogLine::replayed(kind, &text, collapsed, tool_ok)
            })
            .collect();
        head.append(&mut self.lines);
        self.lines = head;
        // ① 工具行映射存的是**行号**。当前门控保证翻页时它为空（回合内拒绝翻页，回合
        //    结束 `reset_run_tools` 清空），所以这段是防御性平移；保留它是因为「索引跟
        //    着行一起偏移」是这份状态的基本不变式，缺了它哪天放开「回合内翻页」，工具
        //    结果就会原地改写掉历史行。
        for idx in self.tool_pending.values_mut() {
            *idx += n;
        }
        // ② 搜索锚点是逻辑行号，先跟着平移。
        self.search_cursor = self.search_cursor.map(|cursor| cursor + n);
        // ③ 命中集合**不靠平移**：只平移会漏掉刚前插进来的行（更早页里的命中），于是
        //    `/find` 的计数偏小、跳转也跳不到它们。翻页是低频操作，整表重扫（O(行数)）
        //    比维护增量便宜得多，也让「跨页命中」真的可搜。
        if self.search.is_some() {
            self.rescan_search();
        }
        // ④ 贴顶例外见方法注释：在顶时把 offset 重新打到哨兵，渲染 clamp 后即贴新内容顶。
        if self.body_at_top {
            self.view_offset = super::SCROLL_TOP;
        }
    }

    /// 回放（最新页 / 前插页）成功后记录游标。
    ///
    /// `window_start == 0` 表示这一页已经含最旧一条：即使服务端同时给了 `has_older`，
    /// 也按「没有更早」处理，否则 `/older` 会反复请求同一页。
    pub fn set_history_page(&mut self, window_start: u32, has_older: bool) {
        self.history_page = Some(HistoryPage {
            window_start,
            has_older: has_older && window_start > 0,
            loading: false,
        });
    }

    /// 翻页请求发出 / 结束。失败路径也必须调 `false`，否则 `/older` 再也发不出去。
    pub fn set_older_loading(&mut self, loading: bool) {
        if let Some(page) = self.history_page.as_mut() {
            page.loading = loading;
        }
    }

    /// 当前会话是否还能往前翻（顶部提示行与 `/older` 共用）。
    #[must_use]
    pub fn has_older_pages(&self) -> bool {
        self.history_page.is_some_and(|page| page.has_older)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 前插：更早的行排在最前，并且工具行映射跟着整体平移。
    #[test]
    fn prepend_keeps_order_and_shifts_the_tool_row_index() {
        let mut st = UiState::new();
        st.push_line(LineKind::User, "新的问题");
        st.push_line(LineKind::Assistant, "新的回答");
        st.tool_pending.insert("call_1".to_string(), 1);

        st.prepend_replayed(vec![
            (LineKind::User, "老问题".to_string(), None),
            (LineKind::Assistant, "老回答".to_string(), None),
        ]);

        assert_eq!(st.lines.len(), 4);
        assert!(st.lines[0].text().contains("老问题"), "更早的行在最前");
        assert!(st.lines[3].text().contains("新的回答"), "原有内容仍在末尾");
        assert_eq!(st.tool_pending.get("call_1"), Some(&3), "工具行映射要平移");
    }

    /// 前插后必须重扫搜索词：只平移命中集合会漏掉更早页里的命中，`/find` 于是计数偏小、
    /// 跳转也跳不到它们（`/help` 说的「跨页命中仍可 /find」就靠这一步成立）。
    #[test]
    fn prepend_rescans_the_search_so_older_hits_are_found() {
        let mut st = UiState::new();
        st.push_line(LineKind::User, "新的这里有 关键词");
        st.search = Some("关键词".to_string());
        st.rescan_search();
        assert_eq!(st.search_hits, vec![0], "前提：当前页只有 1 处命中");
        st.search_cursor = Some(0);

        st.prepend_replayed(vec![(LineKind::User, "老的也有 关键词".to_string(), None)]);

        assert_eq!(st.search_hits, vec![0, 1], "前插的命中也要被找到");
        assert_eq!(st.search_total, 2);
        assert_eq!(st.search_cursor, Some(1), "锚点跟着原来那一行平移");
    }

    /// 贴顶时前插必须**继续贴顶**：否则刚加载的一页落在视口上方，用户还得再按一次
    /// Ctrl+Home 才看得到 —— 这正好抵消了「往上翻」的意图。
    #[test]
    fn prepend_at_top_keeps_the_view_pinned_to_the_top() {
        let mut st = UiState::new();
        st.push_line(LineKind::User, "最新页");
        st.scroll_top();
        st.body_at_top = true; // `render_body` 的回写：视口已贴到内容顶

        st.prepend_replayed(vec![(LineKind::User, "更早页".to_string(), None)]);

        assert_eq!(
            st.view_offset,
            super::super::SCROLL_TOP,
            "在顶时前插要把 offset 重新打到哨兵，渲染 clamp 后才贴到新内容顶"
        );
    }

    /// 不在顶时前插**不动** offset：`view_offset` 是「距底部行数」，头部插入不改变
    /// 底部对齐，用户看到的就是同一段内容（这是 back 语义白拿的锚定）。
    #[test]
    fn prepend_while_scrolled_leaves_the_view_alone() {
        let mut st = UiState::new();
        st.push_line(LineKind::User, "最新页");
        st.view_offset = 12;
        st.body_at_top = false;

        st.prepend_replayed(vec![(LineKind::User, "更早页".to_string(), None)]);

        assert_eq!(st.view_offset, 12, "视线不动");
    }

    /// 空页是 no-op：服务端返回 0 行时不该动 transcript。
    #[test]
    fn prepend_of_empty_page_is_a_no_op() {
        let mut st = UiState::new();
        st.push_line(LineKind::User, "hi");
        st.prepend_replayed(Vec::new());
        assert_eq!(st.lines.len(), 1);
    }

    /// `window_start == 0` 即「已含最旧一条」：服务端仍说 `has_older` 也不认。
    #[test]
    fn window_start_zero_means_no_older_even_if_server_says_so() {
        let mut st = UiState::new();
        st.set_history_page(0, true);
        assert!(!st.has_older_pages());
    }

    /// 回放成功要复位在途标记，否则 `/older` 永远卡在「正在加载」。
    #[test]
    fn set_history_page_clears_loading() {
        let mut st = UiState::new();
        st.set_history_page(40, true);
        st.set_older_loading(true);
        assert!(st.history_page.expect("page").loading);
        st.set_history_page(20, true);
        assert!(!st.history_page.expect("page").loading);
        assert!(st.has_older_pages());
    }

    /// 切换会话必须把游标一起清掉：否则 `/older` 会拿旧会话的游标去请求新会话。
    #[test]
    fn reset_transcript_drops_the_page_cursor() {
        let mut st = UiState::new();
        st.set_history_page(40, true);
        st.reset_transcript();
        assert!(st.history_page.is_none());
        assert!(!st.has_older_pages());
    }
}
