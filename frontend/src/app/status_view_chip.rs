//! 状态栈「主区视图」chip：显示 `chat` / `file`，点一下切换。
//!
//! 与 TUI 状态栈的 `view` 段是**同一个开关**：
//!
//! - 值 `chat` / `file` 两端完全一致（[`view_value`] 与 TUI `status::view_text` 同词），
//!   跨端对照时不用翻译；
//! - 位置也一致 —— 都在状态栈里、都排在权限段之后。TUI 那条「窄屏最先让位」的优先级顺序
//!   在横向 chip 里没有对应物，这里不复制裁剪逻辑（照抄只会得到永远不触发的死分支）。
//!
//! 只本地化标签：TUI 的 label 要凑 5 列定宽，Web 侧栏是横向 chip，排版惯例不同。

use leptos::prelude::*;

use crate::i18n;

use super::ide_layout_switch::toggle_editor_layout;
use super::shell_runtime_context::expect_chat_shell_ctx;

/// chip 的值：`chat` / `file`（与 TUI `status::view_text` 同词同序）。
#[must_use]
fn view_value(editor_open: bool) -> &'static str {
    if editor_open { "file" } else { "chat" }
}

#[component]
pub fn StatusViewChip() -> impl IntoView {
    let ctx = expect_chat_shell_ctx();
    let locale = ctx.locale;
    let toggle = ctx.layout_toggle;
    let editor_open = toggle.editor_layout_mode;
    view! {
        <button
            type="button"
            class="status-chip status-chip-view"
            class:status-chip-view--active=move || editor_open.get()
            data-testid="status-view-chip"
            prop:title=move || i18n::ide_layout_toggle_label(locale.get(), editor_open.get())
            prop:aria-label=move || i18n::ide_layout_toggle_aria(locale.get(), editor_open.get())
            on:click=move |_| toggle_editor_layout(toggle)
        >
            <span class="status-chip-label">{move || i18n::status_chip_view(locale.get())}</span>
            <span class="status-chip-value">{move || view_value(editor_open.get())}</span>
        </button>
    }
}

#[cfg(test)]
mod tests {
    use super::view_value;

    /// 值必须与 TUI `status::view_text` 逐字一致：两端对照就靠这两个短词。
    #[test]
    fn value_matches_the_tui_words() {
        assert_eq!(view_value(false), "chat");
        assert_eq!(view_value(true), "file");
    }
}
