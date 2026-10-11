//! 底栏状态：模型、角色、运行态。

use std::sync::Arc;

use leptos::prelude::*;

use crate::api::load_client_llm_text_fields_from_storage;
use crate::app_prefs::status_bar_effective_model;
use crate::chat_session_state::ChatStreamBusyMemos;
use crate::i18n::{self, Locale};
use crate::user_prefs_sync_state::UserPrefsSyncPhase;

use super::app_shell_ctx::StatusBarFooterSignals;
use super::chat::bump_session_hydrate_nonce;
use super::settings_page::{SettingsSection, navigate_to_settings};
use super::shell_runtime_context::expect_chat_shell_ctx;
use super::status_agent_role_menu::{AgentRoleMenuProps, StatusAgentRoleMenu};
use super::status_context::{StatusBarContextChip, observe_context_watermark};
use super::status_fetch_state::status_bar_should_show_skeleton;
use super::status_tasks_state::StatusTasksSignals;

// ---- 类名契约（状态 → 类名）----
// 集中构造，避免同一类名在多个组件里就地手写；与 `styles/status.css` 选择器一一对应，改动须同步下方单测。

/// 底栏整体类名：任一异常态叠加 `status-bar-fetch-error`。
fn status_bar_footer_class(
    fetch_failed: bool,
    prefs_load_failed: bool,
    hydration_failed: bool,
) -> &'static str {
    if fetch_failed || prefs_load_failed || hydration_failed {
        "status-bar status-bar-fetch-error"
    } else {
        "status-bar"
    }
}

/// 错误面板类名：`--prefs-save` 区分「保存失败」（无重试按钮，与拉取失败共用基类样式）。
fn status_fetch_error_class(is_save_error: bool) -> &'static str {
    if is_save_error {
        "status-fetch-error status-fetch-error--prefs-save"
    } else {
        "status-fetch-error"
    }
}

/// 运行指示器状态：唯一决定 `status-run-<kind>` 类名后缀。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RunIndicatorKind {
    Ready,
    Running,
    Tool,
    Error,
}

impl RunIndicatorKind {
    /// 运行指示器类名：`status-run` + `status-run-{ready|running|tool|error}`。
    fn class(self) -> &'static str {
        match self {
            Self::Ready => "status-run status-run-ready",
            Self::Running => "status-run status-run-running",
            Self::Tool => "status-run status-run-tool",
            Self::Error => "status-run status-run-error",
        }
    }
}

#[component]
fn ConversationHydrationErrorPanel(
    hydration_err: String,
    locale: RwSignal<Locale>,
    on_retry: Arc<dyn Fn() + Send + Sync>,
) -> impl IntoView {
    let hydration_err_for_title = hydration_err.clone();
    let hydration_err_for_body = hydration_err.clone();
    view! {
        <div
            class=status_fetch_error_class(false)
            role="status"
            aria-live="polite"
            data-testid="hydration-parse-error"
        >
            <span class="status-fetch-error-text" title=hydration_err_for_title.clone()>
                {hydration_err_for_body}
            </span>
            <button
                type="button"
                class="btn btn-secondary btn-sm"
                data-testid="hydration-retry"
                on:click=move |_| on_retry()
            >
                {move || i18n::status_retry(locale.get())}
            </button>
        </div>
    }
}

#[component]
fn UserPrefsLoadErrorPanel(
    fetch_err: String,
    user_prefs_reload_nonce: RwSignal<u64>,
    locale: RwSignal<Locale>,
) -> impl IntoView {
    let fetch_err_for_title = fetch_err.clone();
    let fetch_err_for_body = fetch_err.clone();
    view! {
        <div
            class=status_fetch_error_class(false)
            role="status"
            aria-live="polite"
            data-testid="prefs-load-error"
        >
            <span class="status-fetch-error-text" title=fetch_err_for_title.clone()>
                {move || i18n::prefs_load_failed(locale.get(), fetch_err_for_body.as_str())}
            </span>
            <button
                type="button"
                class="btn btn-secondary btn-sm"
                data-testid="prefs-load-retry"
                on:click=move |_| {
                    user_prefs_reload_nonce.update(|n| *n = n.wrapping_add(1));
                }
            >
                {move || i18n::status_retry(locale.get())}
            </button>
        </div>
    }
}

#[component]
fn UserPrefsSaveErrorPanel(save_err: String, locale: RwSignal<Locale>) -> impl IntoView {
    let save_err_for_title = save_err.clone();
    let save_err_for_body = save_err.clone();
    view! {
        <div
            class=status_fetch_error_class(true)
            role="status"
            aria-live="polite"
            data-testid="prefs-save-error"
        >
            <span class="status-fetch-error-text" title=save_err_for_title.clone()>
                {move || i18n::prefs_save_failed(locale.get(), save_err_for_body.as_str())}
            </span>
        </div>
    }
}

#[component]
fn StatusFetchErrorPanel(
    fetch_err: String,
    refresh_status: Arc<dyn Fn() + Send + Sync>,
    settings_page: RwSignal<bool>,
    locale: RwSignal<Locale>,
) -> impl IntoView {
    let fetch_err_for_title = fetch_err.clone();
    let fetch_err_for_body = fetch_err.clone();
    let show_web_bearer_cta = crate::api::is_web_api_credential_error(&fetch_err);
    view! {
        <div
            class=status_fetch_error_class(false)
            role="status"
            aria-live="polite"
        >
            <span class="status-fetch-error-text" title=fetch_err_for_title.clone()>
                {move || i18n::status_fetch_error(locale.get(), fetch_err_for_body.as_str())}
            </span>
            <Show when=move || show_web_bearer_cta>
                <button
                    type="button"
                    class="btn btn-primary btn-sm"
                    data-testid="status-open-web-api-settings"
                    on:click=move |_| {
                        navigate_to_settings(settings_page, SettingsSection::Connection);
                    }
                >
                    {move || i18n::status_open_web_api_settings(locale.get())}
                </button>
            </Show>
            <button
                type="button"
                class="btn btn-secondary btn-sm"
                on:click=move |_| refresh_status()
            >
                {move || i18n::status_retry(locale.get())}
            </button>
        </div>
    }
}

#[derive(Clone, Copy)]
struct StatusBarChipsSignals {
    st: StatusTasksSignals,
    client_llm_storage_tick: RwSignal<u64>,
    selected_agent_role: RwSignal<Option<String>>,
    agent_role_user_override: RwSignal<bool>,
    selected_session_mode: RwSignal<String>,
    session_mode_user_override: RwSignal<bool>,
    llm_thinking_mode: RwSignal<String>,
    locale: RwSignal<Locale>,
    settings_page: RwSignal<bool>,
    context_usage_page: RwSignal<bool>,
}

#[component]
fn StatusBarChipsSkeleton(locale: RwSignal<Locale>) -> impl IntoView {
    view! {
        <div
            class="status-chips-skeleton"
            aria-busy="true"
            prop:aria-label=move || i18n::status_loading_aria(locale.get())
        >
            <span class="status-chip status-chip-skeleton">
                <span class="skeleton skeleton-chip-label"></span>
                <span class="skeleton skeleton-chip-value skeleton-chip-model"></span>
            </span>
            <span class="status-chip status-chip-skeleton status-chip-role">
                <span class="skeleton skeleton-chip-label"></span>
                <span class="skeleton skeleton-chip-value skeleton-chip-role-select"></span>
            </span>
            <span class="status-chip status-chip-skeleton status-chip-context">
                <span class="skeleton skeleton-chip-label"></span>
                <span class="skeleton skeleton-context-bar"></span>
            </span>
        </div>
    }
}

/// 状态栏「思考模式」开关：当前值是否为「开」（本机两态，仅 `on` 为开）。
fn thinking_mode_is_on(current: &str) -> bool {
    current == "on"
}

/// 状态栏「思考模式」开关：点击后的目标值（两态）。
///
/// 当前为 `on` → `off`；否则（`off`）→ `on`。
fn next_thinking_mode(current: &str) -> &'static str {
    if thinking_mode_is_on(current) {
        "off"
    } else {
        "on"
    }
}

/// 状态栏「思考模式」两态开关（`on` / `off`，本机 `localStorage` 持久化）。
///
/// 点击切换后写入本机存储（进程内缓存 + `localStorage`），与设置页 `llm_thinking_mode` 同源。
#[component]
fn StatusBarThinkingToggle(
    llm_thinking_mode: RwSignal<String>,
    locale: RwSignal<Locale>,
) -> impl IntoView {
    let is_on = move || thinking_mode_is_on(&llm_thinking_mode.get());
    view! {
        <button
            type="button"
            class="status-chip status-chip-thinking"
            class:status-chip-thinking--on=move || is_on()
            data-testid="status-thinking-toggle"
            prop:title=move || i18n::status_thinking_tooltip(locale.get())
            prop:aria-pressed=is_on
            on:click=move |_| {
                let next = next_thinking_mode(&llm_thinking_mode.get_untracked());
                llm_thinking_mode.set(next.to_string());
                let loc = locale.get_untracked();
                let (base, model, temp, ctx_tok, _old_think) =
                    load_client_llm_text_fields_from_storage();
                let _ = crate::api::persist_client_llm_to_storage(
                    &base,
                    &model,
                    &temp,
                    &ctx_tok,
                    next,
                    None,
                    loc,
                );
            }
        >
            <span class="status-chip-label">
                {move || i18n::status_chip_thinking(locale.get())}
            </span>
            <span class="status-chip-value">{move || {
                if is_on() {
                    i18n::status_thinking_on(locale.get())
                } else {
                    i18n::status_thinking_off(locale.get())
                }
            }}</span>
        </button>
    }
}

#[component]
fn StatusBarChipsLoaded(
    chips: StatusBarChipsSignals,
    role_menu_open: RwSignal<bool>,
) -> impl IntoView {
    let StatusBarChipsSignals {
        st,
        client_llm_storage_tick,
        selected_agent_role,
        agent_role_user_override,
        selected_session_mode,
        session_mode_user_override,
        llm_thinking_mode,
        locale,
        context_usage_page,
        ..
    } = chips;
    let chat = expect_chat_shell_ctx().chat;
    view! {
        <>
            <span class="status-chip">
                <span class="status-chip-label">
                    {move || i18n::status_chip_model(locale.get())}
                </span>
                <span class="status-chip-value">{move || {
                    let _tick = client_llm_storage_tick.get();
                    let sd = st.status_data.get();
                    let (_, stored_model, _, _, _) = load_client_llm_text_fields_from_storage();
                    status_bar_effective_model(sd.as_ref(), stored_model.as_str())
                }}</span>
            </span>
            <span
                class="status-chip status-chip-role"
                prop:title=move || i18n::status_role_title_attr(locale.get())
            >
                <span class="status-chip-label">
                    {move || i18n::status_role_label(locale.get())}
                </span>
                <StatusAgentRoleMenu props=AgentRoleMenuProps {
                    st,
                    locale,
                    chat,
                    selected_agent_role,
                    agent_role_user_override,
                    selected_session_mode,
                    session_mode_user_override,
                    menu_open: role_menu_open,
                } />
            </span>
            <StatusBarThinkingToggle
                llm_thinking_mode=llm_thinking_mode
                locale=locale
            />
            <StatusBarContextChip
                st=st
                chat=chat
                client_llm_storage_tick=client_llm_storage_tick
                selected_agent_role=selected_agent_role
                locale=locale
                context_usage_page=context_usage_page
            />
        </>
    }
}

#[component]
fn StatusBarChipsRow(
    chips: StatusBarChipsSignals,
    refresh_status: Arc<dyn Fn() + Send + Sync>,
    user_prefs_sync_phase: RwSignal<UserPrefsSyncPhase>,
    user_prefs_load_err: RwSignal<Option<String>>,
    user_prefs_save_err: RwSignal<Option<String>>,
    user_prefs_reload_nonce: RwSignal<u64>,
    conversation_hydration_err: RwSignal<Option<String>>,
    hydration_retry: Arc<dyn Fn() + Send + Sync>,
) -> impl IntoView {
    let StatusBarChipsSignals {
        st,
        locale,
        settings_page,
        ..
    } = chips;
    let role_menu_open = RwSignal::new(false);
    view! {
        <div
            class="status-chips"
            class:status-chips--role-menu-open=move || role_menu_open.get()
        >
            {move || {
                if let Some(err) = conversation_hydration_err.get() {
                    return view! {
                        <ConversationHydrationErrorPanel
                            hydration_err=err
                            locale=locale
                            on_retry=hydration_retry.clone()
                        />
                    }
                    .into_any();
                }
                if user_prefs_sync_phase.get() == UserPrefsSyncPhase::LoadFailed {
                    if let Some(err) = user_prefs_load_err.get() {
                        return view! {
                            <UserPrefsLoadErrorPanel
                                fetch_err=err
                                user_prefs_reload_nonce=user_prefs_reload_nonce
                                locale=locale
                            />
                        }
                        .into_any();
                    }
                }
                if user_prefs_sync_phase.get() == UserPrefsSyncPhase::SaveFailed {
                    if let Some(err) = user_prefs_save_err.get() {
                        return view! {
                            <UserPrefsSaveErrorPanel save_err=err locale=locale />
                        }
                        .into_any();
                    }
                }
                let phase = st.status_fetch_phase.get();
                let has_data = st.status_data.get().is_some();
                let has_error = st.status_fetch_err.get().is_some();
                if status_bar_should_show_skeleton(phase, has_data, has_error) {
                    view! { <StatusBarChipsSkeleton locale=locale /> }.into_any()
                } else if let Some(fetch_err) = st.status_fetch_err.get() {
                    view! {
                        <StatusFetchErrorPanel
                            fetch_err=fetch_err
                            refresh_status=refresh_status.clone()
                            settings_page=settings_page
                            locale=locale
                        />
                    }
                    .into_any()
                } else {
                    view! {
                        <StatusBarChipsLoaded chips=chips role_menu_open=role_menu_open />
                    }
                    .into_any()
                }
            }}
        </div>
    }
}

/// 运行指示器状态归类（error / tool / running / ready）。
fn run_indicator_kind(
    st: StatusTasksSignals,
    status_err: RwSignal<Option<String>>,
    stream_busy_memos: ChatStreamBusyMemos,
) -> RunIndicatorKind {
    if st.status_fetch_err.get().is_some() || status_err.get().is_some() {
        RunIndicatorKind::Error
    } else if stream_busy_memos.tool_timeline_busy_ui.get() {
        RunIndicatorKind::Tool
    } else if stream_busy_memos.model_status_busy.get() {
        RunIndicatorKind::Running
    } else {
        RunIndicatorKind::Ready
    }
}

/// 运行指示器文案（错误 / 工具阶段 / 生成中 / 就绪）。
fn run_indicator_text(
    loc: Locale,
    st: StatusTasksSignals,
    status_err: RwSignal<Option<String>>,
    stream_busy_memos: ChatStreamBusyMemos,
) -> String {
    if st.status_fetch_err.get().is_some() {
        i18n::status_unavailable(loc).to_string()
    } else if let Some(e) = status_err.get() {
        format!("{}{e}", i18n::status_error_prefix(loc))
    } else if stream_busy_memos.tool_timeline_busy_ui.get() {
        i18n::status_tool_running(loc).to_string()
    } else if stream_busy_memos.model_status_busy.get() {
        i18n::status_model_running(loc).to_string()
    } else {
        i18n::status_ready(loc).to_string()
    }
}

#[component]
fn StatusBarRunIndicator(
    st: StatusTasksSignals,
    status_err: RwSignal<Option<String>>,
    stream_busy_memos: ChatStreamBusyMemos,
    locale: RwSignal<Locale>,
) -> impl IntoView {
    view! {
        <span class=move || {
            run_indicator_kind(st, status_err, stream_busy_memos).class()
        }>
            <span class="status-run-dot" aria-hidden="true"></span>
            <span>{move || {
                run_indicator_text(locale.get(), st, status_err, stream_busy_memos)
            }}</span>
        </span>
    }
}

#[component]
fn StatusBarFooterBody(signals: StatusBarFooterSignals) -> impl IntoView {
    let StatusBarFooterSignals {
        status_tasks: st,
        status_err,
        stream_busy_memos,
        client_llm_storage_tick,
        selected_agent_role,
        agent_role_user_override,
        selected_session_mode,
        session_mode_user_override,
        llm_thinking_mode,
        refresh_status,
        user_prefs_reload_nonce,
        user_prefs_sync_phase,
        user_prefs_load_err,
        user_prefs_save_err,
        settings_page,
        context_usage_page,
        conversation_hydration_err,
        ..
    } = signals;
    let locale = expect_chat_shell_ctx().locale;
    let chat = expect_chat_shell_ctx().chat;
    // 会话内容水位：随快照刷新观测同口径内容用量，令底栏显示单调不减（会话切换即重置）。
    Effect::new(move |_| {
        observe_context_watermark(chat, st, client_llm_storage_tick, selected_agent_role);
    });
    let hydration_retry: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        status_err.set(None);
        chat.conversation_hydration_err.set(None);
        bump_session_hydrate_nonce(chat);
    });
    let chips = StatusBarChipsSignals {
        st,
        client_llm_storage_tick,
        selected_agent_role,
        agent_role_user_override,
        selected_session_mode,
        session_mode_user_override,
        llm_thinking_mode,
        locale,
        settings_page,
        context_usage_page,
    };
    view! {
        <footer
            data-testid="status-bar"
            class=move || {
            status_bar_footer_class(
                st.status_fetch_err.get().is_some(),
                user_prefs_sync_phase.get() == UserPrefsSyncPhase::LoadFailed,
                conversation_hydration_err.get().is_some(),
            )
        }>
            <StatusBarChipsRow
                chips=chips
                refresh_status=refresh_status
                user_prefs_sync_phase=user_prefs_sync_phase
                user_prefs_load_err=user_prefs_load_err
                user_prefs_save_err=user_prefs_save_err
                user_prefs_reload_nonce=user_prefs_reload_nonce
                conversation_hydration_err=conversation_hydration_err
                hydration_retry=hydration_retry.clone()
            />
            <StatusBarRunIndicator
                st=st
                status_err=status_err
                stream_busy_memos=stream_busy_memos
                locale=locale
            />
        </footer>
    }
}

pub fn status_bar_footer_view(signals: StatusBarFooterSignals) -> impl IntoView {
    let status_bar_visible = signals.status_bar_visible;
    view! {
        <Show when=move || status_bar_visible.get()>
            <StatusBarFooterBody signals=signals.clone() />
        </Show>
    }
}

#[cfg(test)]
mod thinking_toggle_tests {
    use super::{next_thinking_mode, thinking_mode_is_on};

    #[test]
    fn on_is_considered_on() {
        assert!(thinking_mode_is_on("on"));
    }

    #[test]
    fn off_is_considered_off() {
        assert!(!thinking_mode_is_on("off"));
    }

    #[test]
    fn server_is_considered_off() {
        // `server`（跟随服务端）在状态栏两态开关中视为「关」。
        assert!(!thinking_mode_is_on("server"));
    }

    #[test]
    fn next_from_on_is_off() {
        assert_eq!(next_thinking_mode("on"), "off");
    }

    #[test]
    fn next_from_off_is_on() {
        assert_eq!(next_thinking_mode("off"), "on");
    }

    #[test]
    fn next_from_server_is_on() {
        // 从「跟随服务端」点击开关 → 切到「开」。
        assert_eq!(next_thinking_mode("server"), "on");
    }
}

#[cfg(test)]
mod status_bar_class_tests {
    use super::{RunIndicatorKind, status_bar_footer_class, status_fetch_error_class};

    #[test]
    fn footer_class_is_base_when_healthy() {
        assert_eq!(status_bar_footer_class(false, false, false), "status-bar");
    }

    #[test]
    fn footer_class_marks_any_error_state() {
        // 三个异常来源任一命中都叠加 `-fetch-error`（victauri 用例依赖该后缀）。
        assert_eq!(
            status_bar_footer_class(true, false, false),
            "status-bar status-bar-fetch-error"
        );
        assert_eq!(
            status_bar_footer_class(false, true, false),
            "status-bar status-bar-fetch-error"
        );
        assert_eq!(
            status_bar_footer_class(false, false, true),
            "status-bar status-bar-fetch-error"
        );
    }

    #[test]
    fn fetch_error_class_is_base_unless_save_failure() {
        assert_eq!(status_fetch_error_class(false), "status-fetch-error");
        assert_eq!(
            status_fetch_error_class(true),
            "status-fetch-error status-fetch-error--prefs-save"
        );
    }

    #[test]
    fn run_indicator_class_matches_every_kind() {
        assert_eq!(
            RunIndicatorKind::Ready.class(),
            "status-run status-run-ready"
        );
        assert_eq!(
            RunIndicatorKind::Running.class(),
            "status-run status-run-running"
        );
        assert_eq!(RunIndicatorKind::Tool.class(), "status-run status-run-tool");
        assert_eq!(
            RunIndicatorKind::Error.class(),
            "status-run status-run-error"
        );
    }
}
