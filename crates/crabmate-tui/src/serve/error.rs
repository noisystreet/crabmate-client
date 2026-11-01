use crabmate_client_api::messages::{api_error_code, http_error_text, top_level_error_code};
use serde_json::Value;
use thiserror::Error;

/// 远程终端客户端错误。
#[derive(Debug, Error)]
pub enum TermError {
    #[error("invalid API base URL: {0}")]
    InvalidApiBase(String),
    #[error("HTTP {status}: {body}")]
    Http { status: u16, body: String },
    #[error("request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("stream error: {0}")]
    Stream(String),
    #[error("server run error: {0}")]
    RunError(String),
    #[error("interrupted")]
    Interrupted,
    /// 流在完成前中断（网络/传输错误），且已拿到 job 句柄可续传。
    #[error("stream interrupted at seq {after_seq} (job {job_id}): {cause}")]
    InterruptedStream {
        job_id: u64,
        after_seq: u64,
        cause: String,
    },
    #[error("{0}")]
    Message(String),
}

/// 非 2xx 响应 → 错误。
///
/// 文案优先取 JSON 错误体（顶层 `error` / `message`，或嵌套 `error.message`），能取到稳定的
/// `code` 时前置 `<CODE>: `；既无文案也不成形时回落到截断后的原始响应体。
///
/// `json_api`（通用 JSON 请求）与 `client`（health / approval / cancel）共用，避免同一错误
/// 体在两处呈现不一致。只认顶层 `error` 字段是不够的：受保护 API 的 401 是扁平
/// `{"code":"UNAUTHORIZED","message":…}`，那样会把整段 JSON 糊进 UI（`HTTP 401: {"code":…}`），
/// 排查时分不清是「凭证缺失」还是别的原因。
pub(crate) fn http_error(status: u16, text: &str) -> TermError {
    match error_body_summary(text) {
        Some((code, message)) => TermError::Message(match code {
            Some(code) => format!("{code}: {message}"),
            None => message,
        }),
        None => TermError::Http {
            status,
            body: text.trim().chars().take(400).collect(),
        },
    }
}

/// 从响应体取 `(code, 人读文案)`；两种形状**分开**取，不互相回退。
///
/// - 扁平 `{"code":"UNAUTHORIZED","message":"…","request_id":"…"}`（受保护 API 的鉴权失败）
/// - 嵌套 `{"error":{"code":"CONVERSATION_NOT_FOUND","message":"…"}}`
///
/// 为什么不混着找 `code`：两种形状的 `code` 语义不同，回退查找会让调用方在错误的形状上
/// 分支（见 `crabmate_client_api::messages::api_error_code` / `top_level_error_code` 的文档）。
fn error_body_summary(text: &str) -> Option<(Option<String>, String)> {
    let v: Value = serde_json::from_str(text).ok()?;
    if let Some(inner) = v.get("error").filter(|e| e.is_object()) {
        let message = http_error_text(inner)?;
        return Some((api_error_code(&v).map(str::to_string), message));
    }
    let message = http_error_text(&v)?;
    Some((top_level_error_code(&v).map(str::to_string), message))
}

#[cfg(test)]
mod tests {
    use super::http_error;

    fn display(status: u16, body: &str) -> String {
        http_error(status, body).to_string()
    }

    /// serve 的 401：`code` 与 `message` 都在顶层，二者都要出现在文案里。
    #[test]
    fn flat_code_and_message_are_both_rendered() {
        let body = r#"{"code":"UNAUTHORIZED","message":"缺少或无效的 Web API 凭证（Authorization: Bearer 或 X-API-Key）","request_id":"cm-1"}"#;
        assert_eq!(
            display(401, body),
            "UNAUTHORIZED: 缺少或无效的 Web API 凭证（Authorization: Bearer 或 X-API-Key）"
        );
    }

    /// 嵌套形状：文案在 `error.message`，`code` 在 `error.code` 一层里。
    #[test]
    fn nested_error_object_uses_its_own_code_and_message() {
        let body = r#"{"error":{"code":"CONVERSATION_NOT_FOUND","message":"会话不存在"}}"#;
        assert_eq!(display(404, body), "CONVERSATION_NOT_FOUND: 会话不存在");
    }

    /// 顶层字符串 `error`（老形状）没有 code，不该凭空补前缀。
    #[test]
    fn string_error_without_code_stays_plain() {
        assert_eq!(display(403, r#"{"error":"forbidden"}"#), "forbidden");
    }

    /// 非 JSON（网关 HTML、空 body）回落到 `HTTP {status}: {body}`。
    #[test]
    fn non_json_body_falls_back_to_raw_body() {
        assert_eq!(
            display(502, "  <html>bad gateway</html>  "),
            "HTTP 502: <html>bad gateway</html>"
        );
        assert_eq!(display(500, ""), "HTTP 500: ");
    }

    /// 只有 `code`、文案空白时拿不到人读内容，仍按原始 body 呈现（别丢掉 body 里的 code）。
    #[test]
    fn blank_message_falls_back_to_raw_body() {
        let body = r#"{"code":"X","message":"  "}"#;
        assert_eq!(display(400, body), format!("HTTP 400: {body}"));
    }
}
