//! 会话存储切换（`POST /config/session/conversation-store`，对齐 Desktop 设置页）。
//!
//! 与 `/user-data/*` 那两条不同：这个端点**不是全量 DTO**，也没有「跟随 server」
//! 这一态 —— 它是一次动作（把会话存储换成/换回 SQLite），当前值从
//! `GET /status` 的 `conversation_store_sqlite_active` 读。

use crabmate_client_api::paths;
use serde::Deserialize;

use crate::serve::client::ServeClient;
use crate::serve::error::TermError;

/// `POST /config/session/conversation-store` 的成功体（`{ok, message}`）。
#[derive(Debug, Clone, Deserialize)]
pub struct SessionStoreResponse {
    pub ok: bool,
    #[serde(default)]
    pub message: String,
}

/// 切换会话存储：`sqlite = true` 走 SQLite，`false` 回落到文件存储。
///
/// **业务拒绝与服务端错误分开报**：`ok: false` 带服务端给的原因（如「SQLite 路径
/// 未配置」），那是用户要读的；网络/HTTP 失败才是要重试的。把两者压成同一句会让
/// 面板提示成「保存失败，可按 S 重试」，而重试同一个值永远不会成功。
pub async fn set_session_conversation_store(
    client: &ServeClient,
    sqlite: bool,
) -> Result<(), TermError> {
    let body = serde_json::json!({ "sqlite": sqlite });
    let value = client
        .post_json(paths::CONFIG_SESSION_CONVERSATION_STORE, &body)
        .await?;
    // 老 serve / 反向代理可能给 2xx 空体：那只能是「做了什么但没细说」，按成功处理。
    if value.is_null() {
        return Ok(());
    }
    let resp: SessionStoreResponse = serde_json::from_value(value).map_err(|e| {
        TermError::Message(format!("decode /config/session/conversation-store: {e}"))
    })?;
    if resp.ok {
        return Ok(());
    }
    let detail = resp.message.trim();
    Err(TermError::Message(if detail.is_empty() {
        "serve 拒绝了会话存储切换（未给出原因）".to_string()
    } else {
        detail.to_string()
    }))
}

#[cfg(test)]
mod tests {
    use super::SessionStoreResponse;

    #[test]
    fn failure_body_carries_the_server_reason() {
        let r: SessionStoreResponse =
            serde_json::from_str(r#"{"ok":false,"message":"SQLite 路径未配置"}"#).unwrap();
        assert!(!r.ok);
        assert_eq!(r.message, "SQLite 路径未配置");
    }

    #[test]
    fn success_body_tolerates_a_missing_message() {
        let r: SessionStoreResponse = serde_json::from_str(r#"{"ok":true}"#).unwrap();
        assert!(r.ok);
        assert!(r.message.is_empty());
    }
}
