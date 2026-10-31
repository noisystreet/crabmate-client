//! 工具行的开始 / 结果回填：从 `state.rs` 拆出，压 920 行文件门禁（同 `state_search`）。
//!
//! 这一块是「按工具调用 id 原位更新」的全部逻辑：`tool_start` 记下行号，`tool_end`
//! 找回那一行改文本；续流中段找不到行时才补一行收尾。与「整段折叠」（`state_fold`）
//! 相邻但不同层：那个改的是「哪些行可见」，这里改的是「一行写什么」。

use crate::tui_mode::tool_summary::{tool_end_text, tool_row_text};

use super::{LineKind, LogLine, ToolState, UiState};

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
    pub fn tool_end(
        &mut self,
        tool_call_id: &str,
        name: &str,
        ok: Option<bool>,
        note: Option<&str>,
    ) {
        let display = if name.trim().is_empty() { "tool" } else { name };
        let text = tool_end_text(display, ok, note);
        let idx = self.tool_pending.remove(tool_call_id);
        match idx {
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
            }
        }
    }

    /// 回合开始时清理上轮工具行映射。
    pub fn reset_run_tools(&mut self) {
        self.tool_pending.clear();
    }
}
