//! client 侧用户文案与摘要生成（display 层；不属于契约，Server 不依赖）。
//!
//! 与「解析/视图」分离：契约形状与错误分类见各解析模块，本模块只负责
//! 面向用户的消息拼装（P3 整理；后续新增文案优先落这里）。

use serde_json::Value;

/// 从 JSON 错误体提取结构化文本：优先 `error`，其次 `message`；trim 后非空才采用。
#[must_use]
pub fn http_error_text(val: &Value) -> Option<String> {
    ["error", "message"].iter().find_map(|key| {
        val.get(*key)
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    })
}

/// HTTP 非 2xx JSON 错误体的通用用户文案：`error` → `message` → `HTTP {status}`。
#[must_use]
pub fn http_error_message(val: &Value, status: u16) -> String {
    http_error_text(val).unwrap_or_else(|| format!("HTTP {status}"))
}

/// 从嵌套错误体 `{"error":{"code":"…","message":"…"}}` 取机器可读的 `code`。
///
/// [`http_error_text`] 只认**顶层** `error` / `message` 字符串，而受保护 API 的错误体是
/// `error` **对象**：那里取不到 `code`，拿人读文案反推分支就是把文案当契约。
/// 需要按 `code` 分流时（例如 404 的两义：路由不存在 vs 会话不存在；以及可否重试）用本函数。
///
/// 非 `ApiError` 形状（axum 默认 404 的空 body、非 JSON、`error` 是字符串）一律 `None`——
/// 这个 `None` 本身就是「该 serve 没有这条路由」的判据，不要用状态码顶替。
#[must_use]
pub fn api_error_code(val: &Value) -> Option<&str> {
    val.get("error")?
        .get("code")?
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::{api_error_code, http_error_message, http_error_text};
    use serde_json::json;

    #[test]
    fn api_error_code_reads_the_nested_wire_shape() {
        // 受保护 API 的错误体：`error` 是对象，`code` 在里层。
        assert_eq!(
            api_error_code(&json!({
                "error": {"code": "CONVERSATION_NOT_FOUND", "message": "会话不存在"}
            })),
            Some("CONVERSATION_NOT_FOUND")
        );
        // 顶层字符串 `error` 是另一种形状，没有 code 可取（返回 None 而不是猜）。
        assert_eq!(api_error_code(&json!({"error": "forbidden"})), None);
        // 空白 code 等同没有：不能让 `""` 冒充「有 code」。
        assert_eq!(api_error_code(&json!({"error": {"code": "  "}})), None);
        assert_eq!(api_error_code(&json!({"error": {"code": 7}})), None);
        assert_eq!(api_error_code(&json!({})), None);
        assert_eq!(api_error_code(&json!([])), None);
    }

    #[test]
    fn http_error_prefers_error_over_message() {
        assert_eq!(
            http_error_message(&json!({"error": "forbidden"}), 403),
            "forbidden"
        );
        assert_eq!(
            http_error_message(&json!({"error": "a", "message": "b"}), 403),
            "a"
        );
    }

    #[test]
    fn http_error_falls_back_to_message_then_status() {
        assert_eq!(http_error_message(&json!({"message": "m"}), 502), "m");
        assert_eq!(http_error_message(&json!({}), 502), "HTTP 502");
    }

    #[test]
    fn http_error_text_skips_blank_and_non_string() {
        assert_eq!(http_error_text(&json!({"error": "  "})), None);
        assert_eq!(
            http_error_text(&json!({"error": "  ", "message": " x "})).as_deref(),
            Some("x")
        );
        assert_eq!(http_error_text(&json!({"error": 1})), None);
    }
}
