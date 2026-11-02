//! 分隔线拖拽的**入口**（issue #407 的 P0.5）：谁先吃掉鼠标事件，以及拖动时改宽。
//!
//! 与 `mouse.rs` 分开是因为那边贴着 920 行的文件门禁，而这两件事自成一层：`mouse.rs`
//! 管「点会话 / 点树 / 点正文」，这里只管「这次事件是不是指针手势」。分开之后，
//! 「拖宽顺手切了会话」这类回归的落点也只可能在这一个文件里。
//!
//! 换算本身（第几列 → 多宽、命中哪条线）在 `pane_drag.rs`，是纯函数。

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use super::LaneApp;
use super::layout::panes;
use super::pane_drag::{PaneDrag, drag_target, edge_at as pane_edge_at};

impl LaneApp<'_> {
    /// 入口的前两道门禁：状态区点击（含权限菜单模态）与分隔线拖拽；命中就吃掉这次事件。
    ///
    /// 顺序是重点：状态段先（权限菜单是模态，开着时应当由它吃掉点击、并顺手关掉自己），
    /// 分隔线其次。两处都不能晚于侧栏 —— 状态栈底行与右栏分隔线**同列**，左栏分隔线
    /// 那一列又落在左栏矩形里，晚一步就会被当成「点了会话 / 点了树」。
    pub(super) fn on_mouse_prelude(&mut self, ev: &MouseEvent) -> bool {
        if ev.kind == MouseEventKind::Down(MouseButton::Left)
            && self.on_status_click(ev.column, ev.row)
        {
            return true;
        }
        self.on_pane_drag(ev)
    }

    /// 分隔线拖拽：按下开始、拖动改宽、松开结束。
    ///
    /// 返回 `true` = 这条事件已被拖拽吃掉。`Drag` / `Up` **无论如何**都返回 `true`：
    /// 它们除了拖拽没有别的语义，而放它们继续往下走，就是「拖动经过左栏 → 顺手把
    /// 会话切走」。
    ///
    /// 不在拖拽态的 `Drag` 什么也不改：读取线程只放行左键的 `Drag`，但浮层早退会清掉
    /// 拖拽态（见 `on_mouse`），所以「没有起点却收到 Drag」确实可能出现。
    fn on_pane_drag(&mut self, ev: &MouseEvent) -> bool {
        match ev.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let (width, height) = self.last_size;
                let p = panes(
                    Rect::new(0, 0, width, height),
                    self.st.pane_layout(),
                    self.status_stack_rows(),
                );
                if let Some(edge) = pane_edge_at(&p, ev.column, ev.row) {
                    self.pane_drag = Some(PaneDrag { edge });
                    return true;
                }
                // 不是分隔线：交回给「点会话 / 点树 / 点正文」。
                false
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some(drag) = self.pane_drag {
                    let (width, _) = self.last_size;
                    let next = drag_target(drag.edge, ev.column, width, self.st.pane);
                    // 同列重复的拖拽不写状态：每写一次整段 transcript 都要重排，而快速
                    // 拖动时终端会连发几十条同一列的事件。宽度没变就什么也别做。
                    if next != self.st.pane {
                        self.st.pane = next;
                    }
                }
                true
            }
            MouseEventKind::Up(MouseButton::Left) => {
                self.pane_drag = None;
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use ratatui::layout::Rect;

    use crate::tui_mode::test_support::{test_client, test_prefs, test_shell};

    /// 拖动分隔线会扫过左栏下半的 lane 列表：只有**按下左键**才算点了某条连接，
    /// 否则「拖宽」会顺手把连接切走（`lane_at_click` 在 `shell.rs`）。
    #[test]
    fn only_a_press_selects_a_lane_from_the_list() {
        let client = test_client();
        let mut prefs = [test_prefs(), test_prefs(), test_prefs()];
        let mut app = test_shell(&client, &mut prefs);
        app.lanes[0].last_size = (200, 40);
        app.lanes[0].st.sidebar_visible = true;
        let p = crate::tui_mode::layout::panes_with_lanes(
            Rect::new(0, 0, 200, 40),
            app.lanes[0].st.pane_layout(),
            app.lanes[0].status_stack_rows(),
            app.lanes.len(),
        );
        let lanes = p.left_lanes.expect("三条连接时左栏下半就是 lane 列表");
        let press = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: lanes.x + 2,
            row: lanes.y + 1,
            modifiers: KeyModifiers::NONE,
        };
        assert_eq!(app.lane_at_click(&press), Some(0), "按下 → 第一条连接");
        for kind in [
            MouseEventKind::Drag(MouseButton::Left),
            MouseEventKind::Up(MouseButton::Left),
        ] {
            let ev = MouseEvent { kind, ..press };
            assert_eq!(app.lane_at_click(&ev), None, "{kind:?} 不算点击");
        }
    }
}
