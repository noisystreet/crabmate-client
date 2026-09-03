//! `POST /chat/stream`：消费 AG-UI SSE，输出助手正文增量；流中处理命令审批。

use std::io::{self, Write};

use crabmate::cm_sse_protocol::{
    AgUiParseDispatch, SSE_PROTOCOL_VERSION, classify_ag_ui_sse_data, is_sse_done_sentinel,
    join_sse_data_lines, parse_sse_event_id,
};
use crabmate_client_api::{ChatStreamCoreFields, build_chat_stream_core_body};
use futures_util::StreamExt;
use reqwest::Response;
use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderValue};
use serde_json::Value;

use crate::approval::{
    ApprovalDecision, ApprovalGate, CommandApprovalRequest, parse_command_approval_data,
};
use crate::client::ServeClient;
use crate::error::TermError;

/// 一轮 `/chat/stream` 结束后的摘要。
#[derive(Debug, Clone, Default)]
pub struct ChatStreamOutcome {
    pub conversation_id: Option<String>,
    /// 响应头 `x-stream-job-id`（供 `POST /chat/stream/{job_id}/cancel` 与后续续传）。
    pub job_id: Option<u64>,
    /// 用户 Ctrl+C 打断并已尽力让 serve 停掉该回合。
    pub cancelled_by_user: bool,
    pub last_event_id: u64,
}

/// 随 `/chat/stream` 发送的 `client_llm` 覆盖（同 WASM UI「设置 → API 密钥/模型」的子集）。
///
/// 字段与请求体键一一对应；空白值不发送，全空时整块省略。
#[derive(Debug, Clone, Copy, Default)]
pub struct ClientLlm<'a> {
    /// 模型 API 密钥 → `client_llm.api_key`（serve 无服务端 `API_KEY` 时的客户端自带密钥）。
    pub api_key: Option<&'a str>,
    /// 模型名 → `client_llm.model`。
    pub model: Option<&'a str>,
    /// 模型供应商 base URL → `client_llm.api_base`。
    pub api_base: Option<&'a str>,
}

/// `run_chat_stream` 入参。
#[derive(Debug, Clone, Copy)]
pub struct ChatStreamArgs<'a> {
    pub message: &'a str,
    pub conversation_id: Option<&'a str>,
    pub approval_session_id: &'a str,
    /// 有值时随请求体发送 `client_llm`；`None` / 全空白等价于不发送。
    pub client_llm: Option<ClientLlm<'a>>,
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
        job_id: job_id_from_headers(&resp),
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
    let mut body = build_chat_stream_core_body(ChatStreamCoreFields {
        message: args.message,
        client_sse_protocol: SSE_PROTOCOL_VERSION,
        approval_session_id: Some(args.approval_session_id),
        conversation_id: args.conversation_id,
    });
    if let Some(cl) = args.client_llm
        && let Some(obj) = client_llm_json(cl)
    {
        body["client_llm"] = obj;
    }
    body
}

/// 仅含非空（trim 后）字段的 `client_llm` 对象；全空返回 `None`（不发送整块）。
fn client_llm_json(llm: ClientLlm<'_>) -> Option<Value> {
    let mut map = serde_json::Map::new();
    insert_trimmed(&mut map, "api_base", llm.api_base);
    insert_trimmed(&mut map, "model", llm.model);
    insert_trimmed(&mut map, "api_key", llm.api_key);
    if map.is_empty() {
        None
    } else {
        Some(Value::Object(map))
    }
}

fn insert_trimmed(map: &mut serde_json::Map<String, Value>, key: &str, value: Option<&str>) {
    if let Some(v) = value.map(str::trim).filter(|s| !s.is_empty()) {
        map.insert(key.into(), Value::String(v.to_string()));
    }
}

fn conversation_id_from_headers(resp: &Response) -> Option<String> {
    resp.headers()
        .get("x-conversation-id")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// 响应头 `x-stream-job-id`：SSE 事件的 job 归属（供 cancel / 续传）。
fn job_id_from_headers(resp: &Response) -> Option<u64> {
    resp.headers()
        .get("x-stream-job-id")
        .and_then(|v| v.to_str().ok())
        .and_then(parse_stream_job_id)
}

/// 解析 `x-stream-job-id` 数值；空 / 非数字返回 `None`。
fn parse_stream_job_id(raw: &str) -> Option<u64> {
    raw.trim().parse::<u64>().ok()
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
                if interrupt_stream(client, outcome, err).await? {
                    return Ok(());
                }
                // 旧 serve 无 `x-stream-job-id`：无法 cancel，保持原中断语义。
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

/// Ctrl+C：job_id 已知时先 `POST /chat/stream/{job}/cancel` 让 serve 停掉回合。
///
/// 返回 `Ok(true)` = 已取消并干净收尾；`Ok(false)` = 无 job_id（旧 serve），
/// 调用方按原中断语义处理。取消请求进行中再按一次 Ctrl+C = 强退（130）。
async fn interrupt_stream(
    client: &ServeClient,
    outcome: &mut ChatStreamOutcome,
    err: &mut dyn Write,
) -> Result<bool, TermError> {
    let Some(job_id) = outcome.job_id else {
        return Ok(false);
    };
    let cancel = tokio::select! {
        biased;
        _ = tokio::signal::ctrl_c() => return Err(TermError::Interrupted),
        r = client.cancel_chat_stream(job_id) => r,
    };
    match cancel {
        Ok(()) => {
            let _ = writeln!(
                err,
                "\n[crabmate-tui] stopped by Ctrl+C (job {job_id} cancelled on serve)"
            );
        }
        Err(e) => {
            let _ = writeln!(
                err,
                "\n[crabmate-tui] cancel job {job_id} failed: {e}; 后台回合可能仍在 serve 上运行"
            );
        }
    }
    outcome.cancelled_by_user = true;
    Ok(true)
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

    #[test]
    fn chat_body_includes_client_llm_fields() {
        let body = chat_stream_body(ChatStreamArgs {
            message: "hi",
            conversation_id: Some("c1"),
            approval_session_id: "a1",
            client_llm: Some(ClientLlm {
                api_key: Some(" sk-abc "),
                model: Some("gpt-x"),
                api_base: None,
            }),
        });
        assert_eq!(body["message"], "hi");
        assert_eq!(body["client_sse_protocol"], SSE_PROTOCOL_VERSION);
        assert_eq!(body["client_llm"]["api_key"], "sk-abc");
        assert_eq!(body["client_llm"]["model"], "gpt-x");
        assert!(body["client_llm"].get("api_base").is_none());
    }

    #[test]
    fn chat_body_omits_client_llm_when_empty() {
        let base = chat_stream_body(ChatStreamArgs {
            message: "hi",
            conversation_id: None,
            approval_session_id: "a1",
            client_llm: None,
        });
        assert!(base.get("client_llm").is_none());

        let blank = chat_stream_body(ChatStreamArgs {
            message: "hi",
            conversation_id: None,
            approval_session_id: "a1",
            client_llm: Some(ClientLlm {
                api_key: Some("   "),
                model: None,
                api_base: Some(""),
            }),
        });
        assert_eq!(blank, base);
    }

    #[test]
    fn parses_stream_job_id() {
        assert_eq!(parse_stream_job_id(" 42 "), Some(42));
        assert_eq!(parse_stream_job_id("12"), Some(12));
        assert_eq!(parse_stream_job_id("abc"), None);
        assert_eq!(parse_stream_job_id(""), None);
    }
}
