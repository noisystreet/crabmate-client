//! 一帧的屏幕分区，以及鼠标命中检测用的矩形工具。
//!
//! 抽成纯函数是为了让**渲染与鼠标点击共用同一套矩形**：两边各算一遍迟早漂移，
//! 而漂移的症状（点到隔壁列、点错行）比渲染错位更难察觉。
//!
//! 两栏宽度来自 [`PaneLayout`]（用户选择 + 可见性），并在本文件里按终端宽度夹取一次 ——
//! 这里是「所有矩形」的唯一来源，夹在这里就不可能出现「画的是夹取后的、点的是夹取前的」。

use ratatui::layout::{Constraint, Layout, Rect};

use super::pane_width::PaneLayout;

/// lane 列表标题（`连接 [n]`）占的行数。
const LANE_TITLE_ROWS: u16 = 1;

/// 主体 / 底栏；宽屏时主体再分「左会话 | 聊天 | 右工作区」，右栏底部再切出状态栈。
///
/// **宽屏没有底栏**（`status == None`）：状态改到右栏底部纵向排列，那一行还给正文 ——
/// 与 desktop 的 `.side-status-stack`（`flex-direction: column`，原底栏迁入右栏底部）
/// 同构。窄屏两栏都隐藏、没有地方安置状态栈，底栏**保留**：状态里的「生成中 / 取消中 /
/// 审批阻塞」必须始终可见，不能为了对称把它一起删掉。
pub(super) struct Panes {
    /// 底栏；**仅窄屏**为 `Some`。
    pub status: Option<Rect>,
    /// 左栏会话列（窄屏隐藏时为 `None`）。**不含**底部 lane 列表。
    pub left: Option<Rect>,
    /// 左栏底部的 lane 列表；窄屏时为 `None`。**单 lane 也占位**（高度见 [`lane_pane_height`]）。
    ///
    /// 和 `right` / `right_status` 一样，渲染与鼠标命中都只认这里给出的矩形。
    pub left_lanes: Option<Rect>,
    /// 中列：transcript + composer。
    pub chat: Rect,
    /// 右栏目录树（**不含**底部状态栈）。
    pub right: Option<Rect>,
    /// 右栏底部的状态栈；无右栏时为 `None`（含 `stack_rows == 0`）。
    pub right_status: Option<Rect>,
}

/// lane 区占左栏的高度：**固定为左栏的一半**（会话列表至少留 1 行）。
///
/// 与 lane 条数解耦：条数只影响区内画得出几条（见 [`lane_shown_len`]，放不下的在标题
/// 补 `↓n`），不改变
/// 「lane 区占左栏一半」这条版面契约 —— 否则每加一条连接会话列表就往上缩一截，
/// 版面会随连接数抖动，切 lane 时整屏都在跳。
pub(super) fn lane_pane_height(sidebar_height: u16, lanes_len: usize) -> u16 {
    if lanes_len == 0 {
        return 0;
    }
    (sidebar_height / 2).min(sidebar_height.saturating_sub(1))
}

/// lane 区内实际画得下的条数：**每条 lane 一行**，可用行不够时按行截断。
///
/// **渲染与鼠标命中必须共用这一个口径**（[`lane_index_at`] 是它的反函数）：两边各算
/// 一遍的症状是「点第 2 行切到第 3 条」，比画错更难归因。
///
/// 装不下时不静默丢弃：渲染侧在标题行补 `↓n`，与侧栏列表的溢出计数同一手法。
pub(super) fn lane_shown_len(avail_rows: u16, lanes_len: usize) -> usize {
    lanes_len.min(usize::from(avail_rows))
}

/// lane 区内某一行（0-based、相对 lane 区顶端）对应的 lane 下标；标题行与空白区返回 `None`。
pub(super) fn lane_index_at(area_height: u16, lanes_len: usize, row_in: u16) -> Option<usize> {
    if row_in < LANE_TITLE_ROWS {
        return None;
    }
    let avail = area_height.saturating_sub(LANE_TITLE_ROWS);
    let idx = usize::from(row_in - LANE_TITLE_ROWS);
    (idx < lane_shown_len(avail, lanes_len)).then_some(idx)
}

/// 按终端区域、两栏版面与状态栈高度切分。
///
/// `stack_rows` 由调用方算（见 `status_stack::stack_rows`），这里只负责把它压到
/// 「树至少留 1 行」的范围内 —— 这条不变量放这里而不是让两边各算，是因为漂移的
/// 表现是「树被挤没」，很难归因。
pub(super) fn panes(area: Rect, layout: PaneLayout, stack_rows: u16) -> Panes {
    panes_with_lanes(area, layout, stack_rows, 0)
}

/// 带 lane 列表的分区：左栏再切成「会话列表（上）/ lane 列表（下）」。
///
/// `lanes_len == 0`（没有任何 lane）时左栏不下切（`Panes::left_lanes` 为 `None`）；
/// 有一条及以上就固定切走左栏一半 —— **单 lane 也占位**，否则「现在连的是哪台 serve、
/// 有没有别的连接」在界面上完全不可见。
pub(super) fn panes_with_lanes(
    area: Rect,
    layout: PaneLayout,
    stack_rows: u16,
    lanes_len: usize,
) -> Panes {
    // 宽度在这里夹一次：终端缩小、或用户把两栏拖宽之后再缩小窗口，都会让「用户的选择」
    // 暂时越界。本函数是**全部**矩形（两栏、lane 区、状态栈、聊天列）的唯一来源，
    // 夹在这里就不可能出现「画的是夹取后的、点的是夹取前的」。`clamped` 幂等，
    // 调用方（`draw` 要先知道右栏内宽才能算状态栈行数）先夹过也没关系。
    let layout = layout.clamped(area.width);
    if !layout.visible {
        let rows = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(area);
        return Panes {
            status: Some(rows[1]),
            left: None,
            left_lanes: None,
            chat: rows[0],
            right: None,
            right_status: None,
        };
    }
    let cols = Layout::horizontal([
        Constraint::Length(layout.widths.left),
        Constraint::Min(0),
        Constraint::Length(layout.widths.right),
    ])
    .split(area);
    let stack = stack_rows.min(cols[2].height.saturating_sub(1));
    let right_rows =
        Layout::vertical([Constraint::Min(1), Constraint::Length(stack)]).split(cols[2]);
    // lane 区高度固定为左栏一半、会话区至少留 1 行：两条不变量都算在这里，渲染侧不再
    // 各算一遍，否则「lane 区占了多高」会出现两份互相漂移的答案。
    let lane_h = lane_pane_height(cols[0].height, lanes_len);
    let left_rows =
        Layout::vertical([Constraint::Min(1), Constraint::Length(lane_h)]).split(cols[0]);
    Panes {
        status: None,
        left: Some(left_rows[0]),
        left_lanes: (lane_h > 0).then_some(left_rows[1]),
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

/// 左栏 lane 列表里被点中的 lane 下标；标题行 / 空白区返回 `None`。
///
/// 行号换算只在 [`lane_index_at`] 里做一次：绘制与命中都用 [`panes_with_lanes`] 的
/// 同一份矩形与同一份块高，两边各算一遍迟早漂移（症状是「点第 2 条切到第 3 条」）。
pub(super) fn lane_hit(p: &Panes, lanes_len: usize, column: u16, row: u16) -> Option<usize> {
    let rect = p.left_lanes?;
    if !contains(rect, column, row) {
        return None;
    }
    lane_index_at(rect.height, lanes_len, row.saturating_sub(rect.y))
}

#[cfg(test)]
mod tests {
    use super::super::pane_width::{
        MIN_CHAT_COLS, MIN_WORKSPACE_COLS, PaneWidths, SIDEBAR_WIDTH, WORKSPACE_PANE_WIDTH,
    };
    use super::*;

    /// 两栏可见、默认宽度（宽屏用例的基准版面）。
    fn visible_default() -> PaneLayout {
        PaneLayout {
            visible: true,
            widths: PaneWidths::DEFAULT,
        }
    }

    /// 窄屏：两栏隐藏。
    fn hidden() -> PaneLayout {
        PaneLayout {
            visible: false,
            widths: PaneWidths::DEFAULT,
        }
    }

    /// 宽屏：200 列 × 40 行，状态栈 5 行。
    fn wide() -> Panes {
        panes(Rect::new(0, 0, 200, 40), visible_default(), 5)
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

    /// 用户把左栏调宽：三列按新宽度重排，聊天列吸收差额（`Min(0)` 那一段）。
    #[test]
    fn wider_left_pane_takes_columns_from_the_chat_column() {
        let layout = PaneLayout {
            visible: true,
            widths: PaneWidths {
                left: 40,
                right: WORKSPACE_PANE_WIDTH,
            },
        };
        let p = panes(Rect::new(0, 0, 200, 40), layout, 5);
        let left = p.left.expect("宽屏有左栏");
        assert_eq!(left.width, 40);
        assert_eq!(p.chat.x, 40, "聊天列紧接左栏");
        assert_eq!(p.chat.width, 200 - 40 - WORKSPACE_PANE_WIDTH);
        assert_eq!(p.right.expect("宽屏有右栏").x, 200 - WORKSPACE_PANE_WIDTH);
    }

    /// 终端缩小到「两栏的请求宽度放不下」：由本函数夹取，聊天列仍守住保底 ——
    /// 这正是「夹取只写在一处」的意义，调用方完全不必知道终端有多宽。
    #[test]
    fn layout_clamps_widths_against_the_terminal_and_keeps_the_chat_column() {
        let layout = PaneLayout {
            visible: true,
            widths: PaneWidths {
                left: 60,
                right: 80,
            },
        };
        let p = panes(Rect::new(0, 0, 130, 40), layout, 5);
        let left = p.left.expect("宽屏有左栏");
        let right = p.right.expect("宽屏有右栏");
        assert!(
            right.width >= MIN_WORKSPACE_COLS,
            "右栏不低于自身下限：{}",
            right.width
        );
        // 130 - 40(聊天保底) - 60(左栏) = 30：右栏让到刚好够用，左栏一列不动。
        assert_eq!(
            right.width,
            130 - MIN_CHAT_COLS - left.width,
            "预算不够时由右栏先让步"
        );
        assert_eq!(p.chat.width, MIN_CHAT_COLS, "正文正好保住保底");
        assert_eq!(
            left.width + p.chat.width + right.width,
            130,
            "三列拼满且不重叠"
        );
    }

    #[test]
    fn narrow_layout_keeps_the_bottom_bar() {
        let p = panes(Rect::new(0, 0, 80, 24), hidden(), 5);
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
        let p = panes(Rect::new(0, 0, 200, 6), visible_default(), 40);
        assert_eq!(p.right.expect("宽屏有右栏").height, 1, "树至少留一行");
        assert_eq!(p.right_status.expect("仍有状态栈").height, 5);
    }

    #[test]
    fn zero_stack_rows_leaves_the_tree_full_height() {
        let p = panes(Rect::new(0, 0, 200, 40), visible_default(), 0);
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

    /// lane 区高度**固定**为左栏一半：条数不影响版面高度（1 条与 8 条一样高）。
    #[test]
    fn lane_pane_always_takes_half_of_the_sidebar() {
        assert_eq!(lane_pane_height(40, 1), 20);
        assert_eq!(lane_pane_height(40, 8), 20, "条数不改变版面高度");
        assert_eq!(lane_pane_height(41, 3), 20, "奇数行向下取整");
        assert_eq!(lane_pane_height(40, 0), 0, "没有 lane 才不占版面");
        assert_eq!(lane_pane_height(2, 1), 1, "会话列表至少留 1 行");
        assert_eq!(lane_pane_height(1, 3), 0, "连一行都让不出来就不切");
    }

    #[test]
    fn lanes_split_the_sidebar_at_the_half_line() {
        let p = panes_with_lanes(Rect::new(0, 0, 200, 40), visible_default(), 5, 1);
        let lanes = p.left_lanes.expect("单 lane 也要有 lane 区");
        assert_eq!(lanes.height, 20);
        assert_eq!(p.left.unwrap().height, 20, "会话列表让出另一半");

        // 多 lane 也是同一块高度，只有里面的色块变矮。
        let p = panes_with_lanes(Rect::new(0, 0, 200, 40), visible_default(), 5, 12);
        assert_eq!(p.left_lanes.expect("lane 区仍在").height, 20);
        assert_eq!(p.left.unwrap().height, 20);
    }

    /// 没有 lane（`lanes_len == 0`）：左栏不下切，整块留给会话列表。
    #[test]
    fn zero_lanes_keep_the_sidebar_whole() {
        let p = panes_with_lanes(Rect::new(0, 0, 200, 40), visible_default(), 5, 0);
        assert!(p.left_lanes.is_none(), "没有 lane 就不该凭空多出一块");
        assert_eq!(p.left.unwrap().height, 40);
    }

    /// 左栏隐藏时 lane 区也必须是 `None`：否则窄屏会画到不存在的列上。
    #[test]
    fn hidden_sidebar_has_no_lane_pane() {
        let p = panes_with_lanes(Rect::new(0, 0, 80, 24), hidden(), 5, 4);
        assert!(p.left.is_none());
        assert!(p.left_lanes.is_none());
    }

    /// 一条 lane 一行：可用行不够时按行截断（多出的由标题的 `↓n` 报出）。
    #[test]
    fn shown_len_keeps_one_row_per_lane() {
        assert_eq!(lane_shown_len(19, 0), 0, "没有 lane 就没有行");
        assert_eq!(lane_shown_len(0, 3), 0, "没有可用行就一条都画不了");
        assert_eq!(lane_shown_len(19, 1), 1);
        assert_eq!(lane_shown_len(19, 6), 6);
        assert_eq!(lane_shown_len(19, 20), 19, "20 条只能画 19 条");
    }

    /// 命中与渲染同源：每条 lane 恰好一行，标题行与色条之下的空白不命中。
    #[test]
    fn lane_index_at_matches_one_row_per_lane() {
        // 20 行 lane 区（19 行可用）+ 3 条 lane → 第 1..=3 行各一条。
        assert_eq!(lane_index_at(20, 3, 0), None, "标题行不算");
        assert_eq!(lane_index_at(20, 3, 1), Some(0));
        assert_eq!(lane_index_at(20, 3, 2), Some(1));
        assert_eq!(lane_index_at(20, 3, 3), Some(2));
        assert_eq!(lane_index_at(20, 3, 4), None, "色条之下的空白不命中");
        // 20 条挤在 19 行里：最后一行是第 19 条，第 20 条被截断。
        assert_eq!(lane_index_at(20, 20, 19), Some(18));
        assert_eq!(lane_index_at(20, 20, 20), None);
    }
}
