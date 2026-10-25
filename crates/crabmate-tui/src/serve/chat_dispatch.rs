//! 单行 SSE 动作的副作用落地：审批要异步回传，其余只推进 sink。
//!
//! 独立成模块的两个理由：`chat_stream.rs` 已贴着 920 行门禁（与 `chat_classify.rs` 同一拆法，
//! 分类=纯解析、落地=副作用，本来也是两件事）；且一个 match 里混「异步回传」与七种
//! 「同步写 sink」时，CCN 会随事件类型增加线性上涨 —— 拆成两半各自守着一个小 match。

use crate::serve::approval::{ApprovalDecision, ApprovalGate, CommandApprovalData};
use crate::serve::chat_classify::LineAction;
use crate::serve::chat_stream::StreamSink;
use crate::serve::client::ServeClient;
use crate::serve::error::TermError;

/// 单行动作的副作用分派：审批分支要 `await`，其余走 [`dispatch_sink_action`]。
pub(crate) async fn dispatch_line_action(
    client: &ServeClient,
    approval_session_id: &str,
    action: LineAction,
    sink: &mut dyn StreamSink,
    approval: &mut dyn ApprovalGate,
) -> Result<(), TermError> {
    match action {
        LineAction::Approve(req) => {
            resolve_approval(client, approval_session_id, req, approval).await
        }
        other => dispatch_sink_action(other, sink),
    }
}

/// 非审批行：纯 sink 调用（`WriteOut` 与 `Plain` 都写正文，合并一支）。
fn dispatch_sink_action(action: LineAction, sink: &mut dyn StreamSink) -> Result<(), TermError> {
    match action {
        LineAction::Skip => {}
        LineAction::PromptTokens(snap) => sink.on_prompt_tokens(&snap)?,
        LineAction::ContextTrim { after } => sink.on_context_trim(after)?,
        LineAction::WriteOut(s) | LineAction::Plain(s) => sink.on_text(&s)?,
        LineAction::WriteErr(s) => sink.on_reasoning(&s)?,
        LineAction::System(s) => sink.on_system(&s)?,
        LineAction::ToolStart {
            tool_call_id,
            name,
            note,
        } => sink.on_tool_start(&tool_call_id, &name, note.as_deref())?,
        LineAction::ToolArgs { tool_call_id, args } => sink.on_tool_args(&tool_call_id, &args)?,
        LineAction::ToolResult {
            tool_call_id,
            name,
            ok,
            note,
        } => sink.on_tool_end(&tool_call_id, &name, ok, note.as_deref())?,
        // 入口已拦下审批分支；这支只为 match 穷尽，真走到这里就是分派逻辑错了。
        LineAction::Approve(_) => debug_assert!(false, "审批分支应由 dispatch_line_action 处理"),
    }
    Ok(())
}

/// 单行审批决策：把选择回传给 serve；读决策失败时尽力 deny，避免审批会话悬挂。
async fn resolve_approval(
    client: &ServeClient,
    approval_session_id: &str,
    req: CommandApprovalData,
    approval: &mut dyn ApprovalGate,
) -> Result<(), TermError> {
    match approval.decide(&req) {
        Ok(decision) => {
            client
                .submit_chat_approval(approval_session_id, decision)
                .await
        }
        Err(e) => {
            // 读决策失败 / 中断时尽量 deny，避免 serve 侧审批会话悬挂。
            let _ = client
                .submit_chat_approval(approval_session_id, ApprovalDecision::Deny)
                .await;
            Err(e)
        }
    }
}
