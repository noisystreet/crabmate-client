//! 中部聊天列：终端流消息区、输入框、查找入口。

use std::sync::Arc;

use leptos::prelude::{StoredValue, *};
use leptos::task::spawn_local;
use leptos_dom::helpers::event_target_value;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::Closure;

use super::column_keyboard::ChatColumnHomeEndNav;
use super::composer_file_drop::{
    ComposerDropHighlight, composer_accept_drag_over, handle_composer_file_drop,
    handle_composer_image_paste,
};
use super::composer_follow_up::ComposerStreamFollowUp;
use super::composer_input_stack::ComposerInputStack;
use super::composer_pending_images::ComposerPendingImagesRow;
use super::handles::{
    ChatColumnShell, ChatComposerPaneSignals, ChatFindOverlaySignals, ChatMessagesPaneSignals,
};
use super::scroll_follow::on_content_resize_if_pinned;
use super::scroll_shell::{
    ChatScrollShellSignals, STICK_UNPIN_GAP_PX, on_messages_pointer_scroll_intent,
    on_messages_stick_scroll_event, on_messages_wheel_follow_intent, scroll_gap_px,
    stick_content_root,
};
use super::tui_actions_bar::TuiTurnActionHandlers;
use super::tui_stream_view::ChatTuiStreamView;
use super::user_message_edit::UserMessageEdit;
use crate::api::upload_files_multipart;
use crate::app::status_session_mode_seg::{SessionModeSegProps, StatusSessionModeSeg};
use crate::chat_session_state::ChatSessionSignals;
use crate::i18n;

type ScrollSentinelCallback =
    Closure<dyn Fn(Vec<wasm_bindgen::JsValue>, web_sys::IntersectionObserver)>;
type ScrollResizeCallback = Closure<dyn Fn(Vec<wasm_bindgen::JsValue>, web_sys::ResizeObserver)>;

struct ScrollSentinelObserver {
    _callback: ScrollSentinelCallback,
    observer: web_sys::IntersectionObserver,
}

impl Drop for ScrollSentinelObserver {
    fn drop(&mut self) {
        self.observer.disconnect();
    }
}

struct ScrollContentResizeObserver {
    _callback: ScrollResizeCallback,
    observer: web_sys::ResizeObserver,
}

impl Drop for ScrollContentResizeObserver {
    fn drop(&mut self) {
        self.observer.disconnect();
    }
}

#[component]
fn ChatMessagesScrollShell(
    scroll_shell: ChatScrollShellSignals,
    children: Children,
) -> impl IntoView {
    let sentinel_ref = NodeRef::<leptos::html::Div>::new();
    // 本地 signal 随组件 Owner 销毁；Drop 同时断开 Observer，避免回调失活或泄漏。
    let observer_handle = RwSignal::new_local(None::<ScrollSentinelObserver>);
    let resize_observer_handle = RwSignal::new_local(None::<ScrollContentResizeObserver>);
    let auto_scroll = scroll_shell.auto_scroll_chat;
    let pointer_scroll_active = scroll_shell.pointer_scroll_active;
    let last_scroll_top = RwSignal::new(0_i32);
    scroll_shell.messages_scroller.on_load(move |root| {
        let resize_callback = Closure::new(
            move |_entries: Vec<wasm_bindgen::JsValue>, _observer: web_sys::ResizeObserver| {
                on_content_resize_if_pinned(scroll_shell);
            },
        );
        let Some(content) = stick_content_root(root.as_ref()) else {
            return;
        };
        let resize_observer =
            web_sys::ResizeObserver::new(resize_callback.as_ref().unchecked_ref())
                .expect("ResizeObserver");
        resize_observer.observe(&content);
        resize_observer_handle.set(Some(ScrollContentResizeObserver {
            _callback: resize_callback,
            observer: resize_observer,
        }));
    });
    // 哨兵仅 re-pin：流式增高时 scroll gap 可能短暂超过 NEAR，IO 补齐「滚回底部」恢复跟随。
    // root 用 sentinel.parent（即 `.messages`），避免 NodeRef 尚未就绪时落到 viewport 误 pin。
    // 指针拖拽期间不 pin，避免与离底 unpin 竞态。
    sentinel_ref.on_load(move |el| {
        let ac = auto_scroll;
        let pointer = pointer_scroll_active;
        let cb = Closure::new(
            move |entries: Vec<wasm_bindgen::JsValue>, _observer: web_sys::IntersectionObserver| {
                if pointer.get_untracked() {
                    return;
                }
                if let Some(entry) = entries.first() {
                    if let Ok(entry) = entry
                        .clone()
                        .dyn_into::<web_sys::IntersectionObserverEntry>()
                        && entry.is_intersecting()
                    {
                        // 双保险：哨兵可见且 scroller 近底才 pin，避免误拉回。
                        if let Some(root) = entry
                            .target()
                            .parent_element()
                            .and_then(|p| p.dyn_into::<web_sys::HtmlElement>().ok())
                        {
                            let gap = scroll_gap_px(
                                root.scroll_height(),
                                root.scroll_top(),
                                root.client_height(),
                            );
                            if gap > STICK_UNPIN_GAP_PX {
                                return;
                            }
                        }
                        ac.set(true);
                    }
                }
            },
        );
        let options = web_sys::IntersectionObserverInit::new();
        if let Some(root) = el.parent_element() {
            options.set_root(Some(root.as_ref()));
        }
        let observer =
            web_sys::IntersectionObserver::new_with_options(cb.as_ref().unchecked_ref(), &options)
                .expect("IntersectionObserver");
        observer.observe(&el);
        observer_handle.set(Some(ScrollSentinelObserver {
            _callback: cb,
            observer,
        }));
    });
    view! {
        <div
            class="messages"
            data-testid="chat-messages-scroller"
            node_ref=scroll_shell.messages_scroller
            on:wheel=move |ev: web_sys::WheelEvent| {
                on_messages_wheel_follow_intent(scroll_shell, ev);
            }
            on:pointerdown=move |_| {
                on_messages_pointer_scroll_intent(scroll_shell.pointer_scroll_active, true);
            }
            on:pointerup=move |_| {
                on_messages_pointer_scroll_intent(scroll_shell.pointer_scroll_active, false);
            }
            on:pointercancel=move |_| {
                on_messages_pointer_scroll_intent(scroll_shell.pointer_scroll_active, false);
            }
            on:scroll=move |_ev: web_sys::Event| {
                on_messages_stick_scroll_event(scroll_shell, last_scroll_top);
            }
        >
            <div class="chat-thread">{children()}</div>
            <div data-testid="scroll-sentinel" node_ref=sentinel_ref style="height:1px" />
        </div>
    }
}

#[component]
fn ChatMessagesPane(signals: ChatMessagesPaneSignals) -> impl IntoView {
    let ChatMessagesPaneSignals {
        scroll_shell,
        chat,
        locale,
        apply_assistant_display_filters,
        markdown_render,
        stream_follow_up,
        stream_turn_busy_ui,
        status_err,
        chat_find_query,
        chat_find_match_ids,
        chat_find_cursor,
        chat_find_panel_open,
    } = signals;
    let editing_user_message = RwSignal::new(None::<UserMessageEdit>);
    let action_handlers = TuiTurnActionHandlers {
        chat,
        locale,
        apply_assistant_display_filters,
        stream_follow_up,
        stream_turn_busy_ui,
        status_err,
        editing_user_message,
    };

    view! {
        <ChatMessagesScrollShell scroll_shell>
            <ChatTuiStreamView
                chat=chat
                locale=locale
                apply_assistant_display_filters=apply_assistant_display_filters
                markdown_render=markdown_render
                scroll_shell=scroll_shell
                action_handlers=action_handlers
                find=ChatFindOverlaySignals {
                    query: chat_find_query,
                    match_ids: chat_find_match_ids,
                    cursor: chat_find_cursor,
                    panel_open: chat_find_panel_open,
                }
            />
        </ChatMessagesScrollShell>
    }
}

fn handle_composer_image_input_change(
    ev: web_sys::Event,
    locale: RwSignal<crate::i18n::Locale>,
    pending_images: RwSignal<Vec<String>>,
    status_err: RwSignal<Option<String>>,
) {
    let Some(t) = ev.target() else {
        return;
    };
    let Ok(input) = t.dyn_into::<web_sys::HtmlInputElement>() else {
        return;
    };
    let files = input.files();
    let Some(list) = files else {
        return;
    };
    let n = list.length();
    if n == 0 {
        return;
    }
    let form = web_sys::FormData::new().expect("FormData");
    for i in 0..n {
        if let Some(f) = list.item(i) {
            let name = f.name();
            let _ = form.append_with_blob_and_filename("file", &f, &name);
        }
    }
    spawn_local(async move {
        match upload_files_multipart(&form, locale.get_untracked()).await {
            Ok(urls) => {
                pending_images.update(|v| {
                    for u in urls {
                        if v.len() >= 6 {
                            break;
                        }
                        if !v.contains(&u) {
                            v.push(u);
                        }
                    }
                });
            }
            Err(e) => {
                status_err.set(Some(e));
            }
        }
    });
    input.set_value("");
}

fn click_hidden_composer_image_input() {
    let Some(el) = leptos_dom::helpers::document().get_element_by_id("composer-image-input") else {
        return;
    };
    let Ok(input) = el.dyn_into::<web_sys::HtmlInputElement>() else {
        return;
    };
    input.click();
}

#[component]
fn ComposerImageInput(
    locale: RwSignal<crate::i18n::Locale>,
    pending_images: RwSignal<Vec<String>>,
    status_err: RwSignal<Option<String>>,
) -> impl IntoView {
    view! {
        <input
            type="file"
            class="composer-file-input-hidden"
            id="composer-image-input"
            tabindex="-1"
            accept="image/png,image/jpeg,image/jpg,image/webp,image/gif"
            multiple
            on:change=move |ev: web_sys::Event| {
                handle_composer_image_input_change(ev, locale, pending_images, status_err);
            }
        />
    }
}

#[component]
fn ComposerClarificationField(
    locale: RwSignal<crate::i18n::Locale>,
    pending_clarification: RwSignal<Option<crate::clarification_form::PendingClarificationForm>>,
    idx: usize,
    label: String,
    hint: Option<String>,
    required: bool,
) -> impl IntoView {
    let loc = locale.get_untracked();
    let req_suffix = if required {
        i18n::clarification_required_suffix(loc).to_string()
    } else {
        String::new()
    };
    let hint_view = match hint {
        Some(h) => view! { <span class="composer-clarification-hint">{h}</span> }.into_any(),
        None => view! { <span></span> }.into_any(),
    };
    view! {
        <label class="composer-clarification-field">
            <span class="composer-clarification-label">
                {label}
                {req_suffix}
            </span>
            {hint_view}
            <input
                type="text"
                class="composer-clarification-input"
                data-testid="composer-clarification-input"
                prop:value=move || {
                    pending_clarification.with(|opt| {
                        opt.as_ref()
                            .and_then(|fm| fm.values.get(idx))
                            .cloned()
                            .unwrap_or_default()
                    })
                }
                on:input=move |ev| {
                    let t = event_target_value(&ev);
                    pending_clarification.update(|opt| {
                        if let Some(fm) = opt.as_mut()
                            && fm.values.len() > idx
                        {
                            fm.values[idx] = t;
                        }
                    });
                }
            />
        </label>
    }
}

#[component]
fn ComposerClarificationActions(
    locale: RwSignal<crate::i18n::Locale>,
    pending_clarification: RwSignal<Option<crate::clarification_form::PendingClarificationForm>>,
    stream_turn_busy_ui: Memo<bool>,
    run_send_clarify_sv: StoredValue<Arc<dyn Fn() + Send + Sync>>,
) -> impl IntoView {
    view! {
        <div class="composer-clarification-actions">
            <button
                type="button"
                class="btn btn-muted btn-sm"
                prop:disabled=move || stream_turn_busy_ui.get()
                on:click=move |_| pending_clarification.set(None)
            >
                {move || i18n::clarification_dismiss(locale.get())}
            </button>
            <button
                type="button"
                class="btn btn-primary btn-sm"
                data-testid="composer-clarification-submit"
                prop:disabled=move || stream_turn_busy_ui.get()
                on:click=move |_| run_send_clarify_sv.get_value()()
            >
                {move || i18n::clarification_submit(locale.get())}
            </button>
        </div>
    }
}

fn ensure_clarification_values_len(
    pending_clarification: RwSignal<Option<crate::clarification_form::PendingClarificationForm>>,
    n: usize,
) {
    let needs_resize = pending_clarification
        .with_untracked(|opt| opt.as_ref().is_some_and(|fm| fm.values.len() != n));
    if !needs_resize {
        return;
    }
    pending_clarification.update(|opt| {
        if let Some(fm) = opt.as_mut() {
            fm.values.resize(n, String::new());
        }
    });
}

#[component]
fn ComposerClarificationPanel(
    locale: RwSignal<crate::i18n::Locale>,
    pending_clarification: RwSignal<Option<crate::clarification_form::PendingClarificationForm>>,
    stream_turn_busy_ui: Memo<bool>,
    run_send_clarify_sv: StoredValue<Arc<dyn Fn() + Send + Sync>>,
) -> impl IntoView {
    view! {
        <Show when=move || pending_clarification.get().is_some()>
            <div class="composer-clarification-panel" data-testid="composer-clarification-panel">
                {move || {
                    let Some(form) = pending_clarification.get() else {
                        return view! { <span></span> }.into_any();
                    };
                    let intro = form.intro.clone();
                    let loc = locale.get();
                    let n = form.fields.len();
                    ensure_clarification_values_len(pending_clarification, n);
                    let fields = form
                        .fields
                        .iter()
                        .enumerate()
                        .map(|(i, f)| {
                            view! {
                                <ComposerClarificationField
                                    locale
                                    pending_clarification
                                    idx=i
                                    label=f.label.clone()
                                    hint=f.hint.clone()
                                    required=f.required
                                />
                            }
                            .into_any()
                        })
                        .collect_view();
                    view! {
                        <div class="composer-clarification-title">
                            {i18n::clarification_panel_title(loc)}
                        </div>
                        <p class="composer-clarification-intro">{intro}</p>
                        <div class="composer-clarification-fields">
                            {fields}
                        </div>
                        <ComposerClarificationActions
                            locale
                            pending_clarification
                            stream_turn_busy_ui
                            run_send_clarify_sv
                        />
                    }
                    .into_any()
                }}
            </div>
        </Show>
    }
}

#[component]
fn ComposerQueuedChip(
    locale: RwSignal<crate::i18n::Locale>,
    stream_follow_up: RwSignal<ComposerStreamFollowUp>,
) -> impl IntoView {
    view! {
        <Show when=move || stream_follow_up.get().queued_user_text().is_some()>
            <div class="composer-queued" data-testid="composer-queued">
                <span class="composer-queued-label">
                    {move || i18n::composer_queued_chip(locale.get())}
                </span>
                <span class="composer-queued-preview">
                    {move || {
                        stream_follow_up
                            .get()
                            .queued_user_text()
                            .unwrap_or_default()
                            .chars()
                            .take(72)
                            .collect::<String>()
                    }}
                </span>
                <button
                    type="button"
                    class="btn btn-ghost btn-sm"
                    data-testid="composer-queued-dismiss"
                    prop:aria-label=move || i18n::composer_queued_dismiss(locale.get())
                    on:click=move |_| stream_follow_up.set(ComposerStreamFollowUp::Idle)
                >
                    {move || i18n::composer_queued_dismiss(locale.get())}
                </button>
            </div>
        </Show>
    }
}

#[component]
fn ComposerBarActions(
    locale: RwSignal<crate::i18n::Locale>,
    chat: ChatSessionSignals,
    selected_session_mode: RwSignal<String>,
    session_mode_user_override: RwSignal<bool>,
    composer_mode_open: RwSignal<bool>,
    composer_stop_enabled: Memo<bool>,
    initialized: RwSignal<bool>,
    run_send_message: Arc<dyn Fn() + Send + Sync>,
    trigger_stop: Arc<dyn Fn() + Send + Sync>,
) -> impl IntoView {
    view! {
        <div class="composer-bar-actions">
            <StatusSessionModeSeg props=SessionModeSegProps {
                locale,
                chat,
                selected_session_mode,
                session_mode_user_override,
                menu_open: composer_mode_open,
            } />
            <Show when=move || composer_stop_enabled.get()>
                <button
                    type="button"
                    class="btn btn-muted btn-send-icon"
                    data-testid="chat-stop-button"
                    on:click={
                        let t = Arc::clone(&trigger_stop);
                        move |_| t()
                    }
                    prop:title=move || i18n::composer_stop(locale.get())
                    prop:aria-label=move || i18n::composer_stop(locale.get())
                >
                    <svg
                        class="btn-send-icon-svg"
                        viewBox="0 0 24 24"
                        fill="currentColor"
                        xmlns="http://www.w3.org/2000/svg"
                        aria-hidden="true"
                    >
                        <rect x="6" y="6" width="12" height="12" rx="2" />
                    </svg>
                </button>
            </Show>
            <button
                type="button"
                class="btn btn-primary btn-send-icon"
                data-testid="chat-send-button"
                prop:disabled=move || !initialized.get()
                on:click={
                    let r = Arc::clone(&run_send_message);
                    move |_| r()
                }
                prop:title=move || {
                    if composer_stop_enabled.get() {
                        i18n::composer_send_queue_aria(locale.get())
                    } else {
                        i18n::composer_send_aria(locale.get())
                    }
                }
                prop:aria-label=move || {
                    if composer_stop_enabled.get() {
                        i18n::composer_send_queue_aria(locale.get())
                    } else {
                        i18n::composer_send_aria(locale.get())
                    }
                }
            >
                <svg
                    class="btn-send-icon-svg"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    xmlns="http://www.w3.org/2000/svg"
                    aria-hidden="true"
                >
                    <path d="M22 2 11 13" />
                    <path d="M22 2 15 22 11 13 2 9 22 2Z" />
                </svg>
            </button>
        </div>
    }
}

#[component]
fn ChatComposerPane(signals: ChatComposerPaneSignals) -> impl IntoView {
    let ChatComposerPaneSignals {
        locale,
        pending_images,
        pending_clarification,
        stream_turn_busy_ui,
        composer_stop_enabled,
        status_err,
        run_send_message,
        run_send_clarify_sv,
        trigger_stop,
        initialized,
        composer_input_ref,
        draft,
        composer_mirror_html,
        composer_mirror_scroll_top,
        workspace_path,
        insert_workspace_file_ref,
        chat,
        selected_session_mode,
        session_mode_user_override,
        stream_follow_up,
    } = signals;

    let drop_hl = ComposerDropHighlight::new();
    let composer_mode_open = RwSignal::new(false);

    view! {
        <div
            class="composer composer-ds"
            class:composer-drop-active=move || drop_hl.is_active()
            on:dragenter=move |ev: web_sys::DragEvent| {
                drop_hl.on_drag_enter(&ev);
            }
            on:dragover=move |ev: web_sys::DragEvent| {
                composer_accept_drag_over(&ev);
            }
            on:dragleave=move |ev: web_sys::DragEvent| {
                drop_hl.on_drag_leave(&ev);
            }
            on:drop=move |ev: web_sys::DragEvent| {
                drop_hl.clear();
                let root = workspace_path.get_untracked();
                let insert = insert_workspace_file_ref.get_value();
                handle_composer_file_drop(
                    ev,
                    &root,
                    insert.as_ref(),
                    locale,
                    pending_images,
                    status_err,
                );
            }
            on:paste:capture=move |ev: web_sys::ClipboardEvent| {
                handle_composer_image_paste(ev, locale, pending_images, status_err);
            }
        >
            <div class="composer-inner-ds">
                <ComposerImageInput
                    locale=locale
                    pending_images=pending_images
                    status_err=status_err
                />
                <ComposerPendingImagesRow locale=locale pending_images=pending_images />
                <ComposerClarificationPanel
                    locale=locale
                    pending_clarification=pending_clarification
                    stream_turn_busy_ui=stream_turn_busy_ui
                    run_send_clarify_sv=run_send_clarify_sv
                />
                <ComposerQueuedChip locale=locale stream_follow_up=stream_follow_up />
                <div class="composer-input-row">
                    <button
                        type="button"
                        class="btn btn-muted btn-sm composer-attach-label"
                        data-testid="composer-attach"
                        prop:title=move || i18n::composer_attach_image_aria(locale.get())
                        prop:aria-label=move || i18n::composer_attach_image_aria(locale.get())
                        on:click=move |_| click_hidden_composer_image_input()
                    >
                        <svg
                            viewBox="0 0 24 24"
                            fill="none"
                            stroke="currentColor"
                            stroke-width="2"
                            stroke-linecap="round"
                            stroke-linejoin="round"
                            class="composer-attach-icon"
                            aria-hidden="true"
                        >
                            <rect x="3" y="3" width="18" height="18" rx="2" ry="2" />
                            <circle cx="8.5" cy="8.5" r="1.5" />
                            <path d="m21 15-3.5-3.5a2 2 0 0 0-2.83 0L6 21" />
                        </svg>
                    </button>
                    <ComposerInputStack
                        composer_input_ref=composer_input_ref
                        draft=draft
                        composer_mirror_html=composer_mirror_html
                        composer_mirror_scroll_top=composer_mirror_scroll_top
                        run_send_message=run_send_message.clone()
                        locale=locale
                        workspace_path=workspace_path
                    />
                    <ComposerBarActions
                        locale=locale
                        chat=chat
                        selected_session_mode=selected_session_mode
                        session_mode_user_override=session_mode_user_override
                        composer_mode_open=composer_mode_open
                        composer_stop_enabled=composer_stop_enabled
                        initialized=initialized
                        run_send_message=run_send_message.clone()
                        trigger_stop=trigger_stop.clone()
                    />
                </div>
            </div>
        </div>
    }
}

pub fn chat_column_view(shell: ChatColumnShell) -> impl IntoView {
    let home_end_nav = ChatColumnHomeEndNav::from_composer(&shell.app.chat_composer);
    let run_send_clarify_sv = StoredValue::new(shell.run_send_message.clone());
    view! {
                <div
                    class="chat-column"
                    data-testid="chat-column"
                    on:keydown:capture=home_end_nav.keydown_handler()
                >
                    <ChatMessagesPane signals=shell.messages_pane_signals() />
                    <ChatComposerPane signals=shell.composer_pane_signals(run_send_clarify_sv) />
                </div>
    }
}
