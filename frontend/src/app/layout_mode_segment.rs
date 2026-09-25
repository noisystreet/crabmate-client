//! 对话 / 编辑器主区布局切换（统一壳顶栏最左侧单键切换）。

use leptos::prelude::*;

use crate::app::ide_layout_switch::{IdeLayoutToggleSignals, toggle_editor_layout};
use crate::i18n::{self, Locale};
use crate::icon::Icon;

/// 目标为编辑器布局时显示的代码括号图标。
fn layout_mode_editor_icon() -> AnyView {
    view! {
        <Icon class="layout-mode-toggle-icon">
            <polyline points="16 18 22 12 16 6" />
            <polyline points="8 6 2 12 8 18" />
        </Icon>
    }
    .into_any()
}

/// 目标为对话布局时显示的消息气泡图标。
fn layout_mode_chat_icon() -> AnyView {
    view! {
        <Icon class="layout-mode-toggle-icon">
            <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />
        </Icon>
    }
    .into_any()
}

#[component]
pub fn LayoutModeSegment(
    locale: RwSignal<Locale>,
    layout_toggle: IdeLayoutToggleSignals,
    #[prop(default = "")] extra_class: &'static str,
) -> impl IntoView {
    view! {
        <button
            type="button"
            class=format!("layout-mode-toggle-btn {extra_class}")
            data-testid="layout-mode-toggle"
            prop:aria-controls="layout-mode-panel-main"
            prop:aria-label=move || {
                i18n::ide_layout_toggle_aria(locale.get(), layout_toggle.editor_layout_mode.get())
            }
            prop:title=move || {
                i18n::ide_layout_toggle_label(locale.get(), layout_toggle.editor_layout_mode.get())
            }
            on:click=move |_| toggle_editor_layout(layout_toggle)
        >
            {move || {
                if layout_toggle.editor_layout_mode.get() {
                    layout_mode_chat_icon()
                } else {
                    layout_mode_editor_icon()
                }
            }}
        </button>
    }
}
