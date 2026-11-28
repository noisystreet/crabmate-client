//! 输入缓冲的只读投影与光标导航（判空 / `/` 前缀必要条件 / `backspace` /
//! `delete_forward` / 方向键 / 行首行尾 / 上下行）。
//!
//! 从 `state.rs` 搬出：`UiState` 的主 `impl` 已经四百多行，而这一组是纯光标算术 ——
//! 只碰 `input` / `cursor` 两个私有字段，与其余状态无关。搬出去是因为 issue #424 要往
//! `UiState` 加 `clarify` 字段，而 `state.rs` 正贴着 920 行的文件门禁。
//!
//! 两个**只读**投影（`input_is_blank` / `draft_may_be_slash`）也放这里：它们同样是
//! 「只碰 `input`」的读法，且都是渲染每帧要问的判据 —— 与 `input_line_count` 同一组。
//!
//! 子模块才能直接写私有字段（与 `state_paste` / `state_search` 同一手法）。

use super::UiState;

impl UiState {
    /// 把输入框（内容 + 光标位置）与另一份 `UiState` 整体交换。
    ///
    /// 切会话 / 停泊槽换进换出时，输入框必须**留在视图上**：改用 `set_input` 会把光标弹到
    /// 行尾（`set_input` 里 `cursor = input.len()`），而后台会话每来一批流式增量都要走一次
    /// 换进换出 —— 用户正在打字时会被连续打断，敲进去的字落在错位置。
    ///
    /// 放这里而不是 `state.rs`：那边贴着 920 行门禁，而这是「只碰 `input` / `cursor`」的
    /// 一组算术之一（见模块头注释）。
    pub(crate) fn swap_input_with(&mut self, other: &mut Self) {
        std::mem::swap(&mut self.input, &mut other.input);
        std::mem::swap(&mut self.cursor, &mut other.cursor);
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let idx = self.cursor - 1;
        self.input.remove(idx);
        self.cursor = idx;
    }

    /// Delete：删掉光标**之后**的一个字符（`backspace` 的对偶）。
    ///
    /// 光标已在末尾时什么都不做 —— 与 `backspace` 在开头时的行为对称。光标压在 `\n` 上时
    /// 删掉的就是那个换行，两行随之接上；这也是多行草稿里唯一的「向后删换行」入口。
    pub fn delete_forward(&mut self) {
        if self.cursor >= self.input.len() {
            return;
        }
        self.input.remove(self.cursor);
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

    /// 输入缓冲是否多行（含 `\n`）。在 `input_keys` 里决定 ↑/↓ 是「移光标」还是「历史」。
    #[must_use]
    pub fn is_multiline(&self) -> bool {
        self.input.contains(&'\n')
    }

    /// 输入缓冲是否只有空白（composer 占位提示的判据）。
    ///
    /// 与 `current_input().trim().is_empty()` 等价，但**不构造整个字符串**：这个判断每帧
    /// 都要问一次，长输入下就是每帧一次 O(n) 分配。
    ///
    /// 判据用 `char::is_whitespace`，与 `str::trim` 同一套定义 —— 「只有空白」不存在
    /// 两种口径。
    #[must_use]
    pub fn input_is_blank(&self) -> bool {
        self.input.iter().all(|c| c.is_whitespace())
    }

    /// 草稿**可能**处于 `/` 前缀态的必要条件：跳过前导空白后首字符是 `/`。
    ///
    /// 只是必要条件，不是判定本身 —— 真正的规则仍然只有 `slash_prefix` 一份（还要看
    /// 后续有没有空白或第二个 `/`）。它挡掉的是「绝大多数时候草稿根本不是斜杠命令」，
    /// 那一路不必为此把整个输入缓冲 collect 成 `String`。
    #[must_use]
    pub(super) fn draft_may_be_slash(&self) -> bool {
        self.input
            .iter()
            .find(|c| !c.is_whitespace())
            .is_some_and(|c| *c == '/')
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
}
