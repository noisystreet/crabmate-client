//! IDE 设置：显示块。

use leptos::prelude::*;
use leptos_dom::helpers::event_target_value;

use crate::app::app_signals::IdeEditorSignals;
use crate::app::settings_toggle_switch::SettingsToggleSwitch;
use crate::i18n::{self, Locale};

/// 单个布尔编辑偏好开关（避免组件内重复闭包抬高 CCN）。
#[component]
fn IdeEditorBoolToggle(
    checked: RwSignal<bool>,
    label: Signal<String>,
    test_id: &'static str,
) -> impl IntoView {
    view! {
        <SettingsToggleSwitch
            checked=Signal::derive(move || checked.get())
            label
            on_toggle=move || checked.update(|v| *v = !*v)
            test_id=test_id
        />
    }
}

#[component]
pub(super) fn IdeSettingsDisplayBlock(
    locale: RwSignal<Locale>,
    editor: IdeEditorSignals,
) -> impl IntoView {
    let tab_size_id = "ide-settings-tab-size";

    view! {
        <div class="settings-block">
            <h3 class="settings-block-title">{move || i18n::ide_settings_block_display(locale.get())}</h3>
            <IdeEditorBoolToggle
                checked=editor.line_numbers
                label=Signal::derive(move || i18n::ide_settings_line_numbers(locale.get()).to_string())
                test_id="ide-settings-line-numbers"
            />
            <IdeEditorBoolToggle
                checked=editor.word_wrap
                label=Signal::derive(move || i18n::ide_settings_word_wrap(locale.get()).to_string())
                test_id="ide-settings-word-wrap"
            />
            <div class="settings-field">
                <label class="settings-field-label" for=tab_size_id>
                    {move || i18n::ide_settings_label_tab_size(locale.get())}
                </label>
                <input
                    id=tab_size_id
                    type="number"
                    class="settings-text-input"
                    min="2"
                    max="8"
                    step="1"
                    prop:value=move || i32::from(editor.tab_size.get())
                    on:input=move |ev| {
                        if let Ok(n) = event_target_value(&ev).parse::<i32>() {
                            editor.tab_size.set(n.clamp(2, 8) as u8);
                        }
                    }
                />
            </div>
        </div>
    }
}
