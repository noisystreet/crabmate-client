//! 时间线/气泡等辅助（供 [`super::builders`] 与 [`super::assemble`] 使用）。
//!
//! **消息布局**统一经 [`super::turn_layout::TurnLayout`]；本目录保留索引与文案拼装。

mod indices;
mod run_command_card;
mod stream_diag;
mod text_format;
mod timeline_tail;

pub(crate) use indices::*;
pub(crate) use run_command_card::run_command_card_invocation_line;
pub(crate) use stream_diag::*;
pub(crate) use text_format::*;
pub(crate) use timeline_tail::*;
