//! 输入框的光标与按行导航（`backspace` / `delete_forward` / 方向键 / 行首行尾 / 上下行）。
//!
//! 从 `state.rs` 搬出：`UiState` 的主 `impl` 已经四百多行，而这一组是纯光标算术 ——
//! 只碰 `input` / `cursor` 两个私有字段，与其余状态无关。搬出去是因为 issue #424 要往
//! `UiState` 加 `clarify` 字段，而 `state.rs` 正贴着 920 行的文件门禁。
//!
//! 子模块才能直接写私有字段（与 `state_paste` / `state_search` 同一手法）。

use super::UiState;

impl UiState {
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
