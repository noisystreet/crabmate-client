//! 一帧的屏幕分区，以及鼠标命中检测用的矩形工具。
//!
//! 抽成纯函数是为了让**渲染与鼠标点击共用同一套矩形**：两边各算一遍迟早漂移，
//! 而漂移的症状（点到隔壁列、点错行）比渲染错位更难察觉。

use ratatui::layout::{Constraint, Layout, Rect};

use super::render::{SIDEBAR_WIDTH, WORKSPACE_PANE_WIDTH};

/// 主体 / 底栏；宽屏时主体再分「左会话 | 聊天 | 右工作区」，右栏底部再切出状态栈。
///
/// **宽屏没有底栏**（`status == None`）：状态改到右栏底部纵向排列，那一行还给正文 ——
/// 与 desktop 的 `.side-status-stack`（`flex-direction: column`，原底栏迁入右栏底部）
/// 同构。窄屏两栏都隐藏、没有地方安置状态栈，底栏**保留**：状态里的「生成中 / 取消中 /
/// 审批阻塞」必须始终可见，不能为了对称把它一起删掉。
pub(super) struct Panes {
    /// 底栏；**仅窄屏**为 `Some`。
    pub status: Option<Rect>,
    /// 左栏会话列（窄屏隐藏时为 `None`）。
    pub left: Option<Rect>,
    /// 中列：transcript + composer。
    pub chat: Rect,
    /// 右栏目录树（**不含**底部状态栈）。
    pub right: Option<Rect>,
    /// 右栏底部的状态栈；无右栏时为 `None`（含 `stack_rows == 0`）。
    pub right_status: Option<Rect>,
}

/// 按终端区域、侧栏可见性与状态栈高度切分。
///
/// `stack_rows` 由调用方算（见 `status_stack::stack_rows`），这里只负责把它压到
/// 「树至少留 1 行」的范围内 —— 这条不变量放这里而不是让两边各算，是因为漂移的
/// 表现是「树被挤没」，很难归因。
pub(super) fn panes(area: Rect, sidebar_visible: bool, stack_rows: u16) -> Panes {
    if !sidebar_visible {
        let rows = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(area);
        return Panes {
            status: Some(rows[1]),
            left: None,
            chat: rows[0],
            right: None,
            right_status: None,
        };
    }
    let cols = Layout::horizontal([
        Constraint::Length(SIDEBAR_WIDTH),
        Constraint::Min(0),
        Constraint::Length(WORKSPACE_PANE_WIDTH),
    ])
    .split(area);
    let stack = stack_rows.min(cols[2].height.saturating_sub(1));
    let right_rows =
        Layout::vertical([Constraint::Min(1), Constraint::Length(stack)]).split(cols[2]);
    Panes {
        status: None,
        left: Some(cols[0]),
        chat: cols[1],
        right: Some(right_rows[0]),
        right_status: (stack > 0).then_some(right_rows[1]),
    }
}

/// 鼠标坐标是否落在区域内。
///
/// 自带一份而不依赖 `Rect::contains`：后者在 ratatui 各版本的入参类型变过，
/// 且空矩形（宽或高为 0）在此必须明确返回 false。
pub(super) fn contains(area: Rect, column: u16, row: u16) -> bool {
    area.width > 0
        && area.height > 0
        && column >= area.x
        && column < area.x.saturating_add(area.width)
        && row >= area.y
        && row < area.y.saturating_add(area.height)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 宽屏：200 列 × 40 行，状态栈 5 行。
    fn wide() -> Panes {
        panes(Rect::new(0, 0, 200, 40), true, 5)
    }

    #[test]
    fn wide_layout_has_no_bottom_bar() {
        let p = wide();
        let left = p.left.expect("宽屏有左栏");
        let right = p.right.expect("宽屏有右栏");
        assert_eq!(left.x, 0);
        assert_eq!(left.width, SIDEBAR_WIDTH);
        assert_eq!(right.x, 200 - WORKSPACE_PANE_WIDTH);
        assert_eq!(right.width, WORKSPACE_PANE_WIDTH);
        // 中列起止连续，三列拼满主体且不重叠。
        assert_eq!(p.chat.x, left.width);
        assert_eq!(p.chat.x + p.chat.width, right.x);
        assert_eq!(p.chat.y, 0, "主体从第 0 行起（没有顶栏占行）");
        // **宽屏不再有底栏**：状态进右栏底部，那一行还给正文。
        assert!(p.status.is_none(), "宽屏不该再有底栏");
        assert_eq!(p.chat.height, 40, "底栏消失后正文占满整个高度");
        let stack = p.right_status.expect("宽屏有状态栈");
        assert_eq!(stack.height, 5);
        assert_eq!(stack.y + stack.height, 40, "状态栈贴底");
        assert_eq!(stack.x, right.x, "与右栏同列");
        assert_eq!(stack.width, WORKSPACE_PANE_WIDTH);
        assert_eq!(right.y + right.height, stack.y, "树与状态栈上下相接");
    }

    #[test]
    fn narrow_layout_keeps_the_bottom_bar() {
        let p = panes(Rect::new(0, 0, 80, 24), false, 5);
        assert!(p.left.is_none());
        assert!(p.right.is_none());
        assert!(p.right_status.is_none(), "没有右栏就没有状态栈");
        assert_eq!(p.chat.width, 80);
        // 窄屏没有地方安置状态栈，底栏保留（否则「生成中 / 审批阻塞」无处显示）。
        let status = p.status.expect("窄屏保留底栏");
        assert_eq!(status.y, 23);
        assert_eq!(p.chat.height, 23);
    }

    #[test]
    fn stack_rows_are_clamped_so_the_tree_keeps_a_row() {
        // 要求 40 行状态栈、终端只有 6 行 → 树至少留 1 行。
        let p = panes(Rect::new(0, 0, 200, 6), true, 40);
        assert_eq!(p.right.expect("宽屏有右栏").height, 1, "树至少留一行");
        assert_eq!(p.right_status.expect("仍有状态栈").height, 5);
    }

    #[test]
    fn zero_stack_rows_leaves_the_tree_full_height() {
        let p = panes(Rect::new(0, 0, 200, 40), true, 0);
        assert!(p.right_status.is_none());
        assert_eq!(p.right.expect("宽屏有右栏").height, 40);
    }

    #[test]
    fn contains_is_half_open_and_rejects_empty() {
        let r = Rect::new(10, 5, 4, 2);
        assert!(contains(r, 10, 5));
        assert!(contains(r, 13, 6));
        assert!(!contains(r, 14, 6), "右边界不包含");
        assert!(!contains(r, 13, 7), "下边界不包含");
        assert!(!contains(r, 9, 5));
        assert!(!contains(Rect::new(10, 5, 0, 2), 10, 5), "空矩形不命中");
        assert!(!contains(Rect::new(10, 5, 4, 0), 10, 5), "空矩形不命中");
    }
}
