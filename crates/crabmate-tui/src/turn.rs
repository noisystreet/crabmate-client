//! 单轮对话：生成 approval_session_id 并跑 `/chat/stream`。

use std::io;

use anyhow::Result;
use crabmate_tui_core::{
    ApprovalGate, ChatStreamArgs, ChatStreamOutcome, ClientLlm, ServeClient,
    new_approval_session_id, run_chat_stream,
};

pub async fn run_turn(
    client: &ServeClient,
    message: &str,
    conversation_id: Option<&str>,
    client_llm: Option<ClientLlm<'_>>,
    approval: &mut dyn ApprovalGate,
) -> Result<ChatStreamOutcome> {
    let approval_session_id = new_approval_session_id();
    // 不跨 await 长期持有 StdoutLock/StderrLock（审批提示需自行写 stderr）。
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    let outcome = run_chat_stream(
        client,
        ChatStreamArgs {
            message,
            conversation_id,
            approval_session_id: &approval_session_id,
            client_llm,
        },
        &mut stdout,
        &mut stderr,
        approval,
    )
    .await?;
    Ok(outcome)
}
