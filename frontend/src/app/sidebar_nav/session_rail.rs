use leptos::prelude::*;

use crate::a11y::{aria_current_true_or_false, context_menu_keydown_anchor};
use crate::chat_session_state::ChatSessionSignals;
use crate::i18n;
use crate::session_ops::{SessionContextAnchor, switch_active_session_after_composer_flush};
use crate::session_search::{
    MESSAGE_SEARCH_MAX_HITS, MessageSearchHit, collect_message_search_hits, normalize_search_query,
    session_title_matches,
};
use crate::session_sort::sorted_sessions_clone;
use crate::storage::ChatSession;

/// 侧栏会话列表滚动区内共享信号（缩短 [`nav_rail_session_scroll_inner`] 形参列表）。
#[derive(Clone, Copy)]
pub(super) struct NavRailSessionScrollSignals {
    pub(super) locale: RwSignal<crate::i18n::Locale>,
    pub(super) sidebar_search_panel_open: RwSignal<bool>,
    pub(super) sidebar_filter_debounced: RwSignal<String>,
    pub(super) global_message_filter_debounced: RwSignal<String>,
    pub(super) chat: ChatSessionSignals,
    pub(super) draft: RwSignal<String>,
    pub(super) mobile_nav_open: RwSignal<bool>,
    pub(super) session_context_menu: RwSignal<Option<SessionContextAnchor>>,
    pub(super) sidebar_rail_ctx_menu: RwSignal<Option<(f64, f64)>>,
    pub(super) focus_message_id_after_nav: RwSignal<Option<String>>,
    pub(super) apply_assistant_display_filters: RwSignal<bool>,
}

/// 侧栏搜索命中与会话行按钮共享的导航信号（缩短 [`nav_search_hit_button`] / [`nav_session_row_button`] 形参列表）。
#[derive(Clone, Copy)]
pub(super) struct NavRailHitRowNavSignals {
    chat: ChatSessionSignals,
    draft: RwSignal<String>,
    session_context_menu: RwSignal<Option<SessionContextAnchor>>,
    sidebar_rail_ctx_menu: RwSignal<Option<(f64, f64)>>,
    mobile_nav_open: RwSignal<bool>,
    locale: RwSignal<crate::i18n::Locale>,
    focus_message_id_after_nav: RwSignal<Option<String>>,
}

pub(super) fn nav_rail_hit_row_nav_signals_from_scroll(
    s: &NavRailSessionScrollSignals,
) -> NavRailHitRowNavSignals {
    NavRailHitRowNavSignals {
        chat: s.chat,
        draft: s.draft,
        session_context_menu: s.session_context_menu,
        sidebar_rail_ctx_menu: s.sidebar_rail_ctx_menu,
        mobile_nav_open: s.mobile_nav_open,
        locale: s.locale,
        focus_message_id_after_nav: s.focus_message_id_after_nav,
    }
}

pub(super) fn nav_rail_session_scroll_inner(s: NavRailSessionScrollSignals) -> impl IntoView {
    let hit_row_nav = nav_rail_hit_row_nav_signals_from_scroll(&s);
    let NavRailSessionScrollSignals {
        locale,
        sidebar_search_panel_open,
        sidebar_filter_debounced,
        global_message_filter_debounced,
        chat,
        apply_assistant_display_filters,
        ..
    } = s;
    let sessions = chat.sessions;
    move || {
        let search_ui_open = sidebar_search_panel_open.get();
        let needle = if search_ui_open {
            normalize_search_query(&sidebar_filter_debounced.get())
        } else {
            String::new()
        };
        let msg_needle = if search_ui_open {
            normalize_search_query(&global_message_filter_debounced.get())
        } else {
            String::new()
        };
        let v: Vec<ChatSession> = sorted_sessions_clone(&sessions.get())
            .into_iter()
            .filter(|s| session_title_matches(s, &needle))
            .collect();
        let hits = if msg_needle.is_empty() {
            Vec::new()
        } else {
            let overlay = chat.stream_text_overlay.get();
            sessions.with(|list| {
                collect_message_search_hits(
                    list,
                    &msg_needle,
                    MESSAGE_SEARCH_MAX_HITS,
                    locale.get(),
                    apply_assistant_display_filters.get(),
                    overlay.as_ref(),
                )
            })
        };
        let hit_views = if !msg_needle.is_empty() {
            if hits.is_empty() {
                view! {
                    <div class="nav-search-hits-empty" role="status">
                        {move || i18n::nav_no_message_hits(locale.get())}
                    </div>
                }
                .into_any()
            } else {
                hits.into_iter()
                    .map(|h| nav_search_hit_button(h, hit_row_nav.clone()))
                    .collect_view()
                    .into_any()
            }
        } else {
            ().into_any()
        };
        view! {
            <div class="nav-search-hits" role="region" prop:aria-label=move || i18n::nav_search_hits_region(locale.get())>
                {hit_views}
            </div>
            {v.into_iter()
                .map(|sess| {
                    nav_session_row_button(sess, hit_row_nav.clone())
                })
                .collect_view()}
        }
        .into_any()
    }
}

fn session_row_accessible_label(
    stored_title: &str,
    pinned: bool,
    starred: bool,
    loc: crate::i18n::Locale,
) -> String {
    let mut parts = vec![i18n::session_title_for_display(stored_title, loc)];
    if pinned {
        parts.push(i18n::session_badge_pin_aria(loc).to_string());
    }
    if starred {
        parts.push(i18n::session_badge_star_aria(loc).to_string());
    }
    parts.join(", ")
}

fn nav_search_hit_button(h: MessageSearchHit, nav: NavRailHitRowNavSignals) -> impl IntoView {
    let NavRailHitRowNavSignals {
        chat,
        draft,
        session_context_menu,
        sidebar_rail_ctx_menu,
        focus_message_id_after_nav,
        mobile_nav_open,
        locale,
    } = nav;
    let sid = h.session_id.clone();
    let mid = h.message_id.clone();
    let title = h.session_title.clone();
    let snip = h.snippet.clone();
    view! {
        <button
            type="button"
            class="nav-search-hit"
            on:click=move |_| {
                session_context_menu.set(None);
                sidebar_rail_ctx_menu.set(None);
                switch_active_session_after_composer_flush(chat, draft, &sid, true);
                focus_message_id_after_nav.set(Some(mid.clone()));
                mobile_nav_open.set(false);
            }
        >
            <span class="nav-search-hit-title">
                {move || {
                    i18n::session_title_for_display(&title, locale.get())
                }}
            </span>
            <span class="nav-search-hit-snippet">{snip}</span>
        </button>
    }
}

fn session_row_is_streaming(chat: ChatSessionSignals, session_id: &str) -> bool {
    chat.stream_transport.get().bound_session_id() == Some(session_id)
}

fn on_nav_session_row_context_keydown(
    ev: web_sys::KeyboardEvent,
    session_id: String,
    session_context_menu: RwSignal<Option<SessionContextAnchor>>,
    sidebar_rail_ctx_menu: RwSignal<Option<(f64, f64)>>,
) {
    use super::session_row_press::open_session_context_menu;
    let Some((x, y)) = context_menu_keydown_anchor(&ev) else {
        return;
    };
    open_session_context_menu(
        session_id,
        x,
        y,
        session_context_menu,
        sidebar_rail_ctx_menu,
    );
}

fn nav_session_row_inner(
    title: String,
    n: usize,
    is_pinned: bool,
    is_starred: bool,
    locale: RwSignal<crate::i18n::Locale>,
    chat: ChatSessionSignals,
    session_id_streaming: String,
) -> impl IntoView {
    view! {
        <span class="nav-session-title-row">
            <span class="nav-session-badges">
                <Show when=move || is_pinned>
                    <span
                        class="nav-session-badge nav-session-badge-pin"
                        aria-hidden="true"
                        prop:title=move || i18n::session_badge_pin_aria(locale.get())
                    >
                        "📌"
                    </span>
                </Show>
                <Show when=move || is_starred>
                    <span
                        class="nav-session-badge nav-session-badge-star"
                        aria-hidden="true"
                        prop:title=move || i18n::session_badge_star_aria(locale.get())
                    >
                        "★"
                    </span>
                </Show>
            </span>
            <span class="nav-session-title">
                {move || i18n::session_title_for_display(&title, locale.get())}
            </span>
        </span>
        <span class="nav-session-meta">
            <Show when=move || session_row_is_streaming(chat, session_id_streaming.as_str())>
                <span
                    class="nav-session-streaming-badge"
                    data-testid="nav-session-streaming"
                    role="status"
                >
                    {move || i18n::session_row_streaming_label(locale.get())}
                </span>
            </Show>
            <span class="nav-session-count">
                {move || i18n::session_row_msg_count(locale.get(), n)}
            </span>
        </span>
    }
}

fn nav_session_row_button(s: ChatSession, nav: NavRailHitRowNavSignals) -> impl IntoView {
    use std::rc::Rc;

    use super::session_row_press::{build_session_row_press_handlers, session_row_item_class};

    let NavRailHitRowNavSignals {
        chat,
        draft,
        session_context_menu,
        sidebar_rail_ctx_menu,
        mobile_nav_open,
        locale,
        ..
    } = nav;
    let active_id = chat.active_id;
    let session_id_class = s.id.clone();
    let session_id_testid = s.id.clone();
    let session_id_click = s.id.clone();
    // 供「生成中」badge 的独立 move 闭包使用，避免与 class 闭包争抢 `session_id_class` 所有权。
    let session_id_streaming = session_id_class.clone();
    let session_id_keydown = s.id.clone();
    let session_id_aria = s.id.clone();
    let title = s.title.clone();
    let title_aria = title.clone();
    let n = s.messages.len();
    let is_pinned = s.pinned;
    let is_starred = s.starred;

    let press =
        build_session_row_press_handlers(s.id.clone(), session_context_menu, sidebar_rail_ctx_menu);
    let on_contextmenu = Rc::clone(&press.on_contextmenu);
    let on_pointerdown = Rc::clone(&press.on_pointerdown);
    let on_pointermove = Rc::clone(&press.on_pointermove);
    let on_pointer_end = Rc::clone(&press.on_pointer_end);
    let on_pointer_end_cancel = Rc::clone(&press.on_pointer_end);
    let on_pointer_end_leave = Rc::clone(&press.on_pointer_end);
    let try_consume_suppress_click = Rc::clone(&press.try_consume_suppress_click);

    view! {
        <button
            type="button"
            data-testid=format!("nav-session-{session_id_testid}")
            class=move || {
                session_row_item_class(
                    active_id.get() == session_id_class,
                    is_pinned,
                    is_starred,
                    session_row_is_streaming(chat, session_id_class.as_str()),
                )
            }
            prop:aria-current=move || {
                aria_current_true_or_false(active_id.get() == session_id_aria)
            }
            prop:aria-label=move || {
                session_row_accessible_label(
                    &title_aria,
                    is_pinned,
                    is_starred,
                    locale.get(),
                )
            }
            on:contextmenu=move |ev| on_contextmenu(ev)
            on:keydown=move |ev: web_sys::KeyboardEvent| {
                on_nav_session_row_context_keydown(
                    ev,
                    session_id_keydown.clone(),
                    session_context_menu,
                    sidebar_rail_ctx_menu,
                );
            }
            on:pointerdown=move |ev| on_pointerdown(ev)
            on:pointermove=move |ev| on_pointermove(ev)
            on:pointerup=move |_| on_pointer_end()
            on:pointercancel=move |_| on_pointer_end_cancel()
            on:pointerleave=move |_| on_pointer_end_leave()
            on:click=move |_| {
                if try_consume_suppress_click() {
                    return;
                }
                session_context_menu.set(None);
                sidebar_rail_ctx_menu.set(None);
                switch_active_session_after_composer_flush(chat, draft, &session_id_click, true);
                mobile_nav_open.set(false);
            }
        >
            {nav_session_row_inner(
                title,
                n,
                is_pinned,
                is_starred,
                locale,
                chat,
                session_id_streaming,
            )}
        </button>
    }
}

#[cfg(test)]
mod session_row_accessible_label_tests {
    use super::session_row_accessible_label;
    use crate::i18n::Locale;

    #[test]
    fn title_only_when_no_badges() {
        assert_eq!(
            session_row_accessible_label("Hello", false, false, Locale::En),
            "Hello"
        );
    }

    #[test]
    fn appends_pinned_and_starred_in_english() {
        let s = session_row_accessible_label("Hello", true, true, Locale::En);
        assert!(s.contains("Hello"));
        assert!(s.contains("Pinned"));
        assert!(s.contains("Starred"));
    }
}
