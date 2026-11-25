//! 全屏模式 sink：把流事件发往 UI 事件通道（经 `LaneTx` 标注来源 lane）。
//!
//! 从 `worker.rs` 抽出：那里贴着 920 行文件门禁，而这块只是「流事件 → UI 事件」的纯转发，
//! 与 worker 的 job 调度无关。与 `worker_cancel.rs` 同一拆法（`#[path]` 子模块，
//! 不为一个结构体新开一层目录）。
//!
//! **一批内合并**：正文增量按 token 到达，逐条发 `Text(delta)` 意味着「每个 token 一次
//! `String` 分配 + 一次 mpsc 消息 + 一次事件循环唤醒 + 一次帧开销」。同一个 HTTP chunk 里
//! 解析出的连续增量因此先攒在这里，到 [`StreamSink::on_batch_end`]（chunk 边界）一次发出。
//!
//! 合并边界是**一个 chunk**，不是时间窗：别处（工具行 / 系统行 / 问卷 / 用量…）任何回调
//! 到来前都会先刷攒下的增量，`on_finished` 再兜一次底。于是「正文永远排在它之后的事件之前」
//! 这条顺序不变 —— 合并只改变粒度，不改变顺序。

use crabmate_client_api::prompt_tokens::TiktokenPromptTokensSnapshot;
use crabmate_client_api::timeline_skill::SkillInjectInfo;

use crate::serve::StreamSink;
use crate::serve::TermError;

use super::{LaneTx, UiEvent};

/// 一批最多攒多少字节就先发出去。
///
/// 上限的意义不是省内存，而是**延迟兜底**：serve 一次推下一个很大的 chunk（例如一次补全
/// 几百行）时，攒到上限就切一批，主线程不必等整个 chunk 解析完才有东西可画。
const MAX_BATCH_BYTES: usize = 4096;

/// 待发的一批增量。正文与思维链**不混**一批：两者在界面上是不同行、不同样式。
struct PendingDelta {
    /// `true` = 思维链（`UiEvent::Thinking`），`false` = 正文（`UiEvent::Text`）。
    thinking: bool,
    text: String,
}

/// 全屏模式 sink：把流事件发往 UI 事件通道（经 [`LaneTx`] 标注来源 lane）。
pub(super) struct UiSink {
    /// 该 lane 的出站通道；构造点在 `worker` 的回合调度处。
    pub(super) out: LaneTx,
    /// 本批已攒、尚未发出的增量（见模块头注释）。
    pending: Option<PendingDelta>,
}

impl UiSink {
    pub(super) fn new(out: LaneTx) -> Self {
        Self { out, pending: None }
    }

    /// 发掉攒下的增量；没有就什么也不做。
    ///
    /// 空串批直接丢弃：`push_delta` 在合并前就挡掉了空增量，走到这里只剩「换类时刚好没攒到东西」。
    fn flush_pending(&mut self) -> Result<(), TermError> {
        let Some(pending) = self.pending.take() else {
            return Ok(());
        };
        if pending.text.is_empty() {
            return Ok(());
        }
        if pending.thinking {
            self.out.send(UiEvent::Thinking(pending.text))
        } else {
            self.out.send(UiEvent::Text(pending.text))
        }
    }

    /// 把一条增量并进当前批；换类（正文 ↔ 思维）或攒满一档就先刷。
    fn push_delta(&mut self, delta: &str, thinking: bool) -> Result<(), TermError> {
        if delta.is_empty() {
            return Ok(());
        }
        let (same_kind, full) = match self.pending.as_ref() {
            Some(p) => (
                p.thinking == thinking,
                p.text.len().saturating_add(delta.len()) > MAX_BATCH_BYTES,
            ),
            None => (false, false),
        };
        if !same_kind || full {
            self.flush_pending()?;
        }
        match self.pending.as_mut() {
            Some(p) => p.text.push_str(delta),
            None => {
                self.pending = Some(PendingDelta {
                    thinking,
                    text: delta.to_string(),
                });
            }
        }
        Ok(())
    }
}

impl StreamSink for UiSink {
    fn on_text(&mut self, delta: &str) -> Result<(), TermError> {
        self.push_delta(delta, false)
    }

    fn on_reasoning(&mut self, delta: &str) -> Result<(), TermError> {
        self.push_delta(delta, true)
    }

    fn on_system(&mut self, line: &str) -> Result<(), TermError> {
        if line.is_empty() {
            return Ok(());
        }
        self.flush_pending()?;
        self.out.send(UiEvent::System(line.to_string()))
    }

    fn on_context_trim(&mut self, after: u32) -> Result<(), TermError> {
        self.flush_pending()?;
        self.out.send(UiEvent::ContextTrim(after))
    }

    fn on_skill_inject(&mut self, info: &SkillInjectInfo) -> Result<(), TermError> {
        self.flush_pending()?;
        self.out.send(UiEvent::SkillInject(info.clone()))
    }

    fn on_clarify(
        &mut self,
        info: &crabmate_client_api::sse_dispatch::ClarificationQuestionnaireInfo,
    ) -> Result<(), TermError> {
        self.flush_pending()?;
        self.out.send(UiEvent::Clarify(info.clone()))
    }

    fn on_prompt_tokens(&mut self, snap: &TiktokenPromptTokensSnapshot) -> Result<(), TermError> {
        self.flush_pending()?;
        self.out.send(UiEvent::PromptTokens(Box::new(snap.clone())))
    }

    fn on_tool_start(
        &mut self,
        tool_call_id: &str,
        name: &str,
        note: Option<&str>,
    ) -> Result<(), TermError> {
        // 先刷正文：工具行必须排在「模型说到这一步」之后，否则 transcript 里会看到
        // 「调用 read_file」出现在它上面那句解释之前。
        self.flush_pending()?;
        self.out.send(UiEvent::ToolStart {
            tool_call_id: tool_call_id.to_string(),
            name: name.to_string(),
            note: note.map(str::to_string),
        })
    }

    fn on_tool_end(
        &mut self,
        tool_call_id: &str,
        name: &str,
        ok: Option<bool>,
        note: Option<&str>,
        detail: Option<&str>,
    ) -> Result<(), TermError> {
        self.flush_pending()?;
        self.out.send(UiEvent::ToolEnd {
            tool_call_id: tool_call_id.to_string(),
            name: name.to_string(),
            ok,
            note: note.map(str::to_string),
            detail: detail.map(str::to_string),
        })
    }

    /// 一个 HTTP chunk 的边界：把攒下的增量一次发出去。
    fn on_batch_end(&mut self) -> Result<(), TermError> {
        self.flush_pending()
    }

    /// 流收尾兜底。`on_batch_end` 已覆盖常态，这里防「最后一批之后没有边界」。
    fn on_finished(&mut self) -> Result<(), TermError> {
        self.flush_pending()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    use crate::tui_mode::inbound::Inbound;
    use crate::tui_mode::worker::LaneId;

    fn sink() -> (UiSink, mpsc::Receiver<Inbound>) {
        let (tx, rx) = mpsc::channel();
        (UiSink::new(LaneTx::new(LaneId(1), tx)), rx)
    }

    /// 通道里的事件摊平：lane 标签由 `LaneTx` 负责，测试只看载荷。
    fn drain(rx: &mpsc::Receiver<Inbound>) -> Vec<UiEvent> {
        rx.try_iter()
            .filter_map(|msg| match msg {
                Inbound::FromLane(_, ev) => Some(ev),
                _ => None,
            })
            .collect()
    }

    /// 合并的全部意义：同一批里的多个增量只产生一条事件，且文本原样拼回去。
    #[test]
    fn one_batch_merges_into_a_single_text_event() {
        let (mut sink, rx) = sink();
        for part in ["你", "好", "，世界"] {
            sink.on_text(part).expect("send");
        }
        assert!(drain(&rx).is_empty(), "批边界之前不该有事件");
        sink.on_batch_end().expect("flush");
        let events = drain(&rx);
        assert_eq!(events.len(), 1, "三条增量应合成一条事件");
        assert!(matches!(&events[0], UiEvent::Text(t) if t == "你好，世界"));
    }

    /// 顺序不能因合并而变：工具行必须排在它前面那段正文之后。
    #[test]
    fn tool_start_flushes_pending_text_first() {
        let (mut sink, rx) = sink();
        sink.on_text("先说明").expect("send");
        sink.on_tool_start("id-1", "read_file", Some("a.txt"))
            .expect("send");
        let events = drain(&rx);
        assert_eq!(events.len(), 2, "正文与工具行是两条事件");
        assert!(matches!(&events[0], UiEvent::Text(t) if t == "先说明"));
        assert!(matches!(&events[1], UiEvent::ToolStart { name, .. } if name == "read_file"));
    }

    /// 思维链与正文样式不同，不能并进同一批。
    #[test]
    fn reasoning_and_text_never_share_a_batch() {
        let (mut sink, rx) = sink();
        sink.on_reasoning("想一下").expect("send");
        sink.on_text("答案").expect("send");
        sink.on_batch_end().expect("flush");
        let events = drain(&rx);
        assert_eq!(events.len(), 2, "换类即切批");
        assert!(matches!(&events[0], UiEvent::Thinking(t) if t == "想一下"));
        assert!(matches!(&events[1], UiEvent::Text(t) if t == "答案"));
    }

    /// 空增量不产生事件（也不留下一个空批让后面的 flush 发空串）。
    #[test]
    fn empty_delta_and_empty_batch_send_nothing() {
        let (mut sink, rx) = sink();
        sink.on_text("").expect("send");
        sink.on_batch_end().expect("flush");
        sink.on_finished().expect("flush");
        assert!(drain(&rx).is_empty());
    }

    /// 越过字节上限就切批：大 chunk 不该让主线程等到整批解析完才有东西可画。
    #[test]
    fn oversized_batch_is_split_at_the_byte_limit() {
        let (mut sink, rx) = sink();
        let half = "x".repeat(MAX_BATCH_BYTES / 2 + 1);
        sink.on_text(&half).expect("send");
        sink.on_text(&half).expect("send");
        let events = drain(&rx);
        assert_eq!(events.len(), 1, "越过上限应先发前一批");
        assert!(matches!(&events[0], UiEvent::Text(t) if t.len() == half.len()));
        sink.on_batch_end().expect("flush");
        assert_eq!(drain(&rx).len(), 1, "剩下的那半批在边界发出");
    }
}
