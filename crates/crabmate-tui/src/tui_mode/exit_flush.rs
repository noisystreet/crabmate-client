//! 退出前的 best-effort 落盘（`Ctrl+C` / `Ctrl+D` / `/quit` 共用这一条路径）。
//!
//! TUI 不持有会话正文：正文随流式写入服务端存储，客户端也没有「保存消息」的端点
//! （`paths` 里与对话相关的只有 `GET /conversation/messages` 与 `DELETE`）。所以退出时
//! 能补的只有索引上的两件事：
//!
//! 1. **会话绑定**：`/conv new` 建的行在首轮结束后才拿到 `conversation_id`，worker 那条
//!    `BindServer` 若还没落地，下次启动就回放不出这段对话（行仍是一条空会话）；
//! 2. **输入框草稿**：对齐 Web 的 `flush_composer_draft_to_session`。
//!
//! 两者合并进一次 `SessionIndexOp::FlushExit`（一次 `GET → 应用 → PUT`）：每个 op 都是
//! 「读全量索引 → 改 → 写全量」，两次独立写入之间被另一端插进来就会互相覆盖。
//!
//! 两条硬约束：**有界**（第二下 Ctrl+C 的语义就是「立刻走」，强退只给 400ms）、
//! **不影响退出码**（失败只打一行 stderr）。没东西可写时一个请求都不发。
//!
//! **明确不做的**：首轮还在飞就强退时，客户端并不知道 `conversation_id`（它只在回合
//! 结束时才赋值），所以那种情况下没有绑定可补——正文仍在服务端，但索引里那条行会留在
//! 「未绑定」状态。要修它得让服务端在流式中途就报出 `conversation_id`，是另一个决定。

use std::time::{Duration, Instant};

use super::{LaneApp, TuiApp};
use crate::serve::{SessionIndexOp, apply_web_session_op};

/// 普通退出的预算：一次索引写（`GET` + `PUT`）。
const NORMAL_BUDGET: Duration = Duration::from_millis(1000);
/// 强退（回合还在跑时第二次 Ctrl+C）只留最短的机会。
const FORCE_BUDGET: Duration = Duration::from_millis(400);

/// 退出落盘的截止时刻：**所有 lane 共用一份**，否则 N 条 lane 会把退出延迟放大 N 倍。
///
/// `force` = 还有 lane 在跑（强退语义），只留最短机会，不等它落定。
fn exit_flush_deadline(force: bool) -> Instant {
    Instant::now() + if force { FORCE_BUDGET } else { NORMAL_BUDGET }
}

/// 退出落盘要做的事；由 [`plan_exit_flush`] 从 UI 状态算出，便于单测。
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct ExitFlushPlan {
    /// 待补的绑定：`(本地行 id, conversation_id)`。
    bind: Option<(String, String)>,
    /// 待写回的草稿：`(本地行 id, 草稿全文)`。
    draft: Option<(String, String)>,
}

impl ExitFlushPlan {
    /// 合并成一次索引写；没东西可写时返回 `None`（调用方据此一个请求也不发）。
    ///
    /// 两件事的落点必然一致：草稿优先落在待绑定行上（见 [`plan_exit_flush`]），
    /// 没有待绑定行时才退到 `conversation_id` 对应的行——而那种情况下 `bind` 本就为空。
    #[must_use]
    fn into_op(self) -> Option<SessionIndexOp> {
        let bind_row = self.bind.as_ref().map(|(row, _)| row.clone());
        let draft_row = self.draft.as_ref().map(|(row, _)| row.clone());
        debug_assert!(
            bind_row.is_none() || draft_row.is_none() || bind_row == draft_row,
            "绑定与草稿的落点必须一致，否则合并成一次 PUT 会写错行"
        );
        let local_id = bind_row.or(draft_row)?;
        Some(SessionIndexOp::FlushExit {
            local_id,
            conversation_id: self.bind.map(|(_, cid)| cid),
            draft: self.draft.map(|(_, text)| text),
        })
    }
}

/// [`plan_exit_flush`] 的输入：只取决策需要的字段，免得把整个 `LaneApp` 拖进单测。
#[derive(Debug, Clone, Copy)]
pub(super) struct ExitFlushView<'a> {
    /// 当前绑定的服务端会话（`None` = 还没发送过 / 刚 `/conv new` / 首轮未结束）。
    pub conversation_id: Option<&'a str>,
    /// `/conv new` 建的行 id；绑定成功后会被清空，所以非空即「还没绑定」。
    pub pending_row: Option<&'a str>,
    /// `NewBlank` 还在写：此时行 id 未知。
    pub row_pending: bool,
    /// 当前 `conversation_id` 对应的索引行 id（草稿落点的兜底）。
    pub row_for_conversation: Option<&'a str>,
    /// 输入框全文。
    pub draft: &'a str,
}

/// 从 UI 状态算出退出时要做什么。
pub(super) fn plan_exit_flush(view: ExitFlushView<'_>) -> ExitFlushPlan {
    // 行 id 还没到手（`NewBlank` 在途）就无从绑定：为它再等一轮写入不值得，直接放弃。
    let bind = match (view.row_pending, view.pending_row, view.conversation_id) {
        (false, Some(row), Some(cid)) => Some((row.to_string(), cid.to_string())),
        _ => None,
    };
    // 全空白的草稿不写：那等于「没草稿」，写了反而让空白行不再可回收
    // （`session_is_disposable_blank` 要求 `draft` 为空，非空就把空会话钉在侧栏）。
    let draft = if view.draft.trim().is_empty() {
        None
    } else {
        view.pending_row
            .or(view.row_for_conversation)
            .map(|row| (row.to_string(), view.draft.to_string()))
    };
    ExitFlushPlan { bind, draft }
}

impl LaneApp<'_> {
    /// 退出落盘；返回需要告知用户的失败摘要（`None` = 没需求或已写入）。
    ///
    /// 失败只影响这条提示，**不影响退出码**：会话正文本来就在服务端，这里补的是
    /// 「下次能不能接上」这一点元数据。
    ///
    /// 截止时刻由调用方给：多 lane 时外壳让所有 lane 共用同一份预算
    /// （见 [`TuiApp::flush_all_before_exit`]），lane 数不该放大退出延迟。
    pub(super) async fn flush_before_exit(&mut self, deadline: Instant) -> Option<String> {
        let op = self.exit_flush_plan().into_op()?;
        match self.apply_index_op_within(&op, deadline).await {
            Ok(()) => None,
            Err(e) => Some(format!("退出前落盘未完成，下次可能接不上该会话：{e}")),
        }
    }

    /// 从当前 UI 状态算出落盘计划（取快照，不持有 `self` 的借用）。
    fn exit_flush_plan(&self) -> ExitFlushPlan {
        let draft = self.st.input_snapshot();
        let conversation_id = self.st.conversation_id.clone();
        let row_for_conversation = conversation_id
            .as_deref()
            .and_then(|cid| self.row_id_for_conversation(cid));
        plan_exit_flush(ExitFlushView {
            conversation_id: conversation_id.as_deref(),
            pending_row: self.index.local_new_session.as_deref(),
            row_pending: self.index.new_session_pending,
            row_for_conversation: row_for_conversation.as_deref(),
            draft: &draft,
        })
    }

    /// 用剩余预算执行一次索引写；预算耗尽就不再发请求。
    async fn apply_index_op_within(
        &mut self,
        op: &SessionIndexOp,
        deadline: Instant,
    ) -> Result<(), String> {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err("退出预算已用尽".to_string());
        }
        // `Cow<ServeClient>` 解引用成 `&ServeClient`（不必为此 import 类型名）。
        let client = &*self.client;
        match tokio::time::timeout(left, apply_web_session_op(client, op)).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(e.to_string()),
            Err(_) => Err("请求超时".to_string()),
        }
    }
}

impl TuiApp<'_> {
    /// 退出前把**所有** lane 的草稿 / 待补绑定落盘，返回需要打印的失败摘要。
    ///
    /// `Drafts` 是内存态、只在退出时写回服务端索引：只 flush 当前 lane 会让其它 lane
    /// 未发送的输入直接丢失（issue #338 的症状 B）。背景图同理 —— kitty 的像素配额是
    /// 终端级的，退出时不还，下一次运行会顶着旧的占用。
    ///
    /// 预算是**一份总预算**而不是每条 lane 一份：`/quit` 后等 N 秒不可接受，仍按「有界
    /// best-effort」取舍 —— 预算内没写完的 lane 只体现在返回的提示里，不影响退出码。
    /// 但逐条**均分剩余**预算：某条 lane 的 serve 不可达时会一直吃到截止时刻，均分之后
    /// 它最多吃掉 1/N，其余 lane 仍有窗口把那句草稿写出去。
    pub(super) async fn flush_all_before_exit(&mut self) -> Vec<String> {
        // 任一 lane 仍在跑 ⇒ 强退预算。比只看当前 lane 的 `st.running` 准：
        // 聚焦的那条空闲、后台还在生成时，同样不该等它落定。
        let end = exit_flush_deadline(self.lanes.iter().any(|l| l.st.running));
        let count = self.lanes.len();
        let mut warns = Vec::new();
        for (i, lane) in self.lanes.iter_mut().enumerate() {
            let left = end.saturating_duration_since(Instant::now());
            // 还剩几条就分成几份（`count - i >= 1`，不会除零）。第一条卡在死 serve 上
            // 也不会让后面的 lane 拿到零预算 —— 那正是「退出时丢掉别的 lane 的草稿」
            // 的另一种写法。
            let deadline = Instant::now() + left / (count - i) as u32;
            lane.clear_background();
            if let Some(warn) = lane.flush_before_exit(deadline).await {
                warns.push(format!("`{}`：{warn}", lane.label));
            }
        }
        warns
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{test_client, test_prefs, test_shell};
    use super::*;

    fn view<'a>(
        conversation_id: Option<&'a str>,
        pending_row: Option<&'a str>,
        row_pending: bool,
        row_for_conversation: Option<&'a str>,
        draft: &'a str,
    ) -> ExitFlushView<'a> {
        ExitFlushView {
            conversation_id,
            pending_row,
            row_pending,
            row_for_conversation,
            draft,
        }
    }

    /// 干净退出（无待绑定、无草稿）：合不出 op，一个请求都不发。
    #[test]
    fn idle_exit_plans_nothing() {
        let plan = plan_exit_flush(view(Some("c1"), None, false, Some("s1"), ""));
        assert!(plan.into_op().is_none());
    }

    /// `/conv new` 建的行还没绑定，且已拿到 `conversation_id`：补绑定。
    #[test]
    fn pending_bind_is_flushed() {
        let plan = plan_exit_flush(view(Some("c1"), Some("s1"), false, None, ""));
        assert_eq!(plan.bind, Some(("s1".into(), "c1".into())));
        assert!(plan.draft.is_none());
    }

    /// `NewBlank` 在途：行 id 未知，放弃绑定（不再多等一轮写入）。
    #[test]
    fn bind_waits_for_row_id() {
        let plan = plan_exit_flush(view(Some("c1"), Some("s1"), true, None, ""));
        assert!(plan.bind.is_none());
    }

    /// 还没发送过任何消息 / 首轮还没结束：没有 `conversation_id`，无从绑定。
    #[test]
    fn bind_needs_conversation_id() {
        let plan = plan_exit_flush(view(None, Some("s1"), false, None, ""));
        assert!(plan.bind.is_none());
    }

    /// 草稿落在待绑定行上；没有待绑定行时按 `conversation_id` 找行。
    #[test]
    fn draft_targets_pending_row_then_conversation_row() {
        let with_pending =
            plan_exit_flush(view(Some("c1"), Some("s1"), false, Some("s2"), "写一半"));
        assert_eq!(with_pending.draft, Some(("s1".into(), "写一半".into())));

        let fallback = plan_exit_flush(view(Some("c1"), None, false, Some("s2"), "写一半"));
        assert_eq!(fallback.draft, Some(("s2".into(), "写一半".into())));
    }

    /// 全空白草稿不写：写了会让空白行不再可回收。
    #[test]
    fn blank_draft_is_not_written() {
        let plan = plan_exit_flush(view(Some("c1"), None, false, Some("s1"), " \n\t "));
        assert!(plan.draft.is_none());
    }

    /// 行都定位不到时，草稿没有落点，直接放弃。
    #[test]
    fn draft_without_any_row_is_dropped() {
        let plan = plan_exit_flush(view(None, None, false, None, "写一半"));
        assert!(plan.draft.is_none());
    }

    /// 两件事都在时合并成**一次**写入，且两件事都带上。
    #[test]
    fn bind_and_draft_merge_into_single_op() {
        let plan = plan_exit_flush(view(Some("c1"), Some("s1"), false, None, "写一半"));
        let op = plan.into_op().expect("应有待写内容");
        assert_eq!(
            op,
            SessionIndexOp::FlushExit {
                local_id: "s1".into(),
                conversation_id: Some("c1".into()),
                draft: Some("写一半".into()),
            }
        );
    }

    /// 只有草稿时，`conversation_id` 为 `None`（不该顺手重写绑定）。
    #[test]
    fn draft_only_op_carries_no_bind() {
        let plan = plan_exit_flush(view(Some("c1"), None, false, Some("s2"), "写一半"));
        let op = plan.into_op().expect("应有待写内容");
        assert_eq!(
            op,
            SessionIndexOp::FlushExit {
                local_id: "s2".into(),
                conversation_id: None,
                draft: Some("写一半".into()),
            }
        );
    }

    /// 所有 lane 都没有待写内容时一个请求都不发（`test_client` 指向 1 端口，真发必失败）。
    ///
    /// 多 lane 的遍历不能退化成「无条件给每条 lane 发一次写入」：干净退出仍应是零请求。
    #[tokio::test]
    async fn flush_all_sends_nothing_when_every_lane_is_clean() {
        let client = test_client();
        let mut prefs = [test_prefs(), test_prefs()];
        let mut app = test_shell(&client, &mut prefs);

        assert!(
            app.flush_all_before_exit().await.is_empty(),
            "干净退出不该产生任何落盘提示"
        );
    }

    /// 每条 lane 都要**被尝试**，且第一条连不上时不能把后面的预算吃光。
    ///
    /// `test_client` 指向 1 端口，两条必然各自失败 —— 于是 `warns` 的条数与 label 前缀
    /// 正好把「遍历了全部 lane」与「失败信息能定位到哪条」一起钉住。只判长度的写法
    /// （把循环换成 `lanes.first_mut()`）会立刻变成 1 条而失败。
    #[tokio::test]
    async fn flush_all_attempts_every_lane_and_tags_the_warning() {
        let client = test_client();
        let mut prefs = [test_prefs(), test_prefs()];
        let mut app = test_shell(&client, &mut prefs);
        for lane in app.lanes.iter_mut() {
            // 待绑定行 + 已有 conversation_id ⇒ `into_op()` 一定产生一次写入。
            lane.st.conversation_id = Some("c1".to_string());
            lane.index.local_new_session = Some("s1".to_string());
        }

        let warns = app.flush_all_before_exit().await;

        assert_eq!(
            warns.len(),
            2,
            "两条 lane 都要落盘，不能只落第一条：{warns:?}"
        );
        assert!(warns[0].starts_with("`lane-1`"), "{warns:?}");
        assert!(warns[1].starts_with("`lane-2`"), "{warns:?}");
    }
}
