//! 工作区目录树状态逻辑：目录缓存扁平化、光标位移与展开/折叠（纯状态层）。
//!
//! 数据仍在 [`super::state::UiState`]，本模块承载树模型与操作。拆文件以控制
//! `state.rs` 单文件行数（fn-nloc 门禁 ≤ 920）与函数复杂度。

use std::collections::{HashMap, HashSet};

use crabmate_tui_core::{WorkspaceDirData, WorkspaceDirEntry};

use super::state::UiState;

/// 工作区目录树展开后的单个可见行（由各目录缓存扁平化而来，供绘制与光标移动共用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WsRow {
    /// 缩进层级（0 = 根下第一层）。
    pub indent: u8,
    /// 相对工作区根的路径（POSIX，根下文件即文件名；目录为子树键）。
    pub rel: String,
    /// 展示名（文件名/目录名）。
    pub name: String,
    pub is_dir: bool,
    /// 目录是否处于展开态（决定 `▾/▸` 标记）。
    pub expanded: bool,
    /// 目录子列表在途的占位行（灰色「…」）。
    pub loading: bool,
}

/// 父相对路径 + 子名 → 子相对路径（根下子项返回 `name`）。
#[must_use]
pub fn ws_child_rel(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{parent}/{name}")
    }
}

/// 相对路径的父目录（根下文件/夹返回 `""`）。
#[must_use]
pub fn ws_parent_rel(rel: &str) -> String {
    rel.rfind('/')
        .map(|i| rel[..i].to_string())
        .unwrap_or_default()
}

/// 展开中但子列表尚未到达的目录占位行路径（不会与真实条目重名）。
fn ws_loading_rel(rel: &str) -> String {
    format!("{rel}/\u{22ef}_loading")
}

/// 把已加载目录缓存 + 展开集扁平化为可见行（深度优先；目录紧跟其后代）。
/// 展开但缓存缺失的目录在下方插入一个灰色「…」占位行（子列表在途）。
fn build_ws_rows(
    cache: &HashMap<String, Vec<WorkspaceDirEntry>>,
    expanded: &HashSet<String>,
) -> Vec<WsRow> {
    fn push_children(
        out: &mut Vec<WsRow>,
        cache: &HashMap<String, Vec<WorkspaceDirEntry>>,
        expanded: &HashSet<String>,
        parent_rel: &str,
        indent: u8,
    ) {
        let Some(entries) = cache.get(parent_rel) else {
            return;
        };
        for entry in entries {
            let rel = ws_child_rel(parent_rel, &entry.name);
            let open = entry.is_dir && expanded.contains(&rel);
            out.push(WsRow {
                indent,
                rel: rel.clone(),
                name: entry.name.clone(),
                is_dir: entry.is_dir,
                expanded: open,
                loading: false,
            });
            if open && cache.contains_key(&rel) {
                push_children(out, cache, expanded, &rel, indent + 1);
            } else if open {
                out.push(WsRow {
                    indent: indent + 1,
                    rel: ws_loading_rel(&rel),
                    name: "\u{22ef}".to_string(),
                    is_dir: false,
                    expanded: false,
                    loading: true,
                });
            }
        }
    }
    let mut rows = Vec::new();
    push_children(&mut rows, cache, expanded, "", 0);
    rows
}

impl UiState {
    // ── 工作区目录树（仿 Desktop 工作区侧栏） ──────────────

    /// 数据/展开态变化后重算可见行（不在此处收敛光标，位移语义交给各调用点；
    /// 需要收敛的调用点在自己算完净位移后 clamp）。
    fn ws_refresh_rows(&mut self) {
        self.ws_rows = build_ws_rows(&self.ws_dir_cache, &self.ws_expanded);
    }

    /// 光标收敛到当前可见行界内。
    fn ws_clamp_cursor(&mut self) {
        let last = self.ws_rows.len().saturating_sub(1);
        if self.ws_cursor > last {
            self.ws_cursor = last;
        }
    }

    /// 记录根目录列表已入队（进入工作区视图前的去重标记）。
    pub fn ws_begin_root_fetch(&mut self) {
        self.ws_root_pending = true;
    }

    /// 根目录列表成功到达：整体替换树并刷新顶栏路径。
    pub fn ws_root_replace(&mut self, data: WorkspaceDirData) {
        let p = data.path.trim();
        self.workspace_path = (!p.is_empty()).then(|| p.to_string());
        self.ws_ready = true;
        self.ws_root_pending = false;
        self.ws_root_err = None;
        self.ws_expanded.clear();
        self.ws_loading.clear();
        self.ws_dir_cache.clear();
        self.ws_dir_cache.insert(String::new(), data.entries);
        self.ws_cursor = 0;
        self.ws_refresh_rows();
    }

    /// 根目录列表失败：保留旧数据，记录占位错误；返回是否需要提示（由事件循环发系统行）。
    pub fn ws_root_failed(&mut self, err: &str) -> bool {
        self.ws_root_pending = false;
        let fresh = !self.ws_ready && self.ws_dir_cache.is_empty();
        if fresh {
            self.ws_root_err = Some(err.to_string());
        }
        fresh
    }

    /// 展开/折叠光标所在目录；需要拉子列表时返回其相对路径（事件循环据此入队）。
    pub fn ws_toggle_dir(&mut self) -> Option<String> {
        let row = self.ws_rows.get(self.ws_cursor).filter(|r| r.is_dir)?;
        let rel = row.rel.clone();
        if self.ws_expanded.contains(&rel) {
            self.ws_expanded.remove(&rel);
            self.ws_refresh_rows();
            return None;
        }
        self.ws_expanded.insert(rel.clone());
        let fetch = !self.ws_dir_cache.contains_key(&rel) && !self.ws_loading.contains(&rel);
        if fetch {
            self.ws_loading.insert(rel.clone());
        }
        self.ws_refresh_rows();
        fetch.then_some(rel)
    }

    /// 向左：折叠展开中的目录；否则跳到父目录行（根行不动作）。
    pub fn ws_left(&mut self) {
        let Some(row) = self.ws_rows.get(self.ws_cursor) else {
            return;
        };
        if row.is_dir && row.expanded {
            self.ws_expanded.remove(&row.rel);
            self.ws_refresh_rows();
            return;
        }
        let parent = ws_parent_rel(&row.rel);
        if parent.is_empty() {
            return;
        }
        if let Some(idx) = self.ws_rows.iter().position(|r| r.rel == parent) {
            self.ws_cursor = idx;
        }
    }

    /// 上/下移动树光标（空树不动作）。
    pub fn ws_move(&mut self, up: bool) {
        if self.ws_rows.is_empty() {
            return;
        }
        if up {
            self.ws_cursor = self.ws_cursor.saturating_sub(1);
        } else if self.ws_cursor + 1 < self.ws_rows.len() {
            self.ws_cursor += 1;
        }
    }

    /// 子目录列表成功到达：写入缓存（折叠中的也缓存，便于再次展开）。
    /// 目录行之下的行会被占位→真实子行的替换顶开/收拢，光标按净位移保持指向原条目。
    pub fn ws_dir_ok(&mut self, rel: &str, data: WorkspaceDirData) {
        self.ws_loading.remove(rel);
        let dir_idx = self.ws_rows.iter().position(|r| r.rel == rel);
        let before_len = self.ws_rows.len();
        self.ws_dir_cache.insert(rel.to_string(), data.entries);
        self.ws_refresh_rows();
        if let Some(di) = dir_idx {
            let shift = self.ws_rows.len() as isize - before_len as isize;
            let cursor = self.ws_cursor as isize;
            if shift != 0 && cursor > di as isize {
                self.ws_cursor = (cursor + shift).max(0) as usize;
            }
        }
        self.ws_clamp_cursor();
    }

    /// 子目录列表失败：目录仍在展开中则收起并让光标回到该目录行；返回是否收起。
    pub fn ws_dir_failed(&mut self, rel: &str) -> bool {
        self.ws_loading.remove(rel);
        let was_expanded = self.ws_expanded.remove(rel);
        if was_expanded {
            let dir_idx = self.ws_rows.iter().position(|r| r.rel == rel);
            let before_len = self.ws_rows.len();
            self.ws_refresh_rows();
            if let Some(di) = dir_idx {
                let shift = self.ws_rows.len() as isize - before_len as isize;
                let cursor = self.ws_cursor as isize;
                if shift != 0 && cursor > di as isize {
                    self.ws_cursor = (cursor + shift).max(0) as usize;
                }
            }
            self.ws_clamp_cursor();
        }
        was_expanded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ent(name: &str, is_dir: bool) -> WorkspaceDirEntry {
        WorkspaceDirEntry {
            name: name.to_string(),
            is_dir,
        }
    }

    fn data(path: &str, entries: Vec<WorkspaceDirEntry>) -> WorkspaceDirData {
        WorkspaceDirData {
            path: path.to_string(),
            entries,
            error: None,
        }
    }

    #[test]
    fn ws_rel_helpers() {
        assert_eq!(ws_child_rel("", "src"), "src");
        assert_eq!(ws_child_rel("src", "main.rs"), "src/main.rs");
        assert_eq!(ws_parent_rel("src/lib/mod.rs"), "src/lib");
        assert_eq!(ws_parent_rel("main.rs"), "");
    }

    #[test]
    fn ws_root_replace_flattens_root_rows() {
        let mut s = UiState::new();
        s.ws_root_replace(data(
            "/data/p",
            vec![
                ent("src", true),
                ent("README.md", false),
                ent("target", true),
            ],
        ));
        assert!(s.ws_ready);
        assert_eq!(s.workspace_path.as_deref(), Some("/data/p"));
        assert_eq!(s.ws_rows.len(), 3);
        assert_eq!(s.ws_rows[0].rel, "src");
        assert_eq!(s.ws_rows[0].indent, 0);
        assert!(s.ws_rows[0].is_dir && !s.ws_rows[0].expanded);
        assert_eq!(s.ws_rows[1].name, "README.md");
        // 空树时光标收敛到 0
        s.ws_cursor = 9;
        s.ws_root_replace(data("/empty", Vec::new()));
        assert!(s.ws_rows.is_empty());
        assert_eq!(s.ws_cursor, 0);
    }

    #[test]
    fn ws_expand_fetches_and_inserts_children() {
        let mut s = UiState::new();
        s.ws_root_replace(data("/p", vec![ent("src", true)]));
        assert_eq!(s.ws_toggle_dir(), Some("src".to_string()), "应请求子列表");
        assert!(s.ws_loading.contains("src"));
        // 列表在途时再 toggle 不会重复请求（先折叠再展开）
        assert_eq!(s.ws_toggle_dir(), None, "折叠不请求");
        assert_eq!(s.ws_toggle_dir(), None, "加载中不重复请求");
        s.ws_dir_ok(
            "src",
            data("/p/src", vec![ent("lib", true), ent("main.rs", false)]),
        );
        assert_eq!(s.ws_rows.len(), 3, "src 展开后其后代可见");
        assert_eq!(s.ws_rows[1].rel, "src/lib");
        assert_eq!(s.ws_rows[1].indent, 1);
        assert_eq!(s.ws_rows[2].name, "main.rs");
        // 折叠 src → 后代行消失，光标留在 src
        s.ws_cursor = 0;
        assert_eq!(s.ws_toggle_dir(), None);
        assert_eq!(s.ws_rows.len(), 1);
        assert!(!s.ws_expanded.contains("src"));
    }

    #[test]
    fn ws_dir_ok_caches_when_collapsed_early() {
        let mut s = UiState::new();
        s.ws_root_replace(data("/p", vec![ent("docs", true)]));
        assert_eq!(s.ws_toggle_dir(), Some("docs".to_string()));
        // 结果到达前用户已折叠：缓存仍写入，再次展开无需请求
        assert_eq!(s.ws_toggle_dir(), None);
        s.ws_dir_ok("docs", data("/p/docs", vec![ent("a.md", false)]));
        assert_eq!(s.ws_rows.len(), 1);
        assert_eq!(s.ws_toggle_dir(), None, "已有缓存不再请求");
        assert_eq!(s.ws_rows.len(), 2);
    }

    #[test]
    fn ws_dir_failed_collapses_and_reports() {
        let mut s = UiState::new();
        s.ws_root_replace(data("/p", vec![ent("x", true)]));
        assert_eq!(s.ws_toggle_dir(), Some("x".to_string()));
        assert!(s.ws_dir_failed("x"), "展开中的目录失败应收起并上报");
        assert!(!s.ws_expanded.contains("x"));
        assert!(!s.ws_loading.contains("x"));
        assert_eq!(s.ws_rows.len(), 1);
        // 已折叠目录收到迟到的失败结果不应提示
        assert_eq!(s.ws_toggle_dir(), Some("x".to_string()));
        assert_eq!(s.ws_toggle_dir(), None, "先折叠");
        assert!(!s.ws_dir_failed("x"));
    }

    #[test]
    fn ws_cursor_moves_and_left_goes_parent() {
        let mut s = UiState::new();
        s.ws_root_replace(data("/p", vec![ent("a", true), ent("b", true)]));
        s.ws_toggle_dir();
        s.ws_dir_ok("a", data("/p/a", vec![ent("a1.txt", false)]));
        // rows: a(0) a1.txt(1) b(2)
        s.ws_cursor = 1;
        s.ws_left();
        assert_eq!(s.ws_cursor, 0, "文件行向左跳到父目录");
        s.ws_toggle_dir(); // 折叠 a
        s.ws_move(false); // 到 b
        assert_eq!(s.ws_rows[s.ws_cursor].rel, "b");
        s.ws_left(); // b 未展开 → 父是根，不动作
        assert_eq!(s.ws_rows[s.ws_cursor].rel, "b");
        s.ws_move(true);
        s.ws_move(true); // 顶部不动
        assert_eq!(s.ws_cursor, 0);
    }

    #[test]
    fn ws_root_failed_keeps_old_tree() {
        let mut s = UiState::new();
        s.ws_root_replace(data("/p", vec![ent("keep", true)]));
        assert!(!s.ws_root_failed("busy"), "已有数据时失败不提示、保留旧树");
        assert!(s.ws_ready);
        assert_eq!(s.ws_rows.len(), 1);
        // 首次加载失败：占位错误 + 后续可重试
        let mut t = UiState::new();
        assert!(t.ws_root_failed("down"));
        assert_eq!(t.ws_root_err.as_deref(), Some("down"));
        t.ws_begin_root_fetch();
        t.ws_root_replace(data("/p", vec![]));
        assert_eq!(t.ws_root_err, None);
    }

    #[test]
    fn ws_expand_shows_loading_placeholder_until_loaded() {
        let mut s = UiState::new();
        s.ws_root_replace(data("/p", vec![ent("a", true)]));
        assert_eq!(s.ws_toggle_dir(), Some("a".to_string()));
        assert_eq!(s.ws_rows.len(), 2, "在途时目录下方应有占位行");
        assert!(s.ws_rows[1].loading);
        assert_eq!(s.ws_rows[1].name, "\u{22ef}");
        // 空目录到达：占位消失、仍保持展开（无子行即空目录）
        s.ws_dir_ok("a", data("/p/a", Vec::new()));
        assert_eq!(s.ws_rows.len(), 1);
        assert!(!s.ws_rows[0].loading);
    }

    #[test]
    fn ws_dir_ok_shifts_cursor_past_inserted_children() {
        let mut s = UiState::new();
        s.ws_root_replace(data("/p", vec![ent("a", true), ent("b", false)]));
        assert_eq!(s.ws_toggle_dir(), Some("a".to_string()));
        // rows: a(0) …(1) b(2) → 光标移到 b
        s.ws_move(false);
        s.ws_move(false);
        assert_eq!(s.ws_rows[s.ws_cursor].rel, "b");
        // 两个子行顶开占位：b 应后移到索引 3，光标保持指向 b
        s.ws_dir_ok("a", data("/p/a", vec![ent("a1", false), ent("a2", false)]));
        assert_eq!(s.ws_rows[s.ws_cursor].rel, "b");
        assert_eq!(s.ws_rows.len(), 4);
    }

    #[test]
    fn ws_dir_failed_keeps_cursor_on_following_row() {
        let mut s = UiState::new();
        s.ws_root_replace(data("/p", vec![ent("a", true), ent("b", false)]));
        assert_eq!(s.ws_toggle_dir(), Some("a".to_string()));
        s.ws_move(false);
        s.ws_move(false); // 光标在占位下的 b
        assert!(s.ws_dir_failed("a"));
        assert_eq!(s.ws_rows.len(), 2);
        assert_eq!(
            s.ws_rows[s.ws_cursor].rel, "b",
            "收起顶开下行时保持指向原行"
        );
    }

    #[test]
    fn ws_dir_failed_anchors_cursor_on_dir_row_when_inside() {
        let mut s = UiState::new();
        s.ws_root_replace(data("/p", vec![ent("a", true)]));
        assert_eq!(s.ws_toggle_dir(), Some("a".to_string()));
        s.ws_move(false); // 光标落在占位行
        assert!(s.ws_dir_failed("a"));
        assert_eq!(s.ws_rows.len(), 1);
        assert_eq!(s.ws_rows[s.ws_cursor].rel, "a", "占位行消失后回到目录行");
    }
}
