//! `think` / `redacted_thinking` 标签剥离与内联思维链拆分（与 [`super::plan_fence`] 互补）。

mod filter;
mod scan;

pub(crate) use filter::{
    assistant_thinking_body_and_answer_raw, filter_assistant_thinking_markers_for_display,
    filter_redacted_thinking_for_display, strip_echoed_thinking_section,
};
