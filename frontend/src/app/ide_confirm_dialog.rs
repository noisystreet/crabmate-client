//! IDE 未保存更改等确认对话框。

use leptos::prelude::*;

use crate::app::app_signals::IdeChromeSignals;
use crate::app::focusable_menu::FocusableModalPanel;
use crate::i18n::{self, Locale};
use crate::ide_confirm::{IdeConfirmPrompt, resolve_ide_confirm};

fn prompt_message(pending: Option<IdeConfirmPrompt>) -> String {
    pending.map(|p| p.message).unwrap_or_default()
}

fn prompt_cancel_label(pending: Option<IdeConfirmPrompt>, locale: Locale) -> String {
    pending
        .map(|p| p.cancel_label)
        .unwrap_or_else(|| i18n::ide_confirm_cancel(locale).to_string())
}

fn prompt_ok_label(pending: Option<IdeConfirmPrompt>, locale: Locale) -> String {
    pending
        .map(|p| p.ok_label)
        .unwrap_or_else(|| i18n::ide_confirm_ok(locale).to_string())
}

#[component]
fn IdeConfirmDialogPanel(locale: RwSignal<Locale>, chrome: IdeChromeSignals) -> impl IntoView {
    let confirm = chrome.confirm_signals();
    view! {
        <FocusableModalPanel
            class="modal ide-confirm-modal"
            labelledby="ide-confirm-title"
            on_escape=Callback::new(move |_| resolve_ide_confirm(confirm, false))
        >
            <div class="modal-head">
                <span id="ide-confirm-title" class="modal-title">
                    {move || i18n::ide_confirm_title(locale.get())}
                </span>
            </div>
            <div class="modal-body">
                <p class="modal-hint">
                    {move || prompt_message(chrome.confirm_pending.get())}
                </p>
            </div>
            <div class="modal-footer actions">
                <button
                    type="button"
                    class="btn btn-secondary"
                    data-testid="ide-confirm-cancel"
                    on:click=move |_| resolve_ide_confirm(confirm, false)
                >
                    {move || prompt_cancel_label(chrome.confirm_pending.get(), locale.get())}
                </button>
                <button
                    type="button"
                    class="btn btn-primary"
                    data-testid="ide-confirm-ok"
                    on:click=move |_| resolve_ide_confirm(confirm, true)
                >
                    {move || prompt_ok_label(chrome.confirm_pending.get(), locale.get())}
                </button>
            </div>
        </FocusableModalPanel>
    }
}

#[component]
pub fn IdeConfirmDialog(locale: RwSignal<Locale>, chrome: IdeChromeSignals) -> impl IntoView {
    view! {
        <Show when=move || chrome.confirm_pending.get().is_some()>
            <div class="modal-backdrop" data-testid="ide-confirm-dialog">
                <IdeConfirmDialogPanel locale chrome />
            </div>
        </Show>
    }
}
