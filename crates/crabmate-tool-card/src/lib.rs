//! 工具结果卡 **compact / detail** 的共享生成（Web SSE 与会话水合）。
//!
//! 本仓 path 依赖；勿再 git 钉 Server `crabmate-tool-card`（W2b 后主仓已无该 member）。

mod card;
mod diff_view;
mod input;
#[allow(dead_code)]
mod locale;
mod parse;
mod plain;
mod stored;
mod strip_ansi;

pub use card::{
    tool_card_compact_text, tool_card_text, tool_detail_scrub_row_redundancy,
    tool_signal_beside_title, tool_signal_beside_tool,
};
pub use diff_view::{
    DIFF_COLLAPSE_MAX_LINES, DetailSegment, DiffLine, DiffLineKind, collapse_diff_lines,
    parse_unified_diff, split_detail_segments,
};
pub use input::{NormalizedToolSnapshotFields, ToolCardInput};
pub use locale::{ToolCardLocale, tool_human_name};
pub use parse::{looks_like_crabmate_tool_envelope, parse_tool_envelope};
pub use stored::{
    COMPACT_MAX_CHARS, ToolStoredText, compact_one_line, tool_stored_text,
    tool_stored_text_from_envelope,
};
pub use strip_ansi::strip_ansi_codes;
