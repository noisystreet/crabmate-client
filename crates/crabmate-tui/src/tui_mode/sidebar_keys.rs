//! 会话栏（左栏）按键分发与删除会话（`Delete` / `Shift+Delete`）。
//!
//! 删除语义对齐 Desktop 的两级快捷键：`Delete` 先确认再删，`Shift+Delete` 直接删。
//! 确认不弹浮层——侧栏底部提示行本来就在，切成 `[y] 删除 [Esc] 取消` 即可，
//! 也不需要把键盘从正在操作的那一栏抢走。
//!
//! 按键 → 动作的映射抽成了纯函数 [`sidebar_action`]，相邻行选择抽成了
//! [`adjacent_session`]：这两处是「y 到底删不删」「删完落到哪」这类最容易写错、
//! 又最难在真实终端里复现的地方，单测比手工点更快也更稳。
//!
//! 拆出本模块还有一个工程原因：`mod.rs` 有 920 行文件上限（`scripts/fn-nloc-ratchet.sh`）。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::LaneApp;
use super::state::{Focus, LineKind};
use super::worker::WorkerJob;
use crate::serve::{SessionIndexOp, SessionListRow, conversation_id_for_resume};

/// 会话栏按键归一化后的动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SidebarAction {
    /// 确认态下的 y / Enter：执行删除。
    ConfirmDelete,
    /// 确认态下的 Esc / n：取消删除。
    CancelDelete,
    /// Tab / BackTab / Esc：回到输入框。
    LeaveToInput,
    Prev,
    Next,
    /// Enter：切到选中会话。
    Use,
    /// `Shift` + `Delete`：免确认删除。
    DeleteNow,
    /// `Delete`：先进入确认态。
    DeleteAsk,
    New,
    Refresh,
    /// `w`：聚焦右栏工作区目录树。
    ToWorkspace,
    /// 未绑定的键。
    Ignore,
}

/// 按键 → 动作。`confirm_pending` 为真时 y / Enter / Esc / n 有特殊含义。
fn sidebar_action(confirm_pending: bool, key: &KeyEvent) -> SidebarAction {
    if confirm_pending {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                return SidebarAction::ConfirmDelete;
            }
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                return SidebarAction::CancelDelete;
            }
            _ => {}
        }
    }
    match key.code {
        KeyCode::Tab | KeyCode::BackTab | KeyCode::Esc => SidebarAction::LeaveToInput,
        KeyCode::Up => SidebarAction::Prev,
        KeyCode::Down => SidebarAction::Next,
        KeyCode::Enter => SidebarAction::Use,
        KeyCode::Delete if key.modifiers.contains(KeyModifiers::SHIFT) => SidebarAction::DeleteNow,
        KeyCode::Delete => SidebarAction::DeleteAsk,
        KeyCode::Char('n') => SidebarAction::New,
        KeyCode::Char('r') => SidebarAction::Refresh,
        KeyCode::Char('w') => SidebarAction::ToWorkspace,
        _ => SidebarAction::Ignore,
    }
}

/// 删除后要落到的相邻会话：优先下一行，其次上一行（与焦点自然下落一致）。
fn adjacent_session(sessions: &[SessionListRow], removed_idx: usize) -> Option<&SessionListRow> {
    sessions
        .get(removed_idx + 1)
        .or_else(|| removed_idx.checked_sub(1).and_then(|i| sessions.get(i)))
}

impl LaneApp<'_> {
    /// Tab：从输入框跳到左栏会话，光标落在当前正在用的那一行（没有就第一行）。
    ///
    /// 从 `on_input_key` 下沉：那个 match 已贴着 lizard 的 CCN 门禁（≤ 10），
    /// 这里的内联 if 与 `&&` 正好是压死它的那一根。
    pub(super) fn focus_sidebar_on_current(&mut self) {
        if !self.st.sidebar_visible || self.st.sessions.is_empty() {
            return;
        }
        self.st.selected = self
            .st
            .sessions
            .iter()
            .position(|r| self.st.row_in_use(r))
            .unwrap_or(0);
        self.st.focus = Focus::Sidebar;
    }

    /// 会话栏按键分发（`Focus::Sidebar` 时所有按键都进这里）。
    pub(super) fn on_sidebar_key(&mut self, key: KeyEvent) {
        let pending = self.st.confirm_delete.clone();
        let action = sidebar_action(pending.is_some(), &key);
        match action {
            SidebarAction::ConfirmDelete => {
                if let Some(id) = pending {
                    self.delete_session_row(&id);
                }
                return;
            }
            SidebarAction::CancelDelete => {
                self.st.confirm_delete = None;
                self.st.push_line(LineKind::System, "已取消删除会话");
                return;
            }
            _ => {}
        }
        // 确认态下按了别的键：先作废待确认，再照常处理——用户改主意时不必先按一次
        // 「放弃」，也不会出现「确认的是这一行、删掉的是另一行」。
        if pending.is_some() {
            self.st.confirm_delete = None;
        }
        match action {
            SidebarAction::LeaveToInput => self.st.focus = Focus::Input,
            SidebarAction::Prev => self.st.move_selection(true),
            SidebarAction::Next => self.st.move_selection(false),
            SidebarAction::Use => self.use_selected(),
            SidebarAction::DeleteNow => self.delete_selected_session(true),
            SidebarAction::DeleteAsk => self.delete_selected_session(false),
            SidebarAction::New => self.new_session(),
            SidebarAction::Refresh => self.refresh_sessions(),
            // 焦点换到右栏：待确认删除同样过期。
            SidebarAction::ToWorkspace => {
                self.st.confirm_delete = None;
                self.focus_workspace();
            }
            SidebarAction::Ignore | SidebarAction::ConfirmDelete | SidebarAction::CancelDelete => {}
        }
    }

    /// 左栏删除入口：`immediate`（`Shift+Delete`）跳过确认，否则先记下待确认行。
    ///
    /// 确认态只记住行 id，不锁光标：期间按 ↑↓ 会被当作「取消」，然后照常移动。
    fn delete_selected_session(&mut self, immediate: bool) {
        let Some(row) = self.st.sessions.get(self.st.selected) else {
            self.st.push_line(LineKind::System, "左栏没有可删除的会话");
            return;
        };
        let local_id = row.id.clone();
        if immediate {
            self.delete_session_row(&local_id);
        } else {
            self.st.confirm_delete = Some(local_id);
        }
    }

    /// 删除一条会话行：先改服务端索引（一次 `GET → 删 → PUT`，`PUT` 成功后顺带
    /// 删该行的服务端对话），再复位本地对话态。
    ///
    /// 守卫与 Desktop 对齐：正在生成的那条会话不能删（SSE 收尾还要把回复写回它，
    /// 删了这段回复就静默丢了）。
    fn delete_session_row(&mut self, local_id: &str) {
        self.st.confirm_delete = None;
        let Some(idx) = self.st.sessions.iter().position(|r| r.id == local_id) else {
            // 确认期间被别的端删掉了：不能「按了确认却什么都没发生」。
            self.st.push_line(
                LineKind::System,
                "该会话已不存在（可能已被其他端删除），正在刷新左栏",
            );
            self.refresh_sessions();
            return;
        };
        let row = self.st.sessions[idx].clone();
        let is_current =
            self.st.row_in_use(&row) || self.index.local_new_session.as_deref() == Some(local_id);
        if self.st.running && is_current {
            self.st
                .push_line(LineKind::System, "会话正在生成中：先停止（Ctrl+C）再删除");
            return;
        }
        // 删完当前会话之后要落到哪：相邻行如果有 `conversation_id`，切过去接着聊；
        // 没有（空白会话）就开新会话。删掉当前会话却什么都不做是不行的——之后每条
        // 消息都要落到一条索引行上，而 `bind_new_session` 只认 `/conv new` 建的行
        // （`index.local_new_session`），凭空产生的 conversation 在左栏和其它端都不可见。
        let fallback = is_current
            .then(|| adjacent_session(&self.st.sessions, idx).cloned())
            .flatten();
        // `/conv new` 建的那行被删掉：待绑定记录一并作废，否则后续每轮都会往已删行上绑。
        if self.index.local_new_session.as_deref() == Some(local_id) {
            self.index.local_new_session = None;
            self.index.pending_bind_conversation = None;
            self.index.pending_session_title = None;
            self.index.pending_select_session = None;
        }
        if is_current {
            self.st.conversation_id = None;
            self.st.history_pending = None;
            self.st.reset_transcript();
        }
        // active 落点：与 `fallback` 同一行。服务端只在被删行确实是 active 时才用它，
        // 删非 active 行不会顺手改 active。
        let next_active = fallback.as_ref().map(|r| r.id.clone());
        let title = match row.title.trim() {
            "" => "(untitled)".to_string(),
            t => t.to_string(),
        };
        let op = SessionIndexOp::Delete {
            local_id: local_id.to_string(),
            next_active,
            server_conversation_id: conversation_id_for_resume(&row).map(str::to_string),
        };
        if self.job_tx.send(WorkerJob::SessionIndex(op)).is_err() {
            self.st
                .push_line(LineKind::System, "删除会话失败：worker 已退出");
            return;
        }
        self.st
            .push_line(LineKind::System, &format!("正在删除会话「{title}」…"));
        // 删除先入队，落点后入队：worker 串行执行，索引写入不会和「开新会话」抢顺序。
        if is_current {
            self.land_after_delete(fallback);
        }
    }

    /// 删掉当前会话后的落地：切到相邻会话，或开一条新会话顶上。
    fn land_after_delete(&mut self, fallback: Option<SessionListRow>) {
        let resumable = fallback.as_ref().and_then(|row| {
            conversation_id_for_resume(row).map(|cid| (cid.to_string(), row.title.clone()))
        });
        if let Some((cid, title)) = resumable {
            self.switch_to_conv(cid, Some(&title));
        } else {
            // 相邻行还没绑定 `conversation_id`（或干脆没有别的行）：走「新会话」，
            // 它会复用已有的空白行或新建一条，并把 `local_new_session` 立起来。
            self.new_session();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn shift(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::SHIFT)
    }

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

    #[test]
    fn confirm_state_maps_y_enter_to_delete_and_esc_n_to_cancel() {
        for k in [
            key(KeyCode::Char('y')),
            key(KeyCode::Char('Y')),
            key(KeyCode::Enter),
        ] {
            assert_eq!(
                sidebar_action(true, &k),
                SidebarAction::ConfirmDelete,
                "{:?} 应当确认删除",
                k.code
            );
        }
        for k in [
            key(KeyCode::Esc),
            key(KeyCode::Char('n')),
            key(KeyCode::Char('N')),
        ] {
            assert_eq!(
                sidebar_action(true, &k),
                SidebarAction::CancelDelete,
                "{:?} 应当取消删除",
                k.code
            );
        }
    }

    #[test]
    fn confirm_state_lets_other_keys_through() {
        // 方向键 / n 以外的键在确认态下先作废确认，然后按普通含义处理。
        assert_eq!(sidebar_action(true, &key(KeyCode::Up)), SidebarAction::Prev);
        assert_eq!(
            sidebar_action(true, &key(KeyCode::Delete)),
            SidebarAction::DeleteAsk
        );
        assert_eq!(
            sidebar_action(true, &key(KeyCode::Char('r'))),
            SidebarAction::Refresh
        );
    }

    #[test]
    fn delete_keys_split_by_shift() {
        assert_eq!(
            sidebar_action(false, &key(KeyCode::Delete)),
            SidebarAction::DeleteAsk
        );
        assert_eq!(
            sidebar_action(false, &shift(KeyCode::Delete)),
            SidebarAction::DeleteNow
        );
    }

    #[test]
    fn plain_keys_keep_their_meanings() {
        assert_eq!(
            sidebar_action(false, &key(KeyCode::Tab)),
            SidebarAction::LeaveToInput
        );
        assert_eq!(
            sidebar_action(false, &key(KeyCode::Esc)),
            SidebarAction::LeaveToInput
        );
        assert_eq!(
            sidebar_action(false, &key(KeyCode::Enter)),
            SidebarAction::Use
        );
        assert_eq!(
            sidebar_action(false, &key(KeyCode::Char('n'))),
            SidebarAction::New
        );
        assert_eq!(
            sidebar_action(false, &key(KeyCode::Char('w'))),
            SidebarAction::ToWorkspace
        );
        assert_eq!(
            sidebar_action(false, &key(KeyCode::Char('z'))),
            SidebarAction::Ignore
        );
    }

    #[test]
    fn adjacent_prefers_the_row_below_then_the_one_above() {
        let rows = [row("a", Some("c1")), row("b", None), row("c", Some("c3"))];
        assert_eq!(adjacent_session(&rows, 0).map(|r| r.id.as_str()), Some("b"));
        assert_eq!(adjacent_session(&rows, 1).map(|r| r.id.as_str()), Some("c"));
        // 删最后一行：往上找。
        assert_eq!(adjacent_session(&rows, 2).map(|r| r.id.as_str()), Some("b"));
        // 只有一行：没有落点。
        assert_eq!(adjacent_session(&rows[..1], 0).map(|r| r.id.as_str()), None);
    }

    #[test]
    fn adjacent_carries_the_binding_for_resume() {
        let rows = [row("a", Some("c1")), row("b", Some("c2"))];
        let picked = adjacent_session(&rows, 0).expect("下一行");
        assert_eq!(conversation_id_for_resume(picked), Some("c2"));
        // 空白行（未绑定）：调用方据此走「开新会话」而不是「切过去」。
        let blank = [row("a", Some("c1")), row("b", None)];
        let picked = adjacent_session(&blank, 0).expect("下一行");
        assert_eq!(conversation_id_for_resume(picked), None);
    }
}
