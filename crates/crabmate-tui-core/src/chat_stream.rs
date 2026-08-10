//! `POST /chat/stream`：消费 AG-UI SSE，输出助手正文增量；流中处理命令审批。

use std::io::{self, Write};

use crabmate_sse_protocol::{
    AgUiParseDispatch, SSE_PROTOCOL_VERSION, classify_ag_ui_sse_data, is_sse_done_sentinel,
    join_sse_data_lines, parse_sse_event_id,
};
use futures_util::StreamExt;
use reqwest::Response;
use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderValue};
use serde_json::{Value, json};

use crate::approval::{
    ApprovalDecision, ApprovalGate, CommandApprovalRequest, parse_command_approval_data,
};
use crate::client::ServeClient;
use crate::error::TermError;

/// 一轮 `/chat/stream` 结束后的摘要。
#[derive(Debug, Clone, Default)]
pub struct ChatStreamOutcome {
    pub conversation_id: Option<String>,
    pub last_event_id: u64,
}

/// `run_chat_stream` 入参。
#[derive(Debug, Clone, Copy)]
pub struct ChatStreamArgs<'a> {
    pub message: &'a str,
    pub conversation_id: Option<&'a str>,
    pub approval_session_id: &'a str,
}

/// 运行一轮流式对话：正文写到 `out`，思维链等写到 `err`。
pub async fn run_chat_stream(
    client: &ServeClient,
    args: ChatStreamArgs<'_>,
    out: &mut dyn Write,
    err: &mut dyn Write,
    approval: &mut dyn ApprovalGate,
) -> Result<ChatStreamOutcome, TermError> {
    let resp = post_chat_stream(client, args).await?;
    let mut outcome = ChatStreamOutcome {
        conversation_id: conversation_id_from_headers(&resp)
            .or_else(|| args.conversation_id.map(str::to_string)),
        ..ChatStreamOutcome::default()
    };
    consume_sse_response(
        client,
        args.approval_session_id,
        resp,
        &mut outcome,
        out,
        err,
        approval,
    )
    .await?;
    let _ = out.flush();
    let _ = err.flush();
    Ok(outcome)
}

async fn post_chat_stream(
    client: &ServeClient,
    args: ChatStreamArgs<'_>,
) -> Result<Response, TermError> {
    let url = client.url("/chat/stream")?;
    let body = chat_stream_body(args);
    let mut headers = client.auth_headers()?;
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(ACCEPT, HeaderValue::from_static("text/event-stream"));
    let resp = client
        .http()
        .post(&url)
        .headers(headers)
        .json(&body)
        .send()
        .await?;
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let text = resp.text().await.unwrap_or_default();
    Err(TermError::Http {
        status: status.as_u16(),
        body: text.trim().chars().take(800).collect(),
    })
}

fn chat_stream_body(args: ChatStreamArgs<'_>) -> Value {
    let mut body = json!({
        "message": args.message,
        "client_sse_protocol": SSE_PROTOCOL_VERSION,
        "approval_session_id": args.approval_session_id,
    });
    if let Some(cid) = args
        .conversation_id
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        body["conversation_id"] = json!(cid);
    }
    body
}

fn conversation_id_from_headers(resp: &Response) -> Option<String> {
    resp.headers()
        .get("x-conversation-id")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

async fn consume_sse_response(
    client: &ServeClient,
    approval_session_id: &str,
    resp: Response,
    outcome: &mut ChatStreamOutcome,
    out: &mut dyn Write,
    err: &mut dyn Write,
    approval: &mut dyn ApprovalGate,
) -> Result<(), TermError> {
    let mut buffer = String::new();
    let mut stream = resp.bytes_stream();
    loop {
        let chunk = tokio::select! {
            biased;
            _ = tokio::signal::ctrl_c() => {
                return Err(TermError::Interrupted);
            }
            next = stream.next() => next,
        };
        let Some(chunk) = chunk else {
            break;
        };
        let chunk = chunk.map_err(|e| TermError::Stream(e.to_string()))?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        drain_sse_buffer(
            client,
            approval_session_id,
            &mut buffer,
            outcome,
            out,
            err,
            approval,
        )
        .await?;
    }
    flush_sse_tail(
        client,
        approval_session_id,
        &mut buffer,
        outcome,
        out,
        err,
        approval,
    )
    .await
}

async fn flush_sse_tail(
    client: &ServeClient,
    approval_session_id: &str,
    buffer: &mut String,
    outcome: &mut ChatStreamOutcome,
    out: &mut dyn Write,
    err: &mut dyn Write,
    approval: &mut dyn ApprovalGate,
) -> Result<(), TermError> {
    if buffer.trim().is_empty() {
        return Ok(());
    }
    if !buffer.ends_with("\n\n") {
        buffer.push_str("\n\n");
    }
    drain_sse_buffer(
        client,
        approval_session_id,
        buffer,
        outcome,
        out,
        err,
        approval,
    )
    .await
}

async fn drain_sse_buffer(
    client: &ServeClient,
    approval_session_id: &str,
    buffer: &mut String,
    outcome: &mut ChatStreamOutcome,
    out: &mut dyn Write,
    err: &mut dyn Write,
    approval: &mut dyn ApprovalGate,
) -> Result<(), TermError> {
    while let Some(idx) = buffer.find("\n\n") {
        let block = buffer[..idx].to_string();
        *buffer = buffer[idx + 2..].to_string();
        if block.trim().is_empty() {
            continue;
        }
        if let Some(id) = parse_sse_event_id(&block) {
            outcome.last_event_id = id;
        }
        let Some(data) = join_sse_data_lines(&block) else {
            continue;
        };
        handle_sse_data(client, approval_session_id, &data, out, err, approval).await?;
    }
    Ok(())
}

async fn handle_sse_data(
    client: &ServeClient,
    approval_session_id: &str,
    data: &str,
    out: &mut dyn Write,
    err: &mut dyn Write,
    approval: &mut dyn ApprovalGate,
) -> Result<(), TermError> {
    if is_sse_done_sentinel(data) {
        return Ok(());
    }
    for line in data.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match classify_line(line)? {
            LineAction::Skip => {}
            LineAction::WriteOut(s) => write_out(out, &s)?,
            LineAction::WriteErr(s) => {
                let _ = write!(err, "{s}");
            }
            LineAction::Approve(req) => {
                resolve_approval(client, approval_session_id, req, approval).await?;
            }
            LineAction::Plain(s) => write_out(out, &s)?,
        }
    }
    Ok(())
}

#[derive(Debug)]
enum LineAction {
    Skip,
    WriteOut(String),
    WriteErr(String),
    Approve(CommandApprovalRequest),
    Plain(String),
}

fn classify_line(line: &str) -> Result<LineAction, TermError> {
    if let Some(action) = classify_ag_ui_line(line)? {
        return Ok(action);
    }
    Ok(match classify_ag_ui_sse_data(line) {
        AgUiParseDispatch::Plain => LineAction::Plain(line.to_string()),
        AgUiParseDispatch::Handled | AgUiParseDispatch::StreamEnded => LineAction::Skip,
    })
}

fn classify_ag_ui_line(line: &str) -> Result<Option<LineAction>, TermError> {
    let Ok(val) = serde_json::from_str::<Value>(line) else {
        return Ok(None);
    };
    let Some(t) = val.get("type").and_then(|x| x.as_str()) else {
        return Ok(None);
    };
    Ok(Some(match t {
        "TEXT_MESSAGE_CONTENT" => LineAction::WriteOut(delta_string(&val)),
        "REASONING_MESSAGE_CONTENT" => LineAction::WriteErr(delta_string(&val)),
        "RUN_FINISHED" => LineAction::Skip,
        "RUN_ERROR" => return Err(run_error_from_value(&val)),
        "CUSTOM" => classify_custom(&val),
        _ => LineAction::Skip,
    }))
}

fn classify_custom(val: &Value) -> LineAction {
    if val.get("customType").and_then(|n| n.as_str()) != Some("command_approval") {
        return LineAction::Skip;
    }
    let data = val.get("data").cloned().unwrap_or(Value::Null);
    LineAction::Approve(parse_command_approval_data(&data))
}

fn delta_string(val: &Value) -> String {
    val.get("delta")
        .and_then(|d| d.as_str())
        .unwrap_or("")
        .to_string()
}

fn run_error_from_value(val: &Value) -> TermError {
    let msg = val
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
        .unwrap_or("RUN_ERROR");
    TermError::RunError(msg.to_string())
}

async fn resolve_approval(
    client: &ServeClient,
    approval_session_id: &str,
    req: CommandApprovalRequest,
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

fn write_out(out: &mut dyn Write, s: &str) -> Result<(), TermError> {
    if s.is_empty() {
        return Ok(());
    }
    out.write_all(s.as_bytes())
        .map_err(|e| TermError::Message(format!("stdout write failed: {e}")))?;
    let _ = io::Write::flush(out);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approval::{ApprovalDecision, AutoAllowOnce};

    struct CaptureGate {
        seen: Vec<CommandApprovalRequest>,
        decision: ApprovalDecision,
    }

    impl ApprovalGate for CaptureGate {
        fn decide(&mut self, req: &CommandApprovalRequest) -> Result<ApprovalDecision, TermError> {
            self.seen.push(req.clone());
            Ok(self.decision)
        }
    }

    #[test]
    fn extracts_text_message_content() {
        let data = r#"{"type":"TEXT_MESSAGE_CONTENT","delta":"你好"}"#;
        let mut out = Vec::new();
        match classify_line(data).unwrap() {
            LineAction::WriteOut(s) => write_out(&mut out, &s).unwrap(),
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(String::from_utf8(out).unwrap(), "你好");
    }

    #[test]
    fn classifies_command_approval() {
        let data = r#"{"type":"CUSTOM","customType":"command_approval","data":{"command":"rm","args":"-f"}}"#;
        match classify_line(data).unwrap() {
            LineAction::Approve(req) => {
                assert_eq!(req.command, "rm");
                assert_eq!(req.args, "-f");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn auto_allow_once_gate() {
        let mut g = AutoAllowOnce;
        let req = CommandApprovalRequest {
            command: "x".into(),
            args: String::new(),
            allowlist_key: None,
        };
        assert_eq!(g.decide(&req).unwrap(), ApprovalDecision::AllowOnce);
    }

    #[test]
    fn capture_gate_records() {
        let mut g = CaptureGate {
            seen: Vec::new(),
            decision: ApprovalDecision::Deny,
        };
        let req = CommandApprovalRequest {
            command: "df".into(),
            args: "-h".into(),
            allowlist_key: Some("df".into()),
        };
        assert_eq!(g.decide(&req).unwrap(), ApprovalDecision::Deny);
        assert_eq!(g.seen.len(), 1);
    }
}
