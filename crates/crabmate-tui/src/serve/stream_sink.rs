//! 流增量 → 事件回调的 trait（文本模式写 stdout/stderr，全屏 TUI 发 UI 事件）。
//!
//! 从 `chat_stream.rs` 搬出：那里贴着 920 行门禁，而这块是纯接口声明 ——
//! 加一个事件（issue #424 的 `on_clarify`）就要在主线文件里占十来行，
//! 而它跟「怎么发请求」毫无关系。`serve/mod.rs` 已 `pub use`，调用方不用改路径。

use crate::serve::error::TermError;

/// 流增量 → 事件回调。文本模式（`chat`/`repl`）用写回 stdout/stderr 的实现；
/// 全屏 `tui` 用一个发往 UI 事件通道的实现。
pub trait StreamSink {
    /// 助手正文增量（`TEXT_MESSAGE_CONTENT` / 未分类 Plain 行）。
    fn on_text(&mut self, delta: &str) -> Result<(), TermError>;
    /// 思维链增量（`REASONING_MESSAGE_CONTENT`）。
    fn on_reasoning(&mut self, delta: &str) -> Result<(), TermError>;
    /// 系统行（如 Ctrl+C 停止提示）；文本模式写 stderr，全屏显示为 transcript 系统行。
    fn on_system(&mut self, _line: &str) -> Result<(), TermError> {
        Ok(())
    }
    /// 上下文用量快照（`tiktokenPromptTokens`（`RUN_FINISHED` / `conversation_saved`））：全屏 TUI 用它更新状态行
    /// 的上下文段与会话内水位；文本模式（`chat` / `repl`）没有状态行，默认忽略。
    fn on_prompt_tokens(
        &mut self,
        _snap: &crabmate_client_api::prompt_tokens::TiktokenPromptTokensSnapshot,
    ) -> Result<(), TermError> {
        Ok(())
    }
    /// 上下文用量**回落**（`timeline_log` 的 `kind=context_trim`，携带裁剪后的 `used_input_tokens`）：
    /// 全屏 TUI 用它把会话内水位从旧的峰值降下来；文本模式无状态行，默认忽略。
    fn on_context_trim(&mut self, _after: u32) -> Result<(), TermError> {
        Ok(())
    }
    /// 本轮注入的 skill（`timeline_log` 的 `kind=context_inject` 且 `detail.skills` 非空）：
    /// 全屏 TUI 落一行技能行（对齐 Desktop 的 skill 卡）；文本模式默认忽略——
    /// 它没有卡片，过程行已经能把这一轮讲清楚。
    fn on_skill_inject(
        &mut self,
        _info: &crabmate_client_api::timeline_skill::SkillInjectInfo,
    ) -> Result<(), TermError> {
        Ok(())
    }
    /// 澄清问卷题面（issue #424）。全屏 TUI 据此开表单；文本 / REPL 模式默认忽略 ——
    /// 答案要随**下一条**请求回传，而 REPL 每次输入就是新的一条，没有「下一轮」可承载。
    fn on_clarify(
        &mut self,
        _info: &crabmate_client_api::sse_dispatch::ClarificationQuestionnaireInfo,
    ) -> Result<(), TermError> {
        Ok(())
    }
    /// 工具开始（AG-UI `TOOL_CALL_START`，serve v2 协议）。文本模式默认忽略；
    /// 全屏 TUI 用它渲染"工具行摘要"的开始态。
    ///
    /// `note` = 事件自带的 `summary` / `argsPreview`（「要执行什么」）。
    fn on_tool_start(
        &mut self,
        _tool_call_id: &str,
        _name: &str,
        _note: Option<&str>,
    ) -> Result<(), TermError> {
        Ok(())
    }
    /// `TOOL_CALL_ARGS`：参数全量帧。与 `on_tool_start` 分开到达，供实现方在参数补全后
    /// 刷新工具行文本；默认忽略（文本模式无症状）。
    fn on_tool_args(&mut self, _tool_call_id: &str, _args: &str) -> Result<(), TermError> {
        Ok(())
    }
    /// 工具结果（`TOOL_CALL_RESULT`；非 partial 收尾帧）。`ok=None` = 无结果标记；
    /// `note` 为 serve 摘要（`metadata.summary`），缺省回退输出内容截断；
    /// `detail` 是输出**全文**（`ToolResultInfo.output` 原样；空输出为 `None`）。
    ///
    /// 全文与摘要同一帧到达，所以两者走**同一个**回调而不是再开一个 `on_tool_detail`：
    /// 分两个回调就得保证「详情先于结束」的到达顺序，而结束回调正是消费
    /// 「工具调用 id → 行号」映射的那一个，顺序错了详情就永远落不到行上。
    fn on_tool_end(
        &mut self,
        _tool_call_id: &str,
        _name: &str,
        _ok: Option<bool>,
        _note: Option<&str>,
        _detail: Option<&str>,
    ) -> Result<(), TermError> {
        Ok(())
    }
    /// 流收尾（flush 等）。
    fn on_finished(&mut self) -> Result<(), TermError> {
        Ok(())
    }
}
