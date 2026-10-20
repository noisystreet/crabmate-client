//! 发一轮对话：`ChatStreamOptions` 的装配与交给 worker 的派发。
//!
//! 从 `mod.rs` 拆出（单文件 ≤920 行门禁）。两个函数成对出现的原因：
//! `/resume` 与普通回合必须走 **同一套** 装配，只差一个 `stream_resume`，
//! 否则续流回来的那一轮参数会和原来那次请求不一致（模型、温度、role…）。

use crate::serve::{ChatStreamOptions, StreamCancel, StreamResume, new_approval_session_id};

use super::TuiApp;
use super::resume::TurnEcho;
use super::settings::{build_turn_client_llm, merge_turn, turn_temperature, turn_tool_cache_secs};
use super::state::LineKind;
use super::worker::{TurnRequest, WorkerJob};

impl TuiApp<'_> {
    /// 组装一轮的 `ChatStreamOptions`：override ＞ user-data 合成随轮字段；空白不发送
    /// （回落 serve 默认）。
    pub(super) fn build_turn_opts(
        &self,
        message: &str,
        stream_resume: Option<StreamResume>,
    ) -> ChatStreamOptions {
        let stored = self.persisted.as_ref();
        let model = merge_turn(
            &self.overrides.model,
            stored.and_then(|p| p.model.as_deref()),
        );
        let api_base = merge_turn(
            &self.overrides.api_base,
            stored.and_then(|p| p.api_base.as_deref()),
        );
        let role = merge_turn(
            &self.overrides.agent_role,
            stored.and_then(|p| p.role.as_deref()),
        );
        let session_mode = merge_turn(
            &self.overrides.session_mode,
            stored.and_then(|p| p.session_mode.as_deref()),
        );
        // 随轮 client_llm：override（model/api_base/api_key）＞ user-data（thinking/context）；
        // 装配纯逻辑见 `settings::build_turn_client_llm`（空白不发送，回落 serve 默认）。
        let client_llm = build_turn_client_llm(
            model.as_deref(),
            api_base.as_deref(),
            self.overrides.api_key.as_deref(),
            stored.and_then(|p| p.thinking.as_deref()),
            stored.and_then(|p| p.context_tokens.as_deref()),
        );
        // 温度（顶层 body.temperature，对齐 Desktop）：persisted 存原文，parse 有效才发送。
        let temperature = turn_temperature(stored.and_then(|p| p.temperature.as_deref()));
        // 只读工具缓存禁用（prefs disable=true）时顶层发 `readonly_tool_ttl_cache_secs: 0`。
        let readonly_tool_ttl_cache_secs =
            turn_tool_cache_secs(stored.and_then(|p| p.tool_cache_disabled));
        ChatStreamOptions {
            message: message.to_string(),
            approval_session_id: new_approval_session_id(),
            conversation_id: self.st.conversation_id.clone(),
            client_llm,
            agent_role: role,
            session_mode,
            temperature,
            readonly_tool_ttl_cache_secs,
            stream_resume,
        }
    }

    /// 把已组装好的回合交给 worker：重置运行态、记下原始输入、建取消通道。
    pub(super) fn dispatch_turn(&mut self, opts: ChatStreamOptions) {
        // 在途的历史回包从此没有落点（见 `history::should_apply_reply`）：清掉它，
        // 免得晚到的历史把刚发出的这一轮冲掉。
        self.st.history_pending = None;
        // 新回合：清上轮工具行映射并回到最新视图（搜索词保留、锚点释放）。
        self.st.reset_run_tools();
        self.st.view_offset = 0;
        self.st.release_search_anchor();
        // 断点要能把这一轮原样重发出去（`/resume`）。
        self.sent_turn = Some(TurnEcho {
            message: opts.message.clone(),
            conversation_id: opts.conversation_id.clone(),
        });
        self.st.running = true;
        self.st.cancel_sent = false;
        let cancel = StreamCancel::new();
        self.cancel = Some(cancel.clone());
        let job = TurnRequest {
            opts,
            cancel,
            yes: self.yes,
        };
        if self.job_tx.send(WorkerJob::Turn(Box::new(job))).is_err() {
            self.st.running = false;
            self.sent_turn = None;
            self.st.push_line(LineKind::System, "回合 worker 已退出");
        }
    }
}
