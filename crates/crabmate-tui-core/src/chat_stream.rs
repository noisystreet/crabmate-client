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
use tokio::sync::watch;

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
    /// 用户 Ctrl+C 打断（已尽力让 serve 停掉该回合）。
    pub cancelled_by_user: bool,
    /// cancel 请求是否送达（`cancelled_by_user=true` 且此字段为 `false` 时 job 可能仍在跑，可续传）。
    pub cancel_acknowledged: bool,
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

/// `stream_resume` 续流点：serve 侧仍在跑的 job + 本端已消费到的 SSE 序号。
#[derive(Debug, Clone, Copy)]
pub struct StreamResume {
    pub job_id: u64,
    pub after_seq: u64,
}

/// `run_chat_stream` 入参。
#[derive(Debug, Clone, Copy)]
pub struct ChatStreamArgs<'a> {
    pub message: &'a str,
    pub conversation_id: Option<&'a str>,
    pub approval_session_id: &'a str,
    /// 有值时随请求体发送 `client_llm`；`None` / 全空白等价于不发送。
    pub client_llm: Option<ClientLlm<'a>>,
    /// agent role id（`agent_role`；缺省用 serve 默认）。
    pub agent_role: Option<&'a str>,
    /// 会话模式（`session_mode`：ask / plan / act；缺省用 serve 默认）。
    pub session_mode: Option<&'a str>,
    /// 续传已中断回合（`stream_resume:{job_id, after_seq}`）。
    pub stream_resume: Option<StreamResume>,
}

/// `run_chat_stream_sink` 的完整请求参数（owned；供后台任务使用，无借用生命周期）。
#[derive(Debug, Clone, Default)]
pub struct ChatStreamOptions {
    pub message: String,
    pub approval_session_id: String,
    pub conversation_id: Option<String>,
    pub client_llm: Option<ClientLlmFields>,
    pub agent_role: Option<String>,
    pub session_mode: Option<String>,
    pub stream_resume: Option<StreamResume>,
}

/// `client_llm` 覆盖的 owned 字段；空白值不发送（规则与 [`ClientLlm`] 一致）。
#[derive(Debug, Clone, Default)]
pub struct ClientLlmFields {
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub api_base: Option<String>,
}

impl From<&ChatStreamArgs<'_>> for ChatStreamOptions {
    fn from(a: &ChatStreamArgs<'_>) -> Self {
        Self {
            message: a.message.to_string(),
            approval_session_id: a.approval_session_id.to_string(),
            conversation_id: a.conversation_id.map(str::to_string),
            client_llm: a.client_llm.map(|cl| ClientLlmFields {
                api_key: cl.api_key.map(str::to_string),
                model: cl.model.map(str::to_string),
                api_base: cl.api_base.map(str::to_string),
            }),
            agent_role: a.agent_role.map(str::to_string),
            session_mode: a.session_mode.map(str::to_string),
            stream_resume: a.stream_resume,
        }
    }
}

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
    /// 流收尾（flush 等）。
    fn on_finished(&mut self) -> Result<(), TermError> {
        Ok(())
    }
}

/// SSE 消费期间的外部取消句柄（全屏 TUI 用；文本模式仍由内部 `ctrl_c` 信号驱动）。
///
/// 计数语义对齐文本模式的 Ctrl+C：第 1 次 = 取消当前回合（job 已知则
/// `POST /chat/stream/{job}/cancel`；cancel 未送达保留续流断点）；
/// 第 2 次 = 强退（等同二次 Ctrl+C）。
#[derive(Debug, Clone)]
pub struct StreamCancel {
    tx: watch::Sender<u8>,
    rx: watch::Receiver<u8>,
}

impl Default for StreamCancel {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamCancel {
    #[must_use]
    pub fn new() -> Self {
        let (tx, rx) = watch::channel(0);
        Self { tx, rx }
    }

    /// 请求取消当前回合；第 2 次调用 = 强退。
    pub fn cancel(&self) {
        let n = *self.tx.borrow();
        let _ = self.tx.send(n.saturating_add(1));
    }
}

/// 运行一轮流式对话：正文写到 `out`，思维链等写到 `err`。
///
/// 文本模式入口：内部经 `TextStreamSink` 委托 [`run_chat_stream_sink`]，
/// 保持历史 stdout/stderr 行为与 Ctrl+C 语义不变。
pub async fn run_chat_stream(
    client: &ServeClient,
    args: ChatStreamArgs<'_>,
    out: &mut dyn Write,
    err: &mut dyn Write,
    approval: &mut dyn ApprovalGate,
) -> Result<ChatStreamOutcome, TermError> {
    let opts = ChatStreamOptions::from(&args);
    let mut sink = TextStreamSink { out, err };
    run_chat_stream_sink(client, &opts, &mut sink, approval, None).await
}

/// 事件回调版：文本/思维增量经 `sink` 送达；取消由外部 `cancel` 触发。
///
/// - `cancel = None`：沿用文本模式语义（SSE 循环内监听 `ctrl_c`，一次=取消、二次=强退）。
/// - `cancel = Some(&token)`：由调用方（如全屏 TUI 事件循环）调 `StreamCancel::cancel()`
///   驱动，全屏 raw mode 下 ^C 是按键事件、不会产生 SIGINT。
pub async fn run_chat_stream_sink(
    client: &ServeClient,
    opts: &ChatStreamOptions,
    sink: &mut dyn StreamSink,
    approval: &mut dyn ApprovalGate,
    cancel: Option<&StreamCancel>,
) -> Result<ChatStreamOutcome, TermError> {
    let resp = post_chat_stream(client, opts).await?;
    let mut outcome = ChatStreamOutcome {
        conversation_id: conversation_id_from_headers(&resp)
            .or_else(|| opts.conversation_id.clone()),
        job_id: job_id_from_headers(&resp),
        ..ChatStreamOutcome::default()
    };
    consume_sse_response(client, opts, resp, &mut outcome, sink, approval, cancel).await?;
    sink.on_finished()?;
    Ok(outcome)
}

/// 文本模式 sink：正文 → stdout，思维链/系统行 → stderr（历史行为）。
struct TextStreamSink<'a> {
    out: &'a mut dyn Write,
    err: &'a mut dyn Write,
}

impl StreamSink for TextStreamSink<'_> {
    fn on_text(&mut self, delta: &str) -> Result<(), TermError> {
        write_out(self.out, delta)
    }

    fn on_reasoning(&mut self, delta: &str) -> Result<(), TermError> {
        if !delta.is_empty() {
            let _ = self.err.write_all(delta.as_bytes());
        }
        Ok(())
    }

    fn on_system(&mut self, line: &str) -> Result<(), TermError> {
        if !line.is_empty() {
            let _ = writeln!(self.err, "{line}");
        }
        Ok(())
    }

    fn on_finished(&mut self) -> Result<(), TermError> {
        let _ = self.out.flush();
        let _ = self.err.flush();
        Ok(())
    }
}

async fn post_chat_stream(
    client: &ServeClient,
    opts: &ChatStreamOptions,
) -> Result<Response, TermError> {
    let url = client.url("/chat/stream")?;
    let body = chat_stream_body(opts);
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

fn chat_stream_body(opts: &ChatStreamOptions) -> Value {
    let mut body = build_chat_stream_core_body(ChatStreamCoreFields {
        message: &opts.message,
        client_sse_protocol: SSE_PROTOCOL_VERSION,
        approval_session_id: Some(&opts.approval_session_id),
        conversation_id: opts.conversation_id.as_deref(),
    });
    if let Some(cl) = opts.client_llm.as_ref()
        && let Some(obj) = client_llm_json(cl)
    {
        body["client_llm"] = obj;
    }
    if let Some(map) = body.as_object_mut() {
        insert_trimmed(map, "agent_role", opts.agent_role.as_deref());
        insert_trimmed(map, "session_mode", opts.session_mode.as_deref());
    }
    if let Some(r) = opts.stream_resume {
        body["stream_resume"] = serde_json::json!({
            "job_id": r.job_id,
            "after_seq": r.after_seq,
        });
    }
    body
}

/// 仅含非空（trim 后）字段的 `client_llm` 对象；全空返回 `None`（不发送整块）。
fn client_llm_json(llm: &ClientLlmFields) -> Option<Value> {
    let mut map = serde_json::Map::new();
    insert_trimmed(&mut map, "api_base", llm.api_base.as_deref());
    insert_trimmed(&mut map, "model", llm.model.as_deref());
    insert_trimmed(&mut map, "api_key", llm.api_key.as_deref());
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

/// 流读取失败：已拿到 job 句柄时带出续流点（供调用方 `/resume`）。
fn stream_read_error(outcome: &ChatStreamOutcome, cause: &str) -> TermError {
    match outcome.job_id {
        Some(job_id) => TermError::InterruptedStream {
            job_id,
            after_seq: outcome.last_event_id,
            cause: cause.to_string(),
        },
        None => TermError::Stream(cause.to_string()),
    }
}

async fn consume_sse_response(
    client: &ServeClient,
    opts: &ChatStreamOptions,
    resp: Response,
    outcome: &mut ChatStreamOutcome,
    sink: &mut dyn StreamSink,
    approval: &mut dyn ApprovalGate,
    cancel: Option<&StreamCancel>,
) -> Result<(), TermError> {
    let mut buffer = String::new();
    let mut stream = resp.bytes_stream();
    loop {
        let chunk = tokio::select! {
            biased;
            _ = wait_for_cancel(cancel) => {
                if cancel_current_run(client, outcome, sink, cancel).await? {
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
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => return Err(stream_read_error(outcome, &e.to_string())),
        };
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        drain_sse_buffer(client, opts, &mut buffer, outcome, sink, approval).await?;
    }
    flush_sse_tail(client, opts, &mut buffer, outcome, sink, approval).await
}

/// 等待取消源：`cancel = None`（文本模式）监听 `ctrl_c` 信号；
/// `Some`（全屏 TUI）等待 [`StreamCancel`] 计数 ≥ 1。
async fn wait_for_cancel(cancel: Option<&StreamCancel>) {
    match cancel {
        None => {
            let _ = tokio::signal::ctrl_c().await;
        }
        Some(c) => {
            let mut rx = c.rx.clone();
            while *rx.borrow_and_update() < 1 && rx.changed().await.is_ok() {}
        }
    }
}

/// Ctrl+C / 外部取消：job_id 已知时先 `POST /chat/stream/{job}/cancel` 让 serve 停掉回合。
///
/// 返回 `Ok(true)` = 已取消并干净收尾；`Ok(false)` = 无 job_id（旧 serve），
/// 调用方按原中断语义处理。取消请求进行中再次触发（文本二次 Ctrl+C /
/// [`StreamCancel`] 计数 ≥ 2）= 强退（130 / `TermError::Interrupted`）。
async fn cancel_current_run(
    client: &ServeClient,
    outcome: &mut ChatStreamOutcome,
    sink: &mut dyn StreamSink,
    cancel: Option<&StreamCancel>,
) -> Result<bool, TermError> {
    let Some(job_id) = outcome.job_id else {
        return Ok(false);
    };
    if cancel.is_some_and(|c| *c.rx.borrow() >= 2) {
        return Err(TermError::Interrupted);
    }
    let force_quit = async {
        match cancel {
            None => {
                let _ = tokio::signal::ctrl_c().await;
            }
            Some(c) => {
                let mut rx = c.rx.clone();
                while *rx.borrow_and_update() < 2 && rx.changed().await.is_ok() {}
            }
        }
    };
    let cancel_result = tokio::select! {
        biased;
        _ = force_quit => return Err(TermError::Interrupted),
        r = client.cancel_chat_stream(job_id) => r,
    };
    match cancel_result {
        Ok(()) => {
            outcome.cancel_acknowledged = true;
            sink.on_system(&format!(
                "\n[crabmate-tui] stopped by Ctrl+C (job {job_id} cancelled on serve)"
            ))?;
        }
        Err(e) => {
            sink.on_system(&format!(
                "\n[crabmate-tui] cancel job {job_id} failed: {e}; 后台回合可能仍在 serve 上运行（可 /resume）"
            ))?;
        }
    }
    outcome.cancelled_by_user = true;
    Ok(true)
}

async fn flush_sse_tail(
    client: &ServeClient,
    opts: &ChatStreamOptions,
    buffer: &mut String,
    outcome: &mut ChatStreamOutcome,
    sink: &mut dyn StreamSink,
    approval: &mut dyn ApprovalGate,
) -> Result<(), TermError> {
    if buffer.trim().is_empty() {
        return Ok(());
    }
    if !buffer.ends_with("\n\n") {
        buffer.push_str("\n\n");
    }
    drain_sse_buffer(client, opts, buffer, outcome, sink, approval).await
}

async fn drain_sse_buffer(
    client: &ServeClient,
    opts: &ChatStreamOptions,
    buffer: &mut String,
    outcome: &mut ChatStreamOutcome,
    sink: &mut dyn StreamSink,
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
        handle_sse_data(client, opts, &data, sink, approval).await?;
    }
    Ok(())
}

async fn handle_sse_data(
    client: &ServeClient,
    opts: &ChatStreamOptions,
    data: &str,
    sink: &mut dyn StreamSink,
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
            LineAction::WriteOut(s) => sink.on_text(&s)?,
            LineAction::WriteErr(s) => sink.on_reasoning(&s)?,
            LineAction::Approve(req) => {
                resolve_approval(client, &opts.approval_session_id, req, approval).await?;
            }
            LineAction::Plain(s) => sink.on_text(&s)?,
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

    fn base_opts() -> ChatStreamOptions {
        ChatStreamOptions {
            message: "hi".into(),
            approval_session_id: "a1".into(),
            ..ChatStreamOptions::default()
        }
    }

    #[test]
    fn chat_body_includes_client_llm_fields() {
        let body = chat_stream_body(&ChatStreamOptions {
            conversation_id: Some("c1".into()),
            client_llm: Some(ClientLlmFields {
                api_key: Some(" sk-abc ".into()),
                model: Some("gpt-x".into()),
                api_base: None,
            }),
            ..base_opts()
        });
        assert_eq!(body["message"], "hi");
        assert_eq!(body["client_sse_protocol"], SSE_PROTOCOL_VERSION);
        assert_eq!(body["client_llm"]["api_key"], "sk-abc");
        assert_eq!(body["client_llm"]["model"], "gpt-x");
        assert!(body["client_llm"].get("api_base").is_none());
        assert!(body.get("stream_resume").is_none());
    }

    #[test]
    fn chat_body_omits_client_llm_when_empty() {
        let base = chat_stream_body(&base_opts());
        assert!(base.get("client_llm").is_none());

        let blank = chat_stream_body(&ChatStreamOptions {
            client_llm: Some(ClientLlmFields {
                api_key: Some("   ".into()),
                model: None,
                api_base: Some(String::new()),
            }),
            ..base_opts()
        });
        assert_eq!(blank, base);
    }

    #[test]
    fn chat_body_includes_stream_resume() {
        let body = chat_stream_body(&ChatStreamOptions {
            conversation_id: Some("c1".into()),
            stream_resume: Some(StreamResume {
                job_id: 42,
                after_seq: 7,
            }),
            ..base_opts()
        });
        assert_eq!(body["stream_resume"]["job_id"], 42);
        assert_eq!(body["stream_resume"]["after_seq"], 7);
        assert!(body.get("client_llm").is_none());
    }

    #[test]
    fn chat_body_includes_agent_role_and_session_mode() {
        let body = chat_stream_body(&ChatStreamOptions {
            agent_role: Some(" coder ".into()),
            session_mode: Some("plan".into()),
            ..base_opts()
        });
        assert_eq!(body["agent_role"], "coder");
        assert_eq!(body["session_mode"], "plan");
    }

    #[test]
    fn chat_body_omits_mode_and_role_when_unset() {
        let body = chat_stream_body(&ChatStreamOptions {
            session_mode: Some("   ".into()),
            ..base_opts()
        });
        assert!(body.get("agent_role").is_none());
        assert!(body.get("session_mode").is_none());
    }

    #[test]
    fn chat_body_default_omits_optional_blocks() {
        let body = chat_stream_body(&base_opts());
        assert!(body.get("client_llm").is_none());
        assert!(body.get("agent_role").is_none());
        assert!(body.get("session_mode").is_none());
        assert!(body.get("stream_resume").is_none());
    }

    #[test]
    fn stream_error_keeps_resume_point_when_job_known() {
        let outcome = ChatStreamOutcome {
            job_id: Some(9),
            last_event_id: 3,
            ..ChatStreamOutcome::default()
        };
        match stream_read_error(&outcome, "synthetic stream error") {
            TermError::InterruptedStream {
                job_id, after_seq, ..
            } => {
                assert_eq!(job_id, 9);
                assert_eq!(after_seq, 3);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parses_stream_job_id() {
        assert_eq!(parse_stream_job_id(" 42 "), Some(42));
        assert_eq!(parse_stream_job_id("12"), Some(12));
        assert_eq!(parse_stream_job_id("abc"), None);
        assert_eq!(parse_stream_job_id(""), None);
    }

    #[test]
    fn stream_cancel_counts_first_then_force_quit() {
        let c = StreamCancel::new();
        assert_eq!(*c.rx.borrow(), 0);
        c.cancel();
        assert_eq!(*c.rx.borrow(), 1);
        c.cancel();
        assert_eq!(*c.rx.borrow(), 2);
    }
}
