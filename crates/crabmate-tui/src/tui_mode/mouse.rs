//! 鼠标事件：左栏会话行点击（选中即切换）、右栏工作区目录树的点击与滚轮，以及聊天区滚轮滚动。
//!
//! 两栏各有自己的命中区域：左键点左栏 = 选中并切到那条会话（与 `Enter` 同义），点右栏 =
//! 落树光标并展开/选中；点两栏以外的聊天列仅用于把焦点收回输入框。滚轮在右栏内驱动树光标，
//! 在左栏 / 聊天列滚动 transcript（捕获鼠标后终端不再把滚轮翻成 ↑/↓，必须自己接）。
//! 左栏**刻意不接滚轮**：会话窗口的起点由选中行决定，滚轮只能表现成「顺带改选中」，
//! 而选中即切换 —— 一次误滚就会换掉会话。
//! 所有状态变更都复用键盘已有的状态机（`ws_*` / `use_selected`），避免出现「鼠标能点出、
//! 的状态（占位行不可选中、收起父目录后光标归位等不变量因此自动一致）。

use std::time::{Duration, Instant};

use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use super::LaneApp;
use super::layout::{contains, panes};
use super::state::{Focus, LineKind, LogLine};
use super::workspace_tree::WsActivate;
use super::ws_sidebar::{pane_list_area, sessions_content, sidebar_window, workspace_content};
use super::{status, status_stack};

impl LaneApp<'_> {
    /// 鼠标事件入口。
    ///
    /// 审批浮层/设置面板占用键盘时同样不接受鼠标，否则会绕过
    /// 「审批未决不得输入」「脏字段先确认」这类键盘侧门禁。
    pub(super) fn on_mouse(&mut self, ev: MouseEvent) {
        if self.st.approval.is_some() || self.panel.is_some() {
            // 浮层开着时可能刚好松开左键：拖拽态留着，下一次 `Drag` 会接着改宽，
            // 而用户看到的是一次「点了浮层，栏宽自己变了」。
            if let Some(drag) = self.pane_drag.take() {
                // 手势在这里被强行结束，而宽度可能已经改过：不落盘的话，屏幕上写着新宽度，
                // 重启后却回到默认 —— 而那次 `Up` 已经被浮层吃掉，不会再进来一次。
                self.persist_pane_widths(drag.edge);
            }
            return;
        }
        if self.on_mouse_prelude(&ev) {
            return;
        }
        let (width, height) = self.last_size;
        let p = panes(
            Rect::new(0, 0, width, height),
            self.st.pane_layout(),
            self.status_stack_rows(),
        );
        let (column, row) = (ev.column, ev.row);
        // 窄屏侧栏隐藏时两栏矩形都不存在，鼠标完全不动状态。
        let on_right = p.right.is_some_and(|r| contains(r, column, row));
        let on_left = p.left.is_some_and(|r| contains(r, column, row));
        match ev.kind {
            MouseEventKind::ScrollUp => self.on_scroll(true, on_right),
            MouseEventKind::ScrollDown => self.on_scroll(false, on_right),
            MouseEventKind::Down(MouseButton::Left) if on_right => {
                self.click_workspace(column, row);
            }
            // 左栏会话行：选中即切换（与 Enter 同义），见 `click_sessions`。
            MouseEventKind::Down(MouseButton::Left) if on_left => {
                self.click_sessions(column, row);
            }
            // 状态行 `perm` 段与权限菜单已在入口处处理（`on_status_click`），这里只剩两栏。
            // 点右栏以外：想打字或看聊天，把焦点从侧栏收回输入框
            // （否则后续按键仍落在侧栏，用户会觉得「点了没反应」）。
            // 正文区：落在过程摘要行上 = 切换它的开合（见 `click_body`）。没命中就往下
            // 走兜底，把焦点收回输入框 —— 点聊天区的人多半是想接着打字。
            // 正文区（含聊天列）：链接 → 折叠 → 兜底收焦点，顺序与理由见 `body_click`。
            MouseEventKind::Down(MouseButton::Left) => {
                self.on_body_click(column, row, ev.modifiers.contains(KeyModifiers::CONTROL))
            }
            _ => {}
        }
    }

    /// 滚轮：右栏滚目录树（已聚焦时），其它位置滚 transcript。
    ///
    /// 右栏以外的滚轮（聊天列 / 左栏）步长与键盘单行时的 ↑/↓ 一致。必须自己处理：
    /// 捕获鼠标后终端不再把滚轮翻译成 ↑/↓（xterm alternate-scroll），不接这段就会表现为
    /// 「聊天区滚轮没反应」。
    fn on_scroll(&mut self, up: bool, on_right: bool) {
        if on_right {
            self.scroll_workspace(up);
            return;
        }
        // 编辑器视图占着聊天列：此时该滚的是文件本身。继续滚 transcript 看不见任何变化
        // （那块内容被视图整片盖住），而「滚轮没反应」和「文件到底了」在屏幕上长得一样。
        if self.st.editor.open {
            self.editor_wheel(up);
            return;
        }
        self.st.scroll_lines(up, 3);
    }

    /// 状态区的左键点击：权限菜单开着时先交给它；否则 `perm` 段开菜单、`think` 段切开关
    /// （菜单锚在这一段的矩形上，不居中）。
    ///
    /// 返回 `true` = 已消费（调用方立刻返回，不再走两栏 / 兜底分支）。
    ///
    /// `pub(super)`：入口门禁在 `mouse_drag::on_mouse_prelude` 里，与拖拽同处一层。
    pub(super) fn on_status_click(&mut self, column: u16, row: u16) -> bool {
        if self.st.permission_menu.is_some() {
            return self.on_permission_menu_click(column, row);
        }
        if let Some(rect) = self.hit_status_segment(status::PERM_LABEL, column, row) {
            self.open_permission_menu(rect);
            return true;
        }
        // 思考开关：与 Desktop 底栏那颗 chip 同功能（点一下切一档），
        // 值归一与落盘都在 `status_thinking`。
        if self
            .hit_status_segment(status::THINK_LABEL, column, row)
            .is_some()
        {
            self.toggle_thinking_mode();
            return true;
        }
        // 视图开关：与 `Ctrl+T` / `/editor` / `/chat` 同一入口 —— 状态栏这一段是
        // 「我现在在哪」的唯一常驻指示，点它就该能切，而不是只能记住快捷键。
        if self
            .hit_status_segment(status::VIEW_LABEL, column, row)
            .is_some()
        {
            self.toggle_view();
            return true;
        }
        false
    }

    /// 屏幕坐标是否落在某一段状态段上；命中就返回该段矩形（`perm` 拿它当菜单锚点）。
    ///
    /// 三段（`perm` / `think` / `view`）共用这一个判定：各写一份 `if let … && contains(…)`
    /// 会让「点得到吗」这一件事在同一个函数里付三份圈复杂度（`lizard` 的 CCN 门禁）。
    fn hit_status_segment(&self, label: &str, column: u16, row: u16) -> Option<Rect> {
        let rect = self.status_segment_rect(label)?;
        contains(rect, column, row).then_some(rect)
    }

    /// 状态区某一段在屏幕上的矩形；`None` = 该段不存在 / 被裁掉。
    ///
    /// 宽屏取右栏状态栈的那一行（整行内宽都可点），窄屏取底栏那几列。两处位置都由渲染
    /// 同一套函数算出（`status_stack::segment_row_index` / `status::segment_cols`），这里只做
    /// 坐标换算 —— 排版算法重算一遍就会点偏。
    ///
    /// 返回矩形而不是 `bool`：`perm` 那边除判定命中还要拿它当菜单的锚点。
    ///
    /// 该段不存在时返回 `None`（老 serve 没有 `/permission`、窄屏按优先级被裁掉、
    /// 状态栈处于折叠态），此时点击落到下面的兜底分支，与以前一样只是把焦点收回输入框。
    fn status_segment_rect(&self, label: &str) -> Option<Rect> {
        let (width, height) = self.last_size;
        let info = self.status_info();
        let p = panes(
            Rect::new(0, 0, width, height),
            self.st.pane_layout(),
            self.status_stack_rows(),
        );
        if let Some(area) = p.right_status {
            let inner_w = area.width.saturating_sub(2) as usize;
            let idx = status_stack::segment_row_index(
                &info,
                inner_w,
                self.st.status_stack_collapsed,
                label,
            )?;
            // `mode` 与 `think` 同排一行：行号相同，靠列区间分左右。`think` 只占右半，
            // 点左半（`mode` 那侧）不该切开关。
            let (start, end) = status_stack::segment_cols_in_row(
                &info,
                inner_w,
                self.st.status_stack_collapsed,
                label,
            )?;
            return Some(Rect::new(
                area.x.saturating_add(1).saturating_add(start),
                area.y.saturating_add(idx),
                end.saturating_sub(start),
                1,
            ));
        }
        let area = p.status?;
        let (start, end) = status::segment_cols(&info, area.width as usize, label)?;
        Some(Rect::new(
            area.x.saturating_add(start),
            area.y,
            end.saturating_sub(start),
            1,
        ))
    }

    /// 滚轮：项目池子视图滚候选，目录树滚光标（复用 `ws_move` 的跳过占位语义）。
    ///
    /// 只有树已聚焦时才响应：否则一次误滚（指针恰好停在右栏）就会把焦点从输入框拽到
    /// 侧栏，之后打出的字全变成树的命令键（`r` / `p` / `w`）。想滚树先点它或 Ctrl+W。
    fn scroll_workspace(&mut self, up: bool) {
        if self.st.focus != Focus::Workspace {
            return;
        }
        if self.st.ws_pick.is_some() {
            self.st.ws_pick_move(up);
        } else {
            self.st.ws_move(up);
        }
    }

    /// 左键点击右栏：聚焦 + 落光标；目录行顺带展开/收起（可逆），文件行只选中。
    ///
    /// 文件行不直接插入路径是有意的：那会顶掉输入框里正在写的内容，
    /// 而插入路径仍由 Enter 完成（底部提示行已写明）。
    ///
    /// **编辑器视图开着时，双击文件行 = 打开它**（双击由 [`ClickTracker`] 认出来）：
    /// 视图占的正是聊天区，此时人是在「看文件」，点两下就该换文件，与目录树里的 Enter
    /// 走同一入口（[`Self::open_editor`]）。聊天视图下的双击不加新语义 —— 那里 Enter 是
    /// 「插路径」，双击再插一次只会把草稿顶掉。
    fn click_workspace(&mut self, column: u16, row: u16) {
        let double = self.ws_click.register(Instant::now(), column, row);
        self.apply_workspace_click(column, row, double);
    }

    /// [`Self::click_workspace`] 的落点部分：`double` 由调用方判定（单测直接喂，不必等时钟）。
    fn apply_workspace_click(&mut self, column: u16, row: u16, double: bool) {
        let (width, height) = self.last_size;
        let p = panes(
            Rect::new(0, 0, width, height),
            self.st.pane_layout(),
            self.status_stack_rows(),
        );
        let Some(right) = p.right else {
            return;
        };
        self.st.focus = Focus::Workspace;
        // 焦点是点出来的：底部按键提示行不画（键一按就恢复，见 `LaneApp::on_key`）。
        self.st.focus_by_pointer = true;
        // 用渲染同一份内容算行数，避免再维护一遍「标题 + 占位行」的行数口径。
        let content_width = right.width.saturating_sub(2) as usize;
        let (rows, cursor, _hint) = workspace_content(&self.st, content_width);
        let Some(idx) = hit_item(right, column, row, rows.len(), cursor) else {
            return; // 标题行 / 底部提示行 / 列表末尾的空白：只切焦点
        };
        if self.st.ws_pick.is_some() {
            if self.st.ws_pick_set(idx) {
                // 点击候选即切换（与 Enter 一致）。
                if let Some(name) = self.st.ws_pick_begin_switch() {
                    self.st
                        .push_line(LineKind::System, &format!("正在切换到项目「{name}」…"));
                    self.switch_ws_project(name);
                }
            }
            return;
        }
        if !self.st.ws_set_cursor(idx) {
            return; // 占位行不可选中（与 ↑↓ 跳过一致）
        }
        match self.st.ws_activate() {
            Some(WsActivate::Dir { rel, fetch }) => {
                if fetch {
                    self.fetch_ws_dir(rel);
                }
            }
            // 编辑器视图开着时双击 = 打开这个文件。焦点交回输入框：打开之后按 PgUp/PgDn
            // 该滚文件，不该滚树（焦点若留在侧栏，那几个键走的是 `on_workspace_key`）。
            Some(WsActivate::File(rel)) if double && self.st.editor.open => {
                self.open_editor(&rel);
                self.st.focus = Focus::Input;
                self.st.focus_by_pointer = false;
            }
            Some(WsActivate::File(_)) | None => {}
        }
    }

    /// 左键点击左栏会话行：聚焦会话栏 + 落选中，并**直接切换**（与 `Enter` 同义）。
    ///
    /// 单击即切换是刻意的：多要一次 `Enter` 只会让人以为「点了没反应」。切换的守卫不在这里
    /// 重写——回合进行中拒绝、空白会话只给提示、`/conv new` 的待绑定空行原地不动，全部由
    /// [`LaneApp::use_selected`] 一个入口负责，鼠标与键盘不会给出两套结论。
    fn click_sessions(&mut self, column: u16, row: u16) {
        let (width, height) = self.last_size;
        let p = panes(
            Rect::new(0, 0, width, height),
            self.st.pane_layout(),
            self.status_stack_rows(),
        );
        let Some(left) = p.left else {
            return; // 窄屏：左栏不存在
        };
        self.st.focus = Focus::Sidebar;
        // 焦点是点出来的：底部按键提示行不画（键一按就恢复，见 `LaneApp::on_key`）。
        self.st.focus_by_pointer = true;
        // 焦点/选中行一换，待确认删除即作废：否则会「确认的是这一行、删掉的是另一行」。
        self.st.confirm_delete = None;
        // 列内左右各留 1 列：与 `render_pane` 的内宽口径必须是同一个值。
        let content_width = left.width.saturating_sub(2) as usize;
        let (rows, cursor, _hint) = sessions_content(&self.st, content_width);
        let Some(idx) = hit_item(left, column, row, rows.len(), cursor) else {
            return; // 标题行 / 底部提示行 / 列表末尾的空白：只切焦点
        };
        // 空列表时 `rows[1]` 是「（空）发一条消息后出现」占位行，不能当成会话 0 号。
        if idx >= self.st.sessions.len() {
            return;
        }
        self.st.selected = idx;
        self.use_selected();
    }

    /// 左键点正文区：落在过程摘要行上就切换它的开合，并把折叠光标落上去（之后可直接用
    /// `↑/↓` 换一条、`Enter` 再切）。返回 `true` = 已消费。
    ///
    /// 点在别的正文行（答案 / 用户消息 / 分隔线）上**不**消费：那一下的本意通常是
    /// 「回到输入框」，吞掉它会让焦点停在侧栏不动，正是「点了没反应」的来源。
    pub(super) fn click_body(&mut self, column: u16, row: u16) -> bool {
        let Some(area) = self.st.body_area else {
            return false;
        };
        match body_hit(area, &self.st.body_rows_log, column, row) {
            BodyHit::Outside | BodyHit::Gap => {
                // 点在分隔行 / 空白：导航作废（光标已不在可见内容上），焦点交回兜底分支。
                self.st.fold_cursor = None;
                false
            }
            BodyHit::Log(idx)
                if self.st.lines.get(idx).map(LogLine::kind) == Some(LineKind::Process) =>
            {
                // 键路必须跟光标一起过来：命中后我们 `return true`（不落到兜底的「收回
                // 输入框」），若焦点还留在侧栏，接着按 ↑/↓ 走的是 `on_sidebar_key` ——
                // 用户看到的是「点完摘要，方向键在乱动左边的列表」。
                self.st.focus = Focus::Input;
                self.st.focus_by_pointer = false;
                self.st.confirm_delete = None;
                self.st.fold_cursor = Some(idx);
                self.st.toggle_process_run(idx);
                true
            }
            BodyHit::Log(_) => {
                self.st.fold_cursor = None;
                false
            }
        }
    }
}

/// 双击判定窗口。
///
/// 终端不上报「双击」这个事件（crossterm 的 `MouseEventKind` 没有这一项），只能按
/// 「同一行、短间隔内第二次按下」自己认。400ms 与常见 GUI 的默认值同量级：再短会把
/// 「手慢的正常双击」判成两次单击，再长会让两下无关的单击连成一次「打开文件」。
const DOUBLE_CLICK_WINDOW: Duration = Duration::from_millis(400);

/// 右栏左键的双击识别器（由 `LaneApp` 持有）。
///
/// 只回答一个问题：「这一次按下算不算双击」。命中后记录清空，于是三连击的第二、三下
/// 不会被算成两次打开 —— 每次打开都要发一趟 `GET /workspace/file`。
///
/// 放 `LaneApp` 而不是 `UiState`：与 `pane_drag` 同理，它是一次指针手势的中间态，
/// 不是可渲染的界面状态。
#[derive(Debug, Default)]
pub(super) struct ClickTracker {
    last: Option<(Instant, u16, u16)>,
}

impl ClickTracker {
    /// 记一次左键按下，返回它是否与上一次构成双击。
    ///
    /// 间隔用 `saturating_duration_since`：时钟被回拨时按「间隔 0」处理（不会 panic），
    /// 而落点仍要求同一行列，所以不会把很久以前的点击当成新鲜的前一下。
    pub(super) fn register(&mut self, now: Instant, column: u16, row: u16) -> bool {
        let double = self.last.is_some_and(|(t, c, r)| {
            c == column && r == row && now.saturating_duration_since(t) <= DOUBLE_CLICK_WINDOW
        });
        self.last = if double {
            None
        } else {
            Some((now, column, row))
        };
        double
    }
}

/// 正文区的点击落点。
enum BodyHit {
    /// 不在正文区内（composer、两侧栏、状态区）。
    Outside,
    /// 在正文区内，但这一行不对应任何逻辑行（块间分隔线 / 窗口末尾的空白）。
    Gap,
    /// 落在第 n 条逻辑行上。
    Log(usize),
}

/// 屏幕坐标 → 逻辑行。
///
/// 窗口内容（`body_area` + 逐物理行的 `log_index`）由 `render_body` 每帧回写，这里只做
/// 一次减法：在 `mouse.rs` 里重算一遍折行 / 分隔线 / 窗口偏移，是「点到的行与反色的行
/// 错开」的唯一来源（左栏的 `hit_item` 也正是因为共用渲染口径才不出错）。
fn body_hit(area: Rect, rows: &[Option<usize>], column: u16, row: u16) -> BodyHit {
    if !contains(area, column, row) {
        return BodyHit::Outside;
    }
    match rows.get((row - area.y) as usize).copied().flatten() {
        Some(idx) => BodyHit::Log(idx),
        None => BodyHit::Gap,
    }
}

/// 鼠标坐标 → 条目下标：内容区（[`pane_list_area`]）与裁剪窗口（[`sidebar_window`]）都取
/// 渲染用的那一份，两栏的「标题行占第 0 行、窗口末尾是空白」等口径因此只有一处定义。
///
/// 落在标题行、底部提示行或窗口末尾的空白上返回 `None`（调用方只切焦点，不改选中）。
fn hit_item(area: Rect, column: u16, row: u16, rows_len: usize, cursor: usize) -> Option<usize> {
    let list = pane_list_area(area);
    if !contains(list, column, row) {
        return None;
    }
    let (start, end) = sidebar_window(rows_len, list.height as usize, cursor);
    item_index_at((row - list.y) as usize, start, end)
}

/// 可视行（第 0 行是固定标题）→ 条目下标；越界或落在标题行返回 `None`。
fn item_index_at(rel_row: usize, start: usize, end: usize) -> Option<usize> {
    let rel = rel_row.checked_sub(1)?;
    let idx = start + rel;
    (idx < end).then_some(idx)
}

#[cfg(test)]
#[path = "mouse_tests.rs"]
mod tests;
