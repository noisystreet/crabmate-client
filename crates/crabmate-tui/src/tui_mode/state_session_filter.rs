//! 左栏会话标题过滤（issue #423 的 v1）：过滤词、可见行映射与选中吸附。
//!
//! 从 `state.rs` 搬出：那个文件贴着 920 行门禁，而这里是一组纯映射算术 —— 只有
//! `sessions` / `selected` / `session_filter` 三个字段参与（与 `state_cursor.rs` 同一手法：
//! `state` 的 `#[path]` 子模块，因此能直接读写私有字段）。
//!
//! **选中项仍是全量下标**：`selected` 继续指向 `sessions` 里的行，过滤只派生出
//! [`UiState::visible_indices`]。这样做的好处是 `use_selected` / `delete_*` /
//! `row_in_use` / 会话索引写入那一整条链路一个字都不用改，只有「显示 / 上下移动 /
//! 鼠标命中」三处需要走可见映射。
//!
//! v1 的入口是命令（`/conv filter <词>`），不是左栏内逐字收窄：原因见 issue 的评论
//! （`n` / `r` / `w` 已被占用，字母即过滤会让它们随上下文变义）。

use super::UiState;
// `SessionListRow` 在 `state.rs` 里是私有 `use`，这里从源头取（与会话栏按键模块同一路径）。
use crate::serve::SessionListRow;

/// 清空过滤词的命令参数（与 `/workspace off` 同一套词，省得记两套）。
const CLEAR_WORDS: &[&str] = &["off", "none", "clear"];

impl UiState {
    /// 当前生效的过滤词（已 trim；空串与纯空白一律视为「没有过滤」）。
    #[must_use]
    pub fn filter_term(&self) -> Option<&str> {
        self.session_filter
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
    }

    /// 目标行的可匹配文本：空标题按渲染口径当 `(untitled)`，否则过滤会把它们整批藏掉。
    fn session_haystack(row: &SessionListRow) -> &str {
        let title = row.title.trim();
        if title.is_empty() {
            "(untitled)"
        } else {
            title
        }
    }

    /// 单行是否命中当前过滤词（大小写不敏感子串；无过滤词时恒真）。
    #[must_use]
    pub fn session_matches(&self, row: &SessionListRow) -> bool {
        let Some(term) = self.filter_term() else {
            return true;
        };
        Self::session_haystack(row)
            .to_lowercase()
            .contains(&term.to_lowercase())
    }

    /// 可见行在 `sessions` 里的下标（无过滤词时就是 `0..len`）。
    #[must_use]
    pub fn visible_indices(&self) -> Vec<usize> {
        if self.filter_term().is_none() {
            return (0..self.sessions.len()).collect();
        }
        self.sessions
            .iter()
            .enumerate()
            .filter(|(_, row)| self.session_matches(row))
            .map(|(i, _)| i)
            .collect()
    }

    /// 选中项在**可见列表**里的位置（渲染光标与鼠标命中都用这个口径）。
    ///
    /// 选中项被过滤掉时返回 `None`：调用方据此避免「光标停在看不见的行上」。
    #[must_use]
    pub fn selected_visible_pos(&self) -> Option<usize> {
        let visible = self.visible_indices();
        visible.iter().position(|&i| i == self.selected)
    }

    /// 把选中项吸附到最近的可见行（原来的行被过滤掉时用）。
    ///
    /// 优先**往后**取（用户往下翻的直觉），没有则往前取最后一条；一条都不剩就保持原位
    /// —— 那时光标本来也画不出来，界面上会显示「（无匹配）」。
    pub fn snap_selected_to_visible(&mut self) {
        let visible = self.visible_indices();
        if visible.is_empty() || visible.contains(&self.selected) {
            return;
        }
        let next = visible
            .iter()
            .copied()
            .find(|&i| i >= self.selected)
            .unwrap_or_else(|| visible[visible.len() - 1]);
        self.selected = next;
    }

    /// 上/下移动左栏选中：只在**可见行**间走，到头不越界（空列表 / 无匹配时不动）。
    ///
    /// 从 `state.rs` 搬来：过滤之后「下一条」不再是 `selected + 1`，而可见映射只有
    /// 本模块有 —— 放在两处迟早写出两种口径。
    pub fn move_selection(&mut self, up: bool) {
        let visible = self.visible_indices();
        let Some(pos) = visible.iter().position(|&i| i == self.selected) else {
            // 选中项被过滤掉了（或列表为空）：先落回第一个可见行，下一步再从那里走。
            if let Some(&first) = visible.first() {
                self.selected = first;
            }
            return;
        };
        if up && pos > 0 {
            self.selected = visible[pos - 1];
        } else if !up && let Some(&next) = visible.get(pos + 1) {
            self.selected = next;
        }
    }

    /// 设置 / 清除过滤词：`None` 与空词都表示清除，返回设置后的可见条数。
    ///
    /// 设置后立刻吸附光标 —— 命令刚返回时列表已经变了，光标不能留在被过滤掉的行上。
    pub fn set_session_filter(&mut self, term: Option<String>) -> usize {
        let term = term
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty() && !CLEAR_WORDS.contains(&t.to_lowercase().as_str()));
        self.session_filter = term;
        self.snap_selected_to_visible();
        self.visible_indices().len()
    }

    /// 清除过滤（切到指定会话 / 新建会话时用）。
    pub fn clear_session_filter(&mut self) {
        self.session_filter = None;
    }

    /// 是否有生效的过滤词（渲染标题与提示行据此换文案）。
    #[must_use]
    pub fn session_filter_active(&self) -> bool {
        self.filter_term().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, title: &str) -> SessionListRow {
        SessionListRow {
            id: id.to_string(),
            title: title.to_string(),
            updated_at: 0,
            pinned: false,
            starred: false,
            server_conversation_id: Some(format!("c-{id}")),
            server_revision: None,
            workspace_root: None,
        }
    }

    fn state() -> UiState {
        let mut s = UiState::new();
        s.replace_sessions(vec![
            row("a", "修复登录闪退"),
            row("b", "Add TUI filter"),
            row("c", ""),
            row("d", "重写登录页"),
        ]);
        s
    }

    #[test]
    fn no_filter_keeps_every_row_visible() {
        let s = state();
        assert_eq!(s.visible_indices(), vec![0, 1, 2, 3]);
        assert!(!s.session_filter_active());
        assert_eq!(s.selected_visible_pos(), Some(0));
    }

    #[test]
    fn filter_matches_case_insensitively_and_by_substring() {
        let mut s = state();
        s.set_session_filter(Some("登录".into()));
        assert_eq!(s.visible_indices(), vec![0, 3], "两条标题都含「登录」");

        s.set_session_filter(Some("tui".into()));
        assert_eq!(s.visible_indices(), vec![1], "大小写不敏感");
    }

    #[test]
    fn empty_title_matches_the_untitled_placeholder() {
        let mut s = state();
        s.set_session_filter(Some("untitled".into()));
        assert_eq!(s.visible_indices(), vec![2], "空标题按 (untitled) 参与匹配");
    }

    #[test]
    fn blank_term_clears_the_filter() {
        let mut s = state();
        s.set_session_filter(Some("登录".into()));
        assert_eq!(
            s.set_session_filter(Some("   ".into())),
            4,
            "纯空白等于清除"
        );
        assert!(!s.session_filter_active());

        s.set_session_filter(Some("登录".into()));
        assert_eq!(s.set_session_filter(Some("off".into())), 4, "off 也清除");
        s.set_session_filter(Some("登录".into()));
        assert_eq!(s.set_session_filter(None), 4);
        assert!(!s.session_filter_active());
    }

    #[test]
    fn setting_a_filter_snaps_the_selection_to_a_visible_row() {
        let mut s = state();
        s.selected = 1; // 标题是英文的那条
        s.set_session_filter(Some("登录".into()));
        assert_eq!(s.visible_indices(), vec![0, 3]);
        assert!(s.selected_visible_pos().is_some(), "光标必须落在可见行上");
        assert_eq!(s.selected, 3, "往后取最近的可见行");
    }

    #[test]
    fn snapping_falls_back_to_the_last_row_when_nothing_is_after() {
        let mut s = state();
        s.selected = 3;
        s.set_session_filter(Some("登录".into()));
        assert_eq!(s.selected, 3, "自己就是可见行，原地不动");

        s.selected = 2;
        s.set_session_filter(Some("untitled".into()));
        assert_eq!(s.selected, 2);
    }

    #[test]
    fn no_match_keeps_the_selection_where_it_was() {
        let mut s = state();
        s.selected = 2;
        s.set_session_filter(Some("压根不存在的词".into()));
        assert!(s.visible_indices().is_empty());
        assert_eq!(s.selected, 2, "一条都不剩时不乱改光标");
        assert_eq!(s.selected_visible_pos(), None);
    }

    #[test]
    fn selection_walks_the_visible_rows_only() {
        let mut s = state();
        s.set_session_filter(Some("登录".into()));
        s.selected = 0;
        s.move_selection(false);
        assert_eq!(s.selected, 3, "跳过被过滤掉的两条");
        s.move_selection(false);
        assert_eq!(s.selected, 3, "到底不越界");
        s.move_selection(true);
        assert_eq!(s.selected, 0, "上行同样只走可见行");
    }

    #[test]
    fn refresh_keeps_the_filter_and_resnaps() {
        let mut s = state();
        s.selected = 1;
        s.set_session_filter(Some("登录".into()));
        // 服务端返回的新快照里可见行位置变了。
        s.replace_sessions(vec![row("b", "Add TUI filter"), row("e", "登录重试")]);
        assert!(s.session_filter_active(), "刷新不清过滤词");
        assert_eq!(s.visible_indices(), vec![1]);
        assert_eq!(s.selected, 1, "光标吸附到新的可见行");
    }
}
