//! 单轮 `/chat/stream` 的**粗粒度生命周期**（UI 门闩与 attach 代际的唯一来源）。
//!
//! 归约与谓词已下沉 [`crabmate_client_api::turn_lifecycle`]（#458 的 S1），此处只保留原路径
//! （`crate::app::chat::turn_lifecycle::…`，并经 `app::mod` 别名供 `crate::app::turn_lifecycle::…`），
//! 让两端共用同一份实现。
//!
//! 与 [`super::composer_stream::stream_control_reducer`] 分工不变：后者描述 attach 内 SSE 消费进度；
//! 本模块收敛壳层 busy 语义，供 [`crate::chat_session_state::make_chat_stream_busy_memos`] 读取。

pub(crate) use crabmate_client_api::turn_lifecycle::{
    TurnLifecycleEvent, TurnLifecycleState, apply_turn_lifecycle, turn_lifecycle_coarse_busy,
    turn_lifecycle_model_ui_busy, turn_lifecycle_stream_turn_busy, turn_lifecycle_tool_ui_busy,
};
