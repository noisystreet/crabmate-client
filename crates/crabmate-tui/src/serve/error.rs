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
    // 顶层 Display 只有「error sending request for url (...)」，真正的原因（连接被拒 /
    // 被重置 / 握手超时）在 `source()` 链里 —— 而调用方普遍是 `e.to_string()`，只取顶层
    // 就得到一句无法排查的话。所以这里把整条链一起展开（见 [`error_text_with_sources`]）。
    #[error("request failed: {}", request_error_text(.0))]
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

/// 错误正文的字符上限（与 [`http_error`] 截 body 的口径一致）：这行最终会落进
/// transcript，一条错误不该把版面吃掉。
const MAX_ERROR_TEXT_CHARS: usize = 400;

/// `TermError::Request` 的 Display 用：把 `reqwest` 的 `source()` 链一并展开。
///
/// 为什么不直接 `{0}`：`reqwest::Error` 的顶层 Display 只有
/// `error sending request for url (...)`，而「连接被拒 / 被对端重置 / 握手超时」这些
/// 真正能定性的信息在 source 链里。调用方几乎都是 `e.to_string()`，只取顶层就等于把
/// 原因丢掉 —— 工作区树展开失败时屏幕上只有一句话，排查只能靠猜外部状态。
fn request_error_text(err: &reqwest::Error) -> String {
    error_text_with_sources(err)
}

/// 错误自身 + 逐层 `source()`，用 `: ` 连成一行。
///
/// 相邻两段逐字相同时跳过（重复出现的那段只会让人以为拼接出了错）；末尾按
/// [`MAX_ERROR_TEXT_CHARS`] 截断并补 `…`，不静默丢弃。
fn error_text_with_sources(err: &dyn std::error::Error) -> String {
    let mut text = err.to_string();
    let mut last = text.clone();
    let mut cursor = err.source();
    while let Some(source) = cursor {
        let segment = source.to_string();
        if !segment.trim().is_empty() && segment != last {
            text.push_str(": ");
            text.push_str(&segment);
            last = segment;
        }
        cursor = source.source();
    }
    if text.chars().count() > MAX_ERROR_TEXT_CHARS {
        let mut cut: String = text.chars().take(MAX_ERROR_TEXT_CHARS).collect();
        cut.push('…');
        return cut;
    }
    text
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
    use super::{MAX_ERROR_TEXT_CHARS, error_text_with_sources, http_error};

    /// 造一条多层 `source()` 链（`reqwest::Error` 造不出来，形状一样就够）。
    #[derive(Debug)]
    struct Layer {
        msg: String,
        inner: Option<Box<Layer>>,
    }

    impl std::fmt::Display for Layer {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(&self.msg)
        }
    }

    impl std::error::Error for Layer {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            self.inner
                .as_ref()
                .map(|b| b.as_ref() as &(dyn std::error::Error + 'static))
        }
    }

    /// 依次串起来，第一项是最外层。
    fn chain(msgs: &[&str]) -> Layer {
        msgs.iter()
            .rev()
            .fold(None, |inner, msg| {
                Some(Box::new(Layer {
                    msg: (*msg).to_string(),
                    inner,
                }))
            })
            .map(|boxed| *boxed)
            .expect("至少一层")
    }

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

    /// `reqwest` 的顶层 Display 说明不了原因，能定性的那几层都在链上。
    #[test]
    fn the_whole_source_chain_is_rendered() {
        let e = chain(&[
            "error sending request for url (http://127.0.0.1:8081/workspace?path=fixtures)",
            "client error (Connect)",
            "tcp connect error",
            "Connection refused (os error 111)",
        ]);
        assert_eq!(
            error_text_with_sources(&e),
            "error sending request for url (http://127.0.0.1:8081/workspace?path=fixtures): \
             client error (Connect): tcp connect error: Connection refused (os error 111)"
        );
    }

    /// 相邻重复段只出现一次：同一句话拼两遍会被读成「拼接写错了」。
    #[test]
    fn adjacent_duplicate_segments_are_collapsed() {
        let e = chain(&["boom", "boom", "detail"]);
        assert_eq!(error_text_with_sources(&e), "boom: detail");
    }

    /// 超长链按字符截断并补省略号，而不是静默丢内容。
    #[test]
    fn an_overlong_chain_is_truncated_with_an_ellipsis() {
        let long = "x".repeat(MAX_ERROR_TEXT_CHARS + 50);
        let e = chain(&[long.as_str()]);
        let text = error_text_with_sources(&e);
        assert_eq!(text.chars().count(), MAX_ERROR_TEXT_CHARS + 1);
        assert!(text.ends_with('…'), "{text}");
    }
}
