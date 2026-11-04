//! 状态栈：视图、权限、serve、模型、角色、思考模式、会话用量、运行态。
//!
//! 原底部状态栏已迁入右侧工作区列，纵向堆叠且可折叠（[`side_status_stack_view`]）；
//! 右列收起时由浮动 rail / 移动端顶栏的迷你运行点（[`StatusRunDotButton`]）兜底。

use std::sync::Arc;

use crabmate_client_api::url::api_base_authority;
use leptos::prelude::*;

use crate::api::api_base_url;
use crate::api::load_client_llm_text_fields_from_storage;
use crate::app_prefs::status_bar_effective_model;
use crate::chat_session_state::ChatStreamBusyMemos;
use crate::i18n::{self, Locale};
use crate::icon::icon_chevron_down;
use crate::user_prefs_sync_state::UserPrefsSyncPhase;

use super::app_shell_ctx::StatusBarFooterSignals;
use super::chat::bump_session_hydrate_nonce;
use super::settings_page::{SettingsSection, navigate_to_settings};
use super::shell_runtime_context::expect_chat_shell_ctx;
use super::status_agent_role_menu::{AgentRoleMenuProps, StatusAgentRoleMenu};
use super::status_context::{StatusBarContextChip, observe_context_watermark};
use super::status_fetch_state::status_bar_should_show_skeleton;
use super::status_session_mode_seg::{SessionModeSegProps, StatusSessionModeSeg};
use super::status_tasks_state::StatusTasksSignals;
use super::status_view_chip::StatusViewChip;

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
    // 骨架只覆盖**要等 `/status` 的那几颗** chip（model / role / ctx）：`view` 是本地状态、
    // `perm` 来自 user-data，把它们塞进骨架会画出一个点得动却没有数据的假开关。所以这里的
    // 次序与 [`StatusBarChipsLoaded`] 的真实次序**不逐项对应**，这是有意的。
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

/// `serve` chip 的可见值：`host:port`。
///
/// 空基址（浏览器与 serve 同 Origin，`make web-release` 那一份就是）说「同源」而不是留空：
/// 空 chip 读起来像「没连上」。
#[must_use]
fn serve_short_value(api_base: &str, locale: Locale) -> String {
    let authority = api_base_authority(api_base);
    if authority.is_empty() {
        i18n::status_chip_same_origin(locale).to_string()
    } else {
        authority
    }
}

/// `serve` chip 的 `title`：完整基址（空基址时那句话与可见值同一句）。
///
/// 右列只有几百像素，带路径前缀的自建地址会把后面几颗 chip 全挤走 —— 所以可见值只放
/// `host:port`，完整 URL 由 `title` 承载（与左栏连接行是同一条口径）。
#[must_use]
fn serve_title(api_base: &str, locale: Locale) -> String {
    let base = api_base.trim();
    if base.is_empty() {
        i18n::status_chip_same_origin(locale).to_string()
    } else {
        base.to_string()
    }
}

/// 状态栈的 `serve` chip：这个窗口连的是哪台 serve。
///
/// 与 TUI 状态栈的 `serve` 段是同一件事、同一个词（`crates/crabmate-tui/src/tui_mode/status.rs`
/// 的 `environment_segments`）。位置也在同一处：`view` / `perm` 之后、`model` 之前 ——
/// 「我在哪个视图、能动到什么、连的是哪台」这三件事挨着，跨客户端对照时不用挪眼睛。
#[component]
fn StatusBarServeChip(locale: RwSignal<Locale>) -> impl IntoView {
    view! {
        <span
            class="status-chip"
            data-testid="status-serve-chip"
            prop:title=move || serve_title(&api_base_url(), locale.get())
        >
            <span class="status-chip-label">
                {move || i18n::status_chip_serve(locale.get())}
            </span>
            <span class="status-chip-value">
                {move || serve_short_value(&api_base_url(), locale.get())}
            </span>
        </span>
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
            role="switch"
            class:settings-model-toggle-on=move || is_on()
            data-testid="status-thinking-toggle"
            prop:title=move || i18n::status_thinking_tooltip(locale.get())
            prop:aria-checked=is_on
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
            <span class="settings-model-toggle-track" aria-hidden="true">
                <span class="settings-model-toggle-thumb"></span>
            </span>
        </button>
    }
}

#[component]
fn StatusBarChipsLoaded(
    chips: StatusBarChipsSignals,
    role_menu_open: RwSignal<bool>,
    mode_menu_open: RwSignal<bool>,
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
            // 前两颗 chip 固定是「视图」与「权限」，且「先 view、后 perm」 —— 与 TUI 状态栈的
            // 前两行同序（`crates/crabmate-tui/src/tui_mode/status.rs` 的 `environment_segments`）。
            // 这两项回答「我在哪个视图」与「这一轮最多能动到什么」，比 model / role 这类环境
            // 事实更常被扫到；两端位置一致，才不会出现同一个开关一端排第一、一端排最后。
            <StatusViewChip />
            <crate::app::status_permission_chip::StatusPermissionChip locale=locale />
            <StatusBarServeChip locale=locale />
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
            // 会话模式（`ask` / `plan` / `act`）：与 TUI 的 `mode` 段同一件事、同一个位置
            // （`role` 之后、`think` 之前）。**切换入口也在这里** —— 输入框下方那颗同名选择器
            // 已删除：同一件事挂两个菜单，两份 `menu_open` 迟早不同步，而右列本来就常驻
            // （窄屏 / rail 收起时也在）。
            <span
                class="status-chip status-chip-mode"
                class:status-chip-mode--open=move || mode_menu_open.get()
                data-testid="status-mode-chip"
            >
                <span class="status-chip-label">
                    {move || i18n::status_mode_label(locale.get())}
                </span>
                <StatusSessionModeSeg props=SessionModeSegProps {
                    locale,
                    chat,
                    selected_session_mode,
                    session_mode_user_override,
                    menu_open: mode_menu_open,
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

/// 状态栈「有上拉菜单打开」的判定：`role` / `mode` 两颗 chip 各有自己的菜单。
///
/// 抽成函数是因为它出现在 `class:` 绑定里 —— 就地写 `||` 会把 `StatusBarChipsRow` 的
/// 圈复杂度顶上 `lizard` 的 10 上限（这一层本来就在边缘）。
#[must_use]
fn any_status_menu_open(role_open: bool, mode_open: bool) -> bool {
    role_open || mode_open
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
    let mode_menu_open = RwSignal::new(false);
    view! {
        <div
            class="status-chips"
            // 两个上拉菜单打开时都要放行溢出：菜单本体走 Portal（`position: fixed`），
            // 但触发按钮的 z-index 仍在这一层，裁切会让箭头/悬浮态被吃掉。
            class:status-chips--role-menu-open=move || {
                any_status_menu_open(role_menu_open.get(), mode_menu_open.get())
            }
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
                        <StatusBarChipsLoaded
                            chips=chips
                            role_menu_open=role_menu_open
                            mode_menu_open=mode_menu_open
                        />
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

/// 右列状态栈：把底栏的 chip 纵向堆叠进右侧工作区列，可折叠展开。
///
/// 次序与 **TUI 状态栈的前两行同源**：`view`（视图）在最上、`perm`（权限）第二，其后才是
/// `model` / `role` / `think` / `ctx`，运行指示贴最底。两端各按自己的直觉排，读起来就是
/// 「同一个开关在这里排第一、在那里排最后」（实现见 [`StatusBarChipsLoaded`] 与
/// `crates/crabmate-tui/src/tui_mode/status.rs` 的 `environment_segments`）。
///
/// - 是否挂载由调用方（右列）判定；挂载后常驻显示，仅由表头按钮折叠展开。
/// - `data-testid="status-bar"` 保留在内部 [`StatusBarFooterBody`] 的 `<footer>` 上，e2e 锚点不变。
/// - 折叠只切 CSS 类，`StatusBarFooterBody` 始终挂载，避免 `observe_context_watermark` Effect 因开合重建。
pub fn side_status_stack_view(
    signals: StatusBarFooterSignals,
    locale: RwSignal<Locale>,
    collapsed: RwSignal<bool>,
) -> impl IntoView {
    view! {
        <div
            class="side-status-stack"
            class:side-status-stack--collapsed=move || collapsed.get()
            data-testid="side-status-stack"
        >
            <button
                type="button"
                class="side-status-stack-head"
                data-testid="side-status-stack-toggle"
                prop:aria-expanded=move || (!collapsed.get()).to_string()
                prop:title=move || i18n::side_status_btn_title(locale.get())
                prop:aria-label=move || i18n::side_status_btn_title(locale.get())
                on:click=move |_| collapsed.update(|c| *c = !*c)
            >
                <span class="side-status-stack-title">
                    {move || i18n::side_status_btn_title(locale.get())}
                </span>
                {icon_chevron_down("side-status-stack-chevron")}
            </button>
            <div class="side-status-stack-body">
                <StatusBarFooterBody signals=signals.clone() />
            </div>
        </div>
    }
}

/// 收起态迷你运行点按钮：仅展示运行态色点，点击展开右列状态栈。
///
/// 桌面浮动 rail 与移动端顶栏共用；`title` / `aria-label` 复用运行态文案（含错误信息）。
#[component]
pub fn StatusRunDotButton(
    st: StatusTasksSignals,
    status_err: RwSignal<Option<String>>,
    stream_busy_memos: ChatStreamBusyMemos,
    locale: RwSignal<Locale>,
    on_click: Callback<()>,
) -> impl IntoView {
    let label = move || run_indicator_text(locale.get(), st, status_err, stream_busy_memos);
    view! {
        <button
            type="button"
            class="status-run-dot-btn"
            data-testid="status-run-dot"
            prop:title=label
            prop:aria-label=label
            on:click=move |_| on_click.run(())
        >
            <span class=move || { run_indicator_kind(st, status_err, stream_busy_memos).class() }>
                <span class="status-run-dot" aria-hidden="true"></span>
            </span>
        </button>
    }
}

#[cfg(test)]
mod serve_chip_tests {
    use super::{any_status_menu_open, serve_short_value, serve_title};
    use crate::i18n::{self, Locale};

    /// 任一菜单打开就放行溢出；两颗都关才恢复裁切。
    #[test]
    fn any_menu_open_covers_both_chips() {
        assert!(!any_status_menu_open(false, false));
        assert!(any_status_menu_open(true, false));
        assert!(any_status_menu_open(false, true));
        assert!(any_status_menu_open(true, true));
    }

    /// 可见值只放 `host:port`，完整基址留给 `title`：右列宽度放不下一条带路径的 URL。
    #[test]
    fn serve_chip_shows_the_authority_and_keeps_the_full_base_in_the_title() {
        let base = "https://serve.example.com:8443/prefix";
        assert_eq!(
            serve_short_value(base, Locale::En),
            "serve.example.com:8443"
        );
        assert_eq!(serve_title(base, Locale::En), base);
    }

    /// 空基址（浏览器同 Origin）说「同源」，不留空 —— 空 chip 读起来像「没连上」。
    #[test]
    fn serve_chip_says_same_origin_for_an_empty_base() {
        let same = i18n::status_chip_same_origin(Locale::ZhHans).to_string();
        assert_eq!(serve_short_value("", Locale::ZhHans), same);
        assert_eq!(serve_title("   ", Locale::ZhHans), same);
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
