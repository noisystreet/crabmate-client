//! `GET /conversation/messages`：服务端已持久化会话的消息分页（TUI 只读回放）。
//!
//! TUI 的 transcript 不持有历史（左栏只读瘦行 `SessionListRow`，见 `sessions.rs`），
//! 历史一律从这里按需拉取。**哪条消息出哪一行**的归一是共享层
//! [`crabmate_client_api::conversation_messages`]，本模块只管 HTTP 与参数编码。

use crabmate_client_api::conversation_messages::CONVERSATION_MESSAGES_PAGE_LIMIT;
use crabmate_client_api::paths;

pub use crabmate_client_api::ConversationMessagesResponse;

use crate::serve::client::ServeClient;
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
