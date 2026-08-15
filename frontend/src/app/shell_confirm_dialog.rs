//! 壳层通用确认对话框（会话删除等；替代 WebView 中无效的 `window.confirm`）。

use leptos::prelude::*;

use crate::app::app_signals::ModalSignals;
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
        .unwrap_or_else(|| i18n::confirm_delete_ok(locale).to_string())
}

#[component]
fn ShellConfirmDialogPanel(locale: RwSignal<Locale>, modal: ModalSignals) -> impl IntoView {
    let confirm = modal.confirm_signals();
    view! {
        <FocusableModalPanel
            class="modal ide-confirm-modal"
            labelledby="shell-confirm-title"
            on_escape=Callback::new(move |_| resolve_ide_confirm(confirm, false))
        >
            <div class="modal-head">
                <span id="shell-confirm-title" class="modal-title">
                    {move || i18n::ide_confirm_title(locale.get())}
                </span>
            </div>
            <div class="modal-body">
                <p class="modal-hint">
                    {move || prompt_message(modal.confirm_pending.get())}
                </p>
            </div>
            <div class="modal-footer actions">
                <button
                    type="button"
                    class="btn btn-secondary"
                    data-testid="shell-confirm-cancel"
                    on:click=move |_| resolve_ide_confirm(confirm, false)
                >
                    {move || prompt_cancel_label(modal.confirm_pending.get(), locale.get())}
                </button>
                <button
                    type="button"
                    class="btn btn-danger"
                    data-testid="shell-confirm-ok"
                    on:click=move |_| resolve_ide_confirm(confirm, true)
                >
                    {move || prompt_ok_label(modal.confirm_pending.get(), locale.get())}
                </button>
            </div>
        </FocusableModalPanel>
    }
}

#[component]
pub fn ShellConfirmDialog(locale: RwSignal<Locale>, modal: ModalSignals) -> impl IntoView {
    view! {
        <Show when=move || modal.confirm_pending.get().is_some()>
            <div
                class="modal-backdrop shell-confirm-backdrop"
                data-testid="shell-confirm-dialog"
            >
                <ShellConfirmDialogPanel locale modal />
            </div>
        </Show>
    }
}
