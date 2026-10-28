//! 全屏 `tui` 的断流续传（`/resume`）：断点记录与「什么时候才该留断点」的判定。
//!
//! repl 的等价实现在 `src/main.rs`（`ResumePoint` / `after_turn_outcome` /
//! `capture_resume_point`）：回合结束后，只有 **serve 侧 job 可能仍在跑** 时才留断点，
//! 其余一律清空——否则 `/resume` 会把一个早已跑完的回合再发一遍，模型会重复回答。
//!
//! 两种来源：
//! 1. **传输中断**（`TermError::InterruptedStream`）：已拿到 job 句柄，worker 在错误
//!    变成字符串之前把 `(job_id, after_seq)` 拆出来一起送来（见 `worker::split_turn_result`）。
//! 2. **Ctrl+C 且 cancel 未送达**：cancel 请求失败时 serve 侧的 job 还在跑，
//!    `last_event_id` 就是本端已消费到的位置。
//!
//! 单独成文件是为了压 `mod.rs` 的单文件行数门禁（≤920）；
//! `LaneApp` 的私有字段对本模块可见（子模块可访问祖先模块的私有项）。

use crate::serve::{ChatStreamOutcome, StreamResume};

use super::LaneApp;
use super::state::LineKind;

/// 最近一次「serve 侧仍在跑、但本端已经断开」的回合断点。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumePoint {
    /// 重发时要带上的原始消息（serve 靠它 + `stream_resume` 续上同一 job）。
    pub message: String,
    /// 发起那一轮所属的会话；切过会话后断点失效。
    pub conversation_id: Option<String>,
    pub job_id: u64,
    /// 本端已消费到的 SSE 序号，续流从这里之后开始。
    pub after_seq: u64,
}

/// 一轮发出时的原始输入（断点要能把同一轮原样重发）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnEcho {
    pub message: String,
    pub conversation_id: Option<String>,
}

/// 回合结束后该怎么处置断点：留下（可 `/resume`）或清空。
///
/// - `interrupted`：worker 从 `TermError::InterruptedStream` 取出的续流点（最高优先）。
/// - 否则仅在「用户取消 + cancel 未送达 + 有 job 句柄」时保留。
/// - `sent` 为 `None`（没记下本轮消息，例如异常路径）时无法重发，一律不保留。
#[must_use]
pub fn resume_after_turn(
    outcome: &ChatStreamOutcome,
    sent: Option<&TurnEcho>,
    interrupted: Option<StreamResume>,
) -> Option<ResumePoint> {
    let sent = sent?;
    // 会话 id 取「回合结束后确定的」那个：首轮发起时还 `None`（新会话没有 id），
    // 回合结束才由响应头绑定。若照抄发起时的值，`/resume` 的会话校验会把它
    // 判成「另一个会话」而拒绝——恰恰是最常见的首轮掉线场景。
    let conversation_id = outcome
        .conversation_id
        .clone()
        .or_else(|| sent.conversation_id.clone());
    if let Some(point) = interrupted {
        return Some(ResumePoint {
            message: sent.message.clone(),
            conversation_id,
            job_id: point.job_id,
            after_seq: point.after_seq,
        });
    }
    // cancel 已送达 = serve 确实停了这个回合，没有可续的东西。
    if !(outcome.cancelled_by_user && !outcome.cancel_acknowledged) {
        return None;
    }
    let job_id = outcome.job_id?;
    Some(ResumePoint {
        message: sent.message.clone(),
        conversation_id,
        job_id,
        after_seq: outcome.last_event_id,
    })
}

impl LaneApp<'_> {
    /// 回合结束后更新断点，并在可续时把 `/resume` 这条路告诉用户。
    pub(super) fn record_turn_outcome(
        &mut self,
        outcome: &ChatStreamOutcome,
        interrupted: Option<StreamResume>,
    ) {
        let sent = self.sent_turn.take();
        let point = resume_after_turn(outcome, sent.as_ref(), interrupted);
        if let Some(point) = &point {
            // 两种来源的说法不同：传输中断已经由「回合出错」那行给出 job/seq，
            // 这里只补「怎么救」；cancel 未送达则要说明 job 可能还在跑。
            let line = if interrupted.is_some() {
                format!(
                    "提示：/resume 可从 seq {} 之后续传该回合（serve 侧 job 仍在跑时有效）",
                    point.after_seq
                )
            } else {
                format!(
                    "提示：cancel 未送达，serve 侧 job 可能仍在跑；/resume 可续传（job {}）",
                    point.job_id
                )
            };
            self.st.push_line(LineKind::System, &line);
        }
        self.resume = point;
    }

    /// 切会话 / 新建会话：断点跟着那一轮的会话走，换了就作废。
    pub(super) fn drop_resume_point(&mut self) {
        self.resume = None;
        self.sent_turn = None;
    }

    /// `/resume`：用记录的断点重发上一轮（`stream_resume:{job_id, after_seq}`）。
    pub(super) fn resume_turn(&mut self) {
        if self.st.running {
            self.st.push_line(
                LineKind::System,
                "回合进行中：先 Ctrl+C 取消，结束后再 /resume",
            );
            return;
        }
        let Some(point) = self.resume.take() else {
            self.st.push_line(
                LineKind::System,
                "没有可续传的回合（上一轮已正常结束，或从未产生可续断点）",
            );
            return;
        };
        // 断点属于发起那一刻的会话：期间切过会话（transcript 已换）就丢弃，
        // 否则续传回来的正文会落进另一个会话的记录里。
        if point.conversation_id != self.st.conversation_id {
            self.st.push_line(
                LineKind::System,
                "断点属于另一个会话（切换后已失效）：未续传",
            );
            return;
        }
        let opts = self.build_turn_opts(
            &point.message,
            Some(StreamResume {
                job_id: point.job_id,
                after_seq: point.after_seq,
            }),
        );
        // 续流不重发用户消息（transcript 里已有那一行）。正文能接在原 assistant 块上
        // 就直接接（cancel 未送达的常见情形）；中间隔着错误提示行时另起一块并说明来由。
        if self.st.last_kind() != Some(LineKind::Assistant) {
            self.st.push_line(
                LineKind::System,
                &format!(
                    "↻ 续传中断回合（job {}，从 seq {} 之后继续）",
                    point.job_id, point.after_seq
                ),
            );
        }
        self.dispatch_turn(opts);
    }
}

#[cfg(test)]
mod tests {
    use super::{ResumePoint, TurnEcho, resume_after_turn};
    use crate::serve::{ChatStreamOutcome, StreamResume};

    fn sent() -> TurnEcho {
        TurnEcho {
            message: "你好".into(),
            conversation_id: Some("c1".into()),
        }
    }

    fn outcome(
        job_id: Option<u64>,
        cancelled_by_user: bool,
        cancel_acknowledged: bool,
    ) -> ChatStreamOutcome {
        ChatStreamOutcome {
            job_id,
            last_event_id: 17,
            cancelled_by_user,
            cancel_acknowledged,
            ..ChatStreamOutcome::default()
        }
    }

    #[test]
    fn clean_turn_drops_point() {
        let outcome = outcome(Some(42), false, false);
        assert_eq!(resume_after_turn(&outcome, Some(&sent()), None), None);
    }

    #[test]
    fn cancel_not_acknowledged_keeps_point() {
        let outcome = outcome(Some(42), true, false);
        assert_eq!(
            resume_after_turn(&outcome, Some(&sent()), None),
            Some(ResumePoint {
                message: "你好".into(),
                conversation_id: Some("c1".into()),
                job_id: 42,
                after_seq: 17,
            })
        );
    }

    #[test]
    fn acknowledged_cancel_drops_point() {
        let outcome = outcome(Some(42), true, true);
        assert_eq!(resume_after_turn(&outcome, Some(&sent()), None), None);
    }

    #[test]
    fn transport_interrupt_uses_worker_seq() {
        // 传输中断的 `after_seq` 来自错误本身（可能比 outcome 里的更靠前）。
        let outcome = ChatStreamOutcome::default();
        let point = resume_after_turn(
            &outcome,
            Some(&sent()),
            Some(StreamResume {
                job_id: 7,
                after_seq: 3,
            }),
        );
        assert_eq!(
            point,
            Some(ResumePoint {
                message: "你好".into(),
                conversation_id: Some("c1".into()),
                job_id: 7,
                after_seq: 3,
            })
        );
    }

    #[test]
    fn no_job_id_drops_point() {
        let outcome = outcome(None, true, false);
        assert_eq!(resume_after_turn(&outcome, Some(&sent()), None), None);
    }

    #[test]
    fn first_turn_adopts_the_id_from_the_outcome() {
        // 首轮发起时新会话还没有 id；回合结束才由响应头绑上。
        // 断点必须记这个新 id，否则 `/resume` 的会话校验会当成「另一个会话」拒绝。
        let sent = TurnEcho {
            message: "你好".into(),
            conversation_id: None,
        };
        let mut outcome = outcome(Some(42), true, false);
        outcome.conversation_id = Some("c9".into());
        assert_eq!(
            resume_after_turn(&outcome, Some(&sent), None),
            Some(ResumePoint {
                message: "你好".into(),
                conversation_id: Some("c9".into()),
                job_id: 42,
                after_seq: 17,
            })
        );
    }

    #[test]
    fn missing_sent_message_drops_point() {
        // 连原始消息都没记下来就无从重发——宁可没有断点，也不要发一条空消息。
        let outcome = outcome(Some(42), true, false);
        assert_eq!(resume_after_turn(&outcome, None, None), None);
    }
}
