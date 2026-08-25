//! IDE「新建文件」模态框（替代浏览器 `prompt`）。

use std::sync::Arc;

use leptos::prelude::*;
use leptos_dom::helpers::event_target_value;

use crate::app::app_signals::IdeChromeSignals;
use crate::app::focusable_menu::FocusableModalPanel;
use crate::app::workspace_panel::make_refresh_workspace_after_mutation;
use crate::app::workspace_panel_state::WorkspacePanelSignals;
use crate::i18n::{self, Locale};
use crate::ide_save::{IdeSaveContext, spawn_create_and_open_file};
use crate::workspace_context_menu::WorkspaceTreeRefreshHint;
use crate::workspace_tree::workspace_parent_rel;

#[derive(Clone, Copy)]
pub struct IdeNewFileModalInput {
    pub locale: RwSignal<Locale>,
    pub chrome: IdeChromeSignals,
    pub save_ctx: IdeSaveContext,
    pub workspace_panel: WorkspacePanelSignals,
}

fn close_new_file_modal(chrome: IdeChromeSignals) {
    chrome.new_file_modal_open.set(false);
    chrome.new_file_path_draft.set(String::new());
    chrome.new_file_error.set(None);
}

/// 新建文件相对路径校验：非空、无空白、无 `..` 越界段、无前导 `/` 或 `\`。
fn new_file_rel_valid(rel: &str) -> bool {
    let t = rel.trim();
    if t.is_empty() || t != rel {
        return false;
    }
    if rel.chars().any(|c| c.is_whitespace()) {
        return false;
    }
    if rel.starts_with('/') || rel.contains('\\') {
        return false;
    }
    if rel.split('/').any(|seg| seg.is_empty() || seg == "..") {
        return false;
    }
    true
}

fn submit_new_file(input: IdeNewFileModalInput) {
    let rel = input
        .chrome
        .new_file_path_draft
        .get_untracked()
        .trim()
        .to_string();
    if !new_file_rel_valid(&rel) {
        input.chrome.new_file_error.set(Some(
            i18n::ide_new_file_invalid_path(input.locale.get_untracked()).to_string(),
        ));
        return;
    }
    input.chrome.new_file_error.set(None);
    close_new_file_modal(input.chrome);
    let parent = workspace_parent_rel(rel.as_str());
    let refresh =
        make_refresh_workspace_after_mutation(input.workspace_panel, input.locale.get_untracked());
    let after_create = Arc::new(move || {
        refresh(WorkspaceTreeRefreshHint {
            parent_rel: parent.clone(),
            deleted_rel: None,
        })
    });
    spawn_create_and_open_file(
        input.save_ctx,
        input.locale,
        rel,
        Some(after_create),
        input.chrome.confirm_signals(),
    );
}

fn on_new_file_path_keydown(ev: web_sys::KeyboardEvent, input: IdeNewFileModalInput) {
    if ev.key() != "Enter" {
        return;
    }
    ev.prevent_default();
    submit_new_file(input);
}

#[component]
fn IdeNewFileModalPanel(input: IdeNewFileModalInput) -> impl IntoView {
    let IdeNewFileModalInput { locale, chrome, .. } = input;
    view! {
        <FocusableModalPanel
            class="modal ide-new-file-modal"
            dialog_role="dialog"
            labelledby="ide-new-file-title"
            on_escape=Callback::new(move |_| close_new_file_modal(chrome))
        >
            <div class="modal-head">
                <span id="ide-new-file-title" class="modal-title">
                    {move || i18n::ide_menu_new_file(locale.get())}
                </span>
            </div>
            <div class="modal-body">
                <label class="settings-field-label" for="ide-new-file-path">
                    {move || i18n::ide_new_file_prompt(locale.get())}
                </label>
                <input
                    id="ide-new-file-path"
                    type="text"
                    class="settings-field-input"
                    data-testid="ide-new-file-path-input"
                    prop:placeholder=move || i18n::ide_new_file_placeholder(locale.get())
                    prop:value=move || chrome.new_file_path_draft.get()
                    on:input=move |ev| {
                        chrome.new_file_path_draft.set(event_target_value(&ev));
                        chrome.new_file_error.set(None);
                    }
                    on:keydown=move |ev: web_sys::KeyboardEvent| {
                        on_new_file_path_keydown(ev, input);
                    }
                />
                <Show when=move || chrome.new_file_error.get().is_some()>
                    <p class="ide-new-file-error" role="alert" data-testid="ide-new-file-error">
                        {move || chrome.new_file_error.get().unwrap_or_default()}
                    </p>
                </Show>
            </div>
            <div class="modal-footer actions">
                <button
                    type="button"
                    class="btn btn-secondary"
                    data-testid="ide-new-file-cancel"
                    on:click=move |_| close_new_file_modal(chrome)
                >
                    {move || i18n::ide_new_file_cancel(locale.get())}
                </button>
                <button
                    type="button"
                    class="btn btn-primary"
                    data-testid="ide-new-file-create"
                    on:click=move |_| submit_new_file(input)
                >
                    {move || i18n::ide_new_file_create(locale.get())}
                </button>
            </div>
        </FocusableModalPanel>
    }
}

#[component]
pub fn IdeNewFileModal(input: IdeNewFileModalInput) -> impl IntoView {
    let chrome = input.chrome;
    view! {
        <Show when=move || chrome.new_file_modal_open.get()>
            <div class="modal-backdrop" data-testid="ide-new-file-modal">
                <IdeNewFileModalPanel input />
            </div>
        </Show>
    }
}
