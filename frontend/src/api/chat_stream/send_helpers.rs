//! [`super::send_chat_stream`] 用的抽取逻辑（降低 CCN）。

use web_sys::Response;

use crabmate_sse_protocol::StreamEndReason;

use crate::i18n::Locale;

use super::{
    ChatStreamCallbacks, body_reader, body_reader::ChatStreamBodyConsumeResult, http_request,
};

pub(super) fn chat_stream_fetch_retry_exhausted(
    stream_resume_job_id: Option<u64>,
    attempt: u32,
) -> bool {
    stream_resume_job_id.is_none() || attempt >= 6
}

pub(super) async fn chat_stream_http_error_message(
    resp: &Response,
    loc: Locale,
) -> Result<String, String> {
    let msg = http_request::chat_stream_read_error_body(resp, loc).await?;
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&msg) {
        let text = v
            .get("message")
            .or_else(|| v.get("error"))
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        if let Some(m) = text {
            let code = v.get("code").and_then(|x| x.as_str());
            let request_id = v.get("request_id").and_then(|x| x.as_str());
            let mut out = m.to_string();
            if let Some(c) = code.map(str::trim).filter(|s| !s.is_empty()) {
                out.push_str(&format!(" ({c})"));
            }
            if let Some(r) = request_id.map(str::trim).filter(|s| !s.is_empty()) {
                out.push_str(&format!(" [request_id={r}]"));
            }
            return Ok(out);
        }
    }
    Ok(msg)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ChatStreamConsumeOutcome {
    Finished { saw_stream_ended: bool },
    ResumeReconnect,
}

fn classify_chat_stream_consume_outcome(
    stream_finished_normally: bool,
    saw_stream_ended: bool,
    stream_resume_job_id: Option<u64>,
) -> ChatStreamConsumeOutcome {
    if saw_stream_ended {
        return ChatStreamConsumeOutcome::Finished {
            saw_stream_ended: true,
        };
    }
    // 队列流拥有 job id：即使 WebView 把提前 EOF 报成 `done=true`，缺少 RUN_FINISHED /
    // stream_ended 也不能合成 completed；按 Last-Event-ID 重连补收终答与真实终态。
    if stream_resume_job_id.is_some() || !stream_finished_normally {
        return ChatStreamConsumeOutcome::ResumeReconnect;
    }
    // 无 job id 的内建命令是单帧短响应，兼容其「body EOF 即完成」旧契约。
    ChatStreamConsumeOutcome::Finished {
        saw_stream_ended: false,
    }
}

pub(super) async fn consume_chat_stream_body_phase(
    resp: Response,
    signal: &web_sys::AbortSignal,
    last_event_id: &mut u64,
    cbs: &ChatStreamCallbacks,
    loc: Locale,
    stream_resume_job_id: Option<u64>,
) -> Result<ChatStreamConsumeOutcome, String> {
    let Some(rb) = resp.body() else {
        return Err(crate::i18n::api_err_no_response_body(loc).to_string());
    };
    let ChatStreamBodyConsumeResult {
        stream_finished_normally,
        saw_stream_ended,
    } = body_reader::consume_chat_stream_response_body(
        rb,
        signal,
        last_event_id,
        cbs,
        loc,
        stream_resume_job_id,
    )
    .await?;
    Ok(classify_chat_stream_consume_outcome(
        stream_finished_normally,
        saw_stream_ended,
        stream_resume_job_id,
    ))
}

pub(super) enum ChatStreamRoundOutcome {
    Completed,
    ResumeReconnect,
}

fn dispatch_finished_round_callbacks(
    saw_stream_ended: bool,
    mut on_missing_stream_ended: impl FnMut(),
    mut on_done: impl FnMut(),
) {
    if !saw_stream_ended {
        on_missing_stream_ended();
    }
    on_done();
}

/// 单轮 HTTP 响应：`410` / 错误体 / SSE 体消费与正常收尾回调。
pub(super) async fn run_chat_stream_http_round(
    resp: Response,
    cbs: &ChatStreamCallbacks,
    stream_resume_job_id: &mut Option<u64>,
    signal: &web_sys::AbortSignal,
    last_event_id: &mut u64,
    loc: Locale,
) -> Result<ChatStreamRoundOutcome, String> {
    http_request::apply_chat_stream_response_headers(&resp, cbs, stream_resume_job_id);
    if resp.status() == 410 {
        return Err(crate::i18n::api_err_stream_gone(loc).to_string());
    }
    if !resp.ok() {
        return Err(chat_stream_http_error_message(&resp, loc).await?);
    }
    match consume_chat_stream_body_phase(
        resp,
        signal,
        last_event_id,
        cbs,
        loc,
        *stream_resume_job_id,
    )
    .await?
    {
        ChatStreamConsumeOutcome::Finished { saw_stream_ended } => {
            dispatch_finished_round_callbacks(
                saw_stream_ended,
                || (cbs.on_stream_ended)(StreamEndReason::Completed.to_string(), None),
                || (cbs.on_done)(),
            );
            Ok(ChatStreamRoundOutcome::Completed)
        }
        ChatStreamConsumeOutcome::ResumeReconnect => Ok(ChatStreamRoundOutcome::ResumeReconnect),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ChatStreamConsumeOutcome, classify_chat_stream_consume_outcome,
        dispatch_finished_round_callbacks,
    };
    use std::cell::Cell;

    #[test]
    fn queued_body_eof_without_terminal_reconnects() {
        assert_eq!(
            classify_chat_stream_consume_outcome(true, false, Some(42)),
            ChatStreamConsumeOutcome::ResumeReconnect
        );
    }

    #[test]
    fn queued_body_with_terminal_finishes() {
        assert_eq!(
            classify_chat_stream_consume_outcome(true, true, Some(42)),
            ChatStreamConsumeOutcome::Finished {
                saw_stream_ended: true
            }
        );
    }

    #[test]
    fn builtin_body_eof_without_job_keeps_short_response_compatibility() {
        assert_eq!(
            classify_chat_stream_consume_outcome(true, false, None),
            ChatStreamConsumeOutcome::Finished {
                saw_stream_ended: false
            }
        );
    }

    #[test]
    fn body_completion_dispatches_done_exactly_once_after_run_finished() {
        let ended = Cell::new(0u32);
        let done = Cell::new(0u32);
        dispatch_finished_round_callbacks(
            true,
            || ended.set(ended.get() + 1),
            || done.set(done.get() + 1),
        );
        assert_eq!(ended.get(), 0, "RUN_FINISHED already entered draining");
        assert_eq!(done.get(), 1);
    }

    #[test]
    fn body_completion_synthesizes_missing_end_before_single_done() {
        let ended = Cell::new(0u32);
        let done = Cell::new(0u32);
        dispatch_finished_round_callbacks(
            false,
            || ended.set(ended.get() + 1),
            || done.set(done.get() + 1),
        );
        assert_eq!(ended.get(), 1);
        assert_eq!(done.get(), 1);
    }
}
