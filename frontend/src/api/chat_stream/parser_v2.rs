//! V2 解析器：AG-UI 协议解析的本地适配（解析本体已下沉共享层）。
//!
//! 解析逻辑见 [`crabmate_client_api::ag_ui_parser`]；此处仅实现本地 [`SseParser`] trait。

use crate::sse_dispatch::{SseControlSink, SseDispatch};

use super::sse_parser::SseParser;

/// V2 解析器（AG-UI 协议）。
pub(crate) struct V2Parser;

impl SseParser for V2Parser {
    fn parse(&self, data: &str, sink: &mut SseControlSink<'_>) -> SseDispatch {
        crabmate_client_api::ag_ui_parser::parse_ag_ui_line(data, sink)
    }
}
