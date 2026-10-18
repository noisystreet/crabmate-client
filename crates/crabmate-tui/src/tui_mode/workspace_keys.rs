//! 工作区目录树 / 项目池子视图的按键处理（从 `mod.rs` 拆出，控制单文件行数）。

use crossterm::event::{KeyCode, KeyEvent};

use super::TuiApp;
use super::state::{Focus, LineKind};
use super::workspace_tree::WsActivate;

impl TuiApp<'_> {
    /// 工作区目录树视图按键：浏览目录层级（Enter 展开/插入路径，→ 只展开，← 收起或回父目录）。
    pub(super) fn on_workspace_key(&mut self, key: KeyEvent) {
        if self.st.ws_pick.is_some() {
            self.on_workspace_pick_key(key);
            return;
        }
        match key.code {
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Esc => self.st.focus = Focus::Input,
            KeyCode::Up => self.st.ws_move(true),
            KeyCode::Down => self.st.ws_move(false),
            KeyCode::Home => self.st.ws_move_to_edge(false),
            KeyCode::End => self.st.ws_move_to_edge(true),
            KeyCode::PageUp => {
                let page = self.st.page_rows;
                self.st.ws_page(false, page);
            }
            KeyCode::PageDown => {
                let page = self.st.page_rows;
                self.st.ws_page(true, page);
            }
            KeyCode::Enter => match self.st.ws_activate() {
                Some(WsActivate::Dir { rel, fetch }) => {
                    if fetch {
                        self.fetch_ws_dir(rel);
                    }
                }
                // 文件：插入相对路径并回输入框，省得手敲路径。
                Some(WsActivate::File(rel)) => {
                    self.st.insert_workspace_path(&rel);
                    self.st.focus = Focus::Input;
                }
                None => {}
            },
            KeyCode::Right => {
                if let Some(rel) = self.st.ws_expand_dir() {
                    self.fetch_ws_dir(rel);
                }
            }
            KeyCode::Left => self.st.ws_left(),
            KeyCode::Char('r') => {
                // 根请求在途时忽略连按，避免多个响应各自重置一次树。
                if self.st.ws_begin_refresh() {
                    self.st.push_line(LineKind::System, "正在刷新工作区目录…");
                    self.refresh_workspace(true);
                }
            }
            // w：回到左栏会话列表（w 两侧对称切换）。
            KeyCode::Char('w') => self.st.focus = Focus::Sidebar,
            // p：项目池「选择/切换工作区」（未设置=首次选择；已设置=切到另一项目）。
            KeyCode::Char('p') if self.st.ws_open_pick() => {
                self.fetch_ws_projects();
            }
            _ => {}
        }
    }

    /// 项目池「选择/切换工作区」子视图按键。
    pub(super) fn on_workspace_pick_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Tab | KeyCode::BackTab => self.st.ws_close_pick(),
            KeyCode::Up => self.st.ws_pick_move(true),
            KeyCode::Down => self.st.ws_pick_move(false),
            KeyCode::Char('r') => {
                if self.st.ws_pick_refresh() {
                    self.fetch_ws_projects();
                }
            }
            KeyCode::Enter | KeyCode::Right => {
                if let Some(name) = self.st.ws_pick_begin_switch() {
                    self.st
                        .push_line(LineKind::System, &format!("正在切换到项目「{name}」…"));
                    self.switch_ws_project(name);
                }
            }
            // w：关闭选择并回到左栏会话。
            KeyCode::Char('w') => {
                self.st.ws_close_pick();
                self.st.focus = Focus::Sidebar;
            }
            _ => {}
        }
    }
}
