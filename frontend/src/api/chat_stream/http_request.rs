use wasm_bindgen_futures::JsFuture;
use web_sys::{Request, RequestInit, RequestMode, Response};

use crabmate::cm_sse_protocol::SSE_PROTOCOL_VERSION;
use crabmate_client_api::{ChatStreamCoreFields, merge_chat_stream_core_fields, paths};

use crate::i18n::Locale;

use super::super::browser;
use super::super::client_llm_storage::{
    chat_temperature_override_from_storage, client_llm_json_for_chat_body,
    executor_llm_json_for_chat_body, readonly_tool_ttl_cache_secs_for_chat_body,
};
use super::ChatStreamCallbacks;

/// `build_chat_stream_post_body` 的入参（避免超长形参列表）。
pub(super) struct ChatStreamPostBodyParts<'a> {
    pub(super) message: &'a str,
    pub(super) image_urls: &'a [String],
    pub(super) conversation_id: &'a Option<String>,
    pub(super) agent_role: &'a Option<String>,
    pub(super) session_mode: &'a Option<String>,
    pub(super) approval_session_id: &'a Option<String>,
    pub(super) stream_resume_job_id: Option<u64>,
    pub(super) last_event_id: u64,
    pub(super) clarify_questionnaire_answers: &'a Option<serde_json::Value>,
}

pub(super) fn build_chat_stream_post_body(
    p: ChatStreamPostBodyParts<'_>,
) -> Result<serde_json::Value, String> {
    let ChatStreamPostBodyParts {
        message,
        image_urls,
        conversation_id,
        agent_role,
        session_mode,
        approval_session_id,
        stream_resume_job_id,
        last_event_id,
        clarify_questionnaire_answers,
    } = p;
    let mut body = serde_json::json!({
        "agent_role": agent_role,
    });
    merge_chat_stream_core_fields(
        &mut body,
        ChatStreamCoreFields {
            message,
            client_sse_protocol: SSE_PROTOCOL_VERSION,
            approval_session_id: approval_session_id.as_deref(),
            conversation_id: conversation_id.as_deref(),
        },
    );
    // Web 历史形状：缺省 id 仍发 JSON null（与改前 `Option` 字段一致），避免省略键带来的契约歧义。
    ensure_json_null_if_absent(&mut body, "conversation_id");
    ensure_json_null_if_absent(&mut body, "approval_session_id");
    if let Some(mode) = session_mode
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        body["session_mode"] = serde_json::json!(mode);
    }
    if !image_urls.is_empty() {
        body["image_urls"] = serde_json::json!(image_urls);
    }
    if let Some(cq) = clarify_questionnaire_answers {
        body["clarify_questionnaire_answers"] = cq.clone();
    }
    if let Some(jid) = stream_resume_job_id {
        body["stream_resume"] = serde_json::json!({
            "job_id": jid,
            "after_seq": last_event_id,
        });
    }
    if let Some(cl) = client_llm_json_for_chat_body() {
        body["client_llm"] = cl;
    }
    if let Some(el) = executor_llm_json_for_chat_body() {
        body["executor_llm"] = el;
    }
    if let Some(temp) = chat_temperature_override_from_storage() {
        body["temperature"] = serde_json::json!(temp);
    }
    if let Some(secs) = readonly_tool_ttl_cache_secs_for_chat_body() {
        body["readonly_tool_ttl_cache_secs"] = serde_json::json!(secs);
    }
    Ok(body)
}

fn ensure_json_null_if_absent(body: &mut serde_json::Value, key: &str) {
    let Some(map) = body.as_object_mut() else {
        return;
    };
    map.entry(key.to_string())
        .or_insert(serde_json::Value::Null);
}

pub(super) async fn build_chat_stream_fetch_request(
    body_json: &str,
    signal: &web_sys::AbortSignal,
    last_event_id: u64,
) -> Result<Request, String> {
    browser::ensure_web_api_bearer_hydrated_for_request().await;
    browser::ensure_github_token_hydrated_for_request().await;
    let init = RequestInit::new();
    init.set_method("POST");
    init.set_mode(RequestMode::Cors);
    init.set_credentials(web_sys::RequestCredentials::Include);
    init.set_signal(Some(signal));
    let h = browser::auth_headers();
    let _ = h.set("Content-Type", "application/json");
    if last_event_id > 0 {
        let _ = h.set("Last-Event-ID", &last_event_id.to_string());
    }
    init.set_headers(&h);
    init.set_body(&wasm_bindgen::JsValue::from_str(body_json));
    Request::new_with_str_and_init(&browser::api_url(paths::CHAT_STREAM), &init)
        .map_err(|e| format!("req: {:?}", e))
}

pub(super) fn apply_chat_stream_response_headers(
    resp: &Response,
    cbs: &ChatStreamCallbacks,
    stream_resume_job_id: &mut Option<u64>,
) {
    if let Some(cid) = resp.headers().get("x-conversation-id").ok().flatten() {
        let t = cid.trim();
        if !t.is_empty() {
            (cbs.on_conversation_id)(t.to_string());
        }
    }
    if let Some(jh) = resp.headers().get("x-stream-job-id").ok().flatten() {
        if let Ok(jid) = jh.trim().parse::<u64>() {
            *stream_resume_job_id = Some(jid);
            (cbs.on_stream_job_id)(jid);
        }
    }
}

pub(super) async fn chat_stream_read_error_body(
    resp: &Response,
    loc: Locale,
) -> Result<String, String> {
    let text_promise = resp.text().map_err(|e| format!("text: {:?}", e))?;
    Ok(JsFuture::from(text_promise)
        .await
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_else(|| crate::i18n::api_err_request_failed(loc).to_string()))
}

/// 退避基数：`base = 200ms * 2^attempt`（attempt 从 1 起 → 400/800/1600/3200/6400ms）。
const RETRY_BACKOFF_BASE_MS: u64 = 200;

/// 退避指数的封顶位移：`2^5` → 单次最长 6400ms。
const RETRY_BACKOFF_MAX_SHIFT: u32 = 5;

/// 退避抖动幅度（千分比）：±20%，打散多客户端同时断线后的齐步重连。
const RETRY_BACKOFF_JITTER_PERMILLE: u64 = 200;

/// 退避毫秒数（含 ±20% 抖动）。`unit` ∈ [0, 1)（生产传 `Math.random()`）；
/// `unit = 0.5` 时无抖动偏移，便于单测断言基数。
fn chat_stream_retry_backoff_ms_with_jitter(attempt: u32, unit: f64) -> u64 {
    let base = RETRY_BACKOFF_BASE_MS.saturating_mul(1u64 << attempt.min(RETRY_BACKOFF_MAX_SHIFT));
    let span = 2 * RETRY_BACKOFF_JITTER_PERMILLE;
    let offset = (unit.clamp(0.0, 1.0) * span as f64) as u64;
    base.saturating_mul(1000 - RETRY_BACKOFF_JITTER_PERMILLE + offset) / 1000
}

pub(super) async fn sleep_chat_stream_retry_backoff(attempt: u32) {
    let ms = chat_stream_retry_backoff_ms_with_jitter(attempt, js_sys::Math::random());
    gloo_timers::future::TimeoutFuture::new(ms as u32).await;
}

#[cfg(test)]
mod tests {
    use super::{RETRY_BACKOFF_BASE_MS, chat_stream_retry_backoff_ms_with_jitter};

    /// `unit = 0.5` 落在抖动区间中点：退避即为无抖动基数。
    #[test]
    fn backoff_center_is_unjittered_base() {
        assert_eq!(chat_stream_retry_backoff_ms_with_jitter(0, 0.5), 200);
        assert_eq!(chat_stream_retry_backoff_ms_with_jitter(1, 0.5), 400);
        assert_eq!(chat_stream_retry_backoff_ms_with_jitter(2, 0.5), 800);
        assert_eq!(chat_stream_retry_backoff_ms_with_jitter(5, 0.5), 6400);
        // 指数封顶后不再增长
        assert_eq!(chat_stream_retry_backoff_ms_with_jitter(9, 0.5), 6400);
    }

    /// 抖动边界：`unit` 取端点时恰为 ±20%，且永不超出区间。
    #[test]
    fn backoff_jitter_stays_within_20_percent() {
        for attempt in 1..=4u32 {
            let base = RETRY_BACKOFF_BASE_MS
                .saturating_mul(1u64 << attempt.min(super::RETRY_BACKOFF_MAX_SHIFT));
            assert_eq!(
                chat_stream_retry_backoff_ms_with_jitter(attempt, 0.0),
                base * 80 / 100
            );
            assert_eq!(
                chat_stream_retry_backoff_ms_with_jitter(attempt, 1.0),
                base * 120 / 100
            );
        }
        // 越界 unit 被夹紧，不得产生负值 / 溢出
        assert_eq!(
            chat_stream_retry_backoff_ms_with_jitter(3, -5.0),
            1600 * 80 / 100
        );
        assert_eq!(
            chat_stream_retry_backoff_ms_with_jitter(3, 9.0),
            1600 * 120 / 100
        );
    }
}
