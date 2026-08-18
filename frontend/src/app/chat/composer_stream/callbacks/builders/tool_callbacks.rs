//! 工具相关 SSE 回调工厂（`on_tool_output_chunk` / `on_tool_result` / `on_tool_call`）。

use std::collections::HashMap;
use std::rc::Rc;

use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use super::super::turn_layout::{ResultOnlyToolStep, TurnLayout};
use crate::api::OnToolCallFn;
use crate::api::fetch_tool_job_status;
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

/// 后台任务轮询（指数退避，`200ms << min(attempt,5)`；终态或错误即停止）。
/// 更新 `tool_job_states` map（tool_call_id → 快照），供气泡状态/取消按钮响应式刷新。
fn spawn_tool_job_polling(
    tool_call_id: String,
    job_id: String,
    loc: Locale,
    states: RwSignal<HashMap<String, ToolJobState>>,
) {
    spawn_local(async move {
        let mut attempt: u32 = 0;
        loop {
            match fetch_tool_job_status(&job_id, loc).await {
                Ok(state) => {
                    let terminal = state.is_terminal();
                    states.update(|m| {
                        m.insert(tool_call_id.clone(), state);
                    });
                    if terminal {
                        break;
                    }
                }
                // 404/410（不存在/过期）或网络错误：停止轮询，保留最后一次已知快照。
                Err(_) => break,
            }
            attempt = attempt.saturating_add(1);
            let ms = 200u64.saturating_mul(1u64 << attempt.min(5));
            TimeoutFuture::new(ms as u32).await;
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
