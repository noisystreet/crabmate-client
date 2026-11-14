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
use super::hotkeys::{is_alt_enter, is_ctrl, is_ctrl_e, is_ctrl_f, is_ctrl_o, is_ctrl_w};

impl LaneApp<'_> {
    /// 输入框焦点下的按键。
    ///
    /// `Ctrl+<字母>` 的组合**不在这里**逐个开臂：进折叠导航走 `on_hotkey`，其余组合
    /// （`Ctrl+C` / `Ctrl+D` / `Ctrl+Z`）由事件循环层处理 —— 它们落到下面的 `_ => {}`
    /// 即可，因此不必为「吞掉它们」单开一个 or-pattern 臂。
    pub(super) fn on_input_key(&mut self, key: KeyEvent) {
        // 编辑器视图先拿滚动键与 Esc：它们与 composer 的 PgUp / 方向键语义冲突，
        // 不拦在最前面就会「滚了半页 transcript、文件纹丝不动」。
        if self.st.editor.open && self.on_editor_key(&key) {
            self.st.slash_after_input();
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
            // Delete 是 Backspace 的对偶（向后删）。不开这个臂就会落到下面的 `_`，
            // 也就是「按了没反应」—— 而光标不在行尾时正是最期待它生效的时刻。
            KeyCode::Delete => self.st.delete_forward(),
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

    /// 编辑器视图的按键（返回 `true` = 已消费）。
    ///
    /// 只接管与「滚 transcript / 移动光标」冲突的那些：`PgUp` / `PgDn` 滚一屏、
    /// `Alt+↑/↓` 滚一行、`Ctrl+Home` / `Ctrl+End` 跳首尾、输入为空时 `Esc` 回聊天。
    ///
    /// 其余字符照常进 composer —— `/chat` 与 `/editor <另一个文件>` 必须还能直接敲，
    /// 否则「切回来」就只能靠记住快捷键。`Home` / `End` 不加修饰时仍归输入框（行首 / 行尾），
    /// 因为编辑器里最常用的粗定位是 PgUp/PgDn，而不是这两个键。
    fn on_editor_key(&mut self, key: &KeyEvent) -> bool {
        // 查找相关的键先来：`Ctrl+F` 进查找态，其后字符进的是**查找词**，不是 composer。
        if self.on_editor_find_key(key) {
            return true;
        }
        let page = self.st.page_rows.max(1) as isize;
        match key.code {
            KeyCode::Esc if self.st.current_input().is_empty() => {
                self.editor_close();
                true
            }
            KeyCode::PageUp => {
                self.st.editor.scroll_by(-page);
                true
            }
            KeyCode::PageDown => {
                self.st.editor.scroll_by(page);
                true
            }
            KeyCode::Up if key.modifiers.contains(KeyModifiers::ALT) => {
                self.st.editor.scroll_by(-1);
                true
            }
            KeyCode::Down if key.modifiers.contains(KeyModifiers::ALT) => {
                self.st.editor.scroll_by(1);
                true
            }
            KeyCode::Home if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.st.editor.scroll_to_top();
                true
            }
            KeyCode::End if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.st.editor.scroll_to_bottom();
                true
            }
            _ => false,
        }
    }

    /// 查找态的按键（返回 `true` = 已消费）：`Ctrl+F` 进入，之后 `Enter` / `Shift+Enter`
    /// 跳下 / 上一处、普通字符进查找词、`Backspace` 退格、`Esc` 只关查找（**不退视图** ——
    /// 退视图是输入框为空时的 Esc，两件事）。
    ///
    /// 字符不进 composer 是这里的全部意义：查找态里敲 `a` 必须是在找 `a`，而不是在起草消息。
    fn on_editor_find_key(&mut self, key: &KeyEvent) -> bool {
        if is_ctrl_f(key) {
            self.st.editor.find.begin();
            return true;
        }
        if !self.st.editor.find.open {
            return false;
        }
        match key.code {
            KeyCode::Esc => self.st.editor.find.close(),
            KeyCode::Enter => self.editor_find_step(!key.modifiers.contains(KeyModifiers::SHIFT)),
            KeyCode::Backspace => {
                self.st.editor.find.pop();
                self.editor_find_reveal();
            }
            KeyCode::Char(ch)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.st.editor.find.push(ch);
                self.editor_find_reveal();
            }
            // 这几个键在查找态里必须自己吞掉：它们否则改的是**草稿 / 焦点**，而屏幕上
            // 没有任何东西解释「我明明在找东西，怎么把半句话发出去了 / 焦点怎么跑了」。
            // `Ctrl+O` 是提交、`Ctrl+W` 是进目录树、`Ctrl+E` 是折叠思考、`Tab` 是切焦点。
            KeyCode::Tab => {}
            KeyCode::Char('o' | 'w' | 'e' | 'O' | 'W' | 'E')
                if key.modifiers.contains(KeyModifiers::CONTROL) => {}
            // 其余（方向键 / Home / End / Delete / `Ctrl+C` / `Ctrl+T` / `Ctrl+Y` …）照旧
            // 漏给上层：取消回合与切视图都属于「逃生」，查找态不该把它们扣住。
            _ => return false,
        }
        true
    }

    /// 跳到下一处 / 上一处：先让命中集合跟上内容（查询词或文件可能刚变），再滚到锚点。
    fn editor_find_step(&mut self, forward: bool) {
        self.st.editor.sync_find();
        self.st.editor.find.cycle(forward);
        self.editor_find_reveal();
    }

    /// 把查找锚点滚进视口（没有命中就什么都不动）。
    fn editor_find_reveal(&mut self) {
        let visible = self.st.page_rows.max(1);
        let ed = &mut self.st.editor;
        ed.sync_find();
        if let Some(row) = ed.find.anchor_row() {
            ed.reveal_row(row, visible);
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

    /// `Delete` 在输入框里是向后删。
    ///
    /// 必须在**外壳**上测：`UiState::delete_forward` 的单测保证算术对，这条保证
    /// `on_input_key` 的臂真的接上了 —— 漏掉一条臂的表现就是「按了没反应」。
    #[test]
    fn delete_key_deletes_forward_in_the_composer() {
        app_with_two_runs(|app| {
            for ch in "abc".chars() {
                app.on_key(key(KeyCode::Char(ch), KeyModifiers::NONE));
            }
            app.on_key(key(KeyCode::Left, KeyModifiers::NONE));
            app.on_key(key(KeyCode::Delete, KeyModifiers::NONE));

            assert_eq!(app.st.current_input(), "ab", "删掉光标处的 c，光标留在原处");
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
    /// `Ctrl+F` 之后敲的字符进**查找词**，不进 composer；`Esc` 只关查找、不退视图。
    ///
    /// 这条守的是「两件事不能混」：查找态里敲 `a` 必须是在找 `a`，一旦漏给 composer，
    /// 用户就会在起草一条消息的同时莫名其妙地找了一堆东西。
    #[test]
    fn ctrl_f_takes_the_typing_away_from_the_composer() {
        app_with_two_runs(|app| {
            app.st.editor.open = true;
            app.st.editor.accept_limited("alpha\nbeta\n", 100);

            app.on_key(key(KeyCode::Char('f'), KeyModifiers::CONTROL));
            assert!(app.st.editor.find.open, "Ctrl+F 进查找态");

            app.on_key(key(KeyCode::Char('a'), KeyModifiers::NONE));
            app.on_key(key(KeyCode::Char('l'), KeyModifiers::NONE));
            assert_eq!(app.st.editor.find.query, "al");
            assert!(
                app.st.input_snapshot().is_empty(),
                "查找态里字符不该落进 composer：{:?}",
                app.st.input_snapshot()
            );

            app.on_key(key(KeyCode::Enter, KeyModifiers::NONE));
            assert_eq!(app.st.editor.find.hits, vec![0], "alpha 是唯一命中行");
            assert!(
                !app.st.lines.iter().any(|l| l.kind() == LineKind::User),
                "查找态里的 Enter 是「下一处」，不是提交"
            );

            app.on_key(key(KeyCode::Esc, KeyModifiers::NONE));
            assert!(!app.st.editor.find.open, "Esc 只关查找");
            assert!(
                app.st.editor.open,
                "视图还开着（退视图是输入框为空时的 Esc）"
            );
        });
    }

    /// 查找态里 `Ctrl+O` / `Tab` 也不能漏给上层：前者会提交草稿、后者会切焦点。
    ///
    /// `Ctrl+O` 是最意外的一条 —— 屏幕上没有任何东西解释「我在找东西，怎么半句话发出去了」。
    #[test]
    fn find_mode_keeps_the_draft_and_the_focus() {
        app_with_two_runs(|app| {
            app.st.editor.open = true;
            app.st.editor.accept_limited("alpha\nbeta\n", 100);
            // 先写一个字进 composer（编辑器视图里普通字符照旧进 composer）。
            app.on_key(key(KeyCode::Char('d'), KeyModifiers::NONE));
            app.on_key(key(KeyCode::Char('f'), KeyModifiers::CONTROL));
            assert!(app.st.editor.find.open);

            app.on_key(key(KeyCode::Char('o'), KeyModifiers::CONTROL));
            app.on_key(key(KeyCode::Tab, KeyModifiers::NONE));

            assert!(
                !app.st.lines.iter().any(|l| l.kind() == LineKind::User),
                "查找态里 Ctrl+O 不该提交"
            );
            assert!(
                app.st.input_snapshot().contains('d'),
                "草稿还在：{:?}",
                app.st.input_snapshot()
            );
            assert!(app.st.editor.find.open, "这两个键都不退查找态");
        });
    }

    /// `Enter` 跳到下一处并把锚点滚进视口：只改锚点不滚屏，屏幕上等于什么都没发生。
    #[test]
    fn enter_walks_the_hits_and_scrolls_to_them() {
        app_with_two_runs(|app| {
            app.st.editor.open = true;
            let mut body = String::new();
            for i in 1..=40 {
                if i % 10 == 0 {
                    body.push_str(&format!("hit{i}\n"));
                } else {
                    body.push_str(&format!("plain{i}\n"));
                }
            }
            app.st.editor.accept_limited(&body, 100);

            app.on_key(key(KeyCode::Char('f'), KeyModifiers::CONTROL));
            for ch in "hit".chars() {
                app.on_key(key(KeyCode::Char(ch), KeyModifiers::NONE));
            }
            assert_eq!(
                app.st.editor.find.hits.len(),
                4,
                "{:?}",
                app.st.editor.find.hits
            );
            let first = app.st.editor.scroll;

            app.on_key(key(KeyCode::Enter, KeyModifiers::NONE));
            assert_eq!(app.st.editor.find.position(), Some((2, 4)));
            assert!(
                app.st.editor.scroll >= first,
                "滚动要跟着锚点走（{} → {}）",
                first,
                app.st.editor.scroll
            );

            // 上一处（Shift+Enter）反向走到第一处。
            app.on_key(key(KeyCode::Enter, KeyModifiers::SHIFT));
            assert_eq!(app.st.editor.find.position(), Some((1, 4)));
        });
    }
}
