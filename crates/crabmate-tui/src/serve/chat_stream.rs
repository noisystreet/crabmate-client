//! `POST /chat/stream`：消费 AG-UI SSE，输出助手正文增量；流中处理命令审批。

use std::io::{self, Write};
use std::time::Duration;

use crabmate::cm_sse_protocol::{
    SSE_PROTOCOL_VERSION, is_sse_done_sentinel, join_sse_data_lines, parse_sse_event_id,
};
use crabmate_client_api::{
    ChatStreamCoreFields, build_chat_stream_core_body, insert_trimmed_str,
    llm_context_tokens_for_chat_body, paths, temperature_for_chat_body,
};
use futures_util::StreamExt;
use reqwest::Response;
use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderValue};
use serde_json::Value;
use tokio::sync::watch;

use crate::serve::approval::ApprovalGate;
use crate::serve::chat_classify::classify_line;
use crate::serve::chat_dispatch::dispatch_line_action;
use crate::serve::client::ServeClient;
use crate::serve::error::TermError;
use crate::serve::stream_sink::StreamSink;

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
    /// 权限等级意图（`permission_level`；缺省 = 不表达意图，沿用会话持久值）。
    pub permission_level: Option<&'a str>,
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
    /// 权限等级意图；与 `agent_role` / `session_mode` 同为「本回合显式表达」的列。
    pub permission_level: Option<String>,
    /// 顶层 `temperature`（f64；仅 `Some` 且有限且 `0.0..=2.0` 时随 body 发送，
    /// 语义对齐 Desktop `chat_temperature_override_from_storage`）。
    pub temperature: Option<f64>,
    /// 顶层 `readonly_tool_ttl_cache_secs`（只读工具 TTL 缓存关闭时发 `0`；
    /// 跟随 server 时 `None` 不发，语义对齐 Desktop `readonly_tool_ttl_cache_secs_for_chat_body`）。
    pub readonly_tool_ttl_cache_secs: Option<u64>,
    pub stream_resume: Option<StreamResume>,
    /// 澄清问卷的逐题作答（issue #424）：`{ questionnaire_id, answers }`。
    ///
    /// 只有「提交问卷」那一条请求带它 —— 用户答完的下一轮就是本轮请求，服务端据此
    /// 把答案接回上一轮留下的 `questionnaire_id`。普通回合恒 `None`（不发该键）。
    pub clarify_questionnaire_answers: Option<serde_json::Value>,
}

/// `client_llm` 覆盖的 owned 字段；空白值不发送（规则与 [`ClientLlm`] 一致）。
#[derive(Debug, Clone, Default)]
pub struct ClientLlmFields {
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub api_base: Option<String>,
    /// 思考模式（`client_llm.llm_thinking_mode`）；仅值为 `on` / `off` 时随块发送，
    /// `server` / 空 / 其它值不发送（回落 serve 默认，对齐 Desktop `client_llm_json_for_chat_body`）。
    pub llm_thinking_mode: Option<String>,
    /// 上下文窗口 tokens（`client_llm.llm_context_tokens`）；仅非空数字且 > 0 时
    /// 随块发送为 JSON 数值（空/0/非数字不发送，对齐 Desktop 同函数）。
    pub llm_context_tokens: Option<String>,
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
                llm_thinking_mode: None,
                // 借用的 `ClientLlm`（repl/chat 的 CLI 覆盖）不携带该键；全屏 `tui` 直构
                // `ChatStreamOptions` 时才从持久层带上，转换路径固定为 None。
                llm_context_tokens: None,
            }),
            agent_role: a.agent_role.map(str::to_string),
            session_mode: a.session_mode.map(str::to_string),
            permission_level: a.permission_level.map(str::to_string),
            temperature: None,
            readonly_tool_ttl_cache_secs: None,
            stream_resume: a.stream_resume,
            // 文本 / REPL 路径没有问卷浮层（`on_clarify` 的默认实现是忽略），恒不带。
            clarify_questionnaire_answers: None,
        }
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

    /// 已请求的次数（1 = 取消当前回合，2 = 强退）。
    ///
    /// 只给 crate 内测试断言接线用（`/lane detach` 是否真的动了这条 lane 的取消句柄）：
    /// 生产侧一律通过 [`Self::cancel`] 施加、通过 `rx` 观察，不需要读这个数。
    #[cfg(test)]
    pub(crate) fn requests(&self) -> u8 {
        *self.rx.borrow()
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

    /// 澄清问卷（issue #424）：文本模式必须**至少说出来**，文案与理由见 `text_notes`。
    fn on_clarify(
        &mut self,
        info: &crabmate_client_api::sse_dispatch::ClarificationQuestionnaireInfo,
    ) -> Result<(), TermError> {
        self.on_system(&super::text_notes::clarify_unanswerable_line(
            info.fields.len(),
        ))
    }

    /// 压缩 / 裁剪（issue #478）：文本模式没有会话水位，只报最小事实（见 `trim_line`）。
    fn on_context_trim(&mut self, after: u32) -> Result<(), TermError> {
        let notice = super::trim_line::ContextTrimNotice::after_only(after);
        self.on_system(&super::trim_line::trim_line_text(&notice))
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
    let url = client.url(paths::CHAT_STREAM)?;
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
    // 交给 `error_for` 统一：SSE attach 失败的 401 也要走 `{code, message}` 解析，
    // 否则流式入口的报错又是「HTTP 401: {整段 JSON}」。
    Err(client.error_for(status.as_u16(), &text))
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
        insert_trimmed_str(map, "agent_role", opts.agent_role.as_deref());
        insert_trimmed_str(map, "session_mode", opts.session_mode.as_deref());
        insert_trimmed_str(map, "permission_level", opts.permission_level.as_deref());
    }
    if let Some(t) = temperature_for_chat_body(opts.temperature) {
        body["temperature"] = serde_json::json!(t);
    }
    if let Some(secs) = opts.readonly_tool_ttl_cache_secs {
        body["readonly_tool_ttl_cache_secs"] = serde_json::json!(secs);
    }
    if let Some(r) = opts.stream_resume {
        body["stream_resume"] = serde_json::json!({
            "job_id": r.job_id,
            "after_seq": r.after_seq,
        });
    }
    // 只在提交问卷那一轮出现：普通回合不带该键，服务端按键存在性解析不受影响。
    if let Some(cq) = opts.clarify_questionnaire_answers.as_ref() {
        body["clarify_questionnaire_answers"] = cq.clone();
    }
    body
}

/// 仅含非空（trim 后）字段的 `client_llm` 对象；全空返回 `None`（不发送整块）。
fn client_llm_json(llm: &ClientLlmFields) -> Option<Value> {
    let mut map = serde_json::Map::new();
    insert_trimmed_str(&mut map, "api_base", llm.api_base.as_deref());
    insert_trimmed_str(&mut map, "model", llm.model.as_deref());
    insert_trimmed_str(&mut map, "api_key", llm.api_key.as_deref());
    if let Some(t) = llm.llm_thinking_mode.as_deref().map(str::trim)
        && (t == "on" || t == "off")
    {
        map.insert("llm_thinking_mode".into(), Value::String(t.to_string()));
    }
    if let Some(n) = llm_context_tokens_for_chat_body(llm.llm_context_tokens.as_deref()) {
        map.insert("llm_context_tokens".into(), Value::Number(n.into()));
    }
    if map.is_empty() {
        None
    } else {
        Some(Value::Object(map))
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

/// SSE 字节级空闲上限：serve 的 SSE 走 axum `KeepAlive::default()`（约 15s 一个注释帧），
/// 正常回合不会静默这么久。超过即判定连接已被 NAT / 中间代理黑洞（TCP 未断但无数据），
/// 中断本轮；已拿到 job 句柄时解析为 [`TermError::InterruptedStream`] 以便续流。
const SSE_IDLE_TIMEOUT: Duration = Duration::from_secs(90);

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
        // 每次迭代重建：任何字节到达都会重置空闲计时。
        let idle = tokio::time::sleep(SSE_IDLE_TIMEOUT);
        let chunk = tokio::select! {
            biased;
            _ = wait_for_cancel(cancel) => {
                if cancel_current_run(client, outcome, sink, cancel).await? {
                    return Ok(());
                }
                // 旧 serve 无 `x-stream-job-id`：无法 cancel，保持原中断语义。
                return Err(TermError::Interrupted);
            }
            () = idle => {
                return Err(stream_read_error(
                    outcome,
                    &format!(
                        "no SSE bytes for {}s (serve keep-alive stopped; connection stalled)",
                        SSE_IDLE_TIMEOUT.as_secs()
                    ),
                ));
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
        // 先取出本帧，再就地移除已消费前缀；避免对剩余缓冲整段重建（O(n²)）。
        let block = buffer[..idx].to_string();
        buffer.drain(..idx + 2);
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
        let action = classify_line(line)?;
        dispatch_line_action(client, &opts.approval_session_id, action, sink, approval).await?;
    }
    Ok(())
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
    use crate::serve::approval::{ApprovalDecision, AutoAllowOnce, CommandApprovalData};

    struct CaptureGate {
        seen: Vec<CommandApprovalData>,
        decision: ApprovalDecision,
    }

    impl ApprovalGate for CaptureGate {
        fn decide(&mut self, req: &CommandApprovalData) -> Result<ApprovalDecision, TermError> {
            self.seen.push(req.clone());
            Ok(self.decision)
        }
    }

    /// 问卷答案只在**提交那一轮**出现；普通回合不带该键（服务端按键存在性解析）。
    #[test]
    fn clarify_answers_ride_only_the_submitting_turn() {
        let plain = ChatStreamOptions {
            message: "继续".into(),
            ..ChatStreamOptions::default()
        };
        assert!(
            chat_stream_body(&plain)
                .get("clarify_questionnaire_answers")
                .is_none(),
            "普通回合不该带问卷答案"
        );

        let answering = ChatStreamOptions {
            message: "（已回答澄清问卷）".into(),
            clarify_questionnaire_answers: Some(serde_json::json!({
                "questionnaire_id": "q-42",
                "answers": {"name": "crabmate"}
            })),
            ..ChatStreamOptions::default()
        };
        let body = chat_stream_body(&answering);
        assert_eq!(
            body["clarify_questionnaire_answers"]["questionnaire_id"],
            "q-42"
        );
        assert_eq!(
            body["clarify_questionnaire_answers"]["answers"]["name"],
            "crabmate"
        );
        // 题面本身不发回去：服务端按 `questionnaire_id` 认领，重发会撑大请求体。
        assert!(
            body["clarify_questionnaire_answers"]
                .get("fields")
                .is_none()
        );
    }

    #[test]
    fn auto_allow_once_gate() {
        let mut g = AutoAllowOnce;
        let req = CommandApprovalData {
            command: "x".into(),
            args: String::new(),
            allowlist_key: None,
            permission_level: None,
        };
        assert_eq!(g.decide(&req).unwrap(), ApprovalDecision::AllowOnce);
    }

    #[test]
    fn capture_gate_records() {
        let mut g = CaptureGate {
            seen: Vec::new(),
            decision: ApprovalDecision::Deny,
        };
        let req = CommandApprovalData {
            command: "df".into(),
            args: "-h".into(),
            allowlist_key: Some("df".into()),
            permission_level: None,
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
                llm_thinking_mode: None,
                llm_context_tokens: None,
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
    fn chat_body_includes_thinking_mode_only_when_on_or_off() {
        let body = chat_stream_body(&ChatStreamOptions {
            client_llm: Some(ClientLlmFields {
                llm_thinking_mode: Some(" on ".into()),
                ..ClientLlmFields::default()
            }),
            ..base_opts()
        });
        assert_eq!(body["client_llm"]["llm_thinking_mode"], "on");

        let body = chat_stream_body(&ChatStreamOptions {
            client_llm: Some(ClientLlmFields {
                model: Some("m".into()),
                llm_thinking_mode: Some("off".into()),
                ..ClientLlmFields::default()
            }),
            ..base_opts()
        });
        assert_eq!(body["client_llm"]["llm_thinking_mode"], "off");

        for silent in [None, Some("server".to_string()), Some("".to_string())] {
            let body = chat_stream_body(&ChatStreamOptions {
                client_llm: Some(ClientLlmFields {
                    model: Some("m".into()),
                    llm_thinking_mode: silent,
                    ..ClientLlmFields::default()
                }),
                ..base_opts()
            });
            assert!(
                body["client_llm"].get("llm_thinking_mode").is_none(),
                "server/空思考模式不发送"
            );
        }
    }

    #[test]
    fn chat_body_top_level_temperature_when_valid_range() {
        let body = chat_stream_body(&ChatStreamOptions {
            temperature: Some(0.7),
            ..base_opts()
        });
        assert_eq!(body["temperature"], 0.7);

        let body = chat_stream_body(&ChatStreamOptions {
            temperature: Some(2.0),
            ..base_opts()
        });
        assert_eq!(body["temperature"], 2.0);

        for t in [
            None,
            Some(-0.1),
            Some(2.1),
            Some(f64::NAN),
            Some(f64::INFINITY),
        ] {
            let body = chat_stream_body(&ChatStreamOptions {
                temperature: t,
                ..base_opts()
            });
            assert!(
                body.get("temperature").is_none(),
                "非法/越界温度不发送 {t:?}"
            );
        }
    }

    #[test]
    fn chat_body_context_tokens_only_positive_number() {
        let body = chat_stream_body(&ChatStreamOptions {
            client_llm: Some(ClientLlmFields {
                model: Some("m".into()),
                llm_context_tokens: Some(" 8000 ".into()),
                ..ClientLlmFields::default()
            }),
            ..base_opts()
        });
        assert_eq!(body["client_llm"]["llm_context_tokens"], 8000);

        // 0 / 非数字 / 空 → 键省略（client_llm 仍因 model 存在而发送）。
        for silent in [Some("0".to_string()), Some("abc".to_string()), None] {
            let body = chat_stream_body(&ChatStreamOptions {
                client_llm: Some(ClientLlmFields {
                    model: Some("m".into()),
                    llm_context_tokens: silent.clone(),
                    ..ClientLlmFields::default()
                }),
                ..base_opts()
            });
            assert!(
                body["client_llm"].get("llm_context_tokens").is_none(),
                "0/非数字/空不发送 {silent:?}"
            );
        }
    }

    #[test]
    fn chat_body_readonly_ttl_secs_sends_zero_when_disabled() {
        let body = chat_stream_body(&ChatStreamOptions {
            readonly_tool_ttl_cache_secs: Some(0),
            ..base_opts()
        });
        assert_eq!(body["readonly_tool_ttl_cache_secs"], 0);

        let base = chat_stream_body(&base_opts());
        assert!(base.get("readonly_tool_ttl_cache_secs").is_none());
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
                llm_thinking_mode: Some("server".into()),
                llm_context_tokens: None,
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
    fn chat_body_carries_permission_level_intent_and_trims_it() {
        let body = chat_stream_body(&ChatStreamOptions {
            permission_level: Some(" read ".into()),
            ..base_opts()
        });
        assert_eq!(body["permission_level"], "read");
        // 空白 = 不表达意图（沿用会话持久值），不能发一个空串过去被当成非法档位。
        let blank = chat_stream_body(&ChatStreamOptions {
            permission_level: Some("   ".into()),
            ..base_opts()
        });
        assert!(blank.get("permission_level").is_none());
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
