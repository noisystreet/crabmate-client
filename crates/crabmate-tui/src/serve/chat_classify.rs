//! AG-UI SSE `data:` 块的单行分类：文本 / 思维链 / 命令审批 / 工具调用事件。
//!
//! AG-UI 纯解析已下沉 `crabmate-client-api::ag_ui_parser`；本模块只把共享解析器的回调
//! 收敛成终端所需的 [`LineAction`]（收集器模式），不再自带一份 AG-UI 子集。
//!
//! 独立成模块以免 `chat_stream.rs` 单文件超行数门禁（≤920 行 / CCN ≤10）。

use std::cell::RefCell;

use crabmate_client_api::ag_ui_parser::parse_ag_ui_line;
use crabmate_client_api::sse_dispatch::{
    CommandApprovalData, SseClarifyTraceHooks, SseControlSink, SseDispatch, SseNoticeTimelineHooks,
    SseTurnPhaseHooks, SseWorkspaceToolHooks, ToolResultInfo,
};
use serde_json::Value;

use crate::serve::error::TermError;

/// 摘要行缺省工具名（`metadata.name` 缺失时）。
const DEFAULT_TOOL_NAME: &str = "tool";

/// 摘要行字符上限（字符级截断，不切代理对）。
const NOTE_MAX_CHARS: usize = 160;

/// 畸形审批载荷提示。serve 端审批等待无超时，静默跳过会让回合挂起直到用户停止。
const MALFORMED_APPROVAL_HINT: &str =
    "[crabmate-tui] command_approval 数据形状不符契约，已跳过审批；如回合无响应请停止该回合";

/// 一行 AG-UI SSE 的处置动作（与 Web `parser_v2` / `sse_dispatch` 语义对齐的子集）。
#[derive(Debug)]
pub(crate) enum LineAction {
    Skip,
    WriteOut(String),
    WriteErr(String),
    /// 系统提示行（stderr / 全屏 System 事件），如畸形审批数据提醒。
    System(String),
    Approve(CommandApprovalData),
    /// `TOOL_CALL_START`：工具开始（显示工具行开始态）。
    ToolStart {
        tool_call_id: String,
        name: String,
    },
    /// `TOOL_CALL_RESULT`（非 partial 收尾）：工具结束 + 结果摘要。
    ToolResult {
        tool_call_id: String,
        name: String,
        ok: Option<bool>,
        note: Option<String>,
    },
    Plain(String),
}

/// 单行 AG-UI SSE → 终端动作。
pub(crate) fn classify_line(line: &str) -> Result<LineAction, TermError> {
    let acc = RefCell::new(Collected::default());
    let dispatch = parse_with_collector(line, &acc);
    if dispatch == SseDispatch::Plain {
        // 未知 `type` 的 AG-UI 事件：共享解析器按纯文本回落，TUI 保持现状静默跳过
        // （终端不把原始 JSON 打进聊天正文）。
        if is_ag_ui_event(line) {
            return Ok(LineAction::Skip);
        }
        return Ok(LineAction::Plain(line.to_string()));
    }
    let collected = std::mem::take(&mut *acc.borrow_mut());
    resolve(collected)
}

/// 用一次性收集器跑共享解析器：钩子只写槽位、永不失败，故无需 `?` 传播。
fn parse_with_collector(line: &str, acc: &RefCell<Collected>) -> SseDispatch {
    let mut on_error = |msg: String| acc.borrow_mut().run_error(msg);
    let mut on_delta = |s: String| acc.borrow_mut().text(s);
    let mut on_reasoning_delta = |s: String| acc.borrow_mut().reasoning(s);
    let mut on_tool_call = |name: String,
                            _summary: String,
                            _args_preview: Option<String>,
                            _args_full: Option<String>,
                            _goal_id: Option<String>,
                            tool_call_id: Option<String>| {
        acc.borrow_mut().tool_start(tool_call_id, name);
    };
    let mut on_tool_result = |info: ToolResultInfo| acc.borrow_mut().tool_result(info);
    let mut on_approval = |req: CommandApprovalData| acc.borrow_mut().approve(req);
    let mut on_approval_invalid = || acc.borrow_mut().system(MALFORMED_APPROVAL_HINT);
    let mut sink = SseControlSink {
        on_error: &mut on_error,
        on_delta: Some(&mut on_delta),
        on_reasoning_delta: Some(&mut on_reasoning_delta),
        workspace_tool: SseWorkspaceToolHooks {
            on_tool_call: Some(&mut on_tool_call),
            on_tool_result: Some(&mut on_tool_result),
            on_command_approval_request: Some(&mut on_approval),
            on_command_approval_invalid: Some(&mut on_approval_invalid),
            ..SseWorkspaceToolHooks::default()
        },
        turn_phase: SseTurnPhaseHooks::default(),
        clarify_trace: SseClarifyTraceHooks::default(),
        notice_timeline: SseNoticeTimelineHooks::default(),
    };
    parse_ag_ui_line(line, &mut sink)
}

/// 共享解析器回调的槽位（未消费的事件类型不注册钩子 → 落到 `Skip`）。
#[derive(Default)]
struct Collected {
    /// `TEXT_MESSAGE_CONTENT` 正文增量（同块多行按序拼接）。
    out: Option<String>,
    /// `REASONING_MESSAGE_CONTENT` 思维链增量（同块多行按序拼接）。
    err: Option<String>,
    /// 畸形 `command_approval` 提示行。
    system: Option<String>,
    approval: Option<CommandApprovalData>,
    tool_start: Option<(Option<String>, String)>,
    tool_result: Option<ToolResultInfo>,
    run_error: Option<String>,
}

impl Collected {
    fn text(&mut self, s: String) {
        self.out.get_or_insert_with(String::new).push_str(&s);
    }

    fn reasoning(&mut self, s: String) {
        self.err.get_or_insert_with(String::new).push_str(&s);
    }

    fn system(&mut self, hint: &str) {
        self.system.get_or_insert_with(|| hint.to_string());
    }

    fn approve(&mut self, req: CommandApprovalData) {
        self.approval.get_or_insert(req);
    }

    fn tool_start(&mut self, tool_call_id: Option<String>, name: String) {
        self.tool_start.get_or_insert((tool_call_id, name));
    }

    fn tool_result(&mut self, info: ToolResultInfo) {
        self.tool_result.get_or_insert(info);
    }

    fn run_error(&mut self, msg: String) {
        self.run_error.get_or_insert(msg);
    }
}

/// 槽位 → 终端动作（单行数据块至多命中一类；多命中时按下方顺序取优先级）。
fn resolve(collected: Collected) -> Result<LineAction, TermError> {
    if let Some(msg) = collected.run_error {
        // 回合终止：交由上层收口（与 `chat_stream` 的 `?` 传播一致）。
        return Err(TermError::RunError(msg));
    }
    Ok(if let Some(req) = collected.approval {
        LineAction::Approve(req)
    } else if let Some(hint) = collected.system {
        LineAction::System(hint)
    } else if let Some((tool_call_id, name)) = collected.tool_start {
        LineAction::ToolStart {
            tool_call_id: tool_call_id.unwrap_or_default(),
            name,
        }
    } else if let Some(info) = collected.tool_result {
        tool_result_action(info)
    } else if let Some(s) = collected.err {
        LineAction::WriteErr(s)
    } else if let Some(s) = collected.out {
        LineAction::WriteOut(s)
    } else {
        LineAction::Skip
    })
}

/// 摘要行：`metadata.summary` 优先；缺省回退 `content` 首行截断（不跨行）。
///
/// partial 输出块（长时工具输出流）不由共享解析器送入 `on_tool_result`，故此处不判 partial。
fn tool_result_action(info: ToolResultInfo) -> LineAction {
    let note = info.summary.filter(|s| !s.trim().is_empty()).or_else(|| {
        info.output
            .lines()
            .next()
            .map(trim_tail)
            .filter(|s| !s.is_empty())
    });
    LineAction::ToolResult {
        tool_call_id: info.tool_call_id.unwrap_or_default(),
        name: if info.name.is_empty() {
            DEFAULT_TOOL_NAME.to_string()
        } else {
            info.name
        },
        ok: info.ok,
        note,
    }
}

/// 该行是否为带 `type` 的合法 AG-UI JSON（用于区分未知事件与纯文本增量）。
fn is_ag_ui_event(line: &str) -> bool {
    let Ok(val) = serde_json::from_str::<Value>(line) else {
        return false;
    };
    val.get("type").and_then(Value::as_str).is_some()
}

/// 截断到 ≤[`NOTE_MAX_CHARS`] 字符（字符级，不切代理对）。
fn trim_tail(s: &str) -> String {
    s.chars().take(NOTE_MAX_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_text_message_content() {
        let data = r#"{"type":"TEXT_MESSAGE_CONTENT","delta":"你好"}"#;
        match classify_line(data).unwrap() {
            LineAction::WriteOut(s) => assert_eq!(s, "你好"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn reasoning_message_content_goes_to_stderr_channel() {
        let data = r#"{"type":"REASONING_MESSAGE_CONTENT","delta":"想想"}"#;
        match classify_line(data).unwrap() {
            LineAction::WriteErr(s) => assert_eq!(s, "想想"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn classifies_command_approval() {
        let data = r#"{"type":"CUSTOM","customType":"command_approval","data":{"command":"rm","args":"-f"}}"#;
        match classify_line(data).unwrap() {
            LineAction::Approve(req) => {
                assert_eq!(req.command, "rm");
                assert_eq!(req.args, "-f");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn malformed_approval_data_surfaces_system_line() {
        let data = r#"{"type":"CUSTOM","customType":"command_approval","data":{}}"#;
        match classify_line(data).unwrap() {
            LineAction::System(s) => {
                assert!(s.contains("command_approval"));
                assert!(s.contains("已跳过审批"));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn tool_call_start_parses_name_and_id() {
        let data = r#"{"type":"TOOL_CALL_START","toolCallId":"tc-1","name":"exec","parentMessageId":"m1"}"#;
        match classify_line(data).unwrap() {
            LineAction::ToolStart { tool_call_id, name } => {
                assert_eq!(tool_call_id, "tc-1");
                assert_eq!(name, "exec");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn tool_args_and_end_are_skipped() {
        assert!(matches!(
            classify_line(r#"{"type":"TOOL_CALL_ARGS","toolCallId":"tc-1","args":"{}"}"#).unwrap(),
            LineAction::Skip
        ));
        assert!(matches!(
            classify_line(r#"{"type":"TOOL_CALL_END","toolCallId":"tc-1"}"#).unwrap(),
            LineAction::Skip
        ));
    }

    #[test]
    fn tool_result_uses_summary_note() {
        let data = r#"{"type":"TOOL_CALL_RESULT","toolCallId":"tc-1","content":"huge output...","metadata":{"name":"exec","ok":true,"summary":"exit 0"}}"#;
        match classify_line(data).unwrap() {
            LineAction::ToolResult {
                tool_call_id,
                name,
                ok,
                note,
            } => {
                assert_eq!(tool_call_id, "tc-1");
                assert_eq!(name, "exec");
                assert_eq!(ok, Some(true));
                assert_eq!(note.as_deref(), Some("exit 0"));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn tool_result_partial_chunk_skipped() {
        let data = r#"{"type":"TOOL_CALL_RESULT","toolCallId":"tc-1","content":"partial out","metadata":{"name":"exec","partial":true}}"#;
        assert!(matches!(classify_line(data).unwrap(), LineAction::Skip));
    }

    #[test]
    fn tool_result_falls_back_to_content_first_line() {
        let data = r#"{"type":"TOOL_CALL_RESULT","toolCallId":"tc-1","content":"line1\nline2","metadata":{"name":"exec","ok":false,"exitCode":2}}"#;
        match classify_line(data).unwrap() {
            LineAction::ToolResult { ok, note, name, .. } => {
                assert_eq!(ok, Some(false));
                assert_eq!(name, "exec");
                assert_eq!(note.as_deref(), Some("line1"));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    /// 未知 AG-UI `type` 静默跳过（不把原始 JSON 当正文打进终端）。
    #[test]
    fn unknown_ag_ui_type_is_skipped() {
        assert!(matches!(
            classify_line(r#"{"type":"NOT_A_REAL_EVENT","x":1}"#).unwrap(),
            LineAction::Skip
        ));
    }

    /// 非 AG-UI 文本增量（非 JSON / 无 `type`）按正文回落。
    #[test]
    fn non_ag_ui_payload_falls_back_to_plain() {
        match classify_line("hello world").unwrap() {
            LineAction::Plain(s) => assert_eq!(s, "hello world"),
            other => panic!("unexpected {other:?}"),
        }
        match classify_line(r#"{"foo":1}"#).unwrap() {
            LineAction::Plain(s) => assert_eq!(s, r#"{"foo":1}"#),
            other => panic!("unexpected {other:?}"),
        }
    }

    /// `RUN_ERROR` 经共享解析器格式化（带 `code` / `request_id` 元信息）后终止回合。
    #[test]
    fn run_error_reports_formatted_reason() {
        let data =
            r#"{"type":"RUN_ERROR","error":{"message":"fail","code":"ERR","requestId":"cm-1"}}"#;
        match classify_line(data) {
            Err(TermError::RunError(msg)) => {
                assert!(msg.contains("fail"));
                assert!(msg.contains("ERR"));
                assert!(msg.contains("cm-1"));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn run_finished_is_skipped() {
        assert!(matches!(
            classify_line(r#"{"type":"RUN_FINISHED","threadId":"t","runId":"r"}"#).unwrap(),
            LineAction::Skip
        ));
    }
}
