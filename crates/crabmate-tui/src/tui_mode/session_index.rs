//! `/conv new` 与会话列表登记：把 TUI 新建的会话写回 serve user-data 索引
//! （`/user-data/workspaces/current/sessions`，与 Web / Desktop 同一份，跨端可见）。
//!
//! 拆出本模块以压住 `mod.rs` 行数门禁（≤ 920 行）；索引的纯逻辑与 IO 见
//! [`crate::serve::sessions`]。

use super::TuiApp;
use super::state::{Focus, LineKind};
use super::worker::WorkerJob;
use crate::serve::{
    DEFAULT_SESSION_TITLE, SessionIndexApplied, SessionIndexOp, SessionListRow,
    conversation_id_for_resume, is_default_session_title, title_from_user_prompt,
};

/// `/conv new` 该怎么做（纯判定，见 [`decide_new_session`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NewSessionDecision {
    /// 上一次新建还在写（结果未回）：忽略本次，避免连按插出多行空会话。
    IgnoreInFlight,
    /// 复用上一轮新建、仍未绑定 `conversation_id` 的空行。
    ReuseBlank,
    /// 插入新的空白会话行。
    InsertBlank,
}

/// 决定 `/conv new` 该怎么做。
///
/// `sessions` 是左栏当前快照。只有「快照非空且里面没有待绑定行」才认定该行已被
/// 别的端删除并重新插入；快照为空（还没加载 / 刷新失败 / 真的没会话）时**不做**
/// 这个推断，宁可复用——重复插出的空会话会写进 Web 端侧栏，代价更高。
pub(super) fn decide_new_session(
    pending: bool,
    local_new_session: Option<&str>,
    sessions: &[SessionListRow],
) -> NewSessionDecision {
    if pending {
        return NewSessionDecision::IgnoreInFlight;
    }
    match local_new_session {
        Some(id) if sessions.is_empty() || sessions.iter().any(|r| r.id == id) => {
            NewSessionDecision::ReuseBlank
        }
        _ => NewSessionDecision::InsertBlank,
    }
}

/// 左栏选中的是否正是 `/conv new` 建的那条空行（它已经是当前会话）。
///
/// 「使用」它没有意义：`conversation_id` 会指向一条尚无服务端记录的会话，而且
/// `switch_to_conv` 切走前会把这条空行**回收**掉（见 `leave_current_for_switch`），
/// 等于把刚建的会话删掉、再切到一个不存在的目标上。
pub(super) fn selection_is_pending_blank(
    local_new_session: Option<&str>,
    sessions: &[SessionListRow],
    selected: usize,
) -> bool {
    let Some(id) = local_new_session else {
        return false;
    };
    sessions.get(selected).is_some_and(|r| r.id == id)
}

/// 「行 id 已就绪就立即做、`NewBlank` 还在写就先攒着」的判定（绑定 cid 与起名共用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DeferredAction<'a> {
    /// 行 id 已就绪：立即对这条行执行。
    Now(&'a str),
    /// 行 id 还没到（`NewBlank` 在途）：先缓存，等行 id 到手再执行。
    Defer,
    /// 没有待登记的会话行（未用 `/conv new`、或已绑定过）：什么都不做。
    Skip,
}

/// 见 [`DeferredAction`]。
pub(super) fn plan_deferred_action(
    local_new_session: Option<&str>,
    new_session_pending: bool,
) -> DeferredAction<'_> {
    match local_new_session {
        Some(id) => DeferredAction::Now(id),
        None if new_session_pending => DeferredAction::Defer,
        None => DeferredAction::Skip,
    }
}

/// `NewBlank` 结果回来后的动作（见 [`plan_new_blank_outcome`]）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum NewBlankOutcome {
    /// 写入期间用户已切走、且没有待绑定的轮次：这行没人认领，回收它。
    Reclaim {
        local_id: String,
        /// active 的新落点（`None` 时写入层会拒绝删除 active 行）。
        next_active: Option<String>,
    },
    /// 认领该行，并把在途攒下的标题 / `conversation_id` 一并补上。
    Adopt {
        local_id: String,
        reused_blank: bool,
        title: Option<String>,
        bind_conversation_id: Option<String>,
    },
}

/// 判定 `NewBlank` 回来时该认领还是该回收。
///
/// 回收条件：**已经不在新会话上**（`conversation_id` 指向别的会话）**且没有正在等待绑定
/// 的轮次**。后者是关键——首轮结束拿到 cid、但行 id 还没到的窗口里 `conversation_id`
/// 同样是 `Some`，此时绝不能回收。
pub(super) fn plan_new_blank_outcome(
    applied: &SessionIndexApplied,
    has_conversation: bool,
    pending_bind: Option<&str>,
    pending_title: Option<&str>,
    next_active: Option<String>,
) -> NewBlankOutcome {
    if has_conversation && pending_bind.is_none() {
        return NewBlankOutcome::Reclaim {
            local_id: applied.local_id.clone(),
            next_active,
        };
    }
    NewBlankOutcome::Adopt {
        local_id: applied.local_id.clone(),
        reused_blank: applied.reused_blank,
        title: pending_title.map(str::to_string),
        bind_conversation_id: pending_bind.map(str::to_string),
    }
}

/// `/conv new` 与历史回放相关的 UI 侧状态。
///
/// 前五个字段只在「新建 → 起名 → 绑定 → 切走回收」这条线上用，集中成一个结构体，
/// 好让 `mod.rs` 只留一个字段（该文件有 920 行门禁）；`history_auto_replayed` 是
/// 启动时的一次性标记，放这里同样是为了不给 `mod.rs` 加字段。
#[derive(Debug, Default)]
pub(super) struct UiIndexState {
    /// `/conv new` 在服务端索引里新建（或复用）、尚未绑定 `conversation_id` 的本地会话行 id。
    pub(super) local_new_session: Option<String>,
    /// `NewBlank` 写入在途（结果未回）：挡住连按 `/conv new` 插出多行空会话。
    pub(super) new_session_pending: bool,
    /// 刷新回来后要按 id 选中的会话行（新建行落在服务端数组的哪一位不固定）。
    pub(super) pending_select_session: Option<String>,
    /// `NewBlank` 在途期间结束的那一轮拿到的 `conversation_id`：等行 id 到手再绑定。
    pub(super) pending_bind_conversation: Option<String>,
    /// `NewBlank` 在途期间发出的首条消息算出的标题：等行 id 到手再写。
    pub(super) pending_session_title: Option<String>,
    /// 启动时的「回放上次会话」是否已尝试过（只做一次，避免每次刷新左栏都冲掉 transcript）。
    pub(super) history_auto_replayed: bool,
}

impl TuiApp<'_> {
    /// `/conv new`：清空当前会话，并把新会话登记进服务端索引（左栏立即能看到，Web 端同步可见）。
    pub(super) fn new_session(&mut self) {
        if self.st.running {
            self.st
                .push_line(LineKind::System, "回合进行中：结束后再新建会话");
            return;
        }
        match decide_new_session(
            self.index.new_session_pending,
            self.index.local_new_session.as_deref(),
            &self.st.sessions,
        ) {
            NewSessionDecision::IgnoreInFlight => {
                self.st
                    .push_line(LineKind::System, "上一次新建还在写入：稍候再试");
                return;
            }
            NewSessionDecision::ReuseBlank => {
                self.reset_for_new_chat();
                self.st
                    .push_line(LineKind::System, "已开始新会话（复用列表里的空白会话行）");
                self.refresh_sessions();
                return;
            }
            NewSessionDecision::InsertBlank => {}
        }
        self.reset_for_new_chat();
        // 待绑定行若已被别的端删掉（判定为 InsertBlank 的那条路径），别再拿着它：
        // 否则每轮 bind 都会对同一个不存在的 id 失败；上一轮攒下的待绑定 cid 也一并作废。
        self.index.local_new_session = None;
        self.index.pending_bind_conversation = None;
        self.index.pending_session_title = None;
        let op = SessionIndexOp::NewBlank {
            title: DEFAULT_SESSION_TITLE.to_string(),
        };
        if self.job_tx.send(WorkerJob::SessionIndex(op)).is_err() {
            self.st
                .push_line(LineKind::System, "新建会话失败：worker 已退出");
            return;
        }
        self.index.new_session_pending = true;
        self.st
            .push_line(LineKind::System, "已开始新会话（正在写入会话列表…）");
    }

    /// 新建 / 切换共用的「空对话」复位：清 `conversation_id`、清在途回放、清 transcript、聚焦输入框。
    fn reset_for_new_chat(&mut self) {
        self.st.conversation_id = None;
        self.st.history_pending = None;
        self.st.reset_transcript();
        self.st.focus = Focus::Input;
    }

    /// 首轮结束拿到 `conversation_id` 后，把它补进 `/conv new` 建的那一行。
    ///
    /// 行 id 还没到（`NewBlank` 在途）时先记下这一轮的 `conversation_id`，等行 id 到手
    /// 再补发绑定：与 Web 一致，`conversation_id` 必须落到「这一轮所属的会话行」上。
    /// 没有待绑定的行（未用 `/conv new`、或已绑定过）时什么都不做。
    pub(super) fn bind_new_session(&mut self, conversation_id: &str) {
        let local_id = self.index.local_new_session.clone();
        match plan_deferred_action(local_id.as_deref(), self.index.new_session_pending) {
            DeferredAction::Now(local_id) => {
                let op = SessionIndexOp::BindServer {
                    local_id: local_id.to_string(),
                    conversation_id: conversation_id.to_string(),
                };
                if self.job_tx.send(WorkerJob::SessionIndex(op)).is_err() {
                    self.st.push_line(
                        LineKind::System,
                        "会话绑定失败：worker 已退出（该会话未登记到列表）",
                    );
                }
            }
            DeferredAction::Defer => {
                self.index.pending_bind_conversation = Some(conversation_id.to_string());
            }
            DeferredAction::Skip => {}
        }
    }

    /// 首条用户消息发送时给新建行起名（对齐 Web：仅当标题仍是默认值时才改）。
    ///
    /// 与 Web 的 `is_first_user_turn && is_default_session_title(title)` 同义：只对
    /// `/conv new` 建的那一行、且它还没被起名时动手；标题目录里已有真名就跳过。
    /// 全空白消息不产生标题，保持默认值。
    pub(super) fn retitle_new_session_for_first_turn(&mut self, user_text: &str) {
        let title = title_from_user_prompt(user_text);
        if is_default_session_title(&title) {
            return;
        }
        let local_id = self.index.local_new_session.clone();
        match plan_deferred_action(local_id.as_deref(), self.index.new_session_pending) {
            // 行 id 还没到：先记下标题，等行 id 到手再写。
            DeferredAction::Defer => {
                self.index.pending_session_title = Some(title);
            }
            DeferredAction::Skip => {}
            DeferredAction::Now(local_id) => {
                // 列表里已有这行就按它的真实标题判断（别端起过名就跳过）；还没刷新到
                // 就当作默认标题——那正是 TUI 刚建、还没人起名的空白行。
                let already_named = self
                    .st
                    .sessions
                    .iter()
                    .find(|r| r.id.as_str() == local_id)
                    .is_some_and(|r| !is_default_session_title(&r.title));
                if already_named {
                    return;
                }
                let op = SessionIndexOp::SetTitle {
                    local_id: local_id.to_string(),
                    title,
                };
                let _ = self.job_tx.send(WorkerJob::SessionIndex(op));
            }
        }
    }

    /// 索引写入结果：成功即刷新列表，新行 / `>` 标记都从服务端回读，
    /// 不在本地拼一份可能与服务端不一致的列表。
    pub(super) fn on_session_index(
        &mut self,
        result: Result<SessionIndexApplied, String>,
        op: SessionIndexOp,
    ) {
        // 在途标记只属于 `NewBlank`；成败都要清，否则 `/conv new` 会被永久挡住。
        if matches!(op, SessionIndexOp::NewBlank { .. }) {
            self.index.new_session_pending = false;
        }
        let applied = match result {
            Ok(applied) => applied,
            Err(e) => {
                let what = match op {
                    SessionIndexOp::NewBlank { .. } => "新建会话未写入会话列表",
                    SessionIndexOp::BindServer { .. } => "会话未绑定到列表行",
                    SessionIndexOp::RemoveBlank { .. } => "空白会话未能回收",
                    SessionIndexOp::SetTitle { .. } => "会话标题未写入会话列表",
                    SessionIndexOp::FlushExit { .. } => "退出前的会话落盘未完成",
                    SessionIndexOp::Delete { .. } => "删除会话未写入会话列表",
                };
                // 绑定失败最常见的原因是那条空行已被别的端删掉：丢弃待绑定记录，
                // 否则后续每轮都会重试同一个失败绑定，`/conv new` 还会一直走复用分支。
                if matches!(op, SessionIndexOp::BindServer { .. }) {
                    self.index.local_new_session = None;
                }
                self.st.push_line(LineKind::System, &format!("{what}：{e}"));
                return;
            }
        };
        match op {
            SessionIndexOp::NewBlank { .. } => {
                let plan = plan_new_blank_outcome(
                    &applied,
                    self.st.conversation_id.is_some(),
                    self.index.pending_bind_conversation.as_deref(),
                    self.index.pending_session_title.as_deref(),
                    self.current_row_id(),
                );
                match plan {
                    // 写入期间用户已经切走（`conversation_id` 已指向别的会话）：这行没人
                    // 认领，立刻回收，别在侧栏留一条孤儿空会话；在途攒下的标题也一并作废。
                    NewBlankOutcome::Reclaim {
                        local_id,
                        next_active,
                    } => {
                        self.index.pending_session_title = None;
                        let op = SessionIndexOp::RemoveBlank {
                            local_id,
                            next_active,
                        };
                        let _ = self.job_tx.send(WorkerJob::SessionIndex(op));
                        return;
                    }
                    NewBlankOutcome::Adopt {
                        local_id,
                        reused_blank,
                        title,
                        bind_conversation_id,
                    } => {
                        self.index.local_new_session = Some(local_id.clone());
                        // 光标要落在新行上，但不按下标猜：索引顺序由服务端存储决定，
                        // 等刷新回来的列表里按 id 定位（见 `apply_pending_select`）。
                        self.index.pending_select_session = Some(local_id.clone());
                        // 在途攒下的东西已由 `plan_new_blank_outcome` 取出，这里清空免得重放。
                        self.index.pending_session_title = None;
                        self.index.pending_bind_conversation = None;
                        let text = if reused_blank {
                            "已复用列表里的空白会话作新会话（左侧会话栏）"
                        } else {
                            "已登记新会话（左侧会话栏）"
                        };
                        self.st.push_line(LineKind::System, text);
                        // 行 id 等到了：把在途那一轮攒下的标题 / `conversation_id` 都补上。
                        if let Some(title) = title {
                            let op = SessionIndexOp::SetTitle { local_id, title };
                            let _ = self.job_tx.send(WorkerJob::SessionIndex(op));
                        }
                        if let Some(cid) = bind_conversation_id {
                            self.bind_new_session(&cid);
                        }
                    }
                }
            }
            SessionIndexOp::BindServer { .. } => {
                self.index.local_new_session = None;
                self.st
                    .push_line(LineKind::System, "已把本轮对话绑定到左侧会话行");
            }
            // 回收成功不必刷系统行：少一条空会话是预期结果，不是需要用户知晓的事件。
            SessionIndexOp::RemoveBlank { .. } => {}
            // 起名同理：左栏刷新后自然显示新标题，不必额外打一行。
            SessionIndexOp::SetTitle { .. } => {}
            // 退出落盘发生在退出路径上，屏幕上已经没有可看的地方，静默即可。
            SessionIndexOp::FlushExit { .. } => {}
            // 删除：左栏刷新后那行自然消失，不必额外打一行（发起时已提示「正在删除…」）。
            SessionIndexOp::Delete { .. } => {}
        }
        self.refresh_sessions();
    }

    /// 切走当前会话：离开的是 `/conv new` 建的空行就回收（对齐 Web
    /// `reclaim_disposable_blank_session`），并把服务端 active 指向目标会话行。
    ///
    /// `next_conversation_id` 是要切过去的会话：它在索引里的行 id 既是 active 的新落点，
    /// 也决定被删行能不能删——找不到落点就不删，免得服务端 active 指向已删除的行。
    pub(super) fn leave_current_for_switch(&mut self, next_conversation_id: &str) {
        self.index.pending_select_session = None;
        let Some(leaving) = self.index.local_new_session.take() else {
            return;
        };
        let op = SessionIndexOp::RemoveBlank {
            local_id: leaving,
            next_active: self.row_id_for_conversation(next_conversation_id),
        };
        // 回收是尽力而为：worker 已退出时静默放弃，不打断切换。
        let _ = self.job_tx.send(WorkerJob::SessionIndex(op));
    }

    /// 某个 `conversation_id` 在会话索引里的本地行 id（切走回收与退出落盘都要查它）。
    pub(super) fn row_id_for_conversation(&self, conversation_id: &str) -> Option<String> {
        self.st
            .sessions
            .iter()
            .find(|r| conversation_id_for_resume(r) == Some(conversation_id))
            .map(|r| r.id.clone())
    }

    /// 当前会话对应的本地行 id（服务端 active 的新落点）。
    fn current_row_id(&self) -> Option<String> {
        let cid = self.st.conversation_id.as_deref()?;
        self.row_id_for_conversation(cid)
    }

    /// 刷新回来的列表里按 id 定位待选中行。
    ///
    /// 不在结果回调里直接设下标：`selected = 0` 既假设了新行在首位，又会在刷新到达
    /// 前的窗口里把光标留在别的会话上（此时按 Enter 会切走并丢掉待绑定行）。
    pub(super) fn apply_pending_select(&mut self) {
        let Some(id) = self.index.pending_select_session.clone() else {
            return;
        };
        if let Some(idx) = self.st.sessions.iter().position(|r| r.id == id) {
            self.st.selected = idx;
        }
        self.index.pending_select_session = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str) -> SessionListRow {
        SessionListRow {
            id: id.to_string(),
            title: String::new(),
            updated_at: 0,
            pinned: false,
            starred: false,
            server_conversation_id: None,
            server_revision: None,
            workspace_root: None,
        }
    }

    #[test]
    fn fresh_new_session_inserts_blank_row() {
        assert_eq!(
            decide_new_session(false, None, &[row("s1")]),
            NewSessionDecision::InsertBlank
        );
        assert_eq!(
            decide_new_session(false, None, &[]),
            NewSessionDecision::InsertBlank
        );
    }

    #[test]
    fn in_flight_new_session_is_ignored() {
        // 连按两次 `/conv new`：第二次不能插行（否则列表里多一条没人认领的空会话）。
        assert_eq!(
            decide_new_session(true, None, &[row("s1")]),
            NewSessionDecision::IgnoreInFlight
        );
        assert_eq!(
            decide_new_session(true, Some("s_new"), &[row("s_new")]),
            NewSessionDecision::IgnoreInFlight
        );
    }

    #[test]
    fn unsent_blank_row_is_reused() {
        assert_eq!(
            decide_new_session(false, Some("s_new"), &[row("s_new"), row("s1")]),
            NewSessionDecision::ReuseBlank
        );
    }

    #[test]
    fn deleted_blank_row_falls_back_to_insert() {
        // 那条空行被别的端删掉了：快照非空且不含它 → 重新插入，不能卡在复用分支。
        assert_eq!(
            decide_new_session(false, Some("gone"), &[row("s1")]),
            NewSessionDecision::InsertBlank
        );
    }

    #[test]
    fn empty_snapshot_still_reuses() {
        // 列表还没拉到 / 刷新失败：没有「被删除」的证据，宁可复用也不重复插行。
        assert_eq!(
            decide_new_session(false, Some("s_new"), &[]),
            NewSessionDecision::ReuseBlank
        );
    }

    #[test]
    fn pending_blank_row_is_not_usable() {
        let rows = [row("s_new"), row("s1")];
        assert!(
            selection_is_pending_blank(Some("s_new"), &rows, 0),
            "选中新建的空行：这是当前会话，不该走切换"
        );
        assert!(
            !selection_is_pending_blank(Some("s_new"), &rows, 1),
            "选中别的会话：照常切换"
        );
        assert!(
            !selection_is_pending_blank(None, &rows, 0),
            "没有待绑定行：照常切换"
        );
        assert!(
            !selection_is_pending_blank(Some("s_new"), &rows, 9),
            "下标越界不该 panic，也不该命中"
        );
    }

    #[test]
    fn deferred_action_covers_the_in_flight_window() {
        // 行 id 有了就立即做（即使「在途」标记还没清）。
        assert_eq!(
            plan_deferred_action(Some("s_new"), true),
            DeferredAction::Now("s_new")
        );
        assert_eq!(
            plan_deferred_action(Some("s_new"), false),
            DeferredAction::Now("s_new")
        );
        // 行 id 没有且 `NewBlank` 在途：先攒着，等行 id 到手。
        assert_eq!(plan_deferred_action(None, true), DeferredAction::Defer);
        // 没有待登记的会话行（普通会话）：不能把它的 cid / 标题往索引里塞。
        assert_eq!(plan_deferred_action(None, false), DeferredAction::Skip);
    }

    #[test]
    fn new_blank_outcome_adopts_while_still_on_the_new_session() {
        let applied = SessionIndexApplied {
            local_id: "s_new".into(),
            reused_blank: true,
        };
        assert_eq!(
            plan_new_blank_outcome(&applied, false, None, Some("标题"), None),
            NewBlankOutcome::Adopt {
                local_id: "s_new".into(),
                reused_blank: true,
                title: Some("标题".into()),
                bind_conversation_id: None,
            }
        );
    }

    #[test]
    fn new_blank_outcome_adopts_when_a_turn_is_waiting_to_bind() {
        // 首轮已拿到 cid、行 id 刚回来：`conversation_id` 也是 Some，但必须认领而非回收。
        let applied = SessionIndexApplied {
            local_id: "s_new".into(),
            reused_blank: false,
        };
        assert_eq!(
            plan_new_blank_outcome(&applied, true, Some("c1"), None, Some("s1".into())),
            NewBlankOutcome::Adopt {
                local_id: "s_new".into(),
                reused_blank: false,
                title: None,
                bind_conversation_id: Some("c1".into()),
            }
        );
    }

    #[test]
    fn new_blank_outcome_reclaims_after_switching_away() {
        let applied = SessionIndexApplied {
            local_id: "s_new".into(),
            reused_blank: false,
        };
        assert_eq!(
            plan_new_blank_outcome(&applied, true, None, Some("标题"), Some("s1".into())),
            NewBlankOutcome::Reclaim {
                local_id: "s_new".into(),
                next_active: Some("s1".into()),
            }
        );
    }
}
