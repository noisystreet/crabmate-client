//! `/detail` 的两个类型：侧表的值（`ToolDetail`）与浮层状态（`DetailOverlay`）。
//!
//! 拆成 `state.rs` 的子模块只为压那里贴着 920 行的门禁；语义上它们是 `UiState` 的
//! `tool_details` / `detail` 两个字段的载荷，所以仍然从 `state` re-export。
//!
//! 与 `state_fold.rs` 的分工：折叠改「哪些行可见」（可逆开合），详情只读一行现成文本。

/// 一次工具调用的完整输出（`/detail` 的数据源，issue #428 第 3 条）。
#[derive(Debug, Clone)]
pub struct ToolDetail {
    /// 正文（已在存入时按 `state_tools::DETAIL_MAX_CHARS` 截断）。
    pub text: String,
    /// 是否被 TUI 的内存上限截断 —— 浮层尾行据此说明「看到的不一定是全部」，
    /// 而不是让用户以为工具就只输出了这些。
    pub truncated: bool,
}

/// `/detail` 浮层：把某次工具调用的输出原文铺在主区，可滚动、只读。
#[derive(Debug, Clone, Copy)]
pub struct DetailOverlay {
    /// 被展开的 transcript 逻辑行号（`LineKind::Tool`）。
    pub line: usize,
    /// 折行后的滚动起点（渲染侧按可用高度再夹一次上界）。
    pub scroll: usize,
}
