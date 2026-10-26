//! `GET /conversation/messages`：服务端已持久化会话的消息分页（TUI 只读回放）。
//!
//! TUI 的 transcript 不持有历史（左栏只读瘦行 `SessionListRow`，见 `sessions.rs`），
//! 历史一律从这里按需拉取。**哪条消息出哪一行**的归一是共享层
//! [`crabmate_client_api::conversation_messages`]，本模块只管 HTTP 与参数编码。

use crabmate_client_api::conversation_messages::CONVERSATION_MESSAGES_PAGE_LIMIT;
use crabmate_client_api::conversation_state::{
    ConversationStateOutcome, ConversationStatePatch, classify_conversation_state_response,
};
use crabmate_client_api::paths;
use reqwest::header::{CONTENT_TYPE, HeaderValue};

pub use crabmate_client_api::ConversationMessagesResponse;

use crate::serve::client::{REQUEST_TIMEOUT, ServeClient};
use crate::serve::error::TermError;

/// 拉一页消息（`before_index = None` 取最新一页）。
///
/// 服务端对未知/已过期 `conversation_id` 返回错误而不是空数组，故失败要如实上抛：
/// 回放失败时界面提示「历史载入失败」比静默显示空 transcript 诚实。
pub async fn fetch_conversation_messages(
    client: &ServeClient,
    conversation_id: &str,
    before_index: Option<u32>,
) -> Result<ConversationMessagesResponse, TermError> {
    let path = paths::conversation_messages(
        conversation_id,
        Some(CONVERSATION_MESSAGES_PAGE_LIMIT),
        before_index,
    );
    client.get_json(&path).await
}

/// `POST /conversation/state`：会话级运行态意图的**空闲态**写入口。
///
/// 返回**原始**状态码 + body，不在本层判定结局。这条路由的 404 有两种含义
/// （老 serve 没有路由 / 会话不存在），判别规则在共享层
/// [`crabmate_client_api::conversation_state::classify_conversation_state_response`]，
/// 前端与 TUI 各写一份就是新的漂移点（Desktop `api/http_conversation_state.rs` 同口径：
/// 那边也是「拿到响应就交给共享层」，不预先 `ensure_ok`）。
///
/// 因此这里**不能**走 [`ServeClient::post_json`]：那个在非 2xx 时直接变成 `Err`，
/// 把「服务端拒绝了什么」和「压根没连上」压成同一个东西。
pub async fn post_conversation_state(
    client: &ServeClient,
    patch: &ConversationStatePatch,
) -> Result<(u16, String), TermError> {
    let url = client.url(paths::CONVERSATION_STATE)?;
    let mut headers = client.auth_headers()?;
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    let resp = client
        .http()
        .post(&url)
        .headers(headers)
        .json(patch)
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await?;
    let status = resp.status().as_u16();
    // 4xx/5xx 的 body 也要读：`error.code` 正是分类的依据。
    let body = resp.text().await.unwrap_or_default();
    Ok((status, body))
}

/// 把会话级权限等级意图落盘（`POST /conversation/state`，**只发这一列**）。
///
/// 全屏 TUI 与行模式 repl 共用这一处：两处各拼一次 patch、各 match 一次结局，就是同一条
/// 写路径的两个副本 —— 而这条路由的 404 有两义，判别一旦分成两份就会漂移
/// （[`classify_conversation_state_response`] 的注释把这条列为「新的漂移点」）。
///
/// 只发 `permission_level`：服务端是部分更新，顺带回传 `session_mode` / `agent_role`
/// 会把客户端手里的过期值写回去（Desktop `apply_level` 同一条约束）。
pub async fn write_permission_level(
    client: &ServeClient,
    conversation_id: &str,
    level: &str,
) -> Result<ConversationStateOutcome, TermError> {
    let patch = ConversationStatePatch {
        conversation_id: Some(conversation_id.to_string()),
        permission_level: Some(level.to_string()),
        session_mode: None,
        agent_role: None,
    };
    let (status, body) = post_conversation_state(client, &patch).await?;
    Ok(classify_conversation_state_response(status, &body))
}
