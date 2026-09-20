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

#[cfg(test)]
mod tests {
    use super::{http_error_message, http_error_text};
    use serde_json::json;

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
