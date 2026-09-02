//! 工具相关 SSE 回调工厂（`on_tool_output_chunk` / `on_tool_result` / `on_tool_call`）。

use std::collections::HashMap;
use std::rc::Rc;

use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use super::super::turn_layout::{ResultOnlyToolStep, TurnLayout};
use crate::api::{OnToolCallFn, fetch_tool_job_output, fetch_tool_job_status};
use crate::i18n;
use crate::i18n::Locale;
use crate::message_format::tool_stored_text_from_result_info;
use crate::session_ops::{make_message_id, message_created_ms};
use crate::sse_dispatch::{ToolJobState, ToolOutputChunkInfo, ToolResultInfo};
use crate::storage::{StoredMessage, StoredMessageState};
use crate::timeline_scan::timeline_state_tool;

use super::super::super::context::ChatStreamCallbackCtx;
use super::super::super::per_stream_accum::PerStreamAccum;
use super::super::super::stream_control_reducer::StreamControlEvent;
use super::super::helpers::*;

/// 客户端本地输出保留上限（服务端环形 ≤256 KiB；此处兜底，防长时间任务跨轮累积无界）。
const LOCAL_OUTPUT_MAX_BYTES: usize = 1024 * 1024;

/// 轮询节奏：有新输出（活跃）用快档；连续空闲按 ×2 退避到 `idle_cap`。
const MIN_LIVE_MS: u32 = 500;
/// 实时流开启时的空闲退避上限（有输出立即回快档；接近"实时"且空闲不空转）。
const MAX_IDLE_MS: u32 = 30_000;
/// 兼容回退（旧 serve 无 `/output`）时的空闲退避上限：无输出事件信号，故收紧以保
/// 持终态/取消按钮延迟可感知（历史指数退避最坏 ~6.4s，此处封顶 5s）。
const MAX_FALLBACK_MS: u32 = 5_000;

/// 下一轮等待时长：本 tick 有新输出 → 回到快档；否则 ×2 退避（封顶）。
fn next_poll_interval(current_ms: u32, got_new: bool, fast_ms: u32, idle_cap_ms: u32) -> u32 {
    if got_new {
        fast_ms
    } else {
        current_ms.saturating_mul(2).min(idle_cap_ms)
    }
}

/// 把 `/output` 本 tick 增量并入本地窗口并执行上限裁剪（超限丢最旧、标 `truncated`）。
/// 独立成小函数以控制轮询循环复杂度（lizard CCN ≤ 10）。
fn append_output_window(
    poll: &crate::sse_dispatch::ToolJobOutputPoll,
    lines: &mut Vec<crate::sse_dispatch::ToolJobOutputLine>,
    head: &mut usize,
    out_bytes: &mut usize,
    truncated: &mut bool,
) {
    for it in &poll.items {
        *out_bytes += it.text.len();
        lines.push(it.clone());
    }
    while *out_bytes > LOCAL_OUTPUT_MAX_BYTES && lines.len() > *head + 1 {
        *out_bytes = out_bytes.saturating_sub(lines[*head].text.len());
        *head += 1;
        *truncated = true;
    }
}

/// 后台任务轮询：**自适应退避**——有输出时 ~500ms 快档实时滚动（`tail -f` 体感），
/// 连续空闲按 ×2 退避到 30s（旧 serve 回退路径封顶 5s），收到新输出立即回快档；
/// 终态后**无间隔**继续排水直到 `eof=true` 再停止。
/// 每轮同时取一次状态快照（终态 `exit_code`/`summary`、取消按钮可见性由响应式重建保证）。
/// 更新 `tool_job_states` map（tool_call_id → 快照），供气泡输出/状态/取消按钮响应式刷新。
///
/// **兼容回退**：旧服务端无 `/output` 路由（404）时实时流自动关闭，回退到纯状态轮询
/// （行为同历史版本：拉到终态即停）；`404`/`410`（不存在/过期）与网络错误仍停止，保留
/// 最后一次已知快照。本 tick 取不到输出且已终态时立即停止，避免「无 eof 的空转忙轮询」。
fn spawn_tool_job_polling(
    tool_call_id: String,
    job_id: String,
    loc: Locale,
    states: RwSignal<HashMap<String, ToolJobState>>,
) {
    spawn_local(async move {
        let mut cursor: Option<u64> = None;
        let mut lines: Vec<crate::sse_dispatch::ToolJobOutputLine> = Vec::new();
        let mut head: usize = 0;
        let mut out_bytes: usize = 0;
        let mut truncated = false;
        let mut output_ok = true;
        let mut saw_output = false;
        let mut delay_ms = MIN_LIVE_MS;
        let mut idle_cap = MAX_IDLE_MS;
        loop {
            // 1) 实时输出增量（游标续读；环形截断时服务端标 truncated）。
            let poll = if output_ok {
                match fetch_tool_job_output(&job_id, cursor, loc).await {
                    Ok(p) => {
                        saw_output = true;
                        Some(p)
                    }
                    // 首拉即失败（旧服务端无 `/output`）：永久回退状态轮询；
                    // 已成功过则视为瞬时抖动：本 tick 跳过输出，下 tick 自愈。
                    Err(_) => {
                        if !saw_output {
                            output_ok = false;
                            idle_cap = MAX_FALLBACK_MS;
                        }
                        None
                    }
                }
            } else {
                None
            };
            let got_new = poll.as_ref().is_some_and(|p| !p.items.is_empty());
            let poll_present = poll.is_some();
            // 2) 状态快照（终态 exit/summary 等；与输出增量合并后整体入 map）。
            let state = match fetch_tool_job_status(&job_id, loc).await {
                Ok(s) => s,
                Err(_) => break,
            };
            let terminal = state.is_terminal();
            let mut merged = state;
            merged.output_lines = lines[head..].to_vec();
            merged.output_truncated = truncated;
            let mut drained = false;
            if let Some(p) = poll {
                cursor = Some(p.cursor);
                truncated |= p.truncated;
                drained = p.eof;
                append_output_window(&p, &mut lines, &mut head, &mut out_bytes, &mut truncated);
                merged.output_lines = lines[head..].to_vec();
                merged.next_output_cursor = p.cursor;
                merged.output_truncated = truncated;
                merged.output_eof = p.eof;
            }
            states.update(|m| {
                m.insert(tool_call_id.clone(), merged);
            });
            if drained || (terminal && !poll_present) {
                break;
            }
            if !terminal {
                delay_ms = next_poll_interval(delay_ms, got_new, MIN_LIVE_MS, idle_cap);
                TimeoutFuture::new(delay_ms).await;
            }
        }
    });
}

pub(in super::super) fn make_on_tool_output_chunk(
    stream_ctx: Rc<ChatStreamCallbackCtx>,
) -> Rc<dyn Fn(ToolOutputChunkInfo)> {
    Rc::new(move |info: ToolOutputChunkInfo| {
        if stream_ctx.is_stale() {
            return;
        }
        stream_ctx.scratch.apply_stream_control_event(
            &stream_ctx.shell.stream,
            StreamControlEvent::ToolOutputChunk,
        );
        let tid = info.tool_call_id.trim();
        if tid.is_empty() {
            return;
        }
        // 热路径：只写 tool_output_chunks overlay，不写 sessions。
        stream_ctx.chat.tool_output_chunks.update(|m| {
            m.entry(tid.to_string())
                .and_modify(|t| t.push_str(&info.chunk))
                .or_insert_with(|| {
                    if info.name.as_deref() == Some("terminal_session") {
                        crate::message_format::strip_ansi_codes(&info.chunk)
                    } else {
                        info.chunk.clone()
                    }
                });
        });
    })
}

/// 后台任务（`run_command` 的 `async:true`）：登记初始快照并启动轮询（终态落定后停止）。
/// 无 `tool_job_id` 或无 `tool_call_id` 时直接返回（普通工具结果不受影响）。
fn register_background_tool_job(
    states: RwSignal<HashMap<String, ToolJobState>>,
    info: &ToolResultInfo,
    loc: Locale,
) {
    let Some(job_id) = info.tool_job_id.as_deref().filter(|s| !s.is_empty()) else {
        return;
    };
    let Some(tid) = info
        .tool_call_id
        .as_deref()
        .filter(|t| !t.trim().is_empty())
    else {
        return;
    };
    let initial = ToolJobState {
        id: job_id.to_string(),
        status: info
            .tool_job_status
            .clone()
            .unwrap_or_else(|| "queued".to_string()),
        exit_code: None,
        stdout: None,
        stderr: None,
        summary: None,
        error_code: None,
        failure_category: None,
        workspace_changed: false,
        output_lines: Vec::new(),
        next_output_cursor: 0,
        output_truncated: false,
        output_eof: false,
    };
    states.update(|m| {
        m.insert(tid.to_string(), initial);
    });
    spawn_tool_job_polling(tid.to_string(), job_id.to_string(), loc, states);
}

pub(in super::super) fn make_on_tool_result(
    stream_ctx: Rc<ChatStreamCallbackCtx>,
) -> Rc<dyn Fn(ToolResultInfo)> {
    Rc::new(move |info: ToolResultInfo| {
        if stream_ctx.is_stale() {
            return;
        }
        stream_ctx
            .scratch
            .apply_stream_control_event(&stream_ctx.shell.stream, StreamControlEvent::ToolResult);
        let loc = stream_ctx.locale.get_untracked();
        let stored = tool_stored_text_from_result_info(&info, loc);
        let t = stored.compact.clone();
        let detail = stored.detail.clone();

        let id = make_message_id();
        let tl_ok = info.ok.unwrap_or(true);
        let state = timeline_state_tool(&id, tl_ok);
        let stream_ctx_rc = Rc::clone(&stream_ctx);
        let mut updated_existing = false;
        let mut inserted_new_tool = false;
        stream_ctx.update_bound_session(|s| {
            let tid = info
                .tool_call_id
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty());
            let idx_by_tid = tid.and_then(|t| index_of_loading_tool_by_call_id(&s.messages, t));
            let fifo_id = idx_by_tid
                .is_none()
                .then(|| stream_ctx_rc.scratch.take_pending_tool_fifo_head())
                .flatten();
            let idx_opt = idx_by_tid.or_else(|| {
                fifo_id
                    .as_deref()
                    .and_then(|pid| index_of_message_id(&s.messages, pid))
            });
            if let Some(idx) = idx_opt {
                let m = &mut s.messages[idx];
                m.text = t.clone();
                m.reasoning_text = detail.clone();
                m.state = Some(state.clone());
                m.is_tool = true;
                if m.tool_call_id.is_none() {
                    m.tool_call_id = info.tool_call_id.clone().filter(|x| !x.trim().is_empty());
                }
                if let Some(tn) = non_empty_trimmed_tool_name(&info.name) {
                    m.tool_name = Some(tn);
                }
                updated_existing = true;
            }
            if !updated_existing {
                let msg = StoredMessage {
                    id: id.clone(),
                    role: "system".to_string(),
                    text: t.clone(),
                    reasoning_text: detail.clone(),
                    image_urls: vec![],
                    state: Some(state.clone()),
                    is_tool: true,
                    tool_call_id: info.tool_call_id.clone().filter(|x| !x.trim().is_empty()),
                    tool_name: non_empty_trimmed_tool_name(&info.name),
                    created_at: message_created_ms(),
                };
                s.messages.push(msg);
                inserted_new_tool = true;
            }
        });
        // 工具结果已写入 sessions，清除 overlay 中累积的工具输出。
        if let Some(tid) = info
            .tool_call_id
            .as_deref()
            .filter(|t| !t.trim().is_empty())
        {
            stream_ctx.chat.tool_output_chunks.update(|m| {
                m.remove(tid);
            });
        }
        // 后台任务（`run_command` 的 `async:true`）：登记初始快照并启动轮询（终态落定后停止）。
        register_background_tool_job(stream_ctx.chat.tool_job_states, &info, loc);
        if inserted_new_tool {
            let tool_name = non_empty_trimmed_tool_name(&info.name).unwrap_or_default();
            let declared = info
                .tool_call_id
                .as_deref()
                .map(str::trim)
                .filter(|tid| !tid.is_empty())
                .map(|tid| ResultOnlyToolStep {
                    tool_call_id: tid,
                    name: tool_name.as_str(),
                    summary: t.as_str(),
                });
            TurnLayout::on_tool_result_inserted(&stream_ctx, id.as_str(), declared);
        } else {
            TurnLayout::pin_loading_tail(&stream_ctx);
        }
    })
}

pub(in super::super) fn chat_stream_on_tool_call_builder(
    stream_ctx: Rc<ChatStreamCallbackCtx>,
    accum: Rc<PerStreamAccum>,
) -> OnToolCallFn {
    Rc::new(
        move |name: String,
              summary: String,
              preview: Option<String>,
              full: Option<String>,
              goal_id: Option<String>,
              tool_call_id: Option<String>| {
            if stream_ctx.is_stale() {
                return;
            }
            TurnLayout::demote_answer_before_tools(stream_ctx.as_ref(), accum.as_ref());
            // demote keep-ui；sync_turn_projection 同帧原子移交（I14），release 幂等补清 overlay。
            stream_ctx.scratch.apply_stream_control_event(
                &stream_ctx.shell.stream,
                StreamControlEvent::ToolCallDeclared,
            );
            let _ = goal_id;
            let loc = stream_ctx.locale.get_untracked();
            let core = if name.trim() == "run_command" {
                let inv = run_command_card_invocation_line(
                    summary.as_str(),
                    preview.as_deref(),
                    full.as_deref(),
                );
                if inv.is_empty() {
                    format!("{}{}", i18n::tool_card_prefix(loc), name.trim())
                } else {
                    inv
                }
            } else if !summary.trim().is_empty() {
                summary.trim().to_string()
            } else if !name.trim().is_empty() {
                format!("{}{}", i18n::tool_card_prefix(loc), name.trim())
            } else {
                i18n::tool_card_fallback(loc).to_string()
            };
            let text = to_single_line(&core, 180);
            let detail = if name.trim() == "run_command" && !core.is_empty() {
                format!("tool: {name}\nstatus: running\n$ {core}")
            } else if !name.trim().is_empty() {
                format!("tool: {name}\nstatus: running")
            } else {
                "status: running".to_string()
            };
            let id = make_message_id();
            let tcid = tool_call_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let tool_msg = StoredMessage {
                id: id.clone(),
                role: "system".to_string(),
                text,
                reasoning_text: detail,
                image_urls: vec![],
                state: Some(StoredMessageState::Loading),
                is_tool: true,
                tool_call_id: tcid.clone(),
                tool_name: non_empty_trimmed_tool_name(&name),
                created_at: message_created_ms(),
            };
            if let Some(ref tcid) = tcid {
                stream_ctx
                    .scratch
                    .on_turn_tool_call(tcid.as_str(), name.trim(), core.as_str());
                TurnLayout::on_tool_call_declared(stream_ctx.as_ref(), tool_msg);
                stream_ctx.scratch.sync_turn_projection(stream_ctx.as_ref());
                TurnLayout::release_loading_after_tool_projection(stream_ctx.as_ref());
                stream_ctx.scratch.sync_stream_preview(stream_ctx.as_ref());
            } else {
                TurnLayout::on_tool_call_declared(stream_ctx.as_ref(), tool_msg);
                stream_ctx.scratch.enqueue_pending_tool_message_id(id);
            }
        },
    )
}
