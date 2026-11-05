//! 单轮 `/chat/stream` 的粗粒度生命周期（UI 门闩与 attach 代际的唯一来源）。
//!
//! 由 `frontend/src/app/chat/turn_lifecycle/reducer.rs` 机械迁入（#458）：逻辑逐行保持，
//! Leptos 信号仍在壳侧 dispatch。归约与四个谓词由 Web / TUI 共用同一份实现，
//! 避免「改一边忘了另一边」——此前这条约定只写在注释里。
//!
//! 与 attach 内 SSE 消费进度（前端 `composer_stream::stream_control_reducer`）分工不变：
//! 后者描述单次 attach 的进度，本模块收敛**壳层 busy** 语义。

/// 由各 `ChatStreamCallbacks` 路径喂入的语义事件（非原始 JSON）。
///
/// 原定义在 `frontend/src/app/chat/composer_stream/stream_control_reducer.rs`；因
/// [`TurnLifecycleEvent::SseControl`] 依赖它，一并迁入（前端保留同路径 re-export）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamControlEvent {
    ModelTextDelta,
    AssistantAnswerPhase,
    ToolCallDeclared,
    ToolRunning(bool),
    ToolOutputChunk,
    ToolResult,
    /// 非终态收尾（`stream_draining`）：进入 Draining 文案，**不**释放 abort/resume。
    StreamDraining,
    StreamEnded,
    /// 正常 `on_done` 走完（含尾泡决策后）。
    StreamDone,
    StreamError,
    /// `on_done` 早退：用户取消本轮（与 `StreamDone` 同属会话收口）。
    StreamUserAbort,
}

/// 粗粒度回合阶段（Attaching → Streaming → Draining → Terminal/Idle）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TurnPhase {
    #[default]
    Idle,
    Attaching {
        attach_generation: u64,
    },
    Streaming {
        attach_generation: u64,
        sub: StreamSubPhase,
    },
    Draining {
        attach_generation: u64,
    },
    Terminal {
        attach_generation: u64,
        outcome: TurnOutcome,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamSubPhase {
    ModelOutput,
    ToolUiBusy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnOutcome {
    Done,
    UserAbort,
    Error,
}

/// 由各 attach / SSE / HTTP / 壳层路径喂入。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnLifecycleEvent {
    AttachPrepared {
        attach_generation: u64,
    },
    HttpStreamOpened {
        attach_generation: u64,
    },
    SseControl(StreamControlEvent),
    ShellReleased {
        attach_generation: u64,
    },
    UserAbortRequested {
        attach_generation: u64,
    },
    /// `timeline_log` `final_response`：主答复时间轴结束，释放「模型生成中」；工具可能仍忙。
    TimelineModelFinal {
        attach_generation: u64,
    },
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TurnLifecycleState {
    pub phase: TurnPhase,
    tool_depth: u32,
    tool_running_sse: bool,
    stream_ended_seen: bool,
    saw_model_output: bool,
    model_timeline_final: bool,
    terminated: bool,
}

impl TurnLifecycleState {
    #[must_use]
    pub fn current_attach_generation(self) -> Option<u64> {
        match self.phase {
            TurnPhase::Idle => None,
            TurnPhase::Attaching { attach_generation }
            | TurnPhase::Streaming {
                attach_generation, ..
            }
            | TurnPhase::Draining { attach_generation }
            | TurnPhase::Terminal {
                attach_generation, ..
            } => Some(attach_generation),
        }
    }

    fn tool_ui_active(self) -> bool {
        self.tool_depth > 0 || self.tool_running_sse
    }

    fn recompute_streaming_sub(self) -> StreamSubPhase {
        if self.tool_ui_active() {
            StreamSubPhase::ToolUiBusy
        } else {
            StreamSubPhase::ModelOutput
        }
    }

    fn enter_streaming(self, attach_generation: u64) -> TurnPhase {
        TurnPhase::Streaming {
            attach_generation,
            sub: self.recompute_streaming_sub(),
        }
    }

    fn recompute_after_sse(self, attach_generation: u64) -> TurnPhase {
        if self.terminated {
            return TurnPhase::Terminal {
                attach_generation,
                outcome: TurnOutcome::Done,
            };
        }
        if self.stream_ended_seen {
            return TurnPhase::Draining { attach_generation };
        }
        if self.saw_model_output || self.tool_ui_active() {
            return self.enter_streaming(attach_generation);
        }
        TurnPhase::Attaching { attach_generation }
    }
}

/// 粗 busy（Attaching / Streaming / Draining；不含 loading 占位 / abort 槽）。
#[must_use]
pub fn turn_lifecycle_coarse_busy(state: TurnLifecycleState) -> bool {
    matches!(
        state.phase,
        TurnPhase::Attaching { .. } | TurnPhase::Streaming { .. } | TurnPhase::Draining { .. }
    )
}

/// 状态栏「模型生成中」：Attaching / Draining / 非工具子阶段的 Streaming。
#[must_use]
pub fn turn_lifecycle_model_ui_busy(state: TurnLifecycleState) -> bool {
    if state.model_timeline_final {
        return false;
    }
    matches!(
        state.phase,
        TurnPhase::Attaching { .. }
            | TurnPhase::Draining { .. }
            | TurnPhase::Streaming {
                sub: StreamSubPhase::ModelOutput,
                ..
            }
    )
}

/// 状态栏「工具执行中」与时间线门闩：Streaming 且处于 ToolUiBusy 子阶段。
#[must_use]
pub fn turn_lifecycle_tool_ui_busy(state: TurnLifecycleState) -> bool {
    matches!(
        state.phase,
        TurnPhase::Streaming {
            sub: StreamSubPhase::ToolUiBusy,
            ..
        }
    )
}

/// 合成器 / 停止按钮门闩：lifecycle 粗 busy ∨ **`AbortController` 槽位**（attach 代际内 HTTP 可中止窗口）。
#[must_use]
pub fn turn_lifecycle_stream_turn_busy(state: TurnLifecycleState, abort_present: bool) -> bool {
    turn_lifecycle_coarse_busy(state) || abort_present
}

pub fn apply_turn_lifecycle(state: &mut TurnLifecycleState, ev: TurnLifecycleEvent) {
    match ev {
        TurnLifecycleEvent::AttachPrepared { attach_generation } => {
            state.phase = TurnPhase::Attaching { attach_generation };
            state.tool_depth = 0;
            state.tool_running_sse = false;
            state.stream_ended_seen = false;
            state.saw_model_output = false;
            state.model_timeline_final = false;
            state.terminated = false;
        }
        TurnLifecycleEvent::HttpStreamOpened { attach_generation } => {
            if !generation_matches(state, attach_generation) {
                return;
            }
            state.phase = state.recompute_after_sse(attach_generation);
        }
        TurnLifecycleEvent::SseControl(sse) => {
            if state.terminated {
                return;
            }
            let Some(attach_generation) = state.current_attach_generation() else {
                return;
            };
            apply_sse_event(state, sse);
            state.phase = state.recompute_after_sse(attach_generation);
        }
        TurnLifecycleEvent::ShellReleased { attach_generation } => {
            if !generation_matches(state, attach_generation) {
                return;
            }
            state.phase = TurnPhase::Idle;
            state.tool_depth = 0;
            state.tool_running_sse = false;
            state.stream_ended_seen = false;
            state.saw_model_output = false;
            state.model_timeline_final = false;
            state.terminated = false;
        }
        TurnLifecycleEvent::TimelineModelFinal { attach_generation } => {
            if !generation_matches(state, attach_generation) {
                return;
            }
            state.model_timeline_final = true;
        }
        TurnLifecycleEvent::UserAbortRequested { attach_generation } => {
            if !generation_matches(state, attach_generation) {
                return;
            }
            state.terminated = true;
            state.phase = TurnPhase::Terminal {
                attach_generation,
                outcome: TurnOutcome::UserAbort,
            };
        }
    }
}

fn generation_matches(state: &TurnLifecycleState, attach_generation: u64) -> bool {
    state.current_attach_generation() == Some(attach_generation)
}

fn apply_sse_event(state: &mut TurnLifecycleState, ev: StreamControlEvent) {
    match ev {
        StreamControlEvent::ModelTextDelta | StreamControlEvent::AssistantAnswerPhase => {
            state.saw_model_output = true;
        }
        StreamControlEvent::ToolCallDeclared => {
            state.tool_depth = state.tool_depth.saturating_add(1);
        }
        StreamControlEvent::ToolRunning(b) => {
            state.tool_running_sse = b;
        }
        StreamControlEvent::ToolOutputChunk => {}
        StreamControlEvent::ToolResult => {
            state.tool_depth = state.tool_depth.saturating_sub(1);
        }
        StreamControlEvent::StreamDraining | StreamControlEvent::StreamEnded => {
            state.stream_ended_seen = true;
        }
        StreamControlEvent::StreamDone => {
            state.terminated = true;
            if let Some(attach_generation) = state.current_attach_generation() {
                state.phase = TurnPhase::Terminal {
                    attach_generation,
                    outcome: TurnOutcome::Done,
                };
            }
        }
        StreamControlEvent::StreamError => {
            state.terminated = true;
            if let Some(attach_generation) = state.current_attach_generation() {
                state.phase = TurnPhase::Terminal {
                    attach_generation,
                    outcome: TurnOutcome::Error,
                };
            }
        }
        StreamControlEvent::StreamUserAbort => {
            state.terminated = true;
            if let Some(attach_generation) = state.current_attach_generation() {
                state.phase = TurnPhase::Terminal {
                    attach_generation,
                    outcome: TurnOutcome::UserAbort,
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply_seq(s: &mut TurnLifecycleState, evs: &[TurnLifecycleEvent]) {
        for e in evs {
            apply_turn_lifecycle(s, *e);
        }
    }

    #[test]
    fn attach_open_delta_drains_and_shell_release_idle() {
        let mut s = TurnLifecycleState::default();
        apply_seq(
            &mut s,
            &[
                TurnLifecycleEvent::AttachPrepared {
                    attach_generation: 1,
                },
                TurnLifecycleEvent::HttpStreamOpened {
                    attach_generation: 1,
                },
                TurnLifecycleEvent::SseControl(StreamControlEvent::ModelTextDelta),
                TurnLifecycleEvent::SseControl(StreamControlEvent::StreamEnded),
                TurnLifecycleEvent::SseControl(StreamControlEvent::StreamDone),
                TurnLifecycleEvent::ShellReleased {
                    attach_generation: 1,
                },
            ],
        );
        assert_eq!(s.phase, TurnPhase::Idle);
        assert!(!turn_lifecycle_coarse_busy(s));
    }

    #[test]
    fn tool_call_enters_tool_subphase() {
        let mut s = TurnLifecycleState::default();
        apply_turn_lifecycle(
            &mut s,
            TurnLifecycleEvent::AttachPrepared {
                attach_generation: 2,
            },
        );
        apply_turn_lifecycle(
            &mut s,
            TurnLifecycleEvent::SseControl(StreamControlEvent::ToolCallDeclared),
        );
        assert!(matches!(
            s.phase,
            TurnPhase::Streaming {
                attach_generation: 2,
                sub: StreamSubPhase::ToolUiBusy,
            }
        ));
    }

    #[test]
    fn stale_generation_http_open_is_noop() {
        let mut s = TurnLifecycleState::default();
        apply_turn_lifecycle(
            &mut s,
            TurnLifecycleEvent::AttachPrepared {
                attach_generation: 3,
            },
        );
        apply_turn_lifecycle(
            &mut s,
            TurnLifecycleEvent::AttachPrepared {
                attach_generation: 4,
            },
        );
        apply_turn_lifecycle(
            &mut s,
            TurnLifecycleEvent::HttpStreamOpened {
                attach_generation: 3,
            },
        );
        assert!(matches!(
            s.phase,
            TurnPhase::Attaching {
                attach_generation: 4
            }
        ));
    }

    #[test]
    fn model_and_tool_ui_busy_from_subphase() {
        let mut attaching = TurnLifecycleState::default();
        apply_turn_lifecycle(
            &mut attaching,
            TurnLifecycleEvent::AttachPrepared {
                attach_generation: 1,
            },
        );
        assert!(turn_lifecycle_model_ui_busy(attaching));
        assert!(!turn_lifecycle_tool_ui_busy(attaching));

        let mut tool = TurnLifecycleState::default();
        apply_turn_lifecycle(
            &mut tool,
            TurnLifecycleEvent::AttachPrepared {
                attach_generation: 1,
            },
        );
        apply_turn_lifecycle(
            &mut tool,
            TurnLifecycleEvent::SseControl(StreamControlEvent::ToolCallDeclared),
        );
        assert!(!turn_lifecycle_model_ui_busy(tool));
        assert!(turn_lifecycle_tool_ui_busy(tool));
    }

    #[test]
    fn stream_turn_busy_covers_abort_slot_only_when_idle() {
        let idle = TurnLifecycleState::default();
        assert!(turn_lifecycle_stream_turn_busy(idle, true));
        assert!(!turn_lifecycle_stream_turn_busy(idle, false));

        let mut attaching = TurnLifecycleState::default();
        apply_turn_lifecycle(
            &mut attaching,
            TurnLifecycleEvent::AttachPrepared {
                attach_generation: 1,
            },
        );
        assert!(turn_lifecycle_stream_turn_busy(attaching, false));
    }

    #[test]
    fn timeline_final_drops_model_keeps_tool() {
        let mut s = TurnLifecycleState::default();
        apply_turn_lifecycle(
            &mut s,
            TurnLifecycleEvent::AttachPrepared {
                attach_generation: 1,
            },
        );
        apply_turn_lifecycle(
            &mut s,
            TurnLifecycleEvent::SseControl(StreamControlEvent::ModelTextDelta),
        );
        assert!(turn_lifecycle_model_ui_busy(s));
        assert!(!turn_lifecycle_tool_ui_busy(s));
        apply_turn_lifecycle(
            &mut s,
            TurnLifecycleEvent::TimelineModelFinal {
                attach_generation: 1,
            },
        );
        assert!(!turn_lifecycle_model_ui_busy(s));

        let mut tool = TurnLifecycleState::default();
        apply_turn_lifecycle(
            &mut tool,
            TurnLifecycleEvent::AttachPrepared {
                attach_generation: 2,
            },
        );
        apply_turn_lifecycle(
            &mut tool,
            TurnLifecycleEvent::SseControl(StreamControlEvent::ToolCallDeclared),
        );
        apply_turn_lifecycle(
            &mut tool,
            TurnLifecycleEvent::TimelineModelFinal {
                attach_generation: 2,
            },
        );
        assert!(!turn_lifecycle_model_ui_busy(tool));
        assert!(turn_lifecycle_tool_ui_busy(tool));
    }

    #[test]
    fn stream_ended_enters_draining_until_shell_release() {
        let mut s = TurnLifecycleState::default();
        apply_turn_lifecycle(
            &mut s,
            TurnLifecycleEvent::AttachPrepared {
                attach_generation: 7,
            },
        );
        apply_turn_lifecycle(
            &mut s,
            TurnLifecycleEvent::SseControl(StreamControlEvent::StreamEnded),
        );
        assert!(matches!(s.phase, TurnPhase::Draining { .. }));
        assert!(turn_lifecycle_coarse_busy(s));
        apply_turn_lifecycle(
            &mut s,
            TurnLifecycleEvent::ShellReleased {
                attach_generation: 7,
            },
        );
        assert!(!turn_lifecycle_coarse_busy(s));
    }

    #[test]
    fn shell_release_only_when_generation_matches() {
        let mut s = TurnLifecycleState::default();
        apply_turn_lifecycle(
            &mut s,
            TurnLifecycleEvent::AttachPrepared {
                attach_generation: 5,
            },
        );
        apply_turn_lifecycle(
            &mut s,
            TurnLifecycleEvent::ShellReleased {
                attach_generation: 99,
            },
        );
        assert!(matches!(
            s.phase,
            TurnPhase::Attaching {
                attach_generation: 5
            }
        ));
        assert!(turn_lifecycle_coarse_busy(s));
    }
}
