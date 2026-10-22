//! 搜索（`/find`）的命中集合与跳转 —— 从 `state.rs` 拆出并改为**增量维护**。
//!
//! 拆出来有两个理由：`state.rs` 贴着 920 行门禁；更重要的是命中集合改成增量维护后
//! 逻辑比原来「每次全量重扫」复杂一档，混在百行杂项里不好读。
//!
//! 原来每次内容变化都 `to_lowercase().contains()` 扫**全部行**，而 `stream_delta`
//! 每个增量都会走到那里 —— 一轮长回答就是 O(行数 × 字符数) 的重复扫描，表现是流式
//! 输出越久、输入越卡。现在只在**被改动的那一行**重新判定。
//!
//! 不变式：`search_hits` 升序、无重复，且 `search_total == search_hits.len()`。
//! 改文本的路径必须通知（`search_on_line_added` / `search_on_line_changed`）；
//! 只改 `tool_state` 成败标记、thinking 折叠开关这类不动文本的路径不必通知。

use super::UiState;

impl UiState {
    fn needle(&self) -> Option<&str> {
        self.search.as_deref()
    }

    fn log_matches(&self, idx: usize, needle: &str) -> bool {
        self.lines
            .get(idx)
            .is_some_and(|l| l.text.to_lowercase().contains(needle))
    }

    /// 命中数变化后同步计数；归零时释放锚点（否则状态行会显示 `1/0`）。
    fn sync_search_total(&mut self) {
        self.search_total = self.search_hits.len();
        if self.search_total == 0 {
            self.search_cursor = None;
        }
    }

    /// 全量重扫：改搜索词时用一次，O(行数 × 行长)。
    pub(super) fn rescan_search(&mut self) {
        let mut hits: Vec<usize> = Vec::new();
        if let Some(needle) = self.needle() {
            for i in 0..self.lines.len() {
                if self.log_matches(i, needle) {
                    hits.push(i);
                }
            }
        }
        self.search_hits = hits;
        self.sync_search_total();
    }

    /// 末尾追加了一行（`push_line` / 回放 / 工具行开始）。
    ///
    /// 幂等：`stream_delta` 走 `push_line` 分支后会再通知一次，已记过就返回。
    pub(super) fn search_on_line_added(&mut self) {
        let Some(idx) = self.lines.len().checked_sub(1) else {
            return;
        };
        if self.search_hits.last() == Some(&idx) {
            return;
        }
        let hit = match self.needle() {
            Some(n) => self.log_matches(idx, n),
            None => return,
        };
        if hit {
            self.search_hits.push(idx);
        }
        self.sync_search_total();
    }

    /// 第 `idx` 行文本被替换（工具结果回填、流式续接）：只重判这一行。
    pub(super) fn search_on_line_changed(&mut self, idx: usize) {
        let hit = match self.needle() {
            Some(n) => self.log_matches(idx, n),
            None => return,
        };
        let pos = self.search_hits.binary_search(&idx);
        match (pos.is_ok(), hit) {
            (false, true) => {
                let at = pos.unwrap_or_else(|e| e);
                self.search_hits.insert(at, idx);
            }
            (true, false) => {
                self.search_hits.remove(pos.unwrap_or(0));
            }
            _ => {}
        }
        self.sync_search_total();
    }

    /// 最后一行被续接（流式主路径）。
    pub(super) fn search_on_last_line_changed(&mut self) {
        let Some(idx) = self.lines.len().checked_sub(1) else {
            return;
        };
        self.search_on_line_changed(idx);
    }

    /// 设置新搜索词并从首条命中跳转；返回命中数。
    pub fn start_search(&mut self, term: &str) -> usize {
        self.search = Some(term.trim().to_lowercase());
        self.search_cursor = None;
        self.search_total = 0;
        self.rescan_search();
        if let Some(&first) = self.search_hits.first() {
            self.search_cursor = Some(first);
        }
        self.search_hits.len()
    }

    /// 跳到下一个命中（循环）；无命中返回 `None`。
    pub fn next_search_hit(&mut self) -> Option<usize> {
        let hits = std::mem::take(&mut self.search_hits);
        let after = self.search_cursor.unwrap_or(0);
        let next = hits
            .iter()
            .copied()
            .find(|&i| i > after)
            .or_else(|| hits.first().copied());
        self.search_hits = hits;
        if let Some(n) = next {
            self.search_cursor = Some(n);
        }
        next
    }

    /// 手动滚动后释放锚点（保留搜索词与高亮）。
    pub fn release_search_anchor(&mut self) {
        self.search_cursor = None;
    }

    /// 清除搜索（去掉高亮与锚点）。
    pub fn clear_search(&mut self) {
        self.search = None;
        self.search_cursor = None;
        self.search_total = 0;
        self.search_hits.clear();
    }

    #[must_use]
    pub fn search_active(&self) -> bool {
        self.search.is_some()
    }

    #[must_use]
    pub fn search_term(&self) -> Option<&str> {
        self.search.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui_mode::state::LineKind;

    /// 流式续接：命中状态从无到有只加一条，且计数跟着走（不重扫全表）。
    #[test]
    fn streaming_append_keeps_hits_incremental() {
        let mut s = UiState::new();
        s.push_line(LineKind::User, "先来一句无关的");
        s.start_search("target");
        assert_eq!(s.search_total, 0);

        s.stream_delta(LineKind::Assistant, "half of tar");
        assert_eq!(s.search_total, 0, "还没拼出完整词");

        s.stream_delta(LineKind::Assistant, "get");
        assert_eq!(s.search_total, 1, "拼全后这一行应计入命中");
        assert_eq!(s.search_cursor, None, "流式新命中不抢锚点（沿用旧语义）");
        assert_eq!(s.next_search_hit(), Some(1), "要用户主动跳转才设锚点");

        // 同一条消息继续增长不应重复计数
        s.stream_delta(LineKind::Assistant, " and more target");
        assert_eq!(s.search_total, 1, "同一行不能重复计数");
    }

    /// 工具结果回填会**替换中间行**文本，命中集合要跟着改（不是只看末尾）。
    #[test]
    fn replaced_middle_line_updates_hit_set() {
        let mut s = UiState::new();
        s.tool_start("id-1", "read_file", Some("a.txt"));
        s.push_line(LineKind::User, "tail");
        s.start_search("read_file");
        assert_eq!(s.search_total, 1, "工具行初始就命中");

        // 结果回填：文本换成结果摘要。这里不假设摘要的具体文案（工具名可能被
        // 改写成 `read file` 或省略），只断言**命中集合与正文一致**这个不变式。
        s.tool_end("id-1", "read_file", Some(true), Some("hello"));
        let hit = s
            .lines
            .first()
            .is_some_and(|l| l.text.to_lowercase().contains("read_file"));
        assert_eq!(s.search_total, usize::from(hit), "命中集合必须与正文一致");
        assert_eq!(s.search_hits.len(), usize::from(hit));
    }

    /// 多行命中时命中集合升序，跳转按顺序循环。
    #[test]
    fn hits_stay_sorted_and_cycle() {
        let mut s = UiState::new();
        s.push_line(LineKind::User, "hit a");
        s.push_line(LineKind::User, "no");
        s.push_line(LineKind::User, "hit b");
        assert_eq!(s.start_search("hit"), 2);
        assert_eq!(s.search_cursor, Some(0));
        assert_eq!(s.next_search_hit(), Some(2));
        assert_eq!(s.next_search_hit(), Some(0), "循环回第一个");
        assert_eq!(s.search_hits, vec![0, 2], "集合保持升序");
    }

    /// 清空 transcript 后命中集合不能残留（否则锚点会指到不存在的行）。
    #[test]
    fn reset_transcript_drops_hits() {
        let mut s = UiState::new();
        s.push_line(LineKind::User, "hit");
        s.start_search("hit");
        assert_eq!(s.search_total, 1);
        s.reset_transcript();
        assert!(s.search_hits.is_empty());
        assert_eq!(s.search_total, 0);
    }

    /// 未激活搜索时任何通知都不该建表（流式主路径的常见情形）。
    #[test]
    fn no_search_means_no_scanning() {
        let mut s = UiState::new();
        for _ in 0..100 {
            s.stream_delta(LineKind::Assistant, "x");
        }
        assert!(s.search_hits.is_empty());
        assert_eq!(s.search_total, 0);
    }
}
