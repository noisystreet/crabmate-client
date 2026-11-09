//! 文本模式（`chat` / `repl`）的旁注文案：流里发生了「用户看不到就没法继续」的事件时说什么。
//!
//! 与 [`super::trim_line`] 同一动机：文案与「怎么写出这一行」分开，`chat_stream.rs`
//! 里只剩一行调用（那里贴着 920 行文件门禁）。全屏 `tui` 有自己的渲染（浮层 / transcript
//! 行），不走这里。

/// 澄清问卷（issue #424）：文本模式必须**至少说出来**。
///
/// 默认实现是空操作，而症状与 issue 修的那个「静默丢弃」一模一样：用户只看到工具行，
/// 服务端在等一份谁也不会提交的问卷。`chat` 一轮即退出，`repl` 的下一行会被当成普通消息，
/// 所以这里只提示换 `tui`，不假装能答。
#[must_use]
pub fn clarify_unanswerable_line(question_count: usize) -> String {
    format!(
        "模型请求澄清（{question_count} 题）：文本模式无法作答，请用 `crabmate-tui tui` 打开表单"
    )
}

#[cfg(test)]
mod tests {
    use super::clarify_unanswerable_line;

    /// 题数必须出现在提示里：只说「要澄清」，读者不知道是多少题，也无从判断要不要换界面。
    #[test]
    fn names_the_question_count_and_the_way_out() {
        let line = clarify_unanswerable_line(3);
        assert!(line.contains('3'), "{line}");
        assert!(line.contains("crabmate-tui tui"), "{line}");
    }
}
