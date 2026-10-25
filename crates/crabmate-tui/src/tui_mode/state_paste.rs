//! 粘贴文本写进输入缓冲：归一化换行 / 制表符、丢弃控制符，再一次性插入。
//!
//! 挂在 `state` 下（`#[path]` 子模块）是为了直接操作私有字段 `input` / `cursor`，
//! 同时压住 `state.rs` 的单文件行数门禁（fn-nloc ≤ 920；`state_tests.rs` 同理）。

use super::UiState;

/// 单次粘贴最多插入的字符数，超出截断（调用方据 [`Pasted::truncated`] 提示）。
///
/// 一次粘贴几十万字符通常是把整个文件塞了进来：不拦会让正文渲染卡住整帧。
pub const PASTE_MAX_CHARS: usize = 32 * 1024;

/// 制表符展开成的空格数（正文按字符宽度算列，裸 `\t` 会把后面的文字顶歪）。
const TAB_AS_SPACES: usize = 4;

/// 一次粘贴的落地结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pasted {
    /// 实际插入的字符数（归一化 + 截断之后）。
    pub chars: usize,
    /// 是否因 [`PASTE_MAX_CHARS`] 被截断。
    pub truncated: bool,
}

impl UiState {
    /// 在光标处插入粘贴文本，返回插入量与截断标记。
    ///
    /// 换行统一记成 `\n`：终端送的是 `\r\n`，而单/多行判定（`is_multiline`）只看 `\n`。
    /// 归一化之后**不做提交判定**——粘贴是一段待编辑的草稿，发不发由用户按回车决定。
    pub fn insert_paste(&mut self, text: &str) -> Pasted {
        let mut buf: Vec<char> = Vec::new();
        for c in text.replace("\r\n", "\n").chars() {
            match c {
                '\r' => buf.push('\n'),
                '\t' => buf.extend([' '; TAB_AS_SPACES]),
                // 其余 C0 / DEL 丢掉：粘贴里夹的裸 `\x1b` 会被渲染层当成未闭合的转义序列。
                c if c.is_control() && c != '\n' => {}
                c => buf.push(c),
            }
        }
        let truncated = buf.len() > PASTE_MAX_CHARS;
        buf.truncate(PASTE_MAX_CHARS);
        self.insert_chars(&buf);
        Pasted {
            chars: buf.len(),
            truncated,
        }
    }

    /// 光标处批量插入。用 `splice` 一次搬到位：逐字符 `Vec::insert` 在中间位置是 O(n²)。
    fn insert_chars(&mut self, chars: &[char]) {
        if chars.is_empty() {
            return;
        }
        let at = self.cursor.min(self.input.len());
        // `Splice` 在 drop 时才真正写回；`let _` 绑定只为立即丢弃并满足其 must_use。
        let _ = self.input.splice(at..at, chars.iter().copied());
        self.cursor = at + chars.len();
    }
}

#[cfg(test)]
#[path = "state_paste_tests.rs"]
mod tests;
