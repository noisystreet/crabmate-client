//! 「serve 里存在的会话，索引里必须有一行指向它」的纯判定（issue #618）。
//!
//! 三端的侧栏都是同一份 `sessions` 索引的只读投影，而服务端对它只做透传存储 ——
//! 索引的写入方全是客户端。于是必然出现漂移：新工作区的桶为空、用户没按「新建会话」
//! 就直接发消息、行被别的端删掉，三种情形表现完全一样：**会话在 serve 里真实存在，
//! 侧栏却一行都没有**。
//!
//! 本模块只回答「该不该写」，**不做 IO、不读时钟、不生成 id**：落盘与幂等在各端的写入层
//! （TUI 见 `SessionIndexOp::EnsureListed`）。与 `session_model` 里
//! `reclaim_disposable_blank_session` 一族同属「会话规则只有一份口径」。

use crate::sessions::{SessionListRow, session_row_conversation_id_for_resume};

/// [`plan_session_ensure`] 的判定。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEnsure {
    /// 没有可绑定的会话 id（空 / 全空白）：不写。
    NoConversationId,
    /// 本端有「新建会话」建的行还没绑定：由那条路径认领这一轮，这里不写 ——
    /// 再插一行只会多出一条没人认领的空会话。
    PendingLocalRow,
    /// 索引里已经有行带着这个会话：什么都不用写，`row_id` 是那行的本地 id。
    AlreadyListed { row_id: String },
    /// 没有任何行带着这个会话：需要插入一行并绑定。
    InsertAndBind,
}

/// 判断要不要为 `conversation_id` 补一行。
///
/// `rows` 是**本地快照**，可能过期（别的端刚删 / 刚建）。因此 `InsertAndBind` 的落地层
/// 必须按最新 `GET` 回来的数组再判一次、已存在就退化成空写：这条约定让快照过期只多一次
/// 读，不会多出一行。
pub fn plan_session_ensure(
    rows: &[SessionListRow],
    conversation_id: &str,
    pending_local_row: bool,
) -> SessionEnsure {
    let cid = conversation_id.trim();
    if cid.is_empty() {
        return SessionEnsure::NoConversationId;
    }
    if pending_local_row {
        return SessionEnsure::PendingLocalRow;
    }
    match rows
        .iter()
        .find(|row| session_row_conversation_id_for_resume(row) == Some(cid))
    {
        Some(row) => SessionEnsure::AlreadyListed {
            row_id: row.id.clone(),
        },
        None => SessionEnsure::InsertAndBind,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rows(v: serde_json::Value) -> Vec<SessionListRow> {
        SessionListRow::parse_rows(&v)
    }

    #[test]
    fn missing_conversation_id_never_writes() {
        assert_eq!(
            plan_session_ensure(&rows(json!([])), "", false),
            SessionEnsure::NoConversationId
        );
        assert_eq!(
            plan_session_ensure(&rows(json!([])), "   ", false),
            SessionEnsure::NoConversationId
        );
    }

    #[test]
    fn empty_index_needs_a_row() {
        // 新工作区的桶为空：这正是「左栏一直空」的起点。
        assert_eq!(
            plan_session_ensure(&rows(json!([])), "c1", false),
            SessionEnsure::InsertAndBind
        );
    }

    #[test]
    fn rows_without_this_conversation_need_a_row() {
        // 桶里有别的会话（或一条未绑定的空白行），但没有一行属于这一轮。
        let list = rows(json!([
            {"id":"s1","title":"old","server_conversation_id":"c-old"},
            {"id":"s2","title":"新会话"},
        ]));
        assert_eq!(
            plan_session_ensure(&list, "c1", false),
            SessionEnsure::InsertAndBind
        );
    }

    #[test]
    fn listed_conversation_is_idempotent() {
        let list = rows(json!([
            {"id":"s1","title":"old","server_conversation_id":"c-old"},
            {"id":"s2","title":"mine","server_conversation_id":"c1"},
        ]));
        assert_eq!(
            plan_session_ensure(&list, "c1", false),
            SessionEnsure::AlreadyListed {
                row_id: "s2".into()
            }
        );
        // 行上的 id 可能带空白（透传存储不做归一）：比较前必须 trim，否则会重复建行。
        let padded = rows(json!([{"id":"s9","server_conversation_id":"  c1  "}]));
        assert_eq!(
            plan_session_ensure(&padded, "c1", false),
            SessionEnsure::AlreadyListed {
                row_id: "s9".into()
            }
        );
    }

    #[test]
    fn in_flight_local_row_wins_over_inserting() {
        // 「新建会话」的行还没绑定：它的绑定路径会认领这一轮，这里再插一行就多一条空会话。
        let list = rows(json!([{"id":"s_blank","title":"新会话"}]));
        assert_eq!(
            plan_session_ensure(&list, "c1", true),
            SessionEnsure::PendingLocalRow
        );
        // 连列表都没到也成立：在途标记本身足以说明「有人在管这一轮」。
        assert_eq!(
            plan_session_ensure(&[], "c1", true),
            SessionEnsure::PendingLocalRow
        );
    }
}
