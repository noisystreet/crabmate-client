//! 鼠标事件：右栏工作区目录树的点击与滚轮，以及聊天区滚轮滚动。
//!
//! 目录树只在右栏：左键点击右栏外的区域仅用于把焦点收回输入框；滚轮在右栏内驱动树光标，
//! 在右栏外滚动 transcript（捕获鼠标后终端不再把滚轮翻成 ↑/↓，必须自己接）。
//! 所有状态变更都复用键盘已有的 `ws_*` 状态机，避免出现「鼠标能点出、键盘点不出」
//! 的状态（占位行不可选中、收起父目录后光标归位等不变量因此自动一致）。

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use super::TuiApp;
use super::layout::{contains, panes};
use super::state::{Focus, LineKind};
use super::workspace_tree::WsActivate;
use super::ws_sidebar::{pane_list_area, sidebar_window, workspace_content};

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
        // 窄屏侧栏隐藏时右栏矩形不存在，鼠标完全不动状态。
        let on_right = p.right.is_some_and(|r| contains(r, column, row));
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
        let list = pane_list_area(right);
        if !contains(list, column, row) {
            return; // 点在标题行或底部提示行：只切焦点
        }
        // 用渲染同一份内容算行数，避免再维护一遍「标题 + 占位行」的行数口径。
        let content_width = right.width.saturating_sub(2) as usize;
        let (rows, cursor, _hint) = workspace_content(&self.st, content_width);
        let rel_row = (row - list.y) as usize;
        let (start, end) = sidebar_window(rows.len(), list.height as usize, cursor);
        let Some(idx) = item_index_at(rel_row, start, end) else {
            return; // 标题行或列表末尾的空白
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
}

/// 可视行（第 0 行是固定标题）→ 条目下标；越界或落在标题行返回 `None`。
fn item_index_at(rel_row: usize, start: usize, end: usize) -> Option<usize> {
    let rel = rel_row.checked_sub(1)?;
    let idx = start + rel;
    (idx < end).then_some(idx)
}

#[cfg(test)]
mod tests {
    use super::item_index_at;

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
}
