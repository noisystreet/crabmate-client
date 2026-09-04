//! 全屏 TUI 的 UI 状态：transcript 行 + 单行输入缓冲 + 回合/会话指示。

/// transcript 行类别（决定前缀与配色）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    User,
    Assistant,
    Thinking,
    System,
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
}
