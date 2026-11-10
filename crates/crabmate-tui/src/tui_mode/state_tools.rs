//! 工具行的开始 / 结果回填：从 `state.rs` 拆出，压 920 行文件门禁（同 `state_search`）。
//!
//! 这一块是「按工具调用 id 原位更新」的全部逻辑：`tool_start` 记下行号，`tool_end`
//! 找回那一行改文本；续流中段找不到行时才补一行收尾。与「整段折叠」（`state_fold`）
//! 相邻但不同层：那个改的是「哪些行可见」，这里改的是「一行写什么」。

use crate::tui_mode::tool_summary::{tool_end_text, tool_row_text};

use super::{LineKind, LogLine, ToolDetail, ToolState, UiState};

/// 单次工具输出在 TUI 侧保留的字符上限（64K **字符**）。
///
/// 真正的上界在 **Server** 侧（工具输出的长度上限），这里只是「终端里留多少可回看」：
/// 全量留存会让长会话常驻几十 MB，而 `/detail` 是回看用途，不是归档。
pub(crate) const DETAIL_MAX_CHARS: usize = 64 * 1024;

/// 全部工具详情共用的字符预算（4M 字符）。
///
/// 超出时从**最旧**的开始丢（`BTreeMap` 的迭代顺序即行号顺序）：`/detail` 的默认落点是
/// 最近一条，丢最近的会让命令立刻变得没用。被丢的行在 `/detail` 里表现为「没有详情」，
/// 与回放行走同一句提示。
pub(crate) const DETAIL_TOTAL_CHARS: usize = 4 * 1024 * 1024;

impl UiState {
    // ── 工具行摘要 ───────────────────────────────────────────

    /// 工具开始：记一个「进行中」摘要行，RESULT 到达时原位补标记与状态。
    ///
    /// `note` 是事件里已经给出的「要执行什么」（`TOOL_CALL_START` 的 summary /
    /// argsPreview）——一开始就显示，别等结果回来才让用户看到跑的是什么命令。
    pub fn tool_start(&mut self, tool_call_id: &str, name: &str, note: Option<&str>) {
        let display = if name.trim().is_empty() { "tool" } else { name };
        let idx = self.lines.len();
        self.lines.push(LogLine::with_tool_state(
            LineKind::Tool,
            &tool_row_text(display, note),
            false,
            ToolState::Running,
        ));
        // 没有 id 就无法按 id 找回这一行；结果到达时会另起一行，本行就此停在
        // `Running`——与「id 缺失等于放弃了原位更新」的既有语义一致。
        if !tool_call_id.is_empty() {
            self.tool_pending.insert(tool_call_id.to_string(), idx);
        }
        self.search_on_line_added();
    }

    /// 工具结果：更新对应摘要行（note 与工具名重复时省略前缀，如 `✓ read file: hello.cpp`）；
    /// 找不到 start（如续流中段）则补一行收尾。
    ///
    /// `detail` 是这一帧带回来的输出全文，只进侧表（`/detail`），不参与这一行的文本 ——
    /// 工具行永远只有一行摘要，展开是浮层的事（issue #428 第 3 条）。
    pub fn tool_end(
        &mut self,
        tool_call_id: &str,
        name: &str,
        ok: Option<bool>,
        note: Option<&str>,
        detail: Option<&str>,
    ) {
        let display = if name.trim().is_empty() { "tool" } else { name };
        let text = tool_end_text(display, ok, note);
        let idx = self.tool_pending.remove(tool_call_id);
        let row = match idx {
            Some(i)
                if self
                    .lines
                    .get(i)
                    .is_some_and(|l| l.kind() == LineKind::Tool) =>
            {
                let line = &mut self.lines[i];
                line.finish(text, ok);
                // 原位替换了文本：命中集合要跟着改（可能是中间行，不是末尾）。
                self.search_on_line_changed(i);
                i
            }
            _ => {
                // 没有对应的 start 行（续流中段）：直接补一行「已结束」。
                self.lines.push(LogLine::with_tool_state(
                    LineKind::Tool,
                    &text,
                    false,
                    ToolState::Finished(ok),
                ));
                self.search_on_line_added();
                self.lines.len() - 1
            }
        };
        // 全文挂到**最终落定的那一行**上：原位更新与补行两条路都要挂，否则
        // 「续流中段才拿到的结果」就没有详情可展开。
        self.remember_detail(row, detail);
    }

    /// 把一次工具调用的全文挂到它的行上（`/detail` 的唯一写入点）。
    ///
    /// 截断按**字符**（与 `chat_classify::trim_tail` 同口径）：字节级会把中文切成半个字。
    fn remember_detail(&mut self, row: usize, detail: Option<&str>) {
        let Some(raw) = detail.filter(|t| !t.trim().is_empty()) else {
            return;
        };
        // 同一行再次到达（续流重发结果）时先扣掉旧值，否则预算会被同一段文本重复占用。
        if let Some(prev) = self.tool_details.remove(&row) {
            self.detail_chars = self.detail_chars.saturating_sub(prev.text.chars().count());
        }
        let total = raw.chars().count();
        let text: String = raw.chars().take(DETAIL_MAX_CHARS).collect();
        let kept = text.chars().count();
        self.evict_details(kept);
        self.detail_chars += kept;
        self.tool_details.insert(
            row,
            ToolDetail {
                text,
                truncated: total > kept,
            },
        );
    }

    /// 腾出总量预算（口径见 [`DETAIL_TOTAL_CHARS`]）。
    fn evict_details(&mut self, incoming: usize) {
        while self.detail_chars + incoming > DETAIL_TOTAL_CHARS {
            let Some((_, dropped)) = self.tool_details.pop_first() else {
                break;
            };
            self.detail_chars = self
                .detail_chars
                .saturating_sub(dropped.text.chars().count());
        }
    }

    /// 回合开始时清理上轮工具行映射。
    pub fn reset_run_tools(&mut self) {
        self.tool_pending.clear();
    }
}

#[cfg(test)]
#[path = "state_detail_tests.rs"]
mod detail_tests;
