//! transcript body 的帧间 memo 键。
//!
//! `build_body_rows` 的结果按此键缓存：任一维度变化才重建，否则直接复用。
//! 单独成模块是因为 `mod.rs`（事件循环）已到 920 行门禁上限。

use super::render::chat_body_width;
use super::state::UiState;

/// body 物理行 memo 的失效键：内容代数（`content_rev`）+ 宽度 + 搜索词/锚点。
/// `view_offset`/焦点/审批等「draw 期」因素不在此列（窗口裁剪每帧实时做）。
///
/// 思考行的 spinner **也不在此列**：它由渲染层替换标记字符，不触发重建，
/// 否则停顿期每换一帧都要全量重新 tokenize + 折行整个 transcript。
#[derive(PartialEq, Debug)]
pub(super) struct BodyMemoKey {
    pub(super) width: usize,
    pub(super) rev: u64,
    pub(super) needle: Option<String>,
    pub(super) cursor: Option<usize>,
}

/// 计算当前帧的 memo 键。
pub(super) fn key(st: &UiState, terminal_width: u16) -> BodyMemoKey {
    BodyMemoKey {
        width: chat_body_width(st.sidebar_visible, terminal_width),
        rev: st.content_rev,
        needle: st.search_term().map(str::to_string),
        cursor: st.search_cursor,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui_mode::state::LineKind;

    #[test]
    fn memo_key_changes_on_render_inputs() {
        let mut s = UiState::new();
        let k0 = key(&s, 120);
        s.push_line(LineKind::User, "a");
        assert_ne!(key(&s, 120), k0, "新增行应失效");
        let k1 = key(&s, 120);
        s.stream_delta(LineKind::Assistant, "b");
        assert_ne!(key(&s, 120), k1, "流式追加应失效");
        let k2 = key(&s, 120);
        s.start_search("b");
        assert_ne!(key(&s, 120), k2, "搜索词/锚点应失效");
        let k3 = key(&s, 120);
        s.toggle_thinking();
        assert_ne!(key(&s, 120), k3, "折叠切换应失效");
        // draw 期因素（滚动）不应失效
        let k4 = key(&s, 120);
        s.view_offset = 5;
        assert_eq!(key(&s, 120), k4, "滚动不应失效");
        // 宽度变化应失效
        assert_ne!(key(&s, 120), key(&s, 121), "宽度应失效");
    }
}
