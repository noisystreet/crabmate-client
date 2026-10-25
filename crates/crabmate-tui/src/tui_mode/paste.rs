//! 多行粘贴：终端的 bracketed paste（`Event::Paste`）落到这里。
//!
//! 没有开启 bracketed paste 时（或 `CM_TUI_PASTE=off`），终端把粘贴当成一串普通按键，
//! 换行变成 `KeyCode::Enter` 被 `on_input_key` 当成**提交**——多行内容只发得出第一行，
//! 剩下的落进新草稿；夹带的 `Tab` 还会被当成焦点切换。开启后整段粘贴是一个事件，
//! 走这条路径一次性写进输入框。

use super::TuiApp;
use super::state::LineKind;

impl TuiApp<'_> {
    /// 粘贴文本写进输入框光标处（**不提交**）。
    ///
    /// 超长内容由 [`super::state::UiState::insert_paste`] 截断，这里只负责告知一句。
    pub(super) fn on_paste(&mut self, text: String) {
        let pasted = self.st.insert_paste(&text);
        if pasted.truncated {
            self.st.push_line(
                LineKind::System,
                &format!("粘贴内容过长：只插入前 {} 个字符", pasted.chars),
            );
        }
        // 草稿变了：与键盘输入同一条收尾（`/` 菜单收敛 + skill 候选按需拉取）。
        self.st.slash_after_input();
        self.maybe_fetch_skills();
    }
}
