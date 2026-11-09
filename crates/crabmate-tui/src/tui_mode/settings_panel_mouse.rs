//! 设置面板的鼠标支持（`settings_panel.rs` 的 `#[path]` 子模块）：
//! 屏幕坐标 → 面板元素 → 与键盘同一批状态机动作。
//!
//! 独立成文件的理由：`settings_panel.rs` 贴着 `fn-nloc-ratchet` 的 920 行上限；
//! 也因为「坐标怎么落到第几行」与「按键怎么改状态」是两件事 —— 混在一起时，
//! 「点一行选中相邻那行」这类回归没有单一的落点。
//!
//! 三条约束：
//!
//! 1. **行归属只认渲染那一帧**（[`RowOwner`]）。字段值会折行、主题下拉会插行，
//!    行数取决于当帧的 `PanelCtx` 与排版宽度，鼠标侧重算必然点偏。
//! 2. **门禁与 `handle_key` 同源**：确认关闭 / 保存在途 / 正在编辑时，点击一律不
//!    改状态；只读态交给 `begin_edit` 自己写提示，这里不另判一遍。
//! 3. **没有悬停高亮**：终端只开了 1000/1002/1006，不含 any-event(1003)，指针
//!    移动根本不会上报（理由见 `shell.rs` 的 `MOUSE_ON`）。

use ratatui::layout::Rect;

use super::super::palette::THEME_OPTIONS;
use super::super::render::overlay::{content_inner_rect, settings_panel_layout};
use super::super::settings::FieldAction;
use super::*;

/// 一次左键点击命中了什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PanelHit {
    /// 左导航轨第 `idx` 项（顺序即 [`Section::ALL`]）。
    Nav(usize),
    /// 字段行；`on_value` = 落在标签列之后的**值列**上。
    Field { field: FieldId, on_value: bool },
    /// 主题下拉的第 `pick` 个候选（只有主题行处于编辑态时才会画出来）。
    ThemeOption(usize),
}

/// 屏幕坐标 → 面板元素；没命中返回 `None`。
///
/// `frame_rows` 必须是**当前这一帧**的行归属（[`SettingsPanel::frame_rows_for`]），
/// 调用方要先确认排版宽度没变。
pub(super) fn hit(area: Rect, frame_rows: &[RowOwner], column: u16, row: u16) -> Option<PanelHit> {
    let layout = settings_panel_layout(area);
    if let Some(idx) = hit_nav(layout.nav, column, row) {
        return Some(PanelHit::Nav(idx));
    }
    hit_content(content_inner_rect(layout.content), frame_rows, column, row)
}

/// 左导航轨：点第几行就是第几个分区（渲染侧按 `Section::ALL` 顺序画）。
fn hit_nav(nav: Rect, column: u16, row: u16) -> Option<usize> {
    if !inside(nav, column, row) {
        return None;
    }
    let idx = (row - nav.y) as usize;
    // 窄屏（`nav.width == 0`）时导航轨不存在，`inside` 已经不成立；这里再挡一层
    // 「超出可见高度」：渲染侧 `take(rect.height)` 硬裁,没画出来的项不该能点。
    (idx < Section::ALL.len()).then_some(idx)
}

/// 内容区：按行归属判定。
fn hit_content(inner: Rect, frame_rows: &[RowOwner], column: u16, row: u16) -> Option<PanelHit> {
    if !inside(inner, column, row) {
        return None;
    }
    match frame_rows.get((row - inner.y) as usize)? {
        // 分区说明行 / 下拉省略号：看得见，但键盘也没有对应动作。
        RowOwner::Prose => None,
        RowOwner::ThemeOption(pick) => Some(PanelHit::ThemeOption(*pick)),
        RowOwner::Field { field, .. } => Some(PanelHit::Field {
            field: *field,
            on_value: column >= inner.x.saturating_add(LABEL_PAD as u16),
        }),
    }
}

/// 落在矩形内。宽或高为 0（窄屏不画导航轨）的矩形永远不命中。
fn inside(rect: Rect, column: u16, row: u16) -> bool {
    rect.width > 0
        && rect.height > 0
        && column >= rect.x
        && column < rect.x.saturating_add(rect.width)
        && row >= rect.y
        && row < rect.y.saturating_add(rect.height)
}

impl SettingsPanel {
    /// 记下这一帧的行归属；`width` = 本次 `content()` 的排版宽度。
    pub(super) fn remember_frame(&mut self, width: usize, rows: &[RowOwner]) {
        if self.frame_width == width && self.frame_rows == rows {
            return;
        }
        self.frame_width = width;
        self.frame_rows.clear();
        self.frame_rows.extend_from_slice(rows);
    }

    /// 可用于命中的行归属；排版宽度变了就作废（折行数与下拉位置都会变）。
    pub(super) fn frame_rows_for(&self, width: usize) -> Option<&[RowOwner]> {
        (self.frame_width == width && !self.frame_rows.is_empty())
            .then_some(self.frame_rows.as_slice())
    }

    /// 左键点击：落到哪个元素就做哪件事。
    ///
    /// 不返回 `PanelEffect`：底栏不是按钮，点击产生不了保存 / 关闭，留一个恒为
    /// `None` 的返回值只会让调用方多写一个永远走不到的分支。
    ///
    /// 正在编辑时**不换目标**：键盘侧也要先 Enter 提交，鼠标放宽会让一段没提交的
    /// 缓冲被静默丢掉 —— 但也不该一声不吭，所以底栏说一句怎么出去。唯一的例外是
    /// 主题候选：那几行本来就只在编辑态存在。
    pub(super) fn click(&mut self, hit: PanelHit, ctx: &PanelCtx<'_>) {
        if self.confirm_close || self.is_saving() {
            return;
        }
        match hit {
            PanelHit::Nav(idx) => self.select_section(idx),
            PanelHit::ThemeOption(pick) => self.pick_theme(pick),
            PanelHit::Field { .. } if self.editing.is_some() => self.set_note(
                "正在编辑：先 Enter 确定或 Esc 取消".to_string(),
                NoticeLevel::Muted,
            ),
            PanelHit::Field { field, on_value } => {
                self.focus_row(field);
                if on_value {
                    self.begin_edit(ctx);
                }
            }
        }
    }

    /// 滚轮：浏览态与 ↑↓ 同义（一步一行，clamp 在分区边界）；主题下拉展开时与 ↑↓
    /// 同义地移动候选 —— `move_pick` 正是 `pick_step` 用的那一份，同一个手势不该因
    /// 输入设备不同而语义分叉。
    ///
    /// 不做「滚动内容」：内容区没有 viewport 偏移的概念，加一套要同时改
    /// `render_settings_body` 与下拉窗口，而且「滚了没反应」和「到头了」在屏幕上
    /// 长得一样。滚轮移光标则与键盘完全同源。
    pub(super) fn scroll_row(&mut self, up: bool) {
        if self.confirm_close || self.is_saving() {
            return;
        }
        if let Some(Editing::Theme { pick }) = &mut self.editing {
            // 与 `pick_step` 同一个 clamp：到两端停住，不环绕。
            move_pick(pick, up, THEME_OPTIONS.len());
            return;
        }
        // 其它编辑态（文本缓冲 / 枚举循环）里没有「滚」这件事。
        if self.editing.is_some() {
            return;
        }
        self.note = None;
        self.move_row(up);
    }

    /// 点导航轨：绝对定位到第 `idx` 个分区（不环绕，越界忽略）。
    ///
    /// `pub(super)`：单测要能直接摆到某一分区上渲染，不必绕一圈按键。
    pub(super) fn select_section(&mut self, idx: usize) {
        let Some(section) = Section::ALL.get(idx).copied() else {
            return;
        };
        if section == self.section {
            return;
        }
        self.section = section;
        self.row = 0;
        self.note = None;
    }

    /// 行光标移到 `field` 所在行：点哪行就选中哪行。
    fn focus_row(&mut self, field: FieldId) {
        let Some(idx) = self.section.fields().iter().position(|f| *f == field) else {
            return;
        };
        self.row = idx;
        self.note = None;
    }

    /// 点主题下拉的候选：写下选择并立即提交（与「Enter 确定」同义）。
    fn pick_theme(&mut self, pick: usize) {
        if !matches!(self.editing, Some(Editing::Theme { .. })) {
            return;
        }
        let Some(&option) = THEME_OPTIONS.get(pick) else {
            return;
        };
        self.editing = None;
        self.set_staged(
            FieldId::Theme,
            FieldAction::Write(option.map(str::to_string)),
        );
    }
}
