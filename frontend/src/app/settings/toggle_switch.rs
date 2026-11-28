//! 设置页通用开关（`role="switch"`，样式见 `.settings-model-toggle`）。

use leptos::prelude::*;

#[component]
pub(crate) fn SettingsToggleSwitch(
    checked: Signal<bool>,
    label: Signal<String>,
    on_toggle: impl Fn() + Send + Sync + 'static,
    #[prop(optional)] test_id: Option<&'static str>,
    #[prop(optional)] disabled: Option<Signal<bool>>,
) -> impl IntoView {
    let is_disabled = Signal::derive(move || disabled.is_some_and(|d| d.get()));
    view! {
        <div class="settings-toggle-row">
            <span class="settings-toggle-label">{move || label.get()}</span>
            <button
                type="button"
                class="settings-model-toggle"
                role="switch"
                class:settings-model-toggle-on=move || checked.get()
                prop:disabled=move || is_disabled.get()
                prop:aria-checked=move || checked.get()
                prop:aria-label=move || label.get()
                data-testid=test_id
                on:click=move |_| on_toggle()
            >
                <span class="settings-model-toggle-track" aria-hidden="true">
                    <span class="settings-model-toggle-thumb"></span>
                </span>
            </button>
        </div>
    }
}
