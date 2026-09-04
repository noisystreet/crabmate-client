//! 全屏 TUI 的 UI 状态：transcript 行 + 单行输入缓冲 + 回合/会话指示。

use crabmate_tui_core::SessionListItem;
use serde_json::Value;

/// transcript 行类别（决定前缀与配色）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    User,
    Assistant,
    Thinking,
    System,
}

/// 键盘焦点：底栏输入框或左栏会话列表。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Focus {
    #[default]
    Input,
    Sidebar,
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
}

impl LogLine {
    fn append(&mut self, delta: &str) {
        self.text.push_str(delta);
    }
}

/// 全屏 UI 状态；仅由事件循环触碰（单线程）。
#[derive(Debug, Default)]
pub struct UiState {
    pub lines: Vec<LogLine>,
    /// 输入缓冲（Vec<char> 便于光标插入）。
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
}

impl UiState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_line(&mut self, kind: LineKind, text: &str) {
        self.lines.push(LogLine {
            kind,
            text: text.to_string(),
        });
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
    }

    pub fn current_input(&self) -> String {
        self.input.iter().collect()
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

    pub fn cursor_home(&mut self) {
        self.cursor = 0;
    }

    pub fn cursor_end(&mut self) {
        self.cursor = self.input.len();
    }

    /// 光标前内容（渲染输入时在光标位置切分）。
    #[must_use]
    pub fn input_before_cursor(&self) -> String {
        self.input[..self.cursor.min(self.input.len())]
            .iter()
            .collect()
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
        self.view_offset = 0;
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
        s.reset_transcript();
        assert!(s.lines.is_empty());
        assert_eq!(s.view_offset, 0);
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
