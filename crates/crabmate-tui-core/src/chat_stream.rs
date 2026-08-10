//! `POST /chat/stream`：消费 AG-UI SSE，输出助手正文增量。

use std::io::{self, Write};

use crabmate_sse_protocol::{
    AgUiParseDispatch, SSE_PROTOCOL_VERSION, classify_ag_ui_sse_data, is_sse_done_sentinel,
    join_sse_data_lines, parse_sse_event_id,
};
use futures_util::StreamExt;
use reqwest::Response;
use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderValue};
use serde_json::{Value, json};

use crate::client::ServeClient;
use crate::error::TermError;

/// 一轮 `/chat/stream` 结束后的摘要。
#[derive(Debug, Clone, Default)]
pub struct ChatStreamOutcome {
    pub conversation_id: Option<String>,
    pub last_event_id: u64,
}

/// 运行一轮流式对话：正文写到 `out`，思维链等写到 `err`。
pub async fn run_chat_stream(
    client: &ServeClient,
    message: &str,
    conversation_id: Option<&str>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<ChatStreamOutcome, TermError> {
    let resp = post_chat_stream(client, message, conversation_id).await?;
    let mut outcome = ChatStreamOutcome {
        conversation_id: conversation_id_from_headers(&resp),
        ..ChatStreamOutcome::default()
    };
    consume_sse_response(resp, &mut outcome, out, err).await?;
    let _ = out.flush();
    let _ = err.flush();
    Ok(outcome)
}

async fn post_chat_stream(
    client: &ServeClient,
    message: &str,
    conversation_id: Option<&str>,
) -> Result<Response, TermError> {
    let url = client.url("/chat/stream")?;
    let body = chat_stream_body(message, conversation_id);
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

fn chat_stream_body(message: &str, conversation_id: Option<&str>) -> Value {
    let mut body = json!({
        "message": message,
        "client_sse_protocol": SSE_PROTOCOL_VERSION,
    });
    if let Some(cid) = conversation_id.map(str::trim).filter(|s| !s.is_empty()) {
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
    resp: Response,
    outcome: &mut ChatStreamOutcome,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<(), TermError> {
    let mut buffer = String::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| TermError::Stream(e.to_string()))?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        drain_sse_buffer(&mut buffer, outcome, out, err)?;
    }
    flush_sse_tail(&mut buffer, outcome, out, err)
}

fn flush_sse_tail(
    buffer: &mut String,
    outcome: &mut ChatStreamOutcome,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<(), TermError> {
    if buffer.trim().is_empty() {
        return Ok(());
    }
    if !buffer.ends_with("\n\n") {
        buffer.push_str("\n\n");
    }
    drain_sse_buffer(buffer, outcome, out, err)
}

fn drain_sse_buffer(
    buffer: &mut String,
    outcome: &mut ChatStreamOutcome,
    out: &mut dyn Write,
    err: &mut dyn Write,
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
        handle_sse_data(&data, out, err)?;
    }
    Ok(())
}

fn handle_sse_data(data: &str, out: &mut dyn Write, err: &mut dyn Write) -> Result<(), TermError> {
    if is_sse_done_sentinel(data) {
        return Ok(());
    }
    for line in data.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if handle_ag_ui_line(line, out, err)? {
            continue;
        }
        match classify_ag_ui_sse_data(line) {
            AgUiParseDispatch::Plain => write_out(out, line)?,
            AgUiParseDispatch::Handled | AgUiParseDispatch::StreamEnded => {}
        }
    }
    Ok(())
}

/// 若为带 `type` 的 AG-UI JSON 则处理并返回 `true`。
fn handle_ag_ui_line(
    line: &str,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<bool, TermError> {
    let Ok(val) = serde_json::from_str::<Value>(line) else {
        return Ok(false);
    };
    let Some(t) = val.get("type").and_then(|x| x.as_str()) else {
        return Ok(false);
    };
    match t {
        "TEXT_MESSAGE_CONTENT" => write_delta_field(&val, out)?,
        "REASONING_MESSAGE_CONTENT" => {
            if let Some(delta) = val.get("delta").and_then(|d| d.as_str()) {
                let _ = write!(err, "{delta}");
            }
        }
        "RUN_FINISHED" => {}
        "RUN_ERROR" => return Err(run_error_from_value(&val)),
        "CUSTOM" => check_command_approval(&val)?,
        _ => {}
    }
    Ok(true)
}

fn write_delta_field(val: &Value, out: &mut dyn Write) -> Result<(), TermError> {
    if let Some(delta) = val.get("delta").and_then(|d| d.as_str()) {
        write_out(out, delta)?;
    }
    Ok(())
}

fn run_error_from_value(val: &Value) -> TermError {
    let msg = val
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
        .unwrap_or("RUN_ERROR");
    TermError::RunError(msg.to_string())
}

fn check_command_approval(val: &Value) -> Result<(), TermError> {
    if val.get("customType").and_then(|n| n.as_str()) != Some("command_approval") {
        return Ok(());
    }
    let cmd = val
        .pointer("/data/command")
        .and_then(|c| c.as_str())
        .unwrap_or("?");
    Err(TermError::ApprovalRequired(cmd.to_string()))
}

fn write_out(out: &mut dyn Write, s: &str) -> Result<(), TermError> {
    out.write_all(s.as_bytes())
        .map_err(|e| TermError::Message(format!("stdout write failed: {e}")))?;
    let _ = io::Write::flush(out);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_text_message_content() {
        let data = r#"{"type":"TEXT_MESSAGE_CONTENT","delta":"你好"}"#;
        let mut out = Vec::new();
        let mut err = Vec::new();
        handle_sse_data(data, &mut out, &mut err).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "你好");
    }

    #[test]
    fn approval_custom_errors() {
        let data = r#"{"type":"CUSTOM","customType":"command_approval","data":{"command":"rm"}}"#;
        let mut out = Vec::new();
        let mut err = Vec::new();
        let e = handle_sse_data(data, &mut out, &mut err).unwrap_err();
        assert!(matches!(e, TermError::ApprovalRequired(_)));
    }
}
