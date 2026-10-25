//! `state_paste.rs` 的单测（`#[path]` 子模块）。
//!
//! 与 `state_tests.rs` 分开：那个文件开头已被 lizard 解析成一整段大函数体，
//! 再往里加测试会顶到 fn-nloc 门禁（≤ 200 行）。

use super::{PASTE_MAX_CHARS, UiState};

#[test]
fn paste_normalizes_crlf_into_multiline() {
    // 终端 bracketed paste 送的是 `\r\n`；多行判定只认 `\n`，所以必须归一化。
    let mut s = UiState::new();
    let pasted = s.insert_paste("第一行\r\n第二行\r\n第三行");
    assert_eq!(s.current_input(), "第一行\n第二行\n第三行");
    assert!(s.is_multiline());
    assert_eq!(pasted.chars, 11);
    assert!(!pasted.truncated);
}

#[test]
fn paste_expands_tab_and_drops_control_chars() {
    // `\t` 展开成 4 空格；裸 `\x1b` 与 `\x07` 丢弃（渲染层会当成未闭合转义序列）。
    let mut s = UiState::new();
    s.insert_paste("a\tb\u{1b}[31m\u{7}");
    assert_eq!(s.current_input(), "a    b[31m");
}

#[test]
fn paste_lands_at_cursor() {
    // 光标在中间时插在光标处，且光标落到插入内容之后。
    let mut s = UiState::new();
    s.set_input("ab");
    s.cursor_left();
    let pasted = s.insert_paste("X\nY");
    assert_eq!(s.current_input(), "aX\nYb");
    assert_eq!(pasted.chars, 3);
    s.insert_char('!');
    assert_eq!(s.current_input(), "aX\nY!b");
}

#[test]
fn paste_truncates_at_limit() {
    let mut s = UiState::new();
    let huge = "x".repeat(PASTE_MAX_CHARS + 10);
    let pasted = s.insert_paste(&huge);
    assert!(pasted.truncated);
    assert_eq!(pasted.chars, PASTE_MAX_CHARS);
    assert_eq!(s.current_input().chars().count(), pasted.chars);
}

#[test]
fn paste_of_control_only_text_is_noop() {
    // 只剩控制符的粘贴不该污染草稿，也不该报「已截断」。
    let mut s = UiState::new();
    let pasted = s.insert_paste("\u{1b}\u{7}");
    assert_eq!(pasted.chars, 0);
    assert!(!pasted.truncated);
    assert_eq!(s.current_input(), "");
}
