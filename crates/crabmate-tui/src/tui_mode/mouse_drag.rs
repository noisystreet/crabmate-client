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
use super::pane_drag::{PaneDrag, PaneEdge, drag_target, edge_at as pane_edge_at};
use super::pane_width::PaneWidths;
use super::state::LineKind;

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
                // 一次手势结束才落盘：拖动途中每列都写一次会把磁盘当帧缓冲用。
                // 取 `edge` 是为了**只写被拖的那一栏**（理由见 `persist_pane_widths`）。
                if let Some(drag) = self.pane_drag.take() {
                    self.persist_pane_widths(drag.edge);
                }
                true
            }
            _ => false,
        }
    }

    /// 把宽度存到本机（issue #407 的 P2），**只写被拖的那一栏**。
    ///
    /// 为什么只写一栏：`st.pane` 的另一栏可能已经被**终端预算让位**压过
    /// （`clamped(terminal_width)` 在窄终端里先压右栏）。把那份让位结果当用户选择写回文件，
    /// 症状是「在小窗口里拖了一下左栏，回到大终端右栏就永久变窄了」——让位是显示层的结论，
    /// 不是用户的选择，文件里该留下的是「各栏各自被拖到多宽」。
    ///
    /// 只在合成结果与 [`Self::pane_saved`] 不同时写：左键松开是常事（点会话、点树都会来一次
    /// `Up`），每次写文件既无意义，也会把「上次保存的值」变成噪声。
    ///
    /// 写失败只提示**一次**：鼠标手势是高频动作，每拖一次就再报一次会把 transcript 刷成日志；
    /// 但完全不说是更坏的选择 —— 用户会以为宽度存下来了，下次启动又回到默认却查不出原因。
    /// 失败时**不**更新 `pane_saved`，于是下一次手势会再试。
    ///
    /// `pub(super)`：`mouse.rs` 的浮层早退也会结束一次手势（那时宽度可能已经改过），
    /// 那里同样得落盘 —— 否则屏幕上写着新宽度，重启后却回到默认。
    pub(super) fn persist_pane_widths(&mut self, edge: PaneEdge) {
        let merged = match edge {
            PaneEdge::Left => PaneWidths {
                left: self.st.pane.left,
                right: self.pane_saved.right,
            },
            PaneEdge::Right => PaneWidths {
                left: self.pane_saved.left,
                right: self.st.pane.right,
            },
        };
        if merged == self.pane_saved {
            return;
        }
        match self.pane_store.save(merged) {
            Ok(()) => self.pane_saved = merged,
            Err(err) => {
                if !self.pane_save_warned {
                    self.pane_save_warned = true;
                    self.st.push_line(
                        LineKind::System,
                        &format!("列宽没能存到本机（本次运行内仍然生效）：{err}"),
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use ratatui::layout::Rect;

    use crate::tui_mode::LaneApp;
    use crate::tui_mode::pane_store::{PaneStore, load_from};
    use crate::tui_mode::pane_width::PaneWidths;
    use crate::tui_mode::settings_panel::SettingsPanel;
    use crate::tui_mode::test_support::{test_client, test_prefs, test_shell};

    /// 每个用例一个独立文件：并行跑测试不会互相覆盖对方的落盘结果。
    fn temp_store(tag: &str) -> (PathBuf, PaneStore) {
        let path = std::env::temp_dir().join(format!(
            "crabmate-tui-panes-drag-{}-{tag}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        (path.clone(), PaneStore::at(path))
    }

    /// 按下左栏分隔线、拖到第 `column` 列；返回那条 `Drag`（`Up` 用它构造，列不变）。
    fn drag_left_divider_to(lane: &mut LaneApp<'_>, column: u16) -> MouseEvent {
        let (width, height) = lane.last_size;
        let left = crate::tui_mode::layout::panes(
            Rect::new(0, 0, width, height),
            lane.st.pane_layout(),
            lane.status_stack_rows(),
        )
        .left
        .expect("宽屏有左栏");
        let press = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: left.x + left.width - 1,
            row: left.y,
            modifiers: KeyModifiers::NONE,
        };
        assert!(lane.on_pane_drag(&press), "按下分隔线由拖拽吃掉");
        let drag = MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column,
            ..press
        };
        assert!(lane.on_pane_drag(&drag), "拖动由拖拽吃掉");
        drag
    }

    fn up_from(drag: MouseEvent) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            ..drag
        }
    }

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

    /// 松手 = 一次手势结束，宽度这时才落盘；拖动途中一个字节都不写。
    ///
    /// 拖动期间终端会连发几十条同一/相邻列的事件，若每条都写文件，磁盘就成了帧缓冲。
    #[test]
    fn releasing_after_a_drag_persists_the_new_width() {
        let (path, store) = temp_store("persist");
        let client = test_client();
        let mut prefs = [test_prefs()];
        let mut app = test_shell(&client, &mut prefs);
        let lane = &mut app.lanes[0];
        lane.pane_store = store;
        lane.last_size = (200, 40);
        lane.st.sidebar_visible = true;

        let drag = drag_left_divider_to(lane, 40);
        assert_eq!(lane.st.pane.left, 41, "指针停在第 40 列 → 分隔线右移一列");
        assert_eq!(load_from(&path), None, "拖动途中不落盘");

        assert!(lane.on_pane_drag(&up_from(drag)));
        assert_eq!(
            load_from(&path),
            Some(PaneWidths {
                left: 41,
                right: PaneWidths::DEFAULT.right,
            }),
            "松手后存下来的就是屏幕上那份宽度（另一栏没动）"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// 宽度没变时松开左键（点会话、点树都会来一次 `Up`）不该碰文件。
    #[test]
    fn releasing_without_a_drag_does_not_write_the_file() {
        let (path, store) = temp_store("noop");
        let client = test_client();
        let mut prefs = [test_prefs()];
        let mut app = test_shell(&client, &mut prefs);
        let lane = &mut app.lanes[0];
        lane.pane_store = store;

        let up = MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: 5,
            row: 5,
            modifiers: KeyModifiers::NONE,
        };
        assert!(lane.on_pane_drag(&up), "Up 一律吃掉（它没有别的语义）");
        assert!(!path.exists(), "宽度没变就不该写文件");
    }

    /// **只写被拖的那一栏**：窄终端里另一栏会被聊天列保底压到下限，那份让位是显示层
    /// 结论，不能被当成「用户把右栏拖窄了」写回文件 —— 否则在小窗口里拖一下左栏，
    /// 回到大终端右栏就永久变窄了。
    #[test]
    fn only_the_dragged_column_is_written() {
        let (path, store) = temp_store("one-column");
        let client = test_client();
        let mut prefs = [test_prefs()];
        let mut app = test_shell(&client, &mut prefs);
        let lane = &mut app.lanes[0];
        lane.pane_store = store;
        // 100 列：两栏预算 60，默认 26 + 34 正好用满；把左栏拖宽就只剩让右栏。
        lane.last_size = (100, 40);
        lane.st.sidebar_visible = true;

        let drag = drag_left_divider_to(lane, 90);
        assert_eq!(
            lane.st.pane,
            PaneWidths {
                left: 40,
                right: 20
            },
            "屏幕上右栏确实被让位压到下限"
        );

        assert!(lane.on_pane_drag(&up_from(drag)));
        assert_eq!(
            load_from(&path),
            Some(PaneWidths {
                left: 40,
                right: PaneWidths::DEFAULT.right,
            }),
            "文件里只改了左栏，右栏保留用户原来那份"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// 拖到一半被浮层接管鼠标：手势被强行结束，但宽度已经改过 —— 这时必须落盘，
    /// 因为那次 `Up` 已经被浮层吃掉，不会再进来一次。
    #[test]
    fn an_overlay_taking_the_mouse_persists_the_width_so_far() {
        let (path, store) = temp_store("overlay");
        let client = test_client();
        let mut prefs = [test_prefs()];
        let mut app = test_shell(&client, &mut prefs);
        let lane = &mut app.lanes[0];
        lane.pane_store = store;
        lane.last_size = (200, 40);
        lane.st.sidebar_visible = true;

        let drag = drag_left_divider_to(lane, 40);
        lane.panel = Some(SettingsPanel::new(false));
        lane.on_mouse(up_from(drag));

        assert!(lane.pane_drag.is_none(), "浮层接管时拖拽态必须清掉");
        assert_eq!(
            load_from(&path),
            Some(PaneWidths {
                left: 41,
                right: PaneWidths::DEFAULT.right,
            }),
            "手势中途结束也要把已经改过的宽度存下来"
        );
        let _ = std::fs::remove_file(&path);
    }
}
