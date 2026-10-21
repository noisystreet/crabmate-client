//! 一帧的屏幕分区，以及鼠标命中检测用的矩形工具。
//!
//! 抽成纯函数是为了让**渲染与鼠标点击共用同一套矩形**：两边各算一遍迟早漂移，
//! 而漂移的症状（点到隔壁列、点错行）比渲染错位更难察觉。

use ratatui::layout::{Constraint, Layout, Rect};

use super::render::SIDEBAR_WIDTH;

/// 主体 / 状态栏；宽屏时主体再分「左会话 | 聊天 | 右工作区」三列。
pub(super) struct Panes {
    pub status: Rect,
    /// 左栏会话列（窄屏隐藏时为 `None`）。
    pub left: Option<Rect>,
    /// 中列：transcript + composer。
    pub chat: Rect,
    /// 右栏工作区目录树（窄屏隐藏时为 `None`）。
    pub right: Option<Rect>,
}

/// 按终端区域与侧栏可见性切分。
pub(super) fn panes(area: Rect, sidebar_visible: bool) -> Panes {
    let rows = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(area);
    let (left, chat, right) = if sidebar_visible {
        let cols = Layout::horizontal([
            Constraint::Length(SIDEBAR_WIDTH),
            Constraint::Min(0),
            Constraint::Length(SIDEBAR_WIDTH),
        ])
        .split(rows[0]);
        (Some(cols[0]), cols[1], Some(cols[2]))
    } else {
        (None, rows[0], None)
    };
    Panes {
        status: rows[1],
        left,
        chat,
        right,
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

    fn wide() -> Panes {
        panes(Rect::new(0, 0, 200, 40), true)
    }

    #[test]
    fn wide_layout_has_three_columns() {
        let p = wide();
        let left = p.left.expect("宽屏有左栏");
        let right = p.right.expect("宽屏有右栏");
        assert_eq!(left.x, 0);
        assert_eq!(left.width, SIDEBAR_WIDTH);
        assert_eq!(right.x, 200 - SIDEBAR_WIDTH);
        assert_eq!(right.width, SIDEBAR_WIDTH);
        // 中列起止连续，三列拼满主体且不重叠。
        assert_eq!(p.chat.x, left.width);
        assert_eq!(p.chat.x + p.chat.width, right.x);
        assert_eq!(p.chat.y, 0, "主体从第 0 行起（没有顶栏占行）");
        assert_eq!(p.status.y, 39);
    }

    #[test]
    fn narrow_layout_hides_sidebars() {
        let p = panes(Rect::new(0, 0, 80, 24), false);
        assert!(p.left.is_none());
        assert!(p.right.is_none());
        assert_eq!(p.chat.width, 80);
    }

    #[test]
    fn columns_share_the_same_rows() {
        let p = wide();
        let cols = [p.left.unwrap(), p.chat, p.right.unwrap()];
        for r in cols {
            assert_eq!(r.y, cols[0].y);
            assert_eq!(r.height, cols[0].height);
            // 三列从第 0 行起，到状态栏上沿为止。
            assert_eq!(r.y, 0);
            assert!(r.y + r.height <= p.status.y);
        }
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
