//! 首轮结束后「这一轮的会话必须出现在左栏」的兜底（issue #618）。
//!
//! TUI 的左栏只是 serve 端会话索引的只读投影，服务端不替任何客户端建行；于是新工作区的
//! 空桶、从没 `/conv new` 就发消息、行被别的端删掉这三种情形下，会话在 serve 里真实存在、
//! 左栏却永远空着。判定在共享层 [`plan_session_ensure`]（Web / TUI 同一份口径），这里只做
//! 接线与一句话提示。
//!
//! 单独成模块的直接原因是行数门禁（`fn-nloc` 单文件 ≤ 920）：`session_index.rs` 已经贴着
//! 上限，再塞一个方法就顶线。

use super::LaneApp;
use super::state::LineKind;
use super::worker::WorkerJob;
use crate::serve::{DEFAULT_SESSION_TITLE, SessionEnsure, SessionIndexOp, plan_session_ensure};

impl LaneApp<'_> {
    /// 保证本轮的 `conversation_id` 在左栏有一行（issue #618）。
    ///
    /// 原先只有 `/conv new` 建的行会被绑定：新工作区的索引桶为空、用户直接发消息时，
    /// 会话在 serve 里真实存在、左栏却永远空着（TUI 只读服务端索引，不自己造行）。
    /// 这里补上「没有行就建一行」，判定走共享层 [`plan_session_ensure`]。
    ///
    /// **必须在 [`Self::retitle_by_conversation`] 之前调用**：要建行时这里会把在途标题
    /// 随行写入（`take` 走 `pending_cid_title`），那条路径随即无事可做。
    pub(super) fn ensure_session_listed(&mut self, conversation_id: &str) {
        let pending_local_row =
            self.index.local_new_session.is_some() || self.index.new_session_pending;
        if plan_session_ensure(&self.st.sessions, conversation_id, pending_local_row)
            != SessionEnsure::InsertAndBind
        {
            return;
        }
        let title = self
            .index
            .pending_cid_title
            .take()
            .unwrap_or_else(|| DEFAULT_SESSION_TITLE.to_string());
        let op = SessionIndexOp::EnsureListed {
            conversation_id: conversation_id.to_string(),
            title,
        };
        if self.job_tx.send(WorkerJob::SessionIndex(op)).is_err() {
            self.st
                .push_line(LineKind::System, "会话未能登记到列表：worker 已退出");
        }
    }

    /// 建行失败时把已经取走的标题放回去（issue #618）。
    ///
    /// `pending_cid_title` 只在**首轮**记一次（之后的轮次 `conversation_id` 已非空，起名走
    /// [`Self::retitle_by_conversation`]，而它只认索引里已有的行）。建行失败又把标题丢了，
    /// 这条会话就会永久停在默认标题上 —— 所以失败时原样放回，让下一轮还有机会。
    pub(super) fn restore_listing_title(&mut self, op: &SessionIndexOp) {
        let SessionIndexOp::EnsureListed { title, .. } = op else {
            return;
        };
        if self.index.pending_cid_title.is_none() {
            self.index.pending_cid_title = Some(title.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;
    use crate::serve::SessionListRow;
    use crate::tui_mode::test_support::{test_client, test_prefs, test_shell_with_jobs};

    fn row_with_cid(local_id: &str, cid: &str) -> SessionListRow {
        SessionListRow {
            id: local_id.to_string(),
            title: String::new(),
            updated_at: 0,
            pinned: false,
            starred: false,
            server_conversation_id: Some(cid.to_string()),
            server_revision: None,
            workspace_root: None,
        }
    }

    /// 这一轮发给 worker 的 `EnsureListed`（`(cid, title)`）；其余 job 忽略。
    fn ensure_jobs(rx: &mpsc::Receiver<WorkerJob>) -> Vec<(String, String)> {
        rx.try_iter()
            .filter_map(|job| match job {
                WorkerJob::SessionIndex(SessionIndexOp::EnsureListed {
                    conversation_id,
                    title,
                }) => Some((conversation_id, title)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn empty_sidebar_sends_one_ensure_job_with_the_pending_title() {
        // 新工作区：左栏空，标题由首轮消息算出 —— 两者都要带出去，否则行建了却没名字。
        let client = test_client();
        let mut prefs = [test_prefs()];
        let (mut app, jobs) = test_shell_with_jobs(&client, &mut prefs);
        app.lanes[0].index.pending_cid_title = Some("第一条消息".into());

        app.lanes[0].ensure_session_listed("c1");

        assert_eq!(
            ensure_jobs(&jobs[0]),
            vec![("c1".to_string(), "第一条消息".to_string())]
        );
        assert!(
            app.lanes[0].index.pending_cid_title.is_none(),
            "标题随行写入，不必再发一次 SetTitleByServer"
        );
    }

    #[test]
    fn already_listed_conversation_sends_nothing() {
        // 行已在快照里：这一次不该产生任何写入（幂等靠「不发」，不是「发了再判」）。
        let client = test_client();
        let mut prefs = [test_prefs()];
        let (mut app, jobs) = test_shell_with_jobs(&client, &mut prefs);
        app.lanes[0].st.sessions = vec![row_with_cid("s1", "c1")];

        app.lanes[0].ensure_session_listed("c1");

        assert_eq!(jobs[0].try_iter().count(), 0);
    }

    #[test]
    fn failed_listing_restores_the_title_for_the_next_turn() {
        // 建行失败时标题必须放回去：它只在首轮记一次，丢了这条会话就永久停在默认标题。
        let client = test_client();
        let mut prefs = [test_prefs()];
        let (mut app, _jobs) = test_shell_with_jobs(&client, &mut prefs);
        let op = SessionIndexOp::EnsureListed {
            conversation_id: "c1".into(),
            title: "第一条消息".into(),
        };

        app.lanes[0].restore_listing_title(&op);

        assert_eq!(
            app.lanes[0].index.pending_cid_title.as_deref(),
            Some("第一条消息")
        );
    }
}
