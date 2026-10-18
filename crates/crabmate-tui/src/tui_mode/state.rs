//! 全屏 TUI 的 UI 状态：transcript 行 + 输入缓冲 + 回合/会话指示 +
//! 审批浮层 / 工具行 / 搜索 / thinking 折叠等交互状态。
//!
//! 工作区目录树的状态逻辑在 [`super::workspace_tree`]（拆分以控制单文件行数）。

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::Sender;

use crate::serve::{ApprovalDecision, CommandApprovalData, SessionListRow, WorkspaceDirEntry};

use super::palette::Palette;
use super::serve_defaults::ServeDefaults;
use super::tool_summary::tool_end_text;
use super::workspace_tree::{WsPickState, WsRow};

/// transcript 行类别（决定前缀与配色）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    User,
    Assistant,
    Thinking,
    Tool,
    System,
}

/// 键盘焦点：底栏输入框、左栏会话列表或左栏工作区目录树。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Focus {
    #[default]
    Input,
    /// 左栏会话列表。
    Sidebar,
    /// 左栏工作区目录树（`GET /workspace[?path=]`，仿 Desktop 工作区侧栏）。
    Workspace,
}

/// 审批浮层：serve 等待决策期间 SSE 回合任务阻塞在应答通道上，
/// UI 事件循环保持运行收按键（双向握手，Esc=n 拒绝 / Enter=y 一次 / a 始终）。
#[derive(Debug)]
pub struct ApprovalOverlay {
    pub command: String,
    pub args: String,
    pub allowlist_key: Option<String>,
    /// 决策回传通道（worker 的 gate 阻塞在对应 recv 上）。
    pub answer: Sender<ApprovalDecision>,
}

impl ApprovalOverlay {
    /// 构造决策前的展示文本（与 repl 提示一致）。
    #[must_use]
    pub fn preview(&self) -> String {
        let cmd = self.command.trim();
        let args = self.args.trim();
        if args.is_empty() {
            cmd.to_string()
        } else {
            format!("{cmd} {args}")
        }
    }
}

/// 一条 transcript 记录。
#[derive(Debug, Clone)]
pub struct LogLine {
    pub kind: LineKind,
    pub text: String,
    /// thinking 折叠态（默认折叠；Ctrl+E 切换全局展开）。
    pub collapsed: bool,
    /// 工具行结果（`Some(true)` 成功 / `Some(false)` 失败 / `None` 进行中）；
    /// 仅供渲染分色，不参与文本（文本仍是 `tool_end_text` 的完整串）。
    pub tool_ok: Option<bool>,
    /// 回放的历史行（来自 `GET /conversation/messages`，不属于本次运行的流）。
    ///
    /// 单独标记而不是复用 `tool_ok: None`：`None` 在实时路径里表示「工具正在进行中」，
    /// 渲染层据此显示 `⠿` 进度占位；历史行若共用同一取值会被误画成「还在跑」。
    pub replayed: bool,
}

impl LogLine {
    /// `collapsed` 由调用方按全局 thinking 展开态传入（thinking 默认折叠，
    /// 用户 Ctrl+E 展开后新建的 thinking 行也应保持展开）。
    fn new(kind: LineKind, text: &str, collapsed: bool) -> Self {
        Self {
            kind,
            text: text.to_string(),
            collapsed,
            tool_ok: None,
            replayed: false,
        }
    }

    fn append(&mut self, delta: &str) {
        self.text.push_str(delta);
    }
}

/// 在途的历史回放请求（`GET /conversation/messages`）。
///
/// 携带请求前的绑定与标题：回包要用它们判断「这次回包还算不算数」（用户可能已经
/// 切走或已经开聊），并在失败时把绑定回滚——否则会拿一个可能已被服务端删掉的 id 继续聊。
#[derive(Debug, Clone)]
pub struct HistoryRequest {
    pub conversation_id: String,
    /// 请求前的绑定（`None` = 新会话）。
    pub prev_conversation_id: Option<String>,
    /// 会话标题（提示文案用；没有就是 `(by id)`）。
    pub title: String,
}

/// 全屏 UI 状态；仅由事件循环触碰（单线程）。
#[derive(Debug, Default)]
pub struct UiState {
    pub lines: Vec<LogLine>,
    /// 终端配色板（随 `UserPrefsDto.theme`；启动时解析一次，不随帧变）。
    pub palette: Palette,
    /// 输入缓冲（Vec<char> 便于光标插入；含 `\n` 即多行编辑）。
    input: Vec<char>,
    /// 输入光标位置（字符下标）。
    cursor: usize,
    pub conversation_id: Option<String>,
    /// 在途的历史回放请求（`None` = 没有正在等的回放）。
    ///
    /// 用户一发送消息或新建会话就清空：那时的回包已经没有落点，清屏会丢真实内容。
    pub history_pending: Option<HistoryRequest>,
    /// 是否有回合在跑（SSE 任务未结束）。
    pub running: bool,
    /// 当前回合是否已发过取消（第二次 Ctrl+C = 强退）。
    pub cancel_sent: bool,
    /// 从底部回看的物理行数（0 = 跟随最新）。
    pub view_offset: usize,
    /// PgUp/PgDn 页步（由事件循环按终端高度刷新）。
    pub page_rows: usize,
    /// 键盘焦点（输入框 / 左栏会话）。
    pub focus: Focus,
    /// 左栏会话列表（`fetch_web_sessions` 快照，未经本地转换）。
    pub sessions: Vec<SessionListRow>,
    /// 左栏选中项下标。
    pub selected: usize,
    /// serve 默认偏好（`/status` 拉取）。
    pub serve_defaults: Option<ServeDefaults>,
    /// serve 侧当前活跃会话（左栏 `*` 标记）。
    pub active_session_id: Option<String>,
    /// 当前是否宽到显示左栏（由渲染层每帧刷新）。
    pub sidebar_visible: bool,
    /// 当前工作区路径（`GET /workspace`；顶栏显示，未获取为 None）。
    pub workspace_path: Option<String>,
    /// 工作区目录树：根列表是否已成功加载（false 时渲染占位）。
    pub ws_ready: bool,
    /// 是否有根目录拉取在途（避免重复入队）。
    pub ws_root_pending: bool,
    /// 根目录最近一次拉取失败文案（侧栏占位提示；成功后清空）。
    pub ws_root_err: Option<String>,
    /// 目录相对路径 → 已加载子条目缓存（`""` = 根）。
    pub ws_dir_cache: HashMap<String, Vec<WorkspaceDirEntry>>,
    /// 已展开的目录相对路径。
    pub ws_expanded: HashSet<String>,
    /// 子目录列表在途的路径（防重复请求）。
    pub ws_loading: HashSet<String>,
    /// 扁平化后的可见树行（数据/展开态变化后重算）。
    pub ws_rows: Vec<WsRow>,
    /// 树光标（`ws_rows` 下标，超界时渲染前收敛）。
    pub ws_cursor: usize,
    /// 刷新时目标行在展开目录内、需等子列表重载后才能定位的 rel（best-effort）。
    pub ws_pending_cursor: Option<String>,
    /// 项目池「选择/切换工作区」子视图（`Some` 时工作区列改列候选项目）。
    pub ws_pick: Option<WsPickState>,
    /// 审批浮层（回合暂停等待决策）。
    pub approval: Option<ApprovalOverlay>,
    /// 工具调用 id → transcript 行号（结果到达时原位更新摘要行）。
    tool_pending: HashMap<String, usize>,
    /// 全局 thinking 展开开关（默认折叠）。
    thinking_visible: bool,
    /// 搜索词（/find；lowercase，trim）。
    search: Option<String>,
    /// 当前搜索锚定的逻辑行下标（None = 无锚点，随手动滚动释放）。
    pub search_cursor: Option<usize>,
    /// 当前搜索命中的逻辑行数（状态行提示用）。
    pub search_total: usize,
    /// transcript 内容代数（任何影响渲染的行内容/折叠/清空变化都会递增）。
    /// 供 body 行帧间 memo 作失效键（O(1)，对原位同长编辑也免疫）。
    pub content_rev: u64,
}

impl UiState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            page_rows: 12,
            // 主题在拿到 user-data 前先按终端能力落默认深色。
            palette: Palette::resolve(None),
            ..Self::default()
        }
    }

    pub fn push_line(&mut self, kind: LineKind, text: &str) {
        // thinking 默认折叠；但"全局展开"开启时新建的 thinking 行也应保持展开。
        let collapsed = kind == LineKind::Thinking && !self.thinking_visible;
        self.lines.push(LogLine::new(kind, text, collapsed));
        self.refresh_search_meta();
        self.bump();
    }

    /// 回放一行历史（`GET /conversation/messages`）。
    ///
    /// `tool_ok` 由调用方给：服务端的工具结果消息没有成败信号，故传 `None`，
    /// 由 `replayed` 让渲染层不画「进行中」占位。
    pub fn push_replayed_line(&mut self, kind: LineKind, text: &str, tool_ok: Option<bool>) {
        let collapsed = kind == LineKind::Thinking && !self.thinking_visible;
        self.lines.push(LogLine {
            kind,
            text: text.to_string(),
            collapsed,
            tool_ok,
            replayed: true,
        });
        self.refresh_search_meta();
        self.bump();
    }

    /// 流式增量：若最后一条同类（Assistant/Thinking）则续接，否则开新行。
    pub fn stream_delta(&mut self, kind: LineKind, delta: &str) {
        if delta.is_empty() {
            return;
        }
        let mergeable = matches!(kind, LineKind::Assistant | LineKind::Thinking);
        if mergeable && self.lines.last().is_some_and(|l| l.kind == kind) {
            self.lines.last_mut().expect("checked last").append(delta);
        } else {
            self.push_line(kind, delta);
        }
        self.refresh_search_meta();
        self.bump();
    }

    /// 内容/折叠变化代数 +1（body memo 失效用）。
    fn bump(&mut self) {
        self.content_rev = self.content_rev.wrapping_add(1);
    }

    /// 应用配色板（启动拉到 user-data 后一次；变化时让 body memo 失效重算样式）。
    pub fn set_palette(&mut self, palette: Palette) {
        if self.palette != palette {
            self.palette = palette;
            self.bump();
        }
    }

    // ── 输入缓冲（单行/多行统一） ─────────────────────────────

    pub fn current_input(&self) -> String {
        self.input.iter().collect()
    }

    /// 输入缓冲是否多行（含 `\n`）。
    #[must_use]
    pub fn is_multiline(&self) -> bool {
        self.input.contains(&'\n')
    }

    /// 取走输入并清空（发送后调用）。
    pub fn take_input(&mut self) -> String {
        let s: String = self.input.drain(..).collect();
        self.cursor = 0;
        s
    }

    /// 输入框当前全文（不动光标、不清空）：退出前把草稿写回会话行用。
    #[must_use]
    pub fn input_snapshot(&self) -> String {
        self.input.iter().collect()
    }

    pub fn insert_char(&mut self, c: char) {
        if self.cursor >= self.input.len() {
            self.input.push(c);
            self.cursor = self.input.len();
        } else {
            self.input.insert(self.cursor, c);
            self.cursor += 1;
        }
    }

    /// 光标处插入换行（Alt+Enter 多行编辑）。
    pub fn insert_newline(&mut self) {
        self.insert_char('\n');
    }

    /// 工作区树里回车选中文件：把相对路径插入输入框。
    ///
    /// 两侧都只在会粘住相邻文字时补空格；光标在末尾时右侧固定补一个空格，便于接着写问题。
    pub fn insert_workspace_path(&mut self, rel: &str) {
        if rel.is_empty() {
            return;
        }
        let glued_left = self.cursor > 0 && !self.input[self.cursor - 1].is_whitespace();
        if glued_left {
            self.insert_char(' ');
        }
        for c in rel.chars() {
            self.insert_char(c);
        }
        let at_end = self.cursor == self.input.len();
        let glued_right = !at_end && !self.input[self.cursor].is_whitespace();
        if at_end || glued_right {
            self.insert_char(' ');
        }
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let idx = self.cursor - 1;
        self.input.remove(idx);
        self.cursor = idx;
    }

    pub fn cursor_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn cursor_right(&mut self) {
        if self.cursor < self.input.len() {
            self.cursor += 1;
        }
    }

    /// Home：当前行首（单行输入等价整串开头）。
    pub fn cursor_home(&mut self) {
        if let Some(line_start) = self.cursor_line_start() {
            self.cursor = line_start;
        } else {
            self.cursor = 0;
        }
    }

    /// End：当前行尾（不含行尾 `\n`）。
    pub fn cursor_end(&mut self) {
        if let Some(line_end) = self.cursor_line_end() {
            self.cursor = line_end;
        } else {
            self.cursor = self.input.len();
        }
    }

    /// 光标行起始字符下标。
    fn cursor_line_start(&self) -> Option<usize> {
        self.input[..self.cursor.min(self.input.len())]
            .iter()
            .rposition(|&c| c == '\n')
            .map_or(Some(0), |nl| Some(nl + 1))
    }

    /// 光标行结束字符下标（不含行尾 `\n`）。
    fn cursor_line_end(&self) -> Option<usize> {
        let start = self.cursor_line_start()?;
        let rest = &self.input[start.min(self.input.len())..];
        rest.iter()
            .position(|&c| c == '\n')
            .map_or_else(|| Some(self.input.len()), |nl| Some(start + nl))
    }

    /// 光标所在行号与行内字符列。
    #[must_use]
    pub fn cursor_line_col(&self) -> (usize, usize) {
        let mut line = 0;
        let mut col = 0;
        for (i, &c) in self.input.iter().enumerate() {
            if i >= self.cursor {
                break;
            }
            if c == '\n' {
                line += 1;
                col = 0;
            } else {
                col += 1;
            }
        }
        (line, col)
    }

    /// 每行行首字符下标（含空串行）。
    fn line_starts(&self) -> Vec<usize> {
        let mut starts = vec![0usize];
        for (i, &c) in self.input.iter().enumerate() {
            if c == '\n' {
                starts.push(i + 1);
            }
        }
        starts
    }

    /// 按 `\n` 拆分输入为各行（不含换行符）。
    #[must_use]
    pub fn input_lines(&self) -> Vec<String> {
        self.input
            .split(|&c| c == '\n')
            .map(|s| s.iter().collect())
            .collect()
    }

    /// 输入总行数。
    #[must_use]
    pub fn input_line_count(&self) -> usize {
        self.input.iter().filter(|&&c| c == '\n').count() + 1
    }

    /// 上/下行移动光标（列尽量保持，超出 clamp 到行尾）。
    pub fn move_cursor_vert(&mut self, up: bool) {
        let (line, col) = self.cursor_line_col();
        let count = self.input_line_count();
        if (up && line == 0) || (!up && line + 1 >= count) {
            return;
        }
        let starts = self.line_starts();
        let target = if up { line - 1 } else { line + 1 };
        let start = starts[target];
        let end = starts.get(target + 1).copied().unwrap_or(self.input.len());
        let line_cols = self.input[start..end.min(self.input.len())]
            .iter()
            .take_while(|&&c| c != '\n')
            .count();
        self.cursor = start + col.min(line_cols);
    }

    /// 替换会话列表并 clamp 选中项。
    pub fn replace_sessions(&mut self, sessions: Vec<SessionListRow>) {
        self.sessions = sessions;
        self.selected = self.selected.min(self.sessions.len().saturating_sub(1));
    }

    /// 上/下移动左栏选中（空列表时不变）。
    pub fn move_selection(&mut self, up: bool) {
        if self.sessions.is_empty() {
            return;
        }
        if up {
            self.selected = self.selected.saturating_sub(1);
        } else if self.selected + 1 < self.sessions.len() {
            self.selected += 1;
        }
    }

    /// 当前选中会话的可续聊 server conversation_id（无则 `None`）。
    #[must_use]
    pub fn selected_resume(&self) -> Option<String> {
        self.sessions.get(self.selected).and_then(|s| {
            s.server_conversation_id
                .as_deref()
                .map(str::trim)
                .filter(|r| !r.is_empty())
                .map(str::to_string)
        })
    }

    /// 会话行是否正是当前 `conversation_id`（左栏标记 `>`）。
    #[must_use]
    pub fn row_in_use(&self, row: &SessionListRow) -> bool {
        self.conversation_id
            .as_deref()
            .is_some_and(|c| row.server_conversation_id.as_deref() == Some(c))
    }

    /// 会话切换 / 新建：清空 transcript 回最新视图（v1 无历史回放）。
    pub fn reset_transcript(&mut self) {
        self.lines.clear();
        self.tool_pending.clear();
        self.view_offset = 0;
        self.search = None;
        self.search_cursor = None;
        self.search_total = 0;
        self.bump();
    }

    // ── 审批浮层 ─────────────────────────────────────────────

    /// 收到命令审批请求：记录浮层（供渲染 + 按键应答）。
    pub fn begin_approval(&mut self, req: &CommandApprovalData, answer: Sender<ApprovalDecision>) {
        self.approval = Some(ApprovalOverlay {
            command: req.command.clone(),
            args: req.args.clone(),
            allowlist_key: req.allowlist_key.clone(),
            answer,
        });
    }

    // ── 工具行摘要 ───────────────────────────────────────────

    /// 工具开始：记录一个摘要行，RESULT 到达时原位补结果标记。
    pub fn tool_start(&mut self, tool_call_id: &str, name: &str) {
        let display = if name.trim().is_empty() { "tool" } else { name };
        if tool_call_id.is_empty() {
            self.push_line(LineKind::Tool, display);
            return;
        }
        self.lines
            .push(LogLine::new(LineKind::Tool, display, false));
        self.tool_pending
            .insert(tool_call_id.to_string(), self.lines.len() - 1);
        self.refresh_search_meta();
        self.bump();
    }

    /// 工具结果：更新对应摘要行（note 与工具名重复时省略前缀，如 `✓ read file: hello.cpp`）；
    /// 找不到 start（如续流中段）则补一行收尾。
    pub fn tool_end(
        &mut self,
        tool_call_id: &str,
        name: &str,
        ok: Option<bool>,
        note: Option<&str>,
    ) {
        let display = if name.trim().is_empty() { "tool" } else { name };
        let text = tool_end_text(display, ok, note);
        let idx = self.tool_pending.remove(tool_call_id);
        match idx {
            Some(i) if self.lines.get(i).is_some_and(|l| l.kind == LineKind::Tool) => {
                self.lines[i].text = text;
                self.lines[i].tool_ok = ok;
            }
            _ => {
                self.push_line(LineKind::Tool, &text);
                if let Some(last) = self.lines.last_mut() {
                    last.tool_ok = ok;
                }
            }
        }
        self.refresh_search_meta();
        self.bump();
    }

    /// 回合开始时清理上轮工具行映射。
    pub fn reset_run_tools(&mut self) {
        self.tool_pending.clear();
    }

    // ── thinking 折叠 ────────────────────────────────────────

    /// 全局展开 / 折叠所有 thinking 行（桌面语义默认折叠）。
    pub fn set_thinking_visible(&mut self, visible: bool) {
        self.thinking_visible = visible;
        for line in &mut self.lines {
            if line.kind == LineKind::Thinking {
                line.collapsed = !visible;
            }
        }
        self.bump();
    }

    pub fn thinking_visible(&self) -> bool {
        self.thinking_visible
    }

    /// Ctrl+E：切换 thinking 展开/折叠。
    pub fn toggle_thinking(&mut self) {
        self.set_thinking_visible(!self.thinking_visible);
    }

    // ── 搜索（/find） ────────────────────────────────────────

    fn needle(&self) -> Option<&str> {
        self.search.as_deref()
    }

    fn log_matches(&self, idx: usize, needle: &str) -> bool {
        self.lines
            .get(idx)
            .is_some_and(|l| l.text.to_lowercase().contains(needle))
    }

    /// 命中搜索的逻辑行下标（升序）。
    fn matches(&self) -> Vec<usize> {
        let Some(needle) = self.needle() else {
            return Vec::new();
        };
        (0..self.lines.len())
            .filter(|&i| self.log_matches(i, needle))
            .collect()
    }

    /// 内容变化后同步命中计数（仅搜索激活时扫描，开销与 transcript 长度线性）。
    fn refresh_search_meta(&mut self) {
        if self.search.is_none() {
            self.search_total = 0;
            return;
        }
        self.search_total = self.matches().len();
        if self.search_total == 0 {
            self.search_cursor = None;
        }
    }

    /// 设置新搜索词并从首条命中跳转；返回命中数。
    pub fn start_search(&mut self, term: &str) -> usize {
        self.search = Some(term.trim().to_lowercase());
        self.search_cursor = None;
        self.search_total = 0;
        let hits = self.matches();
        self.search_total = hits.len();
        if let Some(&first) = hits.first() {
            self.search_cursor = Some(first);
        }
        hits.len()
    }

    /// 跳到下一个命中（循环）；无命中返回 `None`。
    pub fn next_search_hit(&mut self) -> Option<usize> {
        let hits = self.matches();
        self.search_total = hits.len();
        if hits.is_empty() {
            return None;
        }
        let after = self.search_cursor.unwrap_or(0);
        let next = hits
            .iter()
            .copied()
            .find(|&i| i > after)
            .or_else(|| hits.first().copied());
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
    }

    #[must_use]
    pub fn search_active(&self) -> bool {
        self.search.is_some()
    }

    #[must_use]
    pub fn search_term(&self) -> Option<&str> {
        self.search.as_deref()
    }

    pub fn scroll_page(&mut self, up: bool) {
        let step = self.page_rows.max(1);
        if up {
            self.view_offset = self.view_offset.saturating_add(step);
        } else {
            self.view_offset = self.view_offset.saturating_sub(step);
        }
        self.release_search_anchor();
    }

    pub fn scroll_top(&mut self) {
        self.view_offset = usize::MAX / 4;
        self.release_search_anchor();
    }

    pub fn scroll_bottom(&mut self) {
        self.view_offset = 0;
        self.release_search_anchor();
    }

    pub fn scroll_lines(&mut self, up: bool, by: usize) {
        if up {
            self.view_offset = self.view_offset.saturating_add(by);
        } else {
            self.view_offset = self.view_offset.saturating_sub(by);
        }
        self.release_search_anchor();
    }
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
