//! SSE 控制面载荷形状与回调分组类型（与 **`dispatch`** 子模块中 **`try_dispatch_sse_control_payload`** 的消费契约一致）。

use serde_json::Value;

use crate::conversation_hydrate::TiktokenPromptTokensSnapshot;

pub use crabmate_client_api::CommandApprovalRequest;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SseDispatch {
    Handled,
    Plain,
    /// 用于 V2Parser 通知 `RUN_FINISHED` / `RUN_ERROR`：`handle_sse_block` 据此设置
    /// `saw_stream_ended` 并触发 `on_stream_ended` / `on_done` / `on_error` 等回调。
    StreamEnded,
}

/// 工作区与工具相关控制面回调（`tool_call` / `tool_running` / 审批等）。
#[allow(clippy::type_complexity)]
#[derive(Default)]
pub struct SseWorkspaceToolHooks<'a> {
    pub on_workspace_changed: Option<&'a mut dyn FnMut()>,
    pub on_tool_call: Option<
        &'a mut dyn FnMut(
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        ),
    >,
    pub on_tool_status_change: Option<&'a mut dyn FnMut(bool)>,
    pub on_parsing_tool_calls_change: Option<&'a mut dyn FnMut(bool)>,
    /// `tool_output_chunk`：工具执行中的输出片段（如 PTY）；最终以 `tool_result` 收束。
    pub on_tool_output_chunk: Option<&'a mut dyn FnMut(ToolOutputChunkInfo)>,
    pub on_tool_result: Option<&'a mut dyn FnMut(ToolResultInfo)>,
    pub on_command_approval_request: Option<&'a mut dyn FnMut(CommandApprovalRequest)>,
}

/// `assistant_answer_phase` 与回合约边界事件（终答相位 / 段落锚点；非已删 staged 编排）。
#[derive(Default)]
pub struct SseTurnPhaseHooks<'a> {
    /// 后续 `on_delta` 为终答正文（此前为思维链）；无链时也会在首段正文前下发。
    pub on_assistant_answer_phase: Option<&'a mut dyn FnMut()>,
    pub on_turn_segment_start: Option<&'a mut dyn FnMut(TurnSegmentStartInfo)>,
    pub on_turn_segment_end: Option<&'a mut dyn FnMut(String)>,
    pub on_turn_tool_phase_end: Option<&'a mut dyn FnMut()>,
}

/// `turn_segment_start` 控制面负载（Web 布局锚点）。
#[derive(Debug, Clone)]
pub struct TurnSegmentStartInfo {
    pub segment_id: String,
    pub kind: String,
    pub before_tool_call_id: Option<String>,
}

/// 澄清问卷与思维迹调试事件。
#[derive(Default)]
pub struct SseClarifyTraceHooks<'a> {
    pub on_clarification_questionnaire: Option<&'a mut dyn FnMut(ClarificationQuestionnaireInfo)>,
    pub on_thinking_trace: Option<&'a mut dyn FnMut(ThinkingTraceInfo)>,
}

/// 会话落盘 revision、`timeline_log`、协议能力等尾部控制面。
#[derive(Default)]
pub struct SseNoticeTimelineHooks<'a> {
    /// `conversation_saved.revision` 与可选 tiktoken；供 `POST /chat/branch` 与底栏上下文用量。
    pub on_conversation_saved_revision:
        Option<&'a mut dyn FnMut(u64, Option<TiktokenPromptTokensSnapshot>)>,
    /// `timeline_log` 事件：审批结果等旁注，写入时间线（不进聊天正文）。
    pub on_timeline_log: Option<&'a mut dyn FnMut(TimelineLogInfo)>,
    /// AG-UI `RUN_FINISHED` / `RUN_ERROR` 触发流结束时的回调（V2Parser 使用）。
    /// 参数为可选 tiktoken（来自 `RUN_FINISHED.tiktokenPromptTokens`；`RUN_ERROR` 为 `None`）。
    pub on_run_finished: Option<&'a mut dyn FnMut(Option<TiktokenPromptTokensSnapshot>)>,
    /// 非终态 `CUSTOM stream_draining`：可提前进入 Draining 文案；**不**标记 `saw_stream_ended`。
    pub on_stream_draining: Option<&'a mut dyn FnMut()>,
    /// AG-UI `STATE_SNAPSHOT`（完整 agent state）。
    ///
    /// Client 当前传 `None`：不在此路径恢复 UI；会话正文/revision 对齐走
    /// `GET /conversation/messages` 水合。保留钩子以便将来按需接线。
    pub on_state_snapshot: Option<&'a mut dyn FnMut(serde_json::Value)>,
}

/// SSE 控制面分发入口：按领域分组回调，V2Parser 分发至此。
pub struct SseControlSink<'a> {
    pub on_error: &'a mut dyn FnMut(String),
    /// AG-UI TEXT_MESSAGE_CONTENT / REASONING_MESSAGE_CONTENT 的正文增量。
    /// V1 路径不经此回调。
    pub on_delta: Option<&'a mut dyn FnMut(String)>,
    pub workspace_tool: SseWorkspaceToolHooks<'a>,
    pub turn_phase: SseTurnPhaseHooks<'a>,
    pub clarify_trace: SseClarifyTraceHooks<'a>,
    pub notice_timeline: SseNoticeTimelineHooks<'a>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // 与后端 JSON 同形；UI 当前仅消费 `chunk` 追加详情。
pub struct ToolOutputChunkInfo {
    pub tool_call_id: String,
    pub name: Option<String>,
    pub seq: u64,
    pub chunk: String,
    pub stream: Option<String>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // 与后端 JSON 同形；展示层当前仅用 name/summary。
pub struct ToolResultInfo {
    pub name: String,
    pub goal_id: Option<String>,
    /// 与对应 `tool_call.tool_call_id` 对齐；缺省时前端按 FIFO 与占位气泡配对。
    pub tool_call_id: Option<String>,
    /// 与 `crabmate_tool.v` 对齐；缺省按 **1**（与后端 `serde(default)` 一致）。
    pub result_version: u32,
    pub summary: Option<String>,
    pub output: String,
    pub ok: Option<bool>,
    pub exit_code: Option<i64>,
    pub error_code: Option<String>,
    /// 与 Rust `tool_error::ToolFailureCategory` 蛇形字符串同源（`invalid_input` 等）。
    pub failure_category: Option<String>,
    /// 可选：与 `read_file` / `read_dir` / `list_tree` 工具输出首行 **`crabmate_tool_output`** 同源（SSE 侧复制），便于 UI 表格化。
    pub structured_preview: Option<Value>,
    /// 后台任务启动帧软字段（`run_command` 的 `async:true`；契约 `background_tool_jobs_contract.md` §2）。
    pub tool_job_id: Option<String>,
    pub tool_job_poll_url: Option<String>,
    /// 发起时恒为 `queued`；后续经轮询端点更新。
    pub tool_job_status: Option<String>,
}

/// 后台工具任务（`run_command` 的 `async:true`）运行态快照（轮询 `GET /tools/jobs/{id}` 结果）。
/// 与契约 §3.1 响应体逐字段对齐；`tool_job_poll_url` 仅存在于启动帧（[`ToolResultInfo`]），不在轮询响应中。
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct ToolJobState {
    #[serde(rename = "tool_job_id")]
    pub id: String,
    /// `queued` | `running` | `succeeded` | `failed` | `cancelled` | `timed_out`。
    pub status: String,
    pub exit_code: Option<i64>,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
    pub summary: Option<String>,
    pub error_code: Option<String>,
    pub failure_category: Option<String>,
    pub workspace_changed: bool,
}

impl ToolJobState {
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        !matches!(self.status.as_str(), "queued" | "running")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 契约 `background_tool_jobs_contract.md` §3.1 轮询响应样例：
    /// 无 `tool_job_poll_url` 字段，必须能反序列化（曾因该字段缺失而 missing field 失败）。
    #[test]
    fn tool_job_state_deserializes_from_contract_poll_response() {
        let raw = r#"{
            "tool_job_id": "tooljob_0123456789abcdef0123456789abcdef",
            "status": "succeeded",
            "exit_code": 0,
            "stdout": "out",
            "stderr": "",
            "summary": "cmd",
            "error_code": null,
            "failure_category": null,
            "workspace_changed": true,
            "result_version": 1
        }"#;
        let state: ToolJobState =
            serde_json::from_str(raw).expect("poll response must deserialize");
        assert_eq!(state.id, "tooljob_0123456789abcdef0123456789abcdef");
        assert_eq!(state.status, "succeeded");
        assert_eq!(state.exit_code, Some(0));
        assert!(state.is_terminal());
    }

    #[test]
    fn tool_job_state_non_terminal_statuses() {
        for s in ["queued", "running"] {
            let state = ToolJobState {
                id: "job-1".into(),
                status: s.into(),
                exit_code: None,
                stdout: None,
                stderr: None,
                summary: None,
                error_code: None,
                failure_category: None,
                workspace_changed: false,
            };
            assert!(!state.is_terminal(), "{s} should be non-terminal");
        }
        for s in ["succeeded", "failed", "cancelled", "timed_out"] {
            let state = ToolJobState {
                id: "job-1".into(),
                status: s.into(),
                exit_code: None,
                stdout: None,
                stderr: None,
                summary: None,
                error_code: None,
                failure_category: None,
                workspace_changed: false,
            };
            assert!(state.is_terminal(), "{s} should be terminal");
        }
    }
}

/// `clarification_questionnaire`：Web 表单用字段子集。
#[derive(Debug, Clone)]
pub struct ClarificationQuestionnaireInfo {
    pub questionnaire_id: String,
    pub intro: String,
    pub fields: Vec<ClarificationFormField>,
}

#[derive(Debug, Clone)]
pub struct ClarificationFormField {
    pub id: String,
    pub label: String,
    pub hint: Option<String>,
    pub required: bool,
}

/// `thinking_trace`：Web 调试台用（不进聊天正文）。
#[derive(Debug, Clone)]
pub struct ThinkingTraceInfo {
    pub op: String,
    pub node_id: Option<String>,
    pub parent_id: Option<String>,
    pub title: Option<String>,
    pub chunk: Option<String>,
    pub context_snapshot: Option<String>,
}

/// `timeline_log`：Web 时间线旁注（审批结果等；不进聊天正文）。
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct TimelineLogInfo {
    pub kind: String,
    pub title: String,
    pub detail: Option<String>,
}
