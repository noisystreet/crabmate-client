//! `/editor` 视图内的**文件内查找**（`Ctrl+F`，issue #421）。
//!
//! 与聊天区的 `/find` 是两件事，刻意不共用一份状态：
//! - 聊天区 `/find` 搜 transcript（`UiState::search_*`），结果是「跳到第几条记录」；
//! - 这里搜**当前打开的那个文件**，结果是「跳到文件第几行」。
//!
//! 合成一份的话，一次 `/find` 会连文件视图里的高亮一起改掉，而屏幕上看不出是哪一边变了。
//! 复用到的只有**高亮的画法**（[`super::super::md::highlight_override`]：命中行改 `hit`
//! 前景、锚定行整行反色）—— 两处颜色所以永远一致，语义却各管各的。
//!
//! 「一行」跟视图本身一样有两种口径（见 [`super::EditorMode`]）：源码视图按**源码行**判定，
//! md 预览按**折行后的物理行**判定（一行源码可能折成几行，按源码行标位置会指错地方）。
//! 因此命中集合带一份「按哪份内容算的」标识，内容 / 模式 / 行数一变就重算。
//!
//! 单独成文件：`editor_view.rs` 与 `md.rs` 都贴着 `fn-nloc` 的 920 行/文件门禁。

use super::EditorMode;

/// 命中集合是按哪份内容 / 哪种排版算出来的。
///
/// 任一字段变化都意味着行号已经漂移（重新读文件、切模式、终端变宽导致折行不同），
/// 沿用旧命中会把高亮画到不相干的行上。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FindKey {
    /// 内容版本（[`super::EditorState::text_rev`]）。
    pub rev: u64,
    /// 渲染模式。
    pub mode: EditorMode,
    /// 行数：源码视图是源码行数，md 预览是折行后的**物理行数**。
    pub rows: usize,
    /// 折行宽度（源码视图恒为 0）。
    ///
    /// 必须进 key：宽度变了折行会整体重排，而**物理行数可能恰好不变**（短文档常见），
    /// 只靠 `rows` 认不出这种变化 —— 那时 `hits` 里的行号指向的是另一段文字。
    pub width: usize,
}

/// 文件内查找的状态（随 [`super::EditorState`] 走 = 每条 lane 一份）。
#[derive(Debug, Default)]
pub(crate) struct FindState {
    /// 查找输入态（`Ctrl+F` 进入，`Esc` 退出）。
    ///
    /// 退出后**保留**查询词与高亮：找完要看命中处的上下文，一按 Esc 就清掉等于让人重找一次。
    pub open: bool,
    /// 查找词（原文，按用户敲的样子显示）。空串 = 没有查找。
    pub query: String,
    /// 命中行号（升序、无重复；口径见模块头注释）。
    pub hits: Vec<usize>,
    /// 锚点在 [`Self::hits`] 里的下标（`None` = 没有命中）。
    pub cursor: Option<usize>,
    /// 命中集合对应的内容标识；`None` = 还没算过，或已被新的查询词作废。
    key: Option<FindKey>,
    /// 视图里此刻有没有正文可搜（见 [`Self::set_body_absent`]）。
    body: bool,
}

impl FindState {
    /// `Ctrl+F`：进查找输入态。不重置查询词与锚点 —— 再按一次接着找同一个词。
    pub fn begin(&mut self) {
        self.open = true;
    }

    /// `Esc`：退出输入态，保留高亮与「第 n / N 处」。
    pub fn close(&mut self) {
        self.open = false;
    }

    /// 查询词变了：命中集合与锚点都作废（行号口径可能整体变化，不能沿用）。
    fn retype(&mut self, edit: impl FnOnce(&mut String)) {
        edit(&mut self.query);
        self.key = None;
        self.cursor = None;
    }

    /// 追加一个字符（查找输入态里的普通字符）。
    pub fn push(&mut self, ch: char) {
        self.retype(|q| q.push(ch));
    }

    /// 退格：删掉最后一**个**字符（按 `char`，不是字节 —— 中文查找词很常见）。
    pub fn pop(&mut self) {
        self.retype(|q| {
            q.pop();
        });
    }

    /// 让命中集合跟上当前内容；标识没变时直接返回（每帧都会走到这里）。
    ///
    /// 行数取自 `key.rows`（与判定口径同一个来源），因此不会出现「按 50 行算、只在 40 行里
    /// 找」这种自相矛盾的组合。`hit(i, needle)` 判定第 `i` 行是否命中，`needle` 已转小写 ——
    /// 比较两侧同一口径，与聊天区 `/find` 一致（大小写不敏感的 grep，而不是精确匹配）。
    pub fn sync(&mut self, key: FindKey, mut hit: impl FnMut(usize, &str) -> bool) {
        // 走到这里说明内容区有正文：渲染前用 `set_body_absent` 已经把「没有正文」挡掉了。
        self.body = true;
        if self.key == Some(key) {
            return;
        }
        let needle = self.query.to_lowercase();
        self.hits = if needle.is_empty() {
            Vec::new()
        } else {
            (0..key.rows).filter(|&i| hit(i, &needle)).collect()
        };
        // 新的查询词从第一处开始（`cursor` 为 `None` 时显示「第 1 / N 处」而不是「第 / N 处」）；
        // 内容变短把锚点甩出去时收敛到最后一处，别留下越界下标。
        self.cursor = match self.hits.len() {
            0 => None,
            len => Some(self.cursor.unwrap_or(0).min(len - 1)),
        };
        self.key = Some(key);
    }

    /// 跳到下一处 / 上一处（循环）；返回新的锚点行号。
    pub fn cycle(&mut self, forward: bool) -> Option<usize> {
        let len = self.hits.len();
        if len == 0 {
            self.cursor = None;
            return None;
        }
        let cur = self.cursor.unwrap_or(usize::from(!forward));
        let next = if forward {
            (cur + 1) % len
        } else {
            (cur + len - 1) % len
        };
        self.cursor = Some(next);
        self.hits.get(next).copied()
    }

    /// 锚点所在行（没有命中就是 `None`）。
    pub fn anchor_row(&self) -> Option<usize> {
        self.cursor.and_then(|c| self.hits.get(c)).copied()
    }

    /// 第 `row` 行是否命中（渲染层逐行问，`hits` 有序所以走二分）。
    pub fn is_hit(&self, row: usize) -> bool {
        self.hits.binary_search(&row).is_ok()
    }

    /// 第 `row` 行是否是**锚定**行（画法与其余命中行不同）。
    pub fn is_anchor(&self, row: usize) -> bool {
        self.anchor_row() == Some(row)
    }

    /// `(第几处, 共几处)`，1-based；没有命中时为 `None`。
    pub fn position(&self) -> Option<(usize, usize)> {
        Some((self.cursor? + 1, self.hits.len()))
    }

    /// 有没有值得显示的东西（有正文、且有查询词或正处在输入态）。
    ///
    /// 「有正文」是条件的一部分：读取中 / 读取失败 / 未选文件 / 空文件时，`第 1 / 3 处`
    /// 与 `无匹配` 都是错话（前者是上一个文件的行号，后者在文件还没到时就下了结论）。
    fn showing(&self) -> bool {
        self.body && (self.open || !self.query.is_empty())
    }

    /// 内容区此刻有没有正文；没有时命中集合与读数一律作废。
    ///
    /// 每帧渲染前调一次（`set_body_absent(false)` 是无操作 —— 有正文时命中集合由
    /// [`Self::sync`] 维护）。**保留** `query` 与输入态：文件读回来还要接着找同一个词，
    /// 而不是让人重打一遍。
    pub fn set_body_absent(&mut self, absent: bool) {
        if !absent {
            return;
        }
        self.body = false;
        self.hits.clear();
        self.cursor = None;
        // key 一起清：否则同一份内容回来后 `sync` 会以为「算过了」而不重扫。
        self.key = None;
    }

    /// 输入态的整行提示（`None` = 不在输入态）。
    ///
    /// 整行让给查找：此刻每个键都归它，底下还写着「怎么回聊天」只会让人以为 Esc 是退视图
    /// （那是输入框为空时的事，查找态里 Esc 只关查找）。
    pub fn prompt(&self) -> Option<String> {
        if !self.open {
            return None;
        }
        Some(format!(
            "查找：{}▏ {} · Enter 下一处 · Shift+Enter 上一处 · Esc 关闭 · 只搜本文件（/find 搜聊天记录）",
            self.query,
            self.count_text()
        ))
    }

    /// 非输入态、但仍有查询词时，加到提示行**最前面**的那一段。
    ///
    /// 位置在前：命中读数是查找唯一的反馈（视图占着主区，写进 transcript 也看不见），
    /// 而窄终端只从尾部裁。
    pub fn inline_note(&self) -> Option<String> {
        (self.showing() && !self.open)
            .then(|| format!("查找 {} {}（Ctrl+F） · ", self.query, self.count_text()))
    }

    /// 「第 n / N 处」/「无匹配」/「输入关键词」。
    fn count_text(&self) -> String {
        if self.query.is_empty() {
            return "输入关键词".to_string();
        }
        match self.position() {
            Some((n, total)) => format!("第 {n} / {total} 处"),
            None => "无匹配".to_string(),
        }
    }
}

/// 一行文本是否命中 `needle`（`needle` 已小写）。
///
/// **不分配**：文件视图上限 20 000 行，而查找输入态每敲一个字都要重扫一遍，逐行
/// `to_lowercase()` 会在按键路径上分配两万个 `String`。绝大多数源码行是纯 ASCII，
/// 逐字节的 `eq_ignore_ascii_case` 窗口比对即可；没有大写字符的行（例如整行中文）小写化
/// 不会改变它，直接 `contains`；只有混了大小写的非 ASCII 行才退回小写化。
///
/// md 预览也走它（调用方先把物理行的可见字符拼进一个复用 buffer）：预览会吃掉围栏、
/// 插语言标签、折叠空行，判定必须落在**画出来的字符**上，而不是源码行。
pub(super) fn line_matches(line: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    if line.is_ascii() {
        let (hay, pat) = (line.as_bytes(), needle.as_bytes());
        return hay.windows(pat.len()).any(|w| w.eq_ignore_ascii_case(pat));
    }
    if !line.chars().any(char::is_uppercase) {
        return line.contains(needle);
    }
    line.to_lowercase().contains(needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synced(query: &str, rows: &[&str]) -> FindState {
        let mut f = FindState {
            query: query.to_string(),
            ..FindState::default()
        };
        let key = FindKey {
            rev: 0,
            mode: EditorMode::Source,
            rows: rows.len(),
            width: 0,
        };
        f.sync(key, |i, n| line_matches(rows[i], n));
        f
    }

    #[test]
    fn hits_are_sorted_lowercase_and_anchored_at_the_first() {
        let f = synced("abc", &["x", "ABC def", "abc", "no"]);
        assert_eq!(f.hits, vec![1, 2], "大小写不敏感，且命中集合升序");
        assert_eq!(f.position(), Some((1, 2)), "新查询从第一处开始");
        assert!(f.is_anchor(1) && !f.is_anchor(2));
    }

    /// 折行宽度变了而物理行数恰好不变时，命中集合必须重算。
    ///
    /// md 预览的「第几行」是折行折出来的：宽度一变每行的内容就整体重排，而总行数完全
    /// 可能一格不差。只按行数判断就会沿用旧行号，高亮画到别的文字上。
    #[test]
    fn a_width_change_with_the_same_row_count_rescans() {
        let mut f = FindState {
            query: "hit".to_string(),
            ..FindState::default()
        };
        let key = |width| FindKey {
            rev: 7,
            mode: EditorMode::Markdown,
            rows: 3,
            width,
        };
        f.sync(key(40), |i, _| i == 2);
        assert_eq!(f.hits, vec![2]);

        let mut calls = 0;
        f.sync(key(41), |_, _| {
            calls += 1;
            false
        });
        assert_eq!(calls, 3, "宽度变了、行数没变，也必须重扫");
        assert!(f.hits.is_empty(), "重扫之后旧命中不能留着");
    }

    #[test]
    fn an_empty_query_clears_the_hits_instead_of_matching_everything() {
        let f = synced("", &["a", "b"]);
        assert!(f.hits.is_empty());
        assert_eq!(f.position(), None);
        assert!(f.count_text().contains("输入关键词"));
    }

    #[test]
    fn no_match_says_so() {
        let f = synced("zzz", &["a", "b"]);
        assert_eq!(f.count_text(), "无匹配");
        assert_eq!(f.position(), None);
    }

    #[test]
    fn cycling_wraps_and_runs_backwards() {
        let mut f = synced("hit", &["hit", "no", "hit", "hit"]);
        assert_eq!(f.position(), Some((1, 3)));
        assert_eq!(f.cycle(true), Some(2));
        assert_eq!(f.cycle(true), Some(3));
        assert_eq!(f.cycle(true), Some(0), "到尾循环回第一处");
        assert_eq!(f.cycle(false), Some(3), "上一处反向循环");
    }

    /// 内容变短把锚点甩出去时收敛，不留越界下标（否则 `第 4 / 2 处`）。
    #[test]
    fn a_shrunk_content_clamps_the_anchor() {
        let mut f = synced("hit", &["hit", "hit", "hit"]);
        f.cycle(true);
        f.cycle(true);
        assert_eq!(f.position(), Some((3, 3)));
        f.sync(
            FindKey {
                rev: 1,
                mode: EditorMode::Source,
                rows: 1,
                width: 0,
            },
            |_, _| true,
        );
        assert_eq!(f.position(), Some((1, 1)), "只剩一行时锚点收敛到它");
    }

    /// 标识没变就不重算（每帧都会调 `sync`，重算等于每帧扫一遍全文）。
    #[test]
    fn an_unchanged_key_skips_the_rescan() {
        let mut f = synced("hit", &["hit"]);
        let mut calls = 0;
        f.sync(
            FindKey {
                rev: 0,
                mode: EditorMode::Source,
                rows: 1,
                width: 0,
            },
            |_, _| {
                calls += 1;
                true
            },
        );
        assert_eq!(calls, 0, "同一份内容不该再扫一遍");
    }

    /// 输入态与「只留高亮」两种提示不同：前者整行让给查找，后者只追加一小段。
    #[test]
    fn the_prompt_and_the_inline_note_are_mutually_exclusive() {
        let mut f = synced("hit", &["hit"]);
        assert!(f.prompt().is_none(), "不在输入态就没有整行提示");
        assert!(
            f.inline_note().unwrap_or_default().contains("第 1 / 1 处"),
            "{:?}",
            f.inline_note()
        );

        f.begin();
        assert!(f.prompt().unwrap_or_default().contains("查找：hit"));
        assert!(f.inline_note().is_none());
    }

    /// 退格按 `char` 删，不按字节：中文查找词删一半会变成乱码。
    #[test]
    fn backspace_drops_one_character() {
        let mut f = FindState::default();
        f.push('笔');
        f.push('记');
        f.pop();
        assert_eq!(f.query, "笔");
    }

    /// 命中判定不分配：纯 ASCII 行逐字节窗口比对，没有大写的行（整行中文）直接 `contains`，
    /// 只有混了大写的非 ASCII 行才退回小写化 —— 这条路径每敲一个字就要走一遍全文件。
    #[test]
    fn line_matches_avoids_lowercasing_whole_lines() {
        assert!(line_matches("fn main() { HIT }", "hit"));
        assert!(line_matches("目标 Target", "target"));
        assert!(line_matches("整行中文，没有大写", "中文"));
        assert!(!line_matches("整行中文", "没这句"));
        // 空查找词不命中：`windows(0)` 会 panic，这条守的是那个早返回。
        assert!(!line_matches("任意", ""));
        // 查找词比行还长。
        assert!(!line_matches("hi", "needle"));
    }
}
