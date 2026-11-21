//! **窗口级**状态（#561）：一个 WebView 窗口共享的那组信号。
//!
//! 判据只有一条：**切到另一条 lane 时，这个值该不该变？** 不该变 → 留在这里；该变 →
//! 放进 [`super::LaneSignals`]。
//!
//! 这一层在 TUI 里没有对应物 —— TUI 是单终端，主题 / 语言 / 列宽都在 `LaneApp.st` 里
//! 跟着 lane 走。Web 侧必须多这一层：窗口只有一个，切 lane 时布局与偏好不该跳。
//!
//! **为什么这个结构存在**：它把「哪些属于窗口」写进类型，而不是留在 `AppSignals` 的
//! 平铺字段里靠注释约定。多 lane 时 [`super::AppSignals::new_lane`] 拿它做模板，复制
//! 出的每一份 lane 共享同一批 signal 句柄（`RwSignal` 是 Copy，`ResizeSignals` 用 `Rc`
//! 克隆），于是偏好天然共享、连接级状态天然隔离。
//!
//! **有意偏离 TUI 的一处**：列宽（`resize` / `shell_ui.side_width`）与设置面板留在窗口级。
//! TUI 把它们放在 `LaneApp` 里，但那边切 lane 只是换一屏文字；Web 切 lane 若连布局宽度都
//! 换，视觉上会「跳」，而宽度本来就只跟窗口大小有关。
//!
//! # 加字段时注意
//!
//! 窗口级字段在**两处**各声明一遍：这里，以及 [`super::AppSignals`] 的平铺字段。
//! 加第 7 个窗口级字段时两处都要改 —— 只改这里的话 [`super::AppSignals::new_lane`] 不会
//! 复制它，多 lane 时那条字段就不再共享，而且**不会报编译错误**。

use super::{
    IdeChromeSignals, IdeEditorSignals, ModalSignals, ResizeSignals, ShellUISignals, SidebarSignals,
};

#[derive(Clone)]
pub struct WindowSignals {
    pub shell_ui: ShellUISignals,
    pub sidebar: SidebarSignals,
    pub modal: ModalSignals,
    pub ide_editor: IdeEditorSignals,
    pub ide_chrome: IdeChromeSignals,
    pub resize: ResizeSignals,
}

impl WindowSignals {
    pub fn new() -> Self {
        Self {
            shell_ui: ShellUISignals::new(),
            sidebar: SidebarSignals::new(),
            modal: ModalSignals::new(),
            ide_editor: IdeEditorSignals::new(),
            ide_chrome: IdeChromeSignals::new(),
            resize: ResizeSignals::new(),
        }
    }
}

impl Default for WindowSignals {
    fn default() -> Self {
        Self::new()
    }
}
