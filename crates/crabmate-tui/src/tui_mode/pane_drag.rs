//! 拖拽分隔线改列宽（issue #407 的 P0.5）。
//!
//! 这里只有纯函数：**边命中**（[`edge_at`]）与「指针在第几列 → 多宽」（[`drag_target`]）。
//! 状态与事件接线在 `mouse.rs`（`LaneApp::on_pane_drag`）。分开是为了让换算能单测 ——
//! 这类算术错了的表现是「往右拖反而变窄」或「拖到边还在长」，在真终端里极难归因。

use ratatui::layout::Rect;

use super::layout::Panes;
use super::pane_width::PaneWidths;

/// 被拖的那一条分隔线。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PaneEdge {
    /// 左栏右侧那条（拖动它改左栏宽度）。
    Left,
    /// 右栏左侧那条（拖动它改右栏宽度）。
    Right,
}

/// 正在进行的拖拽。
///
/// **只记「哪条边」**，不记按下时的列、也不累加增量：宽度每次都由当前指针列重算。
/// 累加增量的话，「拖到边界被夹住、再往回拖」时指针与分隔线会错位——夹住的那几列
/// 已经被算进增量，回程要先补掉它们才会动。
#[derive(Debug, Clone, Copy)]
pub(crate) struct PaneDrag {
    pub edge: PaneEdge,
}

/// 指针落在哪条分隔线上（`None` = 不在任何分隔线上）。
///
/// 与 `pane_render::paint_edge` 的 `width < 2` 同一门槛：**没画出来的线不该能点**
/// （宽度 1 列时那唯一一列就是内容列，画和点都必须让开）。
///
/// 竖直方向认**整列**（含左栏下半的 lane 区、右栏底部状态栈）：那条线是贯穿的，
/// 只认上半段的话，用户拖到下半截会突然「拖不动」，而画面上的线明明还在。
pub(crate) fn edge_at(p: &Panes, column: u16, row: u16) -> Option<PaneEdge> {
    if let Some(left) = p.left.filter(|r| r.width >= 2) {
        // 分隔线占左栏最右一列：`x + width - 1`。
        let divider = left.x + left.width - 1;
        if column == divider && row_in(&[p.left, p.left_lanes], row) {
            return Some(PaneEdge::Left);
        }
    }
    if let Some(right) = p.right.filter(|r| r.width >= 2) {
        // 分隔线占右栏最左一列。
        if column == right.x && row_in(&[p.right, p.right_status], row) {
            return Some(PaneEdge::Right);
        }
    }
    None
}

/// `row` 是否落在这些矩形（同一栏的上段 + 下段）的竖直范围内。
fn row_in(bands: &[Option<Rect>], row: u16) -> bool {
    bands
        .iter()
        .flatten()
        .any(|r| row >= r.y && row < r.y.saturating_add(r.height))
}

/// 把「指针停在第 `column` 列」换算成新的两栏宽度（已按终端宽度夹取）。
///
/// 左栏分隔线在 `left - 1` 列：指针停在该列算出来正好是原宽度，所以**按下不跳**
/// （否则一按就往右偏一列）。右栏同理，`右栏宽 = 终端宽 - 指针列`。
///
/// 夹取交给 [`PaneWidths::clamped`]：拖过界时宽度停在上下限、聊天列保底仍生效，
/// 不需要在这里各写一份边界——那正是 P0 把夹取收进一个函数的原因。
pub(crate) fn drag_target(
    edge: PaneEdge,
    column: u16,
    terminal_width: u16,
    current: PaneWidths,
) -> PaneWidths {
    let want = match edge {
        PaneEdge::Left => PaneWidths {
            left: column.saturating_add(1),
            right: current.right,
        },
        PaneEdge::Right => PaneWidths {
            left: current.left,
            right: terminal_width.saturating_sub(column),
        },
    };
    want.clamped(terminal_width)
}

#[cfg(test)]
mod tests {
    use super::super::pane_width::{
        MAX_SIDEBAR_COLS, MAX_WORKSPACE_COLS, MIN_CHAT_COLS, MIN_WORKSPACE_COLS, PaneLayout,
        PaneWidths,
    };
    use super::*;
    use crate::tui_mode::layout::{panes, panes_with_lanes};

    fn wide_panes() -> Panes {
        panes(
            Rect::new(0, 0, 200, 40),
            PaneLayout {
                visible: true,
                widths: PaneWidths::DEFAULT,
            },
            5,
        )
    }

    #[test]
    fn left_divider_is_the_last_column_of_the_left_pane() {
        let p = wide_panes();
        let left = p.left.expect("宽屏有左栏");
        let divider = left.x + left.width - 1;
        assert_eq!(edge_at(&p, divider, left.y), Some(PaneEdge::Left));
        assert_eq!(edge_at(&p, divider - 1, left.y), None, "内容列不该算分隔线");
        assert_eq!(edge_at(&p, divider + 1, left.y), None, "聊天列不该算");
    }

    /// 分隔线是贯穿整列的：左栏下半（lane 区）与右栏底部（状态栈）也能按住拖。
    #[test]
    fn divider_can_be_grabbed_from_the_lower_half_too() {
        let p = panes_with_lanes(
            Rect::new(0, 0, 200, 40),
            PaneLayout {
                visible: true,
                widths: PaneWidths::DEFAULT,
            },
            5,
            3,
        );
        let left = p.left.expect("有左栏");
        let lanes = p.left_lanes.expect("有 lane 区");
        let divider = left.x + left.width - 1;
        assert_eq!(edge_at(&p, divider, lanes.y + 1), Some(PaneEdge::Left));

        let stack = p.right_status.expect("有状态栈");
        let right = p.right.expect("有右栏");
        assert_eq!(edge_at(&p, right.x, stack.y), Some(PaneEdge::Right));
    }

    #[test]
    fn right_divider_is_the_first_column_of_the_right_pane() {
        let p = wide_panes();
        let right = p.right.expect("宽屏有右栏");
        assert_eq!(edge_at(&p, right.x, right.y), Some(PaneEdge::Right));
        assert_eq!(edge_at(&p, right.x + 1, right.y), None, "内容列不是分隔线");
        assert_eq!(edge_at(&p, right.x - 1, right.y), None, "聊天列不是分隔线");
    }

    #[test]
    fn hidden_panes_have_no_divider() {
        let p = panes(
            Rect::new(0, 0, 80, 24),
            PaneLayout {
                visible: false,
                widths: PaneWidths::DEFAULT,
            },
            5,
        );
        for column in 0..80 {
            assert_eq!(edge_at(&p, column, 0), None, "column={column}");
        }
    }

    /// 指针停在分隔线原列 = 宽度不变（按下不跳）。
    #[test]
    fn grabbing_the_divider_does_not_move_it() {
        let left_width = PaneWidths::DEFAULT.left;
        assert_eq!(
            drag_target(PaneEdge::Left, left_width - 1, 200, PaneWidths::DEFAULT),
            PaneWidths::DEFAULT
        );
        let right_start = 200 - PaneWidths::DEFAULT.right;
        assert_eq!(
            drag_target(PaneEdge::Right, right_start, 200, PaneWidths::DEFAULT),
            PaneWidths::DEFAULT
        );
    }

    #[test]
    fn dragging_right_widens_the_left_pane_by_one_column() {
        let got = drag_target(PaneEdge::Left, 40, 200, PaneWidths::DEFAULT);
        assert_eq!(got.left, 41, "指针在第 40 列 → 分隔线右移到 40，宽度 41");
        assert_eq!(got.right, PaneWidths::DEFAULT.right, "另一边不动");
    }

    #[test]
    fn dragging_left_never_widens_the_right_pane_past_its_limits() {
        // 指针贴住右缘：右栏 = 200 - 199 = 1，被下限抬到 20。
        let got = drag_target(PaneEdge::Right, 199, 200, PaneWidths::DEFAULT);
        assert_eq!(got.right, MIN_WORKSPACE_COLS);
        // 指针越过左栏：右栏算出来比上限还宽，被上限截住，左栏原样不动。
        let got = drag_target(PaneEdge::Right, 0, 200, PaneWidths::DEFAULT);
        assert_eq!(got.right, MAX_WORKSPACE_COLS);
        assert_eq!(got.left, PaneWidths::DEFAULT.left);
    }

    /// 拖过界：宽度停在上下限，聊天列保底仍然生效 —— 这两条都在 `clamped` 里，
    /// 这里只证明拖拽目标确实走它，而不是自己又写一套边界。
    #[test]
    fn drag_target_goes_through_the_same_clamp() {
        let got = drag_target(PaneEdge::Left, 190, 200, PaneWidths::DEFAULT);
        assert_eq!(got.left, MAX_SIDEBAR_COLS, "超过上限就停在 MAX");

        let narrow = drag_target(PaneEdge::Left, 90, 100, PaneWidths::DEFAULT);
        assert_eq!(
            narrow.left as u32 + narrow.right as u32 + MIN_CHAT_COLS as u32,
            100,
            "窄终端里两栏合计正好让出聊天列保底"
        );
    }
}
