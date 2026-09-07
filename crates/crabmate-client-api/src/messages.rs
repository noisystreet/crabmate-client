//! client 侧用户文案与摘要生成（display 层；不属于契约，Server 不依赖）。
//!
//! 与「解析/视图」分离：契约形状与错误分类见各解析模块，本模块只负责
//! 面向用户的消息拼装（P3 整理；后续新增文案优先落这里）。

use serde_json::Value;

/// `POST /workspace*` HTTP 非 2xx：优先 body `error`，否则 `HTTP {status}`。
#[must_use]
pub fn workspace_http_error_message(val: &Value, status: u16) -> String {
    val.get("error")
        .and_then(|e| e.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| format!("HTTP {status}"))
}

#[cfg(test)]
mod tests {
    use super::workspace_http_error_message;
    use serde_json::json;

    #[test]
    fn http_error_prefers_body() {
        assert_eq!(
            workspace_http_error_message(&json!({"error":"forbidden"}), 403),
            "forbidden"
        );
        assert_eq!(workspace_http_error_message(&json!({}), 502), "HTTP 502");
    }
}
