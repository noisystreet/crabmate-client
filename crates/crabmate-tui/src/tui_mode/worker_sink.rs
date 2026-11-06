//! 全屏模式 sink：把流事件发往 UI 事件通道（经 `LaneTx` 标注来源 lane）。
//!
//! 从 `worker.rs` 抽出：那里贴着 920 行文件门禁，而这块只是「流事件 → UI 事件」的纯转发，
//! 与 worker 的 job 调度无关。与 `worker_cancel.rs` 同一拆法（`#[path]` 子模块，
//! 不为一个结构体新开一层目录）。

use crabmate_client_api::prompt_tokens::TiktokenPromptTokensSnapshot;
use crabmate_client_api::timeline_skill::SkillInjectInfo;

use crate::serve::StreamSink;
use crate::serve::TermError;

use super::{LaneTx, UiEvent};

/// 全屏模式 sink：把流事件发往 UI 事件通道（经 [`LaneTx`] 标注来源 lane）。
pub(super) struct UiSink {
    /// 该 lane 的出站通道；构造点在 `worker` 的回合调度处。
    pub(super) out: LaneTx,
}

impl StreamSink for UiSink {
    fn on_text(&mut self, delta: &str) -> Result<(), TermError> {
        if !delta.is_empty() {
            self.out.send(UiEvent::Text(delta.to_string()))?;
        }
        Ok(())
    }

    fn on_reasoning(&mut self, delta: &str) -> Result<(), TermError> {
        if !delta.is_empty() {
            self.out.send(UiEvent::Thinking(delta.to_string()))?;
        }
        Ok(())
    }

    fn on_system(&mut self, line: &str) -> Result<(), TermError> {
        if !line.is_empty() {
            self.out.send(UiEvent::System(line.to_string()))?;
        }
        Ok(())
    }

    fn on_context_trim(&mut self, after: u32) -> Result<(), TermError> {
        self.out.send(UiEvent::ContextTrim(after))?;
        Ok(())
    }

    fn on_skill_inject(&mut self, info: &SkillInjectInfo) -> Result<(), TermError> {
        self.out.send(UiEvent::SkillInject(info.clone()))
    }

    fn on_clarify(
        &mut self,
        info: &crabmate_client_api::sse_dispatch::ClarificationQuestionnaireInfo,
    ) -> Result<(), TermError> {
        self.out.send(UiEvent::Clarify(info.clone()))
    }

    fn on_prompt_tokens(&mut self, snap: &TiktokenPromptTokensSnapshot) -> Result<(), TermError> {
        self.out.send(UiEvent::PromptTokens(Box::new(snap.clone())))
    }

    fn on_tool_start(
        &mut self,
        tool_call_id: &str,
        name: &str,
        note: Option<&str>,
    ) -> Result<(), TermError> {
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
    ) -> Result<(), TermError> {
        self.out.send(UiEvent::ToolEnd {
            tool_call_id: tool_call_id.to_string(),
            name: name.to_string(),
            ok,
            note: note.map(str::to_string),
        })
    }
}
