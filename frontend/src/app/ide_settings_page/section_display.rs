//! IDE 设置：显示块。

use leptos::prelude::*;
use leptos_dom::helpers::event_target_value;

use crate::app::app_signals::IdeEditorSignals;
use crate::app::settings_toggle_switch::SettingsToggleSwitch;
use crate::i18n::{self, Locale};

#[component]
pub(super) fn IdeSettingsDisplayBlock(
    locale: RwSignal<Locale>,
    editor: IdeEditorSignals,
) -> impl IntoView {
    let tab_size_id = "ide-settings-tab-size";

    view! {
        <div class="settings-block">
            <h3 class="settings-block-title">{move || i18n::ide_settings_block_display(locale.get())}</h3>
            <SettingsToggleSwitch
                checked=Signal::derive(move || editor.line_numbers.get())
                label=Signal::derive(move || i18n::ide_settings_line_numbers(locale.get()).to_string())
                on_toggle=move || editor.line_numbers.update(|v| *v = !*v)
                test_id="ide-settings-line-numbers"
            />
            <SettingsToggleSwitch
                checked=Signal::derive(move || editor.word_wrap.get())
                label=Signal::derive(move || i18n::ide_settings_word_wrap(locale.get()).to_string())
                on_toggle=move || editor.word_wrap.update(|v| *v = !*v)
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
