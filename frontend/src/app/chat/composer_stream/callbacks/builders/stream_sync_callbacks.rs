//! `conversation_id` / `conversation_saved.revision` 同步回调工厂。
//!
//! 从 `assemble::build_chat_stream_callbacks` 拆出，避免其 CCN 顶穿棘轮（与 `stream_phase_hooks` 同策略）。
//! 全局 `session_sync` 槽写入门闸见 [`ChatStreamCallbackCtx::is_bound_session_active`](super::super::super::context::ChatStreamCallbackCtx::is_bound_session_active)。

use std::rc::Rc;

use leptos::prelude::*;

use crate::conversation_hydrate::TiktokenPromptTokensSnapshot;
use crate::conversation_prompt_tokens_apply::apply_conversation_prompt_tokens_from_sse;

use super::super::super::context::ChatStreamCallbackCtx;

pub(in super::super) fn make_on_conversation_id_builder(
    stream_ctx: Rc<ChatStreamCallbackCtx>,
) -> Rc<dyn Fn(String)> {
    Rc::new(move |id: String| {
        if stream_ctx.is_stale() {
            return;
        }
        // 全局 session_sync 槽只反映「正在查看的会话」：后台流（已切走）仅落绑定会话记录，
        // 切换回来时由切换 Effect 从记录重推导，避免污染其它会话的同步快照。
        if stream_ctx.is_bound_session_active() {
            stream_ctx
                .chat
                .session_sync
                .update(|s| s.apply_stream_conversation_id(id.clone()));
        }
        stream_ctx.update_bound_session(|s| {
            s.server_conversation_id = Some(id);
            s.server_revision = None;
        });
    })
}

pub(in super::super) fn make_on_conversation_revision_builder(
    stream_ctx: Rc<ChatStreamCallbackCtx>,
) -> Rc<dyn Fn(u64, Option<TiktokenPromptTokensSnapshot>)> {
    Rc::new(
        move |rev: u64, tiktoken: Option<TiktokenPromptTokensSnapshot>| {
            if stream_ctx.is_stale() {
                return;
            }
            if stream_ctx.is_bound_session_active() {
                stream_ctx
                    .chat
                    .session_sync
                    .update(|s| s.apply_saved_revision(rev));
            }
            stream_ctx.update_bound_session(|s| {
                s.server_revision = Some(rev);
            });
            if let (Some(snap), Some(cid)) =
                (tiktoken, stream_ctx.server_conversation_id_for_tokens())
            {
                apply_conversation_prompt_tokens_from_sse(stream_ctx.chat, &cid, snap);
            }
        },
    )
}
