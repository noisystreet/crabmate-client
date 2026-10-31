//! 输入框焦点下的按键分发。
//!
//! 单独成文件的理由与 `sidebar_keys` / `workspace_keys` 相同：`mod.rs` 贴着 920 行文件
//! 上限，而这条 `match` 本来就贴着 lizard 的 CCN ≤ 10。
//!
//! **折叠导航不在这里**：它是显式模式、与焦点无关，拦在 `on_hotkey` 里（否则焦点在
//! 侧栏时 `Ctrl+G` 完全没反应，因为 `on_key` 根本不进这条分支）。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::LaneApp;
use super::history;
use super::hotkeys::{is_alt_enter, is_ctrl, is_ctrl_e, is_ctrl_o, is_ctrl_w};

impl LaneApp<'_> {
    /// 输入框焦点下的按键。
    ///
    /// `Ctrl+<字母>` 的组合**不在这里**逐个开臂：进折叠导航走 `on_hotkey`，其余组合
    /// （`Ctrl+C` / `Ctrl+D` / `Ctrl+Z`）由事件循环层处理 —— 它们落到下面的 `_ => {}`
    /// 即可，因此不必为「吞掉它们」单开一个 or-pattern 臂。
    pub(super) fn on_input_key(&mut self, key: KeyEvent) {
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

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::super::state::{Focus, LineKind};
    use super::super::test_support::{test_app, test_client, test_prefs};
    use super::LaneApp;

    /// 这些必须在**外壳**上测：`UiState` 的单测碰不到 `on_key` 的分派顺序，而顺序错了
    /// 逻辑再对也是「按了 Enter 把半句话发出去」。
    fn app_with_two_runs(f: impl FnOnce(&mut LaneApp<'_>)) {
        let client = test_client();
        let mut prefs = test_prefs();
        let (tx, rx) = mpsc::channel();
        let (job_tx, _job_rx) = mpsc::channel();
        let mut app = test_app(&client, &mut prefs, rx, tx, job_tx);
        app.st.push_line(LineKind::Tool, "✓ a");
        app.st.push_line(LineKind::Tool, "✓ b");
        app.st.fold_process_runs(None);
        f(&mut app);
    }

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    fn hidden(app: &LaneApp<'_>) -> usize {
        app.st.lines.iter().filter(|l| l.is_hidden()).count()
    }

    /// 导航激活时 `Enter` 归导航（开合），**不是**提交。
    #[test]
    fn enter_toggles_instead_of_submitting_while_navigating() {
        app_with_two_runs(|app| {
            assert_eq!(hidden(app), 2);

            app.on_key(key(KeyCode::Char('g'), KeyModifiers::CONTROL));
            assert!(app.st.fold_cursor.is_some(), "Ctrl+G 进导航");

            app.on_key(key(KeyCode::Enter, KeyModifiers::NONE));

            assert_eq!(hidden(app), 0, "Enter 拿去展开了");
            assert!(
                !app.st.lines.iter().any(|l| l.kind() == LineKind::User),
                "不该冒出用户消息（提交会 push 一条）"
            );
        });
    }

    /// `Ctrl+G` 与焦点无关：焦点在侧栏 / 目录树时同样进导航。
    ///
    /// 挂在 `on_input_key` 里时这条不成立（`on_key` 在焦点不是 Input 时根本不走那条），
    /// 而这正是最容易踩的路径 —— 刚点过会话栏的人按 `Ctrl+G` 会以为功能坏了。
    #[test]
    fn fold_navigation_opens_from_the_sidebar_and_the_tree() {
        app_with_two_runs(|app| {
            for focus in [Focus::Sidebar, Focus::Workspace] {
                app.st.fold_cursor = None;
                app.st.focus = focus;

                app.on_key(key(KeyCode::Char('g'), KeyModifiers::CONTROL));

                assert!(app.st.fold_cursor.is_some(), "{focus:?} 聚焦时也该进导航");
            }
        });
    }

    /// 设置面板是模态：面板开着时 `Ctrl+G` 不接管（面板本身仍能被 F6 关掉）。
    #[test]
    fn the_settings_panel_keeps_its_keys() {
        app_with_two_runs(|app| {
            app.toggle_settings();
            assert!(app.panel.is_some(), "面板已打开");

            app.on_key(key(KeyCode::Char('g'), KeyModifiers::CONTROL));

            assert_eq!(app.st.fold_cursor, None, "面板里的 Ctrl+G 不该进导航");
        });
    }

    /// 导航退出后 `Enter` 交回提交路径（这里用「输入框被读出」作代理断言：
    /// 真正提交需要 worker 在跑，外壳测试只验「键回到了原来那条分支」）。
    #[test]
    fn esc_hands_the_keys_back_to_the_composer() {
        app_with_two_runs(|app| {
            app.on_key(key(KeyCode::Char('g'), KeyModifiers::CONTROL));
            assert!(app.st.fold_cursor.is_some());

            app.on_key(key(KeyCode::Esc, KeyModifiers::NONE));
            assert_eq!(app.st.fold_cursor, None, "Esc 退出导航");

            app.on_key(key(KeyCode::Char('x'), KeyModifiers::NONE));
            assert!(
                app.st.input_snapshot().contains('x'),
                "字母重新落进输入框：{:?}",
                app.st.input_snapshot()
            );
        });
    }
}
