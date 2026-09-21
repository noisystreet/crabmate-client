//! 前端 SSE / AG-UI 控制面类型定义（载荷形状与回调分组类型）。
//!
//! 类型本体已下沉至共享层 [`crabmate_client_api::sse_dispatch`]；此处转发以保持
//! `crate::sse_dispatch::*` 既有引用路径。
//!
//! 当前仅 AG-UI（v2）协议；`V2Parser` 将事件分发到 `SseControlSink` 回调。

pub use crabmate_client_api::sse_dispatch::*;
