//! 「管理会话」模态框中的单行。

use leptos::prelude::*;
use leptos_dom::helpers::event_target_value;

use crate::chat_session_state::ChatSessionSignals;
use crate::i18n::{self, Locale};
use crate::session_ops::{
    delete_session_after_confirm, export_session_json_for_id, export_session_markdown_for_id,
    set_session_pinned, set_session_starred, switch_active_session_after_composer_flush,
};

/// 「管理会话」模态单行所需字段（缩短 [`SessionModalRow`] 形参列表；勿命名为 `*Props`，与 Leptos 组件宏生成类型冲突）。
#[derive(Clone)]
pub struct SessionModalRowBundle {
    pub id: String,
    pub title: String,
    pub message_count: usize,
    pub pinned: bool,
    pub starred: bool,
    pub active: bool,
    pub locale: RwSignal<Locale>,
    pub chat: ChatSessionSignals,
    pub draft: RwSignal<String>,
    /// 「管理会话」弹窗开关。
    pub session_modal: RwSignal<bool>,
    pub apply_assistant_display_filters: RwSignal<bool>,
}

fn session_row_outer_class(active: bool) -> &'static str {
    if active {
        "session-row active"
    } else {
        "session-row"
    }
}

fn session_star_action_title(loc: Locale, starred: bool) -> &'static str {
    if starred {
        i18n::ctx_unstar_session(loc)
    } else {
        i18n::ctx_star_session(loc)
    }
}

fn session_pin_action_title(loc: Locale, pinned: bool) -> &'static str {
    if pinned {
        i18n::ctx_unpin_session(loc)
    } else {
        i18n::ctx_pin_session(loc)
    }
}

fn commit_session_rename(
    sessions: RwSignal<Vec<crate::storage::ChatSession>>,
    session_id: &str,
    editing: RwSignal<bool>,
    rename_draft: RwSignal<String>,
) {
    let t = rename_draft.get_untracked().trim().to_string();
    editing.set(false);
    if t.is_empty() {
        return;
    }
    sessions.update(|list| {
        if let Some(s) = list.iter_mut().find(|s| s.id == session_id) {
            s.title = t;
            s.updated_at = js_sys::Date::now() as i64;
        }
    });
}

#[component]
fn SessionModalRowRenameForm(
    chat: ChatSessionSignals,
    session_id: String,
    locale: RwSignal<Locale>,
    editing: RwSignal<bool>,
    rename_draft: RwSignal<String>,
) -> impl IntoView {
    let id_input = session_id.clone();
    view! {
        <div class="session-row-rename">
            <input
                type="text"
                class="settings-field-input session-row-rename-input"
                data-testid=format!("session-modal-rename-input-{id_input}")
                prop:placeholder=move || i18n::session_prompt_title_label(locale.get())
                prop:value=move || rename_draft.get()
                on:input=move |ev| rename_draft.set(event_target_value(&ev))
                on:keydown={
                    let id = session_id.clone();
                    move |ev: web_sys::KeyboardEvent| {
                        if ev.key() == "Enter" {
                            ev.prevent_default();
                            commit_session_rename(chat.sessions, &id, editing, rename_draft);
                        } else if ev.key() == "Escape" {
                            ev.prevent_default();
                            editing.set(false);
                        }
                    }
                }
            />
            <button
                type="button"
                class="btn btn-primary btn-sm"
                data-testid=format!("session-modal-rename-ok-{}", session_id)
                on:click={
                    let id = session_id.clone();
                    move |_| {
                        commit_session_rename(chat.sessions, &id, editing, rename_draft);
                    }
                }
            >
                {move || i18n::session_rename_confirm(locale.get())}
            </button>
            <button
                type="button"
                class="btn btn-ghost btn-sm"
                on:click=move |_| editing.set(false)
            >
                {move || i18n::ide_confirm_cancel(locale.get())}
            </button>
        </div>
    }
}

#[component]
fn SessionModalRowOpenButton(
    bundle: SessionModalRowBundle,
    editing: RwSignal<bool>,
    rename_draft: RwSignal<String>,
) -> impl IntoView {
    let SessionModalRowBundle {
        id,
        title,
        message_count,
        locale,
        chat,
        draft,
        session_modal,
        active,
        ..
    } = bundle;
    let id_open = id.clone();
    let id_rename = id.clone();
    view! {
        <Show when=move || !editing.get()>
            <button
                type="button"
                class="session-open"
                data-testid=format!("session-modal-open-{id_open}")
                prop:aria-current=move || crate::a11y::aria_current_true_or_false(active)
                on:click={
                    let id = id_open.clone();
                    move |_| {
                        switch_active_session_after_composer_flush(chat, draft, &id, true);
                        session_modal.set(false);
                    }
                }
            >
                <span class="session-title">
                    {let t = title.clone();
                        move || i18n::session_title_for_display(&t, locale.get())}
                </span>
                <span class="session-meta">{move || i18n::session_row_msg_count(locale.get(), message_count)}</span>
            </button>
        </Show>
        <Show when=move || editing.get()>
            <SessionModalRowRenameForm
                chat=chat
                session_id=id_rename.clone()
                locale=locale
                editing
                rename_draft=rename_draft
            />
        </Show>
    }
}

#[component]
fn SessionModalRowActions(
    bundle: SessionModalRowBundle,
    editing: RwSignal<bool>,
    rename_draft: RwSignal<String>,
) -> impl IntoView {
    let SessionModalRowBundle {
        id,
        pinned,
        starred,
        locale,
        chat,
        draft,
        apply_assistant_display_filters,
        ..
    } = bundle;
    let id_rename = id.clone();
    let id_star = id.clone();
    let id_pin = id.clone();
    let id_json = id.clone();
    let id_md = id.clone();
    let id_del = id.clone();
    view! {
        <div class="session-row-actions">
            <button
                type="button"
                class="btn btn-ghost btn-sm"
                data-testid=format!("session-modal-star-{id}")
                prop:title=move || session_star_action_title(locale.get(), starred)
                prop:aria-pressed=starred
                on:click={
                    let sessions = chat.sessions;
                    let id = id_star.clone();
                    move |_| set_session_starred(sessions, &id, !starred)
                }
            >
                {if starred { "★" } else { "☆" }}
            </button>
            <button
                type="button"
                class="btn btn-ghost btn-sm"
                data-testid=format!("session-modal-pin-{id}")
                prop:title=move || session_pin_action_title(locale.get(), pinned)
                prop:aria-pressed=pinned
                on:click={
                    let sessions = chat.sessions;
                    let id = id_pin.clone();
                    move |_| set_session_pinned(sessions, &id, !pinned)
                }
            >
                "📌"
            </button>
            <button
                type="button"
                class="btn btn-ghost btn-sm"
                prop:title=move || i18n::session_row_rename_title_attr(locale.get())
                on:click={
                    let sessions = chat.sessions;
                    let id = id_rename.clone();
                    move |_| {
                        let cur = sessions
                            .with(|list| {
                                list.iter()
                                    .find(|s| s.id == id)
                                    .map(|s| s.title.clone())
                                    .unwrap_or_default()
                            });
                        rename_draft.set(cur);
                        editing.set(true);
                    }
                }
            >
                {move || i18n::session_row_rename_button(locale.get())}
            </button>
            <button
                type="button"
                class="btn btn-secondary btn-sm"
                prop:title=move || i18n::session_row_export_json_title(locale.get())
                on:click={
                    let sessions = chat.sessions;
                    let id = id_json.clone();
                    move |_| {
                        export_session_json_for_id(
                            sessions,
                            &id,
                            locale.get_untracked(),
                            apply_assistant_display_filters.get_untracked(),
                        )
                    }
                }
            >
                "JSON"
            </button>
            <button
                type="button"
                class="btn btn-secondary btn-sm"
                prop:title=move || i18n::session_row_export_md_title(locale.get())
                on:click={
                    let sessions = chat.sessions;
                    let id = id_md.clone();
                    move |_| {
                        export_session_markdown_for_id(
                            sessions,
                            &id,
                            locale.get_untracked(),
                            apply_assistant_display_filters.get_untracked(),
                        )
                    }
                }
            >
                "MD"
            </button>
            <button
                type="button"
                class="btn btn-danger btn-sm"
                data-testid=format!("session-modal-delete-{id}")
                prop:title=move || i18n::session_row_delete_title(locale.get())
                on:click={
                    let sessions = chat.sessions;
                    let active_id = chat.active_id;
                    let draft = draft;
                    let session_sync = chat.session_sync;
                    let id = id_del.clone();
                    move |_| {
                        delete_session_after_confirm(
                            sessions,
                            active_id,
                            draft,
                            session_sync,
                            chat.stream_transport,
                            &id,
                            locale.get_untracked(),
                        );
                    }
                }
            >
                {move || i18n::session_row_delete_button(locale.get())}
            </button>
        </div>
    }
}

#[component]
pub fn SessionModalRow(row: SessionModalRowBundle) -> impl IntoView {
    let row_open = row.clone();
    let row_actions = row.clone();
    let editing = RwSignal::new(false);
    let rename_draft = RwSignal::new(String::new());
    view! {
        <div class=session_row_outer_class(row.active)>
            <SessionModalRowOpenButton bundle=row_open editing rename_draft=rename_draft />
            <SessionModalRowActions bundle=row_actions editing rename_draft=rename_draft />
        </div>
    }
}
