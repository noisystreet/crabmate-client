//! 正文区左键点击的分派：链接（`Ctrl+单击`）→ 过程摘要折叠 → 兜底收回焦点。
//!
//! 从 `mouse.rs` 拆出来有两个硬理由：
//! - `on_mouse` 的圈复杂度贴着门禁（CCN≤10），再在同一函数里加一层修饰键判断就破线；
//! - `mouse.rs` 也贴着 920 行文件门禁，这三步分派搬走后两边都有余量。
//!
//! 顺序是有意的：链接最具体（命中一列就算），其次是摘要行的开合，最后才是
//! 「点正文 = 回输入框」的兜底 —— 兜底若先跑，前面两种就永远轮不到。

use super::LaneApp;
use super::state::Focus;

impl LaneApp<'_> {
    /// 正文区左键点击。`ctrl` 是本次点击是否带 Ctrl —— 只有它才允许打开链接。
    ///
    /// 普通单击保留原语义（回输入框）：把「打开浏览器」挂在单击上，每次想打字都会弹窗。
    pub(super) fn on_body_click(&mut self, column: u16, row: u16, ctrl: bool) {
        if ctrl && self.click_link(column, row) {
            return;
        }
        if self.click_body(column, row) {
            return;
        }
        // 点右栏以外：想打字或看聊天，把焦点从侧栏收回输入框
        // （否则后续按键仍落在侧栏，用户会觉得「点了没反应」）。
        if matches!(self.st.focus, Focus::Sidebar | Focus::Workspace) {
            self.st.focus = Focus::Input;
            self.st.focus_by_pointer = false;
            // 焦点离开会话栏：作废待确认删除，免得回来按 y 误删。
            self.st.confirm_delete = None;
        }
    }
}
