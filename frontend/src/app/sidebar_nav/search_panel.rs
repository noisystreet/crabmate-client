use leptos::prelude::*;
use leptos_dom::helpers::event_target_value;

use crate::i18n::{self, Locale};

/// 会话 / 全局消息搜索共用输入行（label + placeholder + value 绑定）。
#[component]
fn NavRailSearchInput(
    locale: RwSignal<Locale>,
    value: RwSignal<String>,
    input_id: &'static str,
    input_class: &'static str,
    label_text: fn(Locale) -> &'static str,
    placeholder_text: fn(Locale) -> &'static str,
) -> impl IntoView {
    view! {
        <>
            <label class="nav-rail-search-label" for=input_id>
                {move || label_text(locale.get())}
            </label>
            <input
                id=input_id
                type="search"
                class=input_class
                prop:placeholder=move || placeholder_text(locale.get())
                prop:value=move || value.get()
                on:input=move |ev| {
                    value.set(event_target_value(&ev));
                }
            />
        </>
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn nav_rail_search_panel(
    locale: RwSignal<Locale>,
    sidebar_search_panel_open: RwSignal<bool>,
    sidebar_session_query: RwSignal<String>,
    global_message_query: RwSignal<String>,
) -> impl IntoView {
    view! {
        <Show when=move || sidebar_search_panel_open.get()>
            <div class="nav-rail-search" id="nav-rail-search-panel">
                <div class="nav-rail-search-header">
                    <button
                        type="button"
                        class="btn btn-nav-ghost-ds nav-rail-search-hide"
                        prop:aria-label=move || i18n::nav_hide_search_panel_aria(locale.get())
                        on:click=move |_| sidebar_search_panel_open.set(false)
                    >
                        {move || i18n::nav_hide_search_panel(locale.get())}
                    </button>
                </div>
                <NavRailSearchInput
                    locale=locale
                    value=sidebar_session_query
                    input_id="nav-session-filter"
                    input_class="nav-session-search-input"
                    label_text=i18n::nav_filter_sessions
                    placeholder_text=i18n::nav_ph_filter
                />
                <NavRailSearchInput
                    locale=locale
                    value=global_message_query
                    input_id="nav-msg-search"
                    input_class="nav-global-search-input"
                    label_text=i18n::nav_search_messages
                    placeholder_text=i18n::nav_ph_global_search
                />
            </div>
        </Show>
    }
}
