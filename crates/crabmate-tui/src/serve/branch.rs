//! `POST /chat/branch`（issue #422）：把会话在服务端截断到某一轮用户消息之前。
//!
//! 本模块只管 HTTP 与状态码搬运；**分类与「序号算不算得准」都在共享层**
//! （[`crabmate_client_api::chat_branch`]），因为 Desktop 是同一条语义，两处各写一份就会漂移。
//!
//! 为什么走这条而不是「本地截断 + 重发同一句」：Server 现在会**持久化**被中断的回合，
//! 本地重发会让会话里多出第二条相同的用户消息（Desktop 的 CHANGELOG 记过这次分叉）。

use crabmate_client_api::chat_branch::{
    BranchFailure, ChatBranchRequestBody, classify_branch_response,
};
use crabmate_client_api::paths;
use reqwest::header::{CONTENT_TYPE, HeaderValue};

use crate::serve::client::{REQUEST_TIMEOUT, ServeClient};

/// `POST /chat/branch`：成功返回服务端的新 `revision`。
///
/// 失败三分类见 [`BranchFailure`]：调用方据它决定「清绑定 / 提示先刷新 / 原样上抛」。
pub async fn post_chat_branch(
    client: &ServeClient,
    conversation_id: &str,
    before_user_ordinal: u64,
    expected_revision: u64,
) -> Result<u64, BranchFailure> {
    let url = client
        .url(paths::CHAT_BRANCH)
        .map_err(|e| BranchFailure::Other(e.to_string()))?;
    let headers = client
        .auth_headers()
        .map_err(|e| BranchFailure::Other(e.to_string()))?;
    let body = ChatBranchRequestBody {
        conversation_id: conversation_id.to_string(),
        before_user_ordinal,
        expected_revision,
    };
    let resp = client
        .http()
        .post(&url)
        .headers(headers)
        .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
        .json(&body)
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(|e| BranchFailure::Other(format!("/chat/branch 请求失败：{e}")))?;
    let status = resp.status().as_u16();
    // 4xx/5xx 的正文也要读：错误码与原因就在里面。
    let text = resp.text().await.unwrap_or_default();
    classify_branch_response(status, &text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serve::ConnectionConfig;

    fn client() -> ServeClient {
        ServeClient::new(ConnectionConfig::new("http://127.0.0.1:9", "")).expect("build client")
    }

    /// 请求体字段名必须与契约一致（服务端 `deny_unknown_fields`）：改名会直接 400。
    #[test]
    fn request_body_uses_the_contract_field_names() {
        let body = ChatBranchRequestBody {
            conversation_id: "c1".into(),
            before_user_ordinal: 3,
            expected_revision: 9,
        };
        let json = serde_json::to_value(&body).expect("serialize");
        assert_eq!(json["conversation_id"], "c1");
        assert_eq!(json["before_user_ordinal"], 3);
        assert_eq!(json["expected_revision"], 9);
    }

    /// 连不上时是 `Other`（不是 NotFound）：不能因为网络抖动就去清 `conversation_id` 绑定。
    #[tokio::test]
    async fn unreachable_server_is_not_a_not_found() {
        let err = post_chat_branch(&client(), "c1", 0, 1)
            .await
            .expect_err("必须失败");
        assert!(
            matches!(err, BranchFailure::Other(_)),
            "网络失败要归到 Other，got {err:?}"
        );
    }
}
