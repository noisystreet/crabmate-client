//! 回合末的用量补拉：把「这一轮没带 `tiktokenPromptTokens`」这个洞补上。
//!
//! 从 `mod.rs` 拆出（单文件 ≤920 行门禁）。
//!
//! 状态行的 `ctx` 段只有两个来源：流帧的用量快照（`RUN_FINISHED` /
//! `conversation_saved`）与 `GET /conversation/messages` 的自带快照。新会话尚无 cid 时
//! 还能回落 `/status` 的 system 基线（HEAD 的 `~12k`），但**首轮结束绑定 cid 之后**
//! 基线就不再适用（`UiState::context_text` 只认「无 cid」），此时若这一轮流帧没带用量，
//! 状态行就会从 `~12k` 掉回占位 `—`。
//!
//! Desktop 没有这个问题：它流收尾即 `bump_session_hydrate_nonce` 重新拉
//! `GET /conversation/messages`，用响应里的 `tiktoken_prompt_tokens` 兜底
//! （`frontend/src/app/chat/session_hydrate.rs`）。TUI 原先只在切换 / 恢复会话时水合
//! （`history::replay_into`），回合末没有这一步，于是同一件事在 Desktop 上被水合掩盖、
//! 在 TUI 上直接表现为 `—`。

use crabmate_client_api::prompt_tokens::TiktokenPromptTokensSnapshot;

use super::LaneApp;
use super::worker::WorkerJob;

impl LaneApp<'_> {
    /// 首轮结束后绑上 cid 时补一次用量（只在**还没有实测**时发，避免每轮白拉一页历史）。
    ///
    /// 调用点是 `on_turn_done`：此时 `st.conversation_id` 刚被回填，基线分支同时失效 ——
    /// 两条路要么补上实测，要么掉回占位。
    pub(super) fn hydrate_usage_if_unmeasured(&mut self, conversation_id: &str) {
        if self.st.has_usage_snapshot() {
            return;
        }
        let _ = self.job_tx.send(WorkerJob::RefreshUsage {
            conversation_id: conversation_id.to_string(),
        });
    }

    /// 回合末补拉回来的快照：只在这一轮所属的会话**仍是当前会话**时应用。
    ///
    /// 回包是异步的 —— 期间用户切走、`/conv new` 或恢复别的会话的话，这个数字属于上一个
    /// 会话（水位按 `conversation_id` 归属，见 ADR 0005），不能落到当前会话的 `ctx` 上。
    pub(super) fn on_usage_hydrated(
        &mut self,
        conversation_id: &str,
        snapshot: Option<TiktokenPromptTokensSnapshot>,
    ) {
        if self.st.conversation_id.as_deref() != Some(conversation_id) {
            return;
        }
        if let Some(snap) = snapshot {
            self.st.set_prompt_tokens(snap);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use crate::serve::ConversationMessagesResponse;

    use super::super::test_support::{test_app, test_client, test_prefs};
    use super::super::worker::WorkerJob;
    use super::*;

    /// 与 `GET /conversation/messages` / 流帧同形的快照（字段全填，便于断言）。
    fn snap() -> TiktokenPromptTokensSnapshot {
        TiktokenPromptTokensSnapshot {
            prompt_tokens: 1_200,
            tiktoken_model: "gpt-4o".to_string(),
            used_input_tokens: Some(1_500),
            max_input_tokens: Some(9_000),
            reserved_output_tokens: None,
            message_tokens: None,
            tool_schema_tokens: None,
            attachment_tokens: None,
            counting_source: Some("provider_usage".to_string()),
            provider_input_tokens: None,
        }
    }

    /// 这一轮没带实测（`snapshot` 为空）：回合末必须请求补拉，否则状态行只剩占位。
    #[test]
    fn turn_end_without_measurement_asks_for_a_usage_refresh() {
        let client = test_client();
        let mut prefs = test_prefs();
        let (job_tx, job_rx) = mpsc::channel();
        let (ui_tx, ui_rx) = mpsc::channel();
        let mut app = test_app(&client, &mut prefs, ui_rx, ui_tx, job_tx);

        app.st.conversation_id = Some("c1".to_string());
        app.hydrate_usage_if_unmeasured("c1");

        let job = job_rx.try_recv().expect("应请求一次用量补拉");
        match job {
            WorkerJob::RefreshUsage { conversation_id } => assert_eq!(conversation_id, "c1"),
            _ => panic!("期望 RefreshUsage"),
        }
    }

    /// 流帧已经给了实测：不再多拉一页历史（Desktop 的每轮水合比这里贵，TUI 只在缺时补）。
    #[test]
    fn measured_turn_does_not_ask_again() {
        let client = test_client();
        let mut prefs = test_prefs();
        let (job_tx, job_rx) = mpsc::channel();
        let (ui_tx, ui_rx) = mpsc::channel();
        let mut app = test_app(&client, &mut prefs, ui_rx, ui_tx, job_tx);

        app.st.conversation_id = Some("c1".to_string());
        app.st.set_prompt_tokens(snap());
        app.hydrate_usage_if_unmeasured("c1");

        assert!(job_rx.try_recv().is_err(), "已有实测就不该再发用量补拉任务");
    }

    /// 回包晚到时用户已经切走：这个数字属于上一个会话，不能写进当前会话。
    #[test]
    fn hydrated_snapshot_applies_only_to_its_own_conversation() {
        let client = test_client();
        let mut prefs = test_prefs();
        let (job_tx, _job_rx) = mpsc::channel();
        let (ui_tx, ui_rx) = mpsc::channel();
        let mut app = test_app(&client, &mut prefs, ui_rx, ui_tx, job_tx);

        app.st.conversation_id = Some("c2".to_string());
        app.on_usage_hydrated("c1", Some(snap()));
        assert!(
            !app.st.has_usage_snapshot(),
            "切走之后回来的快照必须丢弃（水位按 cid 归属）"
        );

        app.on_usage_hydrated("c2", Some(snap()));
        assert!(app.st.has_usage_snapshot(), "当前会话的快照要落地");
    }

    /// 拉到了响应但没有 `tiktoken_prompt_tokens`（老会话 / 服务端未计数）：保持占位，不编数字。
    #[test]
    fn missing_snapshot_keeps_the_placeholder() {
        let client = test_client();
        let mut prefs = test_prefs();
        let (job_tx, _job_rx) = mpsc::channel();
        let (ui_tx, ui_rx) = mpsc::channel();
        let mut app = test_app(&client, &mut prefs, ui_rx, ui_tx, job_tx);

        app.st.conversation_id = Some("c1".to_string());
        app.on_usage_hydrated("c1", None);
        assert!(!app.st.has_usage_snapshot());
        assert_eq!(
            app.st.context_text(Some("1000000"), Some(12_000)),
            "— / 1000k"
        );
    }

    /// 类型名在测试里出现一次，确保 `ConversationMessagesResponse` 仍是快照的宿主
    /// （`job_refresh_usage` 从它里面取 `tiktoken_prompt_tokens`；字段一改名这里就编译失败）。
    #[test]
    fn hydration_response_still_carries_the_snapshot() {
        let resp: ConversationMessagesResponse = serde_json::from_value(serde_json::json!({
            "conversation_id": "c1",
            "revision": 1,
            "messages": [],
            "tiktoken_prompt_tokens": { "prompt_tokens": 1200 }
        }))
        .expect("response parses");
        assert_eq!(
            resp.tiktoken_prompt_tokens.expect("snapshot").prompt_tokens,
            1_200
        );
    }
}
