//! 输入框焦点下的按键分发。
//!
//! 单独成文件的理由与 `sidebar_keys` / `workspace_keys` 相同：`mod.rs` 贴着 920 行文件
//! 上限，而这条 `match` 本来就贴着 lizard 的 CCN ≤ 10 —— 折叠导航（`Ctrl+G`）一进来就
//! 顶到 11。拆出来之后再加键只碰本文件。
//!
//! 折叠导航的拦截放在最前面：`↑/↓/Enter/Space/Esc` 在导航里是「换一条 / 开合 / 退出」，
//! 落进下面那条 match 就变成滚 transcript、提交、清排队 —— 同一个键两种意思，正是
//! 「明明按了 Enter，却把半句话发出去了」的来源。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::LaneApp;
use super::history;
use super::hotkeys::{is_alt_enter, is_ctrl, is_ctrl_e, is_ctrl_o, is_ctrl_w};

impl LaneApp<'_> {
    /// 输入框焦点下的按键。
    ///
    /// `Ctrl+<字母>` 的组合**不在这里**逐个开臂：进折叠导航走
    /// [`super::UiState::fold_nav_key`]（未激活时它也认 `Ctrl+G`），其余组合（`Ctrl+C` /
    /// `Ctrl+D` / `Ctrl+Z`）由事件循环层处理 —— 它们落到下面的 `_ => {}` 即可，因此不必
    /// 为「吞掉它们」单开一个 or-pattern 臂。
    pub(super) fn on_input_key(&mut self, key: KeyEvent) {
        if self.st.fold_nav_key(&key) {
            return;
        }
        match key.code {
            KeyCode::Enter if is_alt_enter(&key) => self.st.insert_newline(),
            KeyCode::Enter => self.on_submit(),
            KeyCode::Char('o') if is_ctrl_o(&key) => self.on_submit(),
            // Ctrl+W：宽屏时聚焦右栏工作区目录树（窄屏忽略，侧栏本就隐藏）。
            KeyCode::Char('w') if is_ctrl_w(&key) && self.st.sidebar_visible => {
                self.focus_workspace();
            }
            KeyCode::Char('e') if is_ctrl_e(&key) => self.st.toggle_thinking_with_notice(),
            // Esc：清掉排队消息（审批浮层 / `/` 菜单 / 设置面板的 Esc 在前面的层就消费了）。
            KeyCode::Esc => {
                self.clear_queued_on_esc();
            }
            KeyCode::Tab => self.focus_sidebar_on_current(),
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.st.insert_char(ch);
            }
            KeyCode::Backspace => self.st.backspace(),
            KeyCode::Left => self.st.cursor_left(),
            KeyCode::Right => self.st.cursor_right(),
            KeyCode::Home if is_ctrl(&key, KeyCode::Home) => self.st.scroll_top(),
            KeyCode::End if is_ctrl(&key, KeyCode::End) => self.st.scroll_bottom(),
            KeyCode::Home => self.st.cursor_home(),
            KeyCode::End => self.st.cursor_end(),
            // 到顶还继续 PgUp，就是「我要看更早的」（判断在 `on_page_up` 里）。
            KeyCode::PageUp => self.on_page_up(),
            KeyCode::PageDown => self.st.scroll_page(false),
            // 多行编辑时 ↑/↓ 走行内光标；单行仍保留"滚动 transcript"的旧行为。
            KeyCode::Up if self.st.is_multiline() => self.st.move_cursor_vert(true),
            KeyCode::Down if self.st.is_multiline() => self.st.move_cursor_vert(false),
            KeyCode::Up => self.st.scroll_lines(true, 3),
            KeyCode::Down => self.st.scroll_lines(false, 3),
            _ => {}
        }
        // 草稿变了：收敛 `/` 菜单的选中项与「Esc 关闭」状态，并确保 skill 候选已拉过。
        self.st.slash_after_input();
        self.maybe_fetch_skills();
    }

    /// `PgUp`：先翻一屏；本来就在顶（= 用户要看更早的）才静默续翻一页历史。
    ///
    /// 抽出来只为压 `on_input_key` 的圈复杂度 —— 那层嵌套 `if` 单独算一个分支，
    /// 而那条 match 贴着 CCN 门禁。
    fn on_page_up(&mut self) {
        let was_at_top = self.st.body_at_top;
        self.st.scroll_page(true);
        if was_at_top {
            history::load_older_on_top(self);
        }
    }
}
