//! AG-UI SSE `data:` 块的单行分类：文本 / 思维链 / 命令审批 / 工具调用事件。
//!
//! 独立成模块以免 `chat_stream.rs` 单文件超行数门禁（≤920 行 / CCN ≤10）。

use crabmate::cm_sse_protocol::{AgUiParseDispatch, classify_ag_ui_sse_data};
use serde_json::Value;

use crate::approval::{CommandApprovalRequest, parse_command_approval_data};
use crate::error::TermError;

/// 一行 AG-UI SSE 的处置动作（与 Web `parser_v2` / `sse_dispatch` 语义对齐的子集）。
#[derive(Debug)]
pub(crate) enum LineAction {
    Skip,
    WriteOut(String),
    WriteErr(String),
    Approve(CommandApprovalRequest),
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

pub(crate) fn classify_line(line: &str) -> Result<LineAction, TermError> {
    if let Some(action) = classify_ag_ui_line(line)? {
        return Ok(action);
    }
    Ok(match classify_ag_ui_sse_data(line) {
        AgUiParseDispatch::Plain => LineAction::Plain(line.to_string()),
        AgUiParseDispatch::Handled | AgUiParseDispatch::StreamEnded => LineAction::Skip,
    })
}

fn classify_ag_ui_line(line: &str) -> Result<Option<LineAction>, TermError> {
    let Ok(val) = serde_json::from_str::<Value>(line) else {
        return Ok(None);
    };
    let Some(t) = val.get("type").and_then(|x| x.as_str()) else {
        return Ok(None);
    };
    Ok(Some(match t {
        "TEXT_MESSAGE_CONTENT" => LineAction::WriteOut(delta_string(&val)),
        "REASONING_MESSAGE_CONTENT" => LineAction::WriteErr(delta_string(&val)),
        "RUN_FINISHED" => LineAction::Skip,
        "RUN_ERROR" => return Err(run_error_from_value(&val)),
        "TOOL_CALL_START" => LineAction::ToolStart {
            tool_call_id: string_field(&val, "toolCallId"),
            name: string_field(&val, "name"),
        },
        // 参数流与工具结束标记不单独成行（摘要行只在 START / 收尾 RESULT 更新）。
        "TOOL_CALL_ARGS" | "TOOL_CALL_END" => LineAction::Skip,
        "TOOL_CALL_RESULT" => classify_tool_result(&val),
        "CUSTOM" => classify_custom(&val),
        _ => LineAction::Skip,
    }))
}

/// 收尾 `TOOL_CALL_RESULT`：提取结果标记与摘要。partial 输出块（`metadata.partial=true`）
/// 属于长时工具的输出流，跳过以免刷屏——摘要行只由真正收尾帧更新。
fn classify_tool_result(val: &Value) -> LineAction {
    let metadata = val.get("metadata");
    if metadata.is_some_and(|m| m.get("partial").and_then(Value::as_bool) == Some(true)) {
        return LineAction::Skip;
    }
    let name = metadata
        .and_then(|m| m.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("tool");
    let ok = metadata.and_then(|m| m.get("ok")).and_then(Value::as_bool);
    // 摘要优先 `metadata.summary`；缺省回退输出内容截断（不跨行）。
    let note = metadata
        .and_then(|m| m.get("summary"))
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
        .or_else(|| {
            val.get("content")
                .and_then(Value::as_str)
                .and_then(|s| s.lines().next())
                .map(trim_tail)
                .filter(|s| !s.is_empty())
        });
    LineAction::ToolResult {
        tool_call_id: string_field(val, "toolCallId"),
        name: name.to_string(),
        ok,
        note,
    }
}

fn string_field(val: &Value, key: &str) -> String {
    val.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// 截断到 ≤160 字符（字符级，不切代理对）。
fn trim_tail(s: &str) -> String {
    s.chars().take(160).collect()
}

fn classify_custom(val: &Value) -> LineAction {
    if val.get("customType").and_then(|n| n.as_str()) != Some("command_approval") {
        return LineAction::Skip;
    }
    let data = val.get("data").cloned().unwrap_or(Value::Null);
    LineAction::Approve(parse_command_approval_data(&data))
}

fn delta_string(val: &Value) -> String {
    val.get("delta")
        .and_then(|d| d.as_str())
        .unwrap_or("")
        .to_string()
}

fn run_error_from_value(val: &Value) -> TermError {
    let msg = val
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
        .unwrap_or("RUN_ERROR");
    TermError::RunError(msg.to_string())
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
}
