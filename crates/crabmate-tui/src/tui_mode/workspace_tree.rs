//! 工作区目录树状态逻辑：目录缓存扁平化、光标位移与展开/折叠（纯状态层）。
//!
//! 数据仍在 [`super::state::UiState`]，本模块承载树模型与操作。拆文件以控制
//! `state.rs` 单文件行数（fn-nloc 门禁 ≤ 920）与函数复杂度。

use std::collections::{HashMap, HashSet};

use crate::serve::{WorkspaceDirData, WorkspaceDirEntry, WorkspaceProjectsData};

use super::state::UiState;

/// 项目池「选择/切换工作区」子视图状态（未设置=首次选择，已设置=切到另一项目；
/// 关闭后为 `None`）。
#[derive(Debug, Clone, Default)]
pub struct WsPickState {
    /// `GET /workspace/projects` 的 `enabled`（false 时提示池未启用）。
    pub enabled: bool,
    /// 池内项目名字列表。
    pub projects: Vec<String>,
    /// 列表拉取中。
    pub loading: bool,
    /// 切换请求在途（避免重复提交）。
    pub busy: bool,
    /// 拉取/切换错误文案（就地提示）。
    pub error: Option<String>,
}

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
    /// 非真实条目（「…」加载占位 /「（空）」占位）：不可选中、不参与计数。
    pub placeholder: bool,
}

/// Enter 在树行上的动作：目录切换展开态，文件交给事件循环插入输入框。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WsActivate {
    /// 目录：已切换展开态；`fetch` 为 true 时需要拉取子列表。
    Dir { rel: String, fetch: bool },
    /// 文件：相对工作区根的 POSIX 路径。
    File(String),
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

/// 展开后无子项时插入的占位行文案（否则「…」一消失就像没响应）。
const EMPTY_DIR_TEXT: &str = "（空）";

/// 空目录占位行路径（不会与真实条目重名）。
fn ws_empty_rel(rel: &str) -> String {
    format!("{rel}/\u{22ef}_empty")
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
                placeholder: false,
            });
            if !open {
                continue;
            }
            match cache.get(&rel) {
                Some(children) => {
                    push_children(out, cache, expanded, &rel, indent + 1);
                    if children.is_empty() {
                        out.push(WsRow {
                            indent: indent + 1,
                            rel: ws_empty_rel(&rel),
                            name: EMPTY_DIR_TEXT.to_string(),
                            is_dir: false,
                            expanded: false,
                            loading: false,
                            placeholder: true,
                        });
                    }
                }
                None => out.push(WsRow {
                    indent: indent + 1,
                    rel: ws_loading_rel(&rel),
                    name: "\u{22ef}".to_string(),
                    is_dir: false,
                    expanded: false,
                    loading: true,
                    placeholder: true,
                }),
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

    /// 光标收敛：先夹到界内，再避开设不可选中的占位行（「…」/「（空）」）。
    fn ws_clamp_cursor(&mut self) {
        if self.ws_rows.is_empty() {
            self.ws_cursor = 0;
            return;
        }
        self.ws_cursor = self.ws_cursor.min(self.ws_rows.len() - 1);
        if !self.ws_rows[self.ws_cursor].placeholder {
            return;
        }
        // 优先向后（视觉下方）找真实条目，找不到再向前找。
        if let Some(i) =
            (self.ws_cursor..self.ws_rows.len()).find(|&i| !self.ws_rows[i].placeholder)
        {
            self.ws_cursor = i;
        } else if let Some(i) = (0..self.ws_cursor)
            .rev()
            .find(|&i| !self.ws_rows[i].placeholder)
        {
            self.ws_cursor = i;
        }
    }

    /// 记录根目录列表已入队（进入工作区视图前的去重标记）。
    pub fn ws_begin_root_fetch(&mut self) {
        self.ws_root_pending = true;
    }

    /// 手动刷新前占位：已有根请求在途时返回 false（避免连按 `r` 排队多个根请求）。
    pub fn ws_begin_refresh(&mut self) -> bool {
        if self.ws_root_pending {
            return false;
        }
        self.ws_root_pending = true;
        true
    }

    /// 根目录列表成功到达：整体替换树并刷新右栏标题的工作区名。
    pub fn ws_root_replace(&mut self, data: WorkspaceDirData) {
        let p = data.path.trim();
        self.workspace_path = (!p.is_empty()).then(|| p.to_string());
        self.ws_ready = true;
        self.ws_root_pending = false;
        self.ws_root_err = None;
        self.ws_expanded.clear();
        self.ws_loading.clear();
        self.ws_dir_cache.clear();
        // 换了工作区：在途的旧读请求回来后必须被丢弃（见 `ws_generation`）。
        self.ws_generation = self.ws_generation.wrapping_add(1);
        self.ws_dir_stale.clear();
        self.ws_dir_cache.insert(String::new(), data.entries);
        self.ws_cursor = 0;
        self.ws_pending_cursor = None;
        self.ws_refresh_rows();
    }

    /// 手动刷新根目录：保留展开集与光标位置；返回需要重新拉取的已展开目录。
    ///
    /// **子目录缓存不清**：刷新让旧值过期，但不该让树变空。曾经的实现在这里
    /// `ws_dir_cache.clear()`，于是每个已展开目录在重拉返回前都退回 `…` 占位 —— 一旦
    /// 这次刷新排在某个慢请求后面（甚至失败），用户看到的就是整棵树突然空掉。现在旧
    /// 子列表继续参与渲染，调用方按返回列表后台重拉，`ws_dir_ok` 到达时按 `rel` 覆盖。
    ///
    /// 展开态与光标照旧保留。若根路径已变化（外部切走工作区），旧展开态与旧缓存都没有
    /// 意义，退回重置语义。
    pub fn ws_root_replace_keep(&mut self, data: WorkspaceDirData) -> Vec<String> {
        let p = data.path.trim();
        if self
            .workspace_path
            .as_deref()
            .is_some_and(|old| old.trim() != p)
        {
            self.ws_root_replace(data);
            return Vec::new();
        }
        let cursor_rel = self
            .ws_rows
            .get(self.ws_cursor)
            .filter(|r| !r.placeholder)
            .map(|r| r.rel.clone());
        let old_cursor = self.ws_cursor;
        self.workspace_path = (!p.is_empty()).then(|| p.to_string());
        self.ws_ready = true;
        self.ws_root_pending = false;
        self.ws_root_err = None;
        // 根列表换成新的；子目录旧值留着继续渲染，等重拉结果按 rel 覆盖。
        self.ws_dir_cache.insert(String::new(), data.entries);
        // 子列表全部视为过期：已展开的由下面的 refetch 立刻重拉，折叠的下次展开时补拉
        // —— 否则 `r` 刷新永远刷不到当时没展开的那些。
        self.ws_dir_stale = self
            .ws_dir_cache
            .keys()
            .filter(|k| !k.is_empty())
            .cloned()
            .collect();
        let mut refetch: Vec<String> = self.ws_expanded.iter().cloned().collect();
        refetch.sort();
        self.ws_loading = refetch.iter().cloned().collect();
        self.ws_refresh_rows();
        // 光标优先按 rel 还原；该行暂不可见（在展开目录内）时落到最近存活祖先，
        // 同时记下目标 rel，等子列表重载后再归位。
        self.ws_cursor = cursor_rel
            .as_deref()
            .and_then(|rel| self.ws_row_index_near(rel))
            .unwrap_or_else(|| old_cursor.min(self.ws_rows.len().saturating_sub(1)));
        self.ws_clamp_cursor();
        self.ws_pending_cursor = cursor_rel.filter(|rel| {
            self.ws_rows
                .get(self.ws_cursor)
                .is_none_or(|r| r.rel != *rel)
        });
        refetch
    }

    /// `rel` 自身或其最近存活祖先的行下标（刷新后光标落点）。
    fn ws_row_index_near(&self, rel: &str) -> Option<usize> {
        let mut cur = rel.to_string();
        loop {
            if let Some(i) = self.ws_rows.iter().position(|r| r.rel == cur) {
                return Some(i);
            }
            if cur.is_empty() {
                return None;
            }
            cur = ws_parent_rel(&cur);
        }
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

    /// 打开目录：写入展开集、按需标记加载中并刷新可见行；返回是否需要拉取子列表。
    fn ws_open_dir(&mut self, rel: &str) -> bool {
        self.ws_expanded.insert(rel.to_string());
        let cached = self.ws_dir_cache.contains_key(rel);
        // 过期缓存也要重拉一次，但**不设 loading**：旧值继续渲染，不退回 `…`。
        let fetch = (!cached || self.ws_dir_stale.contains(rel)) && !self.ws_loading.contains(rel);
        if fetch && !cached {
            self.ws_loading.insert(rel.to_string());
        }
        self.ws_refresh_rows();
        fetch
    }

    /// 展开/折叠光标所在目录；需要拉子列表时返回其相对路径（事件循环据此入队）。
    pub fn ws_toggle_dir(&mut self) -> Option<String> {
        let row = self.ws_rows.get(self.ws_cursor).filter(|r| r.is_dir)?;
        let rel = row.rel.clone();
        self.ws_pending_cursor = None;
        if self.ws_expanded.contains(&rel) {
            self.ws_expanded.remove(&rel);
            self.ws_refresh_rows();
            return None;
        }
        self.ws_open_dir(&rel).then_some(rel)
    }

    /// 向右：只展开目录（已展开则不动，避免 `→` 反而收起）。
    pub fn ws_expand_dir(&mut self) -> Option<String> {
        let row = self
            .ws_rows
            .get(self.ws_cursor)
            .filter(|r| r.is_dir && !r.placeholder)?;
        let rel = row.rel.clone();
        self.ws_pending_cursor = None;
        if self.ws_expanded.contains(&rel) {
            return None;
        }
        self.ws_open_dir(&rel).then_some(rel)
    }

    /// Enter：目录切换展开态；文件返回相对路径（由事件循环插入输入框）。
    pub fn ws_activate(&mut self) -> Option<WsActivate> {
        let row = self.ws_rows.get(self.ws_cursor)?;
        if row.placeholder {
            return None;
        }
        let rel = row.rel.clone();
        self.ws_pending_cursor = None;
        if !row.is_dir {
            return Some(WsActivate::File(rel));
        }
        let fetch = self.ws_toggle_dir().is_some();
        Some(WsActivate::Dir { rel, fetch })
    }

    /// 向左：折叠展开中的目录；否则跳到父目录行（根行不动作）。
    pub fn ws_left(&mut self) {
        self.ws_pending_cursor = None;
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

    /// 上/下移动树光标（跳过不可选中的占位行；到边不动作）。
    pub fn ws_move(&mut self, up: bool) {
        self.ws_pending_cursor = None;
        if self.ws_rows.is_empty() {
            return;
        }
        let mut i = self.ws_cursor.min(self.ws_rows.len() - 1);
        loop {
            if up {
                if i == 0 {
                    return;
                }
                i -= 1;
            } else {
                if i + 1 >= self.ws_rows.len() {
                    return;
                }
                i += 1;
            }
            if !self.ws_rows[i].placeholder {
                self.ws_cursor = i;
                return;
            }
        }
    }

    /// Home/End：跳到首个/末个可选行。
    pub fn ws_move_to_edge(&mut self, end: bool) {
        self.ws_pending_cursor = None;
        let idx = if end {
            self.ws_rows.iter().rposition(|r| !r.placeholder)
        } else {
            self.ws_rows.iter().position(|r| !r.placeholder)
        };
        if let Some(i) = idx {
            self.ws_cursor = i;
        }
    }

    /// PgUp/PgDn：按页步逐行移动（自动跳过占位行，到边即停）。
    pub fn ws_page(&mut self, down: bool, page: usize) {
        for _ in 0..page.max(1) {
            let before = self.ws_cursor;
            self.ws_move(!down);
            if self.ws_cursor == before {
                break;
            }
        }
    }

    /// 鼠标点击落光标：占位行不可选中（与 ↑↓ 跳过的语义一致）；返回是否命中可选行。
    pub fn ws_set_cursor(&mut self, idx: usize) -> bool {
        match self.ws_rows.get(idx) {
            Some(r) if !r.placeholder => {
                self.ws_pending_cursor = None;
                self.ws_cursor = idx;
                true
            }
            _ => false,
        }
    }

    /// 子目录列表成功到达：写入缓存（折叠中的也缓存，便于再次展开）。
    /// 目录行之下的行会被占位→真实子行的替换顶开/收拢，光标按净位移保持指向原条目。
    pub fn ws_dir_ok(&mut self, rel: &str, data: WorkspaceDirData) {
        self.ws_loading.remove(rel);
        self.ws_dir_stale.remove(rel);
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
        self.ws_restore_pending_cursor();
    }

    /// 刷新时记下的目标行已重新可见 → 光标归位（用户此间手动移动过则已清空，不再抢光标）。
    fn ws_restore_pending_cursor(&mut self) {
        let Some(rel) = self.ws_pending_cursor.clone() else {
            return;
        };
        if let Some(i) = self
            .ws_rows
            .iter()
            .position(|r| r.rel == rel && !r.placeholder)
        {
            self.ws_cursor = i;
            self.ws_pending_cursor = None;
        }
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

    // ── 项目池「选择/切换工作区」（未设置=首次选择，已设置=切到另一项目） ──────────

    /// 进入选择子视图（已在选择中不重复）；返回是否需要拉项目池。
    pub fn ws_open_pick(&mut self) -> bool {
        if self.ws_pick.is_some() {
            return false;
        }
        self.ws_pick = Some(WsPickState {
            loading: true,
            ..WsPickState::default()
        });
        self.ws_cursor = 0;
        true
    }

    /// 关闭选择子视图（Esc / 切换成功后回到目录树）。
    pub fn ws_close_pick(&mut self) {
        self.ws_pick = None;
    }

    /// 手动刷新项目池（选择中且空闲）；返回是否需要拉取。
    pub fn ws_pick_refresh(&mut self) -> bool {
        let Some(st) = self.ws_pick.as_mut() else {
            return false;
        };
        if st.busy || st.loading {
            return false;
        }
        st.loading = true;
        st.error = None;
        true
    }

    /// 项目池列表到达（拉取失败也在就地错误里体现）。
    pub fn ws_pick_projects(&mut self, result: Result<WorkspaceProjectsData, String>) {
        let Some(st) = self.ws_pick.as_mut() else {
            return;
        };
        st.loading = false;
        match result {
            Ok(d) => {
                st.enabled = d.enabled;
                st.projects = d.projects;
                st.error = None;
            }
            Err(e) => {
                st.enabled = false;
                st.projects.clear();
                st.error = Some(e);
            }
        }
    }

    /// 选择视图内上/下移动光标（列表内收敛）。
    pub fn ws_pick_move(&mut self, up: bool) {
        let Some(st) = self.ws_pick.as_ref() else {
            return;
        };
        let n = st.projects.len();
        if n == 0 {
            return;
        }
        if up {
            self.ws_cursor = self.ws_cursor.saturating_sub(1);
        } else if self.ws_cursor + 1 < n {
            self.ws_cursor += 1;
        }
    }

    /// 鼠标点击候选：直接落到该行（越界不动）；返回是否命中。
    pub fn ws_pick_set(&mut self, idx: usize) -> bool {
        let Some(st) = self.ws_pick.as_ref() else {
            return false;
        };
        if idx >= st.projects.len() {
            return false;
        }
        self.ws_cursor = idx;
        true
    }

    /// 光标所在项目发起切换；返回其名字（忙/空列表不动作）。
    pub fn ws_pick_begin_switch(&mut self) -> Option<String> {
        let st = self.ws_pick.as_mut()?;
        if st.busy {
            return None;
        }
        let name = st.projects.get(self.ws_cursor)?.clone();
        st.busy = true;
        Some(name)
    }

    /// 切换结果：成功关闭选择回目录树；失败留在选择视图提示。
    pub fn ws_pick_switch_result(&mut self, result: Result<String, String>) {
        let Some(st) = self.ws_pick.as_mut() else {
            return;
        };
        st.busy = false;
        match result {
            Ok(_) => {
                self.ws_pick = None;
                self.ws_cursor = 0;
            }
            Err(e) => st.error = Some(e),
        }
    }
}

#[cfg(test)]
#[path = "workspace_tree_tests.rs"]
mod tests;
