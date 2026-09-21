//! 将 SSE / 水合得到的 tiktoken 快照写入 [`crate::chat_session_state::ConversationPromptTokenHydrate`]。
//!
//! 快照 DTO 与双键（camelCase / snake_case）解析由共享层
//! [`crabmate_client_api::prompt_tokens`] 提供；此处仅保留绑定 leptos 信号的写入动作。

use leptos::prelude::Set;

use crate::chat_session_state::{ChatSessionSignals, ConversationPromptTokenHydrate};
use crabmate_client_api::prompt_tokens::TiktokenPromptTokensSnapshot;

pub use crabmate_client_api::prompt_tokens::parse_tiktoken_prompt_tokens_value;

/// 流式回合结束或 `conversation_saved` 携带的 tiktoken；`conversation_id` 须与当前绑定会话一致。
pub fn apply_conversation_prompt_tokens_from_sse(
    chat: ChatSessionSignals,
    conversation_id: &str,
    snap: TiktokenPromptTokensSnapshot,
) {
    let cid = conversation_id.trim();
    if cid.is_empty() {
        return;
    }
    chat.conversation_prompt_tokens
        .set(Some(ConversationPromptTokenHydrate {
            conversation_id: cid.to_string(),
            tiktoken: Some(snap),
        }));
}
