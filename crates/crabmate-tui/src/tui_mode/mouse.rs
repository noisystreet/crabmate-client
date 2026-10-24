//! 鼠标事件：左栏会话行点击（选中即切换）、右栏工作区目录树的点击与滚轮，以及聊天区滚轮滚动。
//!
//! 两栏各有自己的命中区域：左键点左栏 = 选中并切到那条会话（与 `Enter` 同义），点右栏 =
//! 落树光标并展开/选中；点两栏以外的聊天列仅用于把焦点收回输入框。滚轮在右栏内驱动树光标，
//! 在左栏 / 聊天列滚动 transcript（捕获鼠标后终端不再把滚轮翻成 ↑/↓，必须自己接）。
//! 左栏**刻意不接滚轮**：会话窗口的起点由选中行决定，滚轮只能表现成「顺带改选中」，
//! 而选中即切换 —— 一次误滚就会换掉会话。
//! 所有状态变更都复用键盘已有的状态机（`ws_*` / `use_selected`），避免出现「鼠标能点出、
//! 的状态（占位行不可选中、收起父目录后光标归位等不变量因此自动一致）。

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use super::TuiApp;
use super::layout::{contains, panes};
use super::state::{Focus, LineKind};
use super::workspace_tree::WsActivate;
use super::ws_sidebar::{pane_list_area, sessions_content, sidebar_window, workspace_content};

impl TuiApp<'_> {
    /// 鼠标事件入口。
    ///
    /// 审批浮层/设置面板占用键盘时同样不接受鼠标，否则会绕过
    /// 「审批未决不得输入」「脏字段先确认」这类键盘侧门禁。
    pub(super) fn on_mouse(&mut self, ev: MouseEvent) {
        if self.st.approval.is_some() || self.panel.is_some() {
            return;
        }
        let (width, height) = self.last_size;
        let p = panes(
            Rect::new(0, 0, width, height),
            self.st.sidebar_visible,
            self.status_stack_rows(),
        );
        let (column, row) = (ev.column, ev.row);
        // 窄屏侧栏隐藏时两栏矩形都不存在，鼠标完全不动状态。
        let on_right = p.right.is_some_and(|r| contains(r, column, row));
        let on_left = p.left.is_some_and(|r| contains(r, column, row));
        match ev.kind {
            MouseEventKind::ScrollUp if on_right => self.scroll_workspace(true),
            MouseEventKind::ScrollDown if on_right => self.scroll_workspace(false),
            // 右栏以外的滚轮（聊天列 / 左栏）滚 transcript，步长与键盘单行时的 ↑/↓ 一致。
            // 必须自己处理：捕获鼠标后终端不再把滚轮翻译成 ↑/↓（xterm alternate-scroll），
            // 不接这段就会表现为「聊天区滚轮没反应」。
            MouseEventKind::ScrollUp => self.st.scroll_lines(true, 3),
            MouseEventKind::ScrollDown => self.st.scroll_lines(false, 3),
            MouseEventKind::Down(MouseButton::Left) if on_right => {
                self.click_workspace(column, row);
            }
            // 左栏会话行：选中即切换（与 Enter 同义），见 `click_sessions`。
            MouseEventKind::Down(MouseButton::Left) if on_left => {
                self.click_sessions(column, row);
            }
            // 点右栏以外：想打字或看聊天，把焦点从侧栏收回输入框
            // （否则后续按键仍落在侧栏，用户会觉得「点了没反应」）。
            MouseEventKind::Down(MouseButton::Left) => {
                if matches!(self.st.focus, Focus::Sidebar | Focus::Workspace) {
                    self.st.focus = Focus::Input;
                    // 焦点离开会话栏：作废待确认删除，免得回来按 y 误删。
                    self.st.confirm_delete = None;
                }
            }
            _ => {}
        }
    }

    /// 滚轮：项目池子视图滚候选，目录树滚光标（复用 `ws_move` 的跳过占位语义）。
    ///
    /// 只有树已聚焦时才响应：否则一次误滚（指针恰好停在右栏）就会把焦点从输入框拽到
    /// 侧栏，之后打出的字全变成树的命令键（`r` / `p` / `w`）。想滚树先点它或 Ctrl+W。
    fn scroll_workspace(&mut self, up: bool) {
        if self.st.focus != Focus::Workspace {
            return;
        }
        if self.st.ws_pick.is_some() {
            self.st.ws_pick_move(up);
        } else {
            self.st.ws_move(up);
        }
    }

    /// 左键点击右栏：聚焦 + 落光标；目录行顺带展开/收起（可逆），文件行只选中。
    ///
    /// 文件行不直接插入路径是有意的：那会顶掉输入框里正在写的内容，
    /// 而插入路径仍由 Enter 完成（底部提示行已写明）。
    fn click_workspace(&mut self, column: u16, row: u16) {
        let (width, height) = self.last_size;
        let p = panes(
            Rect::new(0, 0, width, height),
            self.st.sidebar_visible,
            self.status_stack_rows(),
        );
        let Some(right) = p.right else {
            return;
        };
        self.st.focus = Focus::Workspace;
        // 用渲染同一份内容算行数，避免再维护一遍「标题 + 占位行」的行数口径。
        let content_width = right.width.saturating_sub(2) as usize;
        let (rows, cursor, _hint) = workspace_content(&self.st, content_width);
        let Some(idx) = hit_item(right, column, row, rows.len(), cursor) else {
            return; // 标题行 / 底部提示行 / 列表末尾的空白：只切焦点
        };
        if self.st.ws_pick.is_some() {
            if self.st.ws_pick_set(idx) {
                // 点击候选即切换（与 Enter 一致）。
                if let Some(name) = self.st.ws_pick_begin_switch() {
                    self.st
                        .push_line(LineKind::System, &format!("正在切换到项目「{name}」…"));
                    self.switch_ws_project(name);
                }
            }
            return;
        }
        if !self.st.ws_set_cursor(idx) {
            return; // 占位行不可选中（与 ↑↓ 跳过一致）
        }
        match self.st.ws_activate() {
            Some(WsActivate::Dir { rel, fetch }) => {
                if fetch {
                    self.fetch_ws_dir(rel);
                }
            }
            Some(WsActivate::File(_)) | None => {}
        }
    }

    /// 左键点击左栏会话行：聚焦会话栏 + 落选中，并**直接切换**（与 `Enter` 同义）。
    ///
    /// 单击即切换是刻意的：多要一次 `Enter` 只会让人以为「点了没反应」。切换的守卫不在这里
    /// 重写——回合进行中拒绝、空白会话只给提示、`/conv new` 的待绑定空行原地不动，全部由
    /// [`TuiApp::use_selected`] 一个入口负责，鼠标与键盘不会给出两套结论。
    fn click_sessions(&mut self, column: u16, row: u16) {
        let (width, height) = self.last_size;
        let p = panes(
            Rect::new(0, 0, width, height),
            self.st.sidebar_visible,
            self.status_stack_rows(),
        );
        let Some(left) = p.left else {
            return; // 窄屏：左栏不存在
        };
        self.st.focus = Focus::Sidebar;
        // 焦点/选中行一换，待确认删除即作废：否则会「确认的是这一行、删掉的是另一行」。
        self.st.confirm_delete = None;
        // 列内左右各留 1 列：与 `render_pane` 的内宽口径必须是同一个值。
        let content_width = left.width.saturating_sub(2) as usize;
        let (rows, cursor, _hint) = sessions_content(&self.st, content_width);
        let Some(idx) = hit_item(left, column, row, rows.len(), cursor) else {
            return; // 标题行 / 底部提示行 / 列表末尾的空白：只切焦点
        };
        // 空列表时 `rows[1]` 是「（空）发一条消息后出现」占位行，不能当成会话 0 号。
        if idx >= self.st.sessions.len() {
            return;
        }
        self.st.selected = idx;
        self.use_selected();
    }
}

/// 鼠标坐标 → 条目下标：内容区（[`pane_list_area`]）与裁剪窗口（[`sidebar_window`]）都取
/// 渲染用的那一份，两栏的「标题行占第 0 行、窗口末尾是空白」等口径因此只有一处定义。
///
/// 落在标题行、底部提示行或窗口末尾的空白上返回 `None`（调用方只切焦点，不改选中）。
fn hit_item(area: Rect, column: u16, row: u16, rows_len: usize, cursor: usize) -> Option<usize> {
    let list = pane_list_area(area);
    if !contains(list, column, row) {
        return None;
    }
    let (start, end) = sidebar_window(rows_len, list.height as usize, cursor);
    item_index_at((row - list.y) as usize, start, end)
}

/// 可视行（第 0 行是固定标题）→ 条目下标；越界或落在标题行返回 `None`。
fn item_index_at(rel_row: usize, start: usize, end: usize) -> Option<usize> {
    let rel = rel_row.checked_sub(1)?;
    let idx = start + rel;
    (idx < end).then_some(idx)
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

    use super::super::state::Focus;
    use super::super::test_support::{test_app, test_client, test_prefs};
    use super::{TuiApp, item_index_at};
    use crate::serve::SessionListRow;

    fn row(id: &str, cid: Option<&str>) -> SessionListRow {
        SessionListRow {
            id: id.to_string(),
            title: format!("t-{id}"),
            updated_at: 0,
            pinned: false,
            starred: false,
            server_conversation_id: cid.map(str::to_string),
            server_revision: None,
            workspace_root: None,
        }
    }

    /// 宽屏（200×40、两栏可见）下的最小 `TuiApp`。
    /// 通道接收端留在作用域内：切换会话会入队历史回放，没人收就会被当成「worker 已退出」。
    fn with_wide_app(f: impl FnOnce(&mut TuiApp<'_>)) {
        let client = test_client();
        let mut prefs = test_prefs();
        let (tx, rx) = mpsc::channel();
        let (job_tx, _job_rx) = mpsc::channel();
        let mut app = test_app(&client, &mut prefs, rx, tx, job_tx);
        app.last_size = (200, 40);
        app.st.sidebar_visible = true;
        f(&mut app);
    }

    fn click(app: &mut TuiApp<'_>, column: u16, row: u16) {
        app.on_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        });
    }

    fn scroll(app: &mut TuiApp<'_>, kind: MouseEventKind, column: u16, row: u16) {
        app.on_mouse(MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        });
    }

    #[test]
    fn title_row_is_not_an_item() {
        assert_eq!(item_index_at(0, 0, 5), None);
    }

    #[test]
    fn rows_map_to_items_from_window_start() {
        // 窗口裁到条目 3..6，可视行 1 → 条目 3。
        assert_eq!(item_index_at(1, 3, 6), Some(3));
        assert_eq!(item_index_at(3, 3, 6), Some(5));
    }

    #[test]
    fn rows_past_window_end_are_none() {
        assert_eq!(item_index_at(4, 3, 6), None, "列表末尾空白不选中");
    }

    #[test]
    fn empty_window_maps_to_nothing() {
        assert_eq!(item_index_at(1, 0, 0), None);
    }

    /// 点左栏第 2 条会话：选中并**直接切过去**（单击即切换），待确认删除随之作废。
    #[test]
    fn click_session_row_selects_and_switches() {
        with_wide_app(|app| {
            app.st
                .replace_sessions(vec![row("a", Some("c1")), row("b", Some("c2"))]);
            app.st.conversation_id = Some("c1".to_string());
            app.st.confirm_delete = Some("a".to_string());
            // 列内左留白 1 列；第 0 行是标题，故屏幕第 2 行 = 第 2 条会话。
            click(app, 2, 2);
            assert_eq!(app.st.selected, 1, "点第 2 行应选中第 2 条会话");
            assert_eq!(app.st.conversation_id.as_deref(), Some("c2"), "单击即切换");
            assert!(app.st.confirm_delete.is_none(), "点击作废待确认删除");
        });
    }

    /// 点标题行只切焦点：不选会话、不切换。
    #[test]
    fn click_title_row_only_takes_focus() {
        with_wide_app(|app| {
            app.st
                .replace_sessions(vec![row("a", Some("c1")), row("b", Some("c2"))]);
            app.st.conversation_id = Some("c1".to_string());
            click(app, 2, 0);
            assert!(matches!(app.st.focus, Focus::Sidebar), "点左栏应聚焦会话栏");
            assert_eq!(app.st.selected, 0, "标题行不改选中");
            assert_eq!(
                app.st.conversation_id.as_deref(),
                Some("c1"),
                "不该切换会话"
            );
        });
    }

    /// 点底部提示行（列内最后一行）同样只切焦点。
    #[test]
    fn click_hint_row_only_takes_focus() {
        with_wide_app(|app| {
            app.st
                .replace_sessions(vec![row("a", Some("c1")), row("b", Some("c2"))]);
            click(app, 2, 39); // 40 行终端：最后一行留给按键提示
            assert_eq!(app.st.selected, 0);
            assert!(app.st.conversation_id.is_none(), "提示行不该切换会话");
        });
    }

    /// 空列表的「（空）发一条消息后出现」占位行不能被当成会话 0 号。
    #[test]
    fn click_placeholder_row_of_empty_list_does_not_switch() {
        with_wide_app(|app| {
            app.st.replace_sessions(Vec::new());
            click(app, 2, 1);
            assert_eq!(app.st.selected, 0);
            assert!(
                app.st.conversation_id.is_none(),
                "占位行不该切换会话（否则会提示「该会话还没有 conversation_id」）"
            );
        });
    }

    /// 左栏滚轮**不**碰会话列表：选中与当前会话都不动（它滚的是 transcript）。
    #[test]
    fn left_column_scroll_does_not_touch_session_selection() {
        with_wide_app(|app| {
            app.st
                .replace_sessions(vec![row("a", Some("c1")), row("b", Some("c2"))]);
            app.st.conversation_id = Some("c1".to_string());
            scroll(app, MouseEventKind::ScrollDown, 2, 1);
            scroll(app, MouseEventKind::ScrollUp, 2, 1);
            assert_eq!(app.st.selected, 0, "滚轮不改会话选中");
            assert_eq!(
                app.st.conversation_id.as_deref(),
                Some("c1"),
                "滚轮不该切换会话"
            );
        });
    }

    /// 长列表 + 矮终端：命中必须跟着窗口偏移走（`sidebar_window` 的 `start`），
    /// 不能当成「可视第 1 行 = 条目 0」——这正是「点到的行与反色行错开一行」的来源。
    #[test]
    fn click_on_a_scrolled_list_maps_through_the_window_offset() {
        with_wide_app(|app| {
            let rows: Vec<_> = (0..20)
                .map(|i| row(&format!("s{i}"), Some(&format!("c{i}"))))
                .collect();
            app.st.replace_sessions(rows);
            app.st.selected = 15;
            app.st.conversation_id = Some("c15".to_string());
            // 6 行终端：左栏内容区 5 行 = 标题 + 4 条；选中 15 → 窗口条目 12..16。
            app.last_size = (200, 6);
            click(app, 2, 2); // 可视第 2 行 = 窗口内第 1 条 = 条目 13
            assert_eq!(app.st.selected, 13, "命中应加上窗口起点 start=12");
            assert_eq!(
                app.st.conversation_id.as_deref(),
                Some("c13"),
                "切到的应是条目 13，而不是第 2 条会话"
            );
        });
    }

    /// 窄屏没有左栏：左栏位置的点击什么都不做。
    #[test]
    fn narrow_terminal_ignores_left_column_clicks() {
        with_wide_app(|app| {
            app.st.sidebar_visible = false;
            app.last_size = (80, 24);
            app.st.replace_sessions(vec![row("a", Some("c1"))]);
            click(app, 2, 2);
            assert!(matches!(app.st.focus, Focus::Input), "窄屏没有左栏");
            assert!(app.st.conversation_id.is_none(), "窄屏不该切换会话");
        });
    }
}
