//! 全屏 TUI 的 UI 状态：transcript 行 + 输入缓冲 + 回合/会话指示 +
//! 审批浮层 / 工具行 / 搜索 / thinking 折叠等交互状态。

use std::collections::HashMap;
use std::sync::mpsc::Sender;

use crabmate_tui_core::{ApprovalDecision, CommandApprovalRequest, SessionListItem};
use serde_json::Value;

/// transcript 行类别（决定前缀与配色）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    User,
    Assistant,
    Thinking,
    Tool,
    System,
}

/// 键盘焦点：底栏输入框或左栏会话列表。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Focus {
    #[default]
    Input,
    Sidebar,
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

/// serve 默认偏好（`GET /status?view=shell`），override 未设置时状态行回退显示。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServeDefaults {
    pub model: Option<String>,
    pub role: Option<String>,
    pub mode: Option<String>,
}

impl ServeDefaults {
    /// 从 `/status?view=shell` 的 JSON 提取默认 model/role/mode。
    #[must_use]
    pub fn from_status(v: &Value) -> Self {
        let field = |k: &str| {
            v.get(k)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        Self {
            model: field("model"),
            role: field("default_agent_role_id"),
            mode: field("default_session_mode"),
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
}

impl LogLine {
    /// `collapsed` 由调用方按全局 thinking 展开态传入（thinking 默认折叠，
    /// 用户 Ctrl+E 展开后新建的 thinking 行也应保持展开）。
    fn new(kind: LineKind, text: &str, collapsed: bool) -> Self {
        Self {
            kind,
            text: text.to_string(),
            collapsed,
        }
    }

    fn append(&mut self, delta: &str) {
        self.text.push_str(delta);
    }
}

/// 全屏 UI 状态；仅由事件循环触碰（单线程）。
#[derive(Debug, Default)]
pub struct UiState {
    pub lines: Vec<LogLine>,
    /// 输入缓冲（Vec<char> 便于光标插入；含 `\n` 即多行编辑）。
    input: Vec<char>,
    /// 输入光标位置（字符下标）。
    cursor: usize,
    pub conversation_id: Option<String>,
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
    pub sessions: Vec<SessionListItem>,
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
}

impl UiState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            page_rows: 12,
            ..Self::default()
        }
    }

    pub fn push_line(&mut self, kind: LineKind, text: &str) {
        // thinking 默认折叠；但"全局展开"开启时新建的 thinking 行也应保持展开。
        let collapsed = kind == LineKind::Thinking && !self.thinking_visible;
        self.lines.push(LogLine::new(kind, text, collapsed));
        self.refresh_search_meta();
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
    pub fn replace_sessions(&mut self, sessions: Vec<SessionListItem>) {
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
    pub fn row_in_use(&self, row: &SessionListItem) -> bool {
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
    }

    // ── 审批浮层 ─────────────────────────────────────────────

    /// 收到命令审批请求：记录浮层（供渲染 + 按键应答）。
    pub fn begin_approval(
        &mut self,
        req: &CommandApprovalRequest,
        answer: Sender<ApprovalDecision>,
    ) {
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
    }

    /// 工具结果：更新对应摘要行；找不到 start（如续流中段）则补一行收尾。
    pub fn tool_end(
        &mut self,
        tool_call_id: &str,
        name: &str,
        ok: Option<bool>,
        note: Option<&str>,
    ) {
        let idx = self.tool_pending.remove(tool_call_id);
        let mut text = match idx {
            Some(i) if self.lines.get(i).is_some_and(|l| l.kind == LineKind::Tool) => {
                self.lines[i].text.clone()
            }
            _ => {
                let fallback = if name.trim().is_empty() { "tool" } else { name };
                self.push_line(LineKind::Tool, fallback);
                fallback.to_string()
            }
        };
        text.push_str(match ok {
            Some(true) => " ✓",
            Some(false) => " ✗",
            None => " (done)",
        });
        if let Some(n) = note.map(str::trim).filter(|s| !s.is_empty()) {
            text.push_str(" — ");
            text.push_str(n);
        }
        match idx {
            Some(i) if i < self.lines.len() => self.lines[i].text = text,
            _ => {
                // 上面 fallback push 的行就是最后一行
                if let Some(last) = self.lines.last_mut() {
                    last.text = text;
                }
            }
        }
        self.refresh_search_meta();
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
mod tests {
    use super::*;

    #[test]
    fn stream_delta_merges_same_kind() {
        let mut s = UiState::new();
        s.stream_delta(LineKind::Assistant, "你");
        s.stream_delta(LineKind::Assistant, "好");
        assert_eq!(s.lines.len(), 1);
        assert_eq!(s.lines[0].text, "你好");
    }

    #[test]
    fn stream_delta_breaks_on_kind_change() {
        let mut s = UiState::new();
        s.stream_delta(LineKind::Thinking, "think");
        s.stream_delta(LineKind::Assistant, "hi");
        assert_eq!(s.lines.len(), 2);
        assert_eq!(s.lines[0].kind, LineKind::Thinking);
        assert_eq!(s.lines[1].kind, LineKind::Assistant);
    }

    #[test]
    fn system_never_merges() {
        let mut s = UiState::new();
        s.push_line(LineKind::System, "a");
        s.push_line(LineKind::System, "b");
        assert_eq!(s.lines.len(), 2);
    }

    #[test]
    fn input_insert_and_backspace() {
        let mut s = UiState::new();
        s.insert_char('a');
        s.insert_char('b');
        assert_eq!(s.current_input(), "ab");
        s.cursor_left();
        s.insert_char('X');
        assert_eq!(s.current_input(), "aXb");
        s.cursor_left();
        s.backspace();
        assert_eq!(s.current_input(), "Xb");
    }

    #[test]
    fn multiline_insert_newline_and_lines() {
        let mut s = UiState::new();
        for ch in "ab".chars() {
            s.insert_char(ch);
        }
        s.insert_newline();
        for ch in "cd".chars() {
            s.insert_char(ch);
        }
        assert_eq!(s.current_input(), "ab\ncd");
        assert_eq!(s.input_line_count(), 2);
        assert_eq!(s.input_lines(), vec!["ab", "cd"]);
        assert_eq!(s.cursor_line_col(), (1, 2));
    }

    #[test]
    fn multiline_home_end_per_line() {
        let mut s = UiState::new();
        for ch in "ab\ncd".chars() {
            s.insert_char(ch);
        }
        // 光标在第 1 行 "cd" 末尾
        s.cursor_home();
        assert_eq!(s.cursor_line_col(), (1, 0));
        s.cursor_end();
        assert_eq!(s.cursor_line_col(), (1, 2));
        s.cursor_left();
        assert_eq!(s.cursor_line_col(), (1, 1));
    }

    #[test]
    fn multiline_cursor_vertical_moves_and_clamps() {
        let mut s = UiState::new();
        for ch in "abc\nde\nfgh".chars() {
            s.insert_char(ch);
        }
        // 从光标行（行 2）行首出发，一路 Home 到行 0
        s.cursor_home();
        assert_eq!(s.cursor_line_col(), (2, 0));
        s.move_cursor_vert(true);
        assert_eq!(s.cursor_line_col(), (1, 0));
        s.move_cursor_vert(true);
        assert_eq!(s.cursor_line_col(), (0, 0));
        // 顶部再上移不变
        s.move_cursor_vert(true);
        assert_eq!(s.cursor_line_col(), (0, 0));
        // 行尾(0,3)下移 → 行1 col min(3,2)=2；上移回到行0 col2
        s.cursor_end();
        s.move_cursor_vert(false);
        assert_eq!(s.cursor_line_col(), (1, 2));
        s.move_cursor_vert(true);
        assert_eq!(s.cursor_line_col(), (0, 2));
        // 一路下到行 2，行底再下移不变
        s.cursor_end();
        s.move_cursor_vert(false);
        assert_eq!(s.cursor_line_col(), (1, 2));
        s.move_cursor_vert(false);
        assert_eq!(s.cursor_line_col(), (2, 2));
        s.cursor_end();
        assert_eq!(s.cursor_line_col(), (2, 3));
        s.move_cursor_vert(false);
        assert_eq!(s.cursor_line_col(), (2, 3), "bottom clamps");
    }

    #[test]
    fn take_input_clears() {
        let mut s = UiState::new();
        s.insert_char('h');
        assert_eq!(s.take_input(), "h");
        assert_eq!(s.current_input(), "");
    }

    fn session(id: &str, conv: Option<&str>) -> SessionListItem {
        SessionListItem {
            id: id.to_string(),
            title: String::new(),
            server_conversation_id: conv.map(str::to_string),
        }
    }

    #[test]
    fn sessions_clamp_selection() {
        let mut s = UiState::new();
        s.selected = 5;
        let rows = [session("a", None), session("b", Some("c2"))];
        s.replace_sessions(rows.to_vec());
        assert_eq!(s.selected, 1);
    }

    #[test]
    fn session_selection_moves() {
        let mut s = UiState::new();
        let rows = [session("a", None), session("b", None)];
        s.replace_sessions(rows.to_vec());
        s.move_selection(false);
        assert_eq!(s.selected, 1);
        s.move_selection(false);
        assert_eq!(s.selected, 1, "clamped at bottom");
        s.move_selection(true);
        assert_eq!(s.selected, 0);
        s.move_selection(true);
        assert_eq!(s.selected, 0, "clamped at top");
    }

    #[test]
    fn selected_resume_ignores_missing() {
        let mut s = UiState::new();
        let rows = [session("a", None), session("b", Some(" c9 "))];
        s.replace_sessions(rows.to_vec());
        assert_eq!(s.selected_resume(), None, "row a has no conv");
        s.move_selection(false);
        assert_eq!(s.selected_resume(), Some("c9".to_string()));
    }

    #[test]
    fn row_in_use_marks_current_conv() {
        let mut s = UiState::new();
        s.conversation_id = Some("c2".to_string());
        let rows = [session("a", Some("c1")), session("b", Some("c2"))];
        assert!(!s.row_in_use(&rows[0]));
        assert!(s.row_in_use(&rows[1]));
    }

    #[test]
    fn reset_transcript_clears_lines() {
        let mut s = UiState::new();
        s.push_line(LineKind::User, "hi");
        s.view_offset = 3;
        s.tool_start("tc-1", "exec");
        s.start_search("hi");
        s.reset_transcript();
        assert!(s.lines.is_empty());
        assert_eq!(s.view_offset, 0);
        assert!(!s.search_active());
    }

    #[test]
    fn thinking_collapsed_by_default_and_toggles() {
        let mut s = UiState::new();
        s.push_line(LineKind::Thinking, "deep thought");
        assert!(s.lines[0].collapsed);
        assert!(!s.thinking_visible());
        s.toggle_thinking();
        assert!(s.thinking_visible());
        assert!(!s.lines[0].collapsed);
        s.toggle_thinking();
        assert!(s.lines[0].collapsed);
    }

    #[test]
    fn new_thinking_rows_follow_global_expand_state() {
        let mut s = UiState::new();
        s.push_line(LineKind::User, "q");
        // 先展开（此时还没有 thinking 行）
        s.toggle_thinking();
        // User 行之后新开的 thinking 块应保持展开
        s.stream_delta(LineKind::Thinking, "思考a");
        assert!(!s.lines.last().unwrap().collapsed);
        // 同块续接仍展开
        s.stream_delta(LineKind::Thinking, "思考b");
        assert!(!s.lines.last().unwrap().collapsed);
        // 跨 Tool 行后再次出现的 thinking 块也应展开
        s.push_line(LineKind::Tool, "exec");
        s.stream_delta(LineKind::Thinking, "思考c");
        assert!(!s.lines.last().unwrap().collapsed);
        // 折叠后新建的 thinking 块恢复默认折叠
        s.toggle_thinking();
        s.push_line(LineKind::System, "x");
        s.stream_delta(LineKind::Thinking, "思考d");
        assert!(s.lines.last().unwrap().collapsed);
    }

    #[test]
    fn tool_row_start_then_end_updates_in_place() {
        let mut s = UiState::new();
        s.tool_start("tc-9", "exec");
        assert_eq!(s.lines.last().unwrap().kind, LineKind::Tool);
        assert_eq!(s.lines.last().unwrap().text, "exec");
        s.tool_end("tc-9", "exec", Some(true), Some("exit 0"));
        assert_eq!(s.lines.last().unwrap().text, "exec ✓ — exit 0");
        assert_eq!(s.lines.len(), 1);
    }

    #[test]
    fn tool_end_without_start_pushes_fallback_row() {
        let mut s = UiState::new();
        s.tool_end("tc-miss", "patch", Some(false), Some("rejected"));
        assert_eq!(s.lines.last().unwrap().text, "patch ✗ — rejected");
    }

    #[test]
    fn search_start_finds_and_highlights_roundtrip() {
        let mut s = UiState::new();
        s.push_line(LineKind::User, "alpha");
        s.push_line(LineKind::Assistant, "Beta beta");
        s.push_line(LineKind::System, "gamma");
        assert_eq!(s.start_search("beta"), 1);
        assert_eq!(s.search_cursor, Some(1));
        assert_eq!(s.search_total, 1);
        // 下一个命中循环回第一个
        assert_eq!(s.next_search_hit(), Some(1));
        s.clear_search();
        assert!(!s.search_active());
    }

    #[test]
    fn search_skips_non_matching_lines() {
        let mut s = UiState::new();
        s.push_line(LineKind::User, "x");
        s.push_line(LineKind::Assistant, "你好世界");
        assert_eq!(s.start_search("世界"), 1);
        assert_eq!(s.search_cursor, Some(1));
    }

    #[test]
    fn search_total_refreshes_on_new_content() {
        let mut s = UiState::new();
        s.push_line(LineKind::User, "x");
        assert_eq!(s.start_search("boom"), 0);
        assert_eq!(s.search_total, 0);
        // 流式新内容命中：状态行计数应立刻刷新（跳转仍需用户 /find）
        s.stream_delta(LineKind::Assistant, "boom now");
        assert_eq!(s.search_total, 1);
        assert_eq!(s.search_cursor, None);
        // 同块续接命中次数不变
        s.stream_delta(LineKind::Assistant, " boom");
        assert_eq!(s.search_total, 1);
        // 工具行摘要更新为命中（原位文本变更也刷新）
        s.push_line(LineKind::Tool, "exec");
        assert_eq!(s.search_total, 1);
        s.tool_end("tc-1", "exec", Some(true), Some("boom done"));
        assert_eq!(s.search_total, 2);
    }

    #[test]
    fn serve_defaults_parse_status() {
        let v = serde_json::json!({
            "model": "gpt-x",
            "default_agent_role_id": "coder",
            "default_session_mode": "plan",
            "api_base": "http://x",
        });
        let d = ServeDefaults::from_status(&v);
        assert_eq!(d.model.as_deref(), Some("gpt-x"));
        assert_eq!(d.role.as_deref(), Some("coder"));
        assert_eq!(d.mode.as_deref(), Some("plan"));
    }

    #[test]
    fn serve_defaults_skip_blank() {
        let v = serde_json::json!({"model": "  "});
        let d = ServeDefaults::from_status(&v);
        assert_eq!(d.model, None);
    }
}
