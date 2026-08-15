//! 设置页「GitHub」分区：本机 Client ID + Device Flow（壳钥匙串 / 浏览器 Cookie）。
//!
//! Device Flow 轮询经 `spawn_local` 脱离组件；用 [`DeviceFlowGate`] / [`bump_device_flow_generation`]
//! 代际作废旧任务（卸载 / 重新授权 / 断开），避免多轮询串线（issue #27）。

use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_dom::helpers::event_target_value;

use crate::api::github_secrets_local::{
    clear_github_connection_local, clear_github_oauth_client_id, github_oauth_client_id,
    github_oauth_client_id_is_set, github_oauth_client_id_is_valid, on_device_flow_success,
    persist_github_oauth_client_id, reconcile_github_connection_status,
};
use crate::api::{
    GithubDeviceStartDto, fetch_github_oauth_device_status, post_github_oauth_device_cancel,
    post_github_oauth_device_logout, post_github_oauth_device_start,
};
use crate::i18n::{self, Locale};
use crate::tauri_shell::tauri_open_external_url;

#[derive(Clone, Copy)]
struct GithubUiSignals {
    github_set: RwSignal<bool>,
    client_id_set: RwSignal<bool>,
    client_id_draft: RwSignal<String>,
    client_id_feedback: RwSignal<Option<String>>,
    user_code: RwSignal<Option<String>>,
    verify_url: RwSignal<Option<String>>,
    status_line: RwSignal<Option<String>>,
    busy: RwSignal<bool>,
    err: RwSignal<Option<String>>,
}

/// 当前全局 Device Flow 代际是否仍等于本任务启动时的快照。
#[must_use]
pub(crate) fn is_device_flow_generation_current(current: u64, mine: u64) -> bool {
    current == mine
}

fn bump_device_flow_generation(flow_gen: RwSignal<u64>) -> u64 {
    let mut out = 0u64;
    flow_gen.update(|n| {
        *n = n.saturating_add(1);
        out = *n;
    });
    out
}

fn device_flow_still_active(flow_gen: RwSignal<u64>, mine: u64) -> bool {
    is_device_flow_generation_current(flow_gen.get_untracked(), mine)
}

fn refresh_github_local_slots(loc: Locale, ui: GithubUiSignals) {
    ui.client_id_set.set(github_oauth_client_id_is_set());
    spawn_local(async move {
        let connected = reconcile_github_connection_status(loc).await;
        ui.github_set.set(connected);
    });
}

fn bump_auth_refresh_nonce(nonce: RwSignal<u64>) {
    nonce.update(|n| *n = n.saturating_add(1));
}

#[derive(Clone, Copy)]
struct DeviceFlowGate {
    flow_gen: RwSignal<u64>,
    flow_mine: u64,
}

impl DeviceFlowGate {
    fn active(self) -> bool {
        device_flow_still_active(self.flow_gen, self.flow_mine)
    }
}

/// 落盘成功后发现代际已失效时，必须补偿清除本机 token（避免「假断开仍带钥」）。
#[must_use]
pub(crate) fn should_rollback_stale_device_token(persist_ok: bool, gate_active: bool) -> bool {
    persist_ok && !gate_active
}

struct TerminalDeviceOutcome {
    state: String,
    error: Option<String>,
    access_token: Option<String>,
    login: Option<String>,
}

async fn persist_device_success_if_current(
    loc: Locale,
    outcome: TerminalDeviceOutcome,
    ui: GithubUiSignals,
    auth_refresh_nonce: RwSignal<u64>,
    gate: DeviceFlowGate,
) {
    if !gate.active() {
        return;
    }
    let persist =
        on_device_flow_success(outcome.access_token.as_deref(), outcome.login.as_deref()).await;
    match persist {
        Ok(()) => {
            if should_rollback_stale_device_token(true, gate.active()) {
                let _ = clear_github_connection_local().await;
                return;
            }
            ui.err.set(None);
            refresh_github_local_slots(loc, ui);
            bump_auth_refresh_nonce(auth_refresh_nonce);
        }
        Err(e) => {
            if gate.active() {
                ui.err.set(Some(e));
            }
        }
    }
    if gate.active() {
        ui.busy.set(false);
    }
}

fn apply_terminal_device_state(
    loc: Locale,
    outcome: TerminalDeviceOutcome,
    ui: GithubUiSignals,
    auth_refresh_nonce: RwSignal<u64>,
    gate: DeviceFlowGate,
) {
    if !gate.active() {
        return;
    }
    match outcome.state.as_str() {
        "success" => {
            spawn_local(async move {
                persist_device_success_if_current(loc, outcome, ui, auth_refresh_nonce, gate).await;
            });
        }
        "denied" | "expired" | "cancelled" | "error" => {
            ui.err.set(Some(outcome.error.unwrap_or_else(|| {
                i18n::settings_github_device_state(loc, &outcome.state)
            })));
            ui.busy.set(false);
        }
        _ => {}
    }
}

fn is_device_terminal(state: &str) -> bool {
    matches!(
        state,
        "success" | "denied" | "expired" | "cancelled" | "error"
    )
}

/// Android 切到 GitHub App 授权时 WebView 常中断 `fetch`（`TypeError: Failed to fetch`）。
/// 仅把网络中断 / 瞬时 HTTP 当作可重试，避免把 CORS、401 等永久失败拖到过期。
pub(crate) fn is_transient_http_status(status: u16) -> bool {
    matches!(status, 408 | 425 | 429 | 500..=599)
}

/// 从 [`crate::i18n::api_err_http_status`] 文案取出状态码；WASM `fetch` 栈不含此前缀。
pub(crate) fn http_status_hint_in_error(err: &str) -> Option<u16> {
    let marker = if err.contains("Request failed (") {
        "Request failed ("
    } else if err.contains("请求失败 (") {
        "请求失败 ("
    } else {
        return None;
    };
    let rest = err.split_once(marker)?.1;
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    let status = digits.parse().ok()?;
    (100..600).contains(&status).then_some(status)
}

pub(crate) fn is_transient_device_poll_error(err: &str) -> bool {
    if let Some(status) = http_status_hint_in_error(err)
        && is_transient_http_status(status)
    {
        return true;
    }
    let e = err.to_ascii_lowercase();
    e.contains("failed to fetch") || e.contains("networkerror")
}

/// `true`：代际已失效，或错误不可恢复，应结束轮询。
pub(crate) fn should_stop_device_poll_on_error(gate_active: bool, err: &str) -> bool {
    !gate_active || !is_transient_device_poll_error(err)
}

/// 返回 `true` 时停止轮询（代际失效、或不可恢复错误）。
fn abort_after_device_poll_error(
    loc: Locale,
    err: String,
    ui: GithubUiSignals,
    gate: DeviceFlowGate,
) -> bool {
    if should_stop_device_poll_on_error(gate.active(), &err) {
        if gate.active() {
            ui.err.set(Some(err));
            ui.busy.set(false);
        }
        return true;
    }
    ui.status_line.set(Some(
        i18n::settings_github_device_poll_retry(loc).to_string(),
    ));
    false
}

async fn poll_until_device_done(
    loc: Locale,
    start: GithubDeviceStartDto,
    ui: GithubUiSignals,
    auth_refresh_nonce: RwSignal<u64>,
    gate: DeviceFlowGate,
) {
    let interval_ms = u32::try_from(start.interval.max(1).saturating_mul(1000)).unwrap_or(5000);
    let expires = start.expires_in.max(60);
    let mut waited = 0u64;
    loop {
        TimeoutFuture::new(interval_ms).await;
        if !gate.active() {
            return;
        }
        waited = waited.saturating_add(start.interval.max(1));
        match fetch_github_oauth_device_status(loc).await {
            Ok(st) => {
                if !gate.active() {
                    return;
                }
                ui.status_line
                    .set(Some(i18n::settings_github_device_state(loc, &st.state)));
                if is_device_terminal(&st.state) {
                    apply_terminal_device_state(
                        loc,
                        TerminalDeviceOutcome {
                            state: st.state,
                            error: st.error,
                            access_token: st.access_token,
                            login: st.login,
                        },
                        ui,
                        auth_refresh_nonce,
                        gate,
                    );
                    return;
                }
            }
            Err(e) => {
                if abort_after_device_poll_error(loc, e, ui, gate) {
                    return;
                }
            }
        }
        if waited >= expires {
            if gate.active() {
                ui.err
                    .set(Some(i18n::settings_github_device_expired(loc).to_string()));
                ui.busy.set(false);
            }
            return;
        }
    }
}

fn spawn_device_connect(
    loc: Locale,
    ui: GithubUiSignals,
    auth_refresh_nonce: RwSignal<u64>,
    flow_gen: RwSignal<u64>,
) {
    let client_id = github_oauth_client_id();
    if client_id.trim().is_empty() {
        ui.err
            .set(Some(i18n::settings_github_client_id_required(loc).into()));
        return;
    }
    // 作废进行中的轮询；必要时取消远端上一轮 Device Flow。
    let flow_mine = bump_device_flow_generation(flow_gen);
    let gate = DeviceFlowGate {
        flow_gen,
        flow_mine,
    };
    ui.busy.set(true);
    ui.err.set(None);
    ui.status_line.set(None);
    ui.user_code.set(None);
    ui.verify_url.set(None);
    spawn_local(async move {
        let _ = post_github_oauth_device_cancel(loc).await;
        if !gate.active() {
            return;
        }
        match post_github_oauth_device_start(&client_id, loc).await {
            Ok(start) => {
                if !gate.active() {
                    return;
                }
                ui.user_code.set(Some(start.user_code.clone()));
                ui.verify_url
                    .set(Some(start.verification_uri_complete.clone()));
                tauri_open_external_url(&start.verification_uri_complete);
                poll_until_device_done(loc, start, ui, auth_refresh_nonce, gate).await;
            }
            Err(e) => {
                if gate.active() {
                    ui.err.set(Some(e));
                    ui.busy.set(false);
                }
            }
        }
    });
}

fn spawn_device_disconnect(
    loc: Locale,
    ui: GithubUiSignals,
    auth_refresh_nonce: RwSignal<u64>,
    flow_gen: RwSignal<u64>,
) {
    // 即使 busy（授权轮询中）也允许断开：先抬升代际停本地 poll。
    let flow_mine = bump_device_flow_generation(flow_gen);
    let gate = DeviceFlowGate {
        flow_gen,
        flow_mine,
    };
    ui.busy.set(true);
    ui.err.set(None);
    spawn_local(async move {
        let _ = post_github_oauth_device_cancel(loc).await;
        // 代际已易主则不得再 logout/清本机，避免误删新一轮授权写入的 token。
        if !gate.active() {
            return;
        }

        let mut remote_errs: Vec<String> = Vec::new();
        if let Err(e) = post_github_oauth_device_logout(loc).await {
            remote_errs.push(e);
        }
        if !gate.active() {
            return;
        }
        if let Err(e) = clear_github_connection_local().await {
            remote_errs.push(e);
        }
        if !gate.active() {
            return;
        }

        ui.github_set.set(false);
        ui.user_code.set(None);
        ui.verify_url.set(None);
        ui.status_line.set(None);
        if remote_errs.is_empty() {
            ui.err.set(None);
        } else {
            ui.err.set(Some(i18n::settings_github_disconnect_partial(
                loc,
                &remote_errs.join("；"),
            )));
        }
        bump_auth_refresh_nonce(auth_refresh_nonce);
        ui.busy.set(false);
    });
}

fn spawn_save_client_id(loc: Locale, ui: GithubUiSignals) {
    if ui.busy.get_untracked() {
        return;
    }
    let draft = ui.client_id_draft.get_untracked();
    ui.busy.set(true);
    ui.err.set(None);
    ui.client_id_feedback.set(None);
    let trimmed = draft.trim().to_string();
    if trimmed.is_empty() {
        apply_client_id_clear_result(loc, ui, clear_github_oauth_client_id());
    } else if !github_oauth_client_id_is_valid(&trimmed) {
        ui.err
            .set(Some(i18n::settings_github_client_id_invalid(loc).into()));
    } else {
        match persist_github_oauth_client_id(&trimmed) {
            Ok(()) => {
                ui.client_id_draft.set(String::new());
                ui.client_id_set.set(true);
                ui.client_id_feedback
                    .set(Some(i18n::settings_github_client_id_saved(loc).into()));
            }
            Err(e) => ui
                .err
                .set(Some(i18n::settings_github_client_id_storage_failed(
                    loc, &e,
                ))),
        }
    }
    refresh_github_local_slots(loc, ui);
    ui.busy.set(false);
}

fn apply_client_id_clear_result(loc: Locale, ui: GithubUiSignals, result: Result<(), String>) {
    match result {
        Ok(()) => {
            ui.client_id_draft.set(String::new());
            ui.client_id_set.set(false);
            ui.client_id_feedback
                .set(Some(i18n::settings_github_client_id_cleared(loc).into()));
        }
        Err(e) => ui
            .err
            .set(Some(i18n::settings_github_client_id_storage_failed(
                loc, &e,
            ))),
    }
}

fn spawn_clear_client_id(loc: Locale, ui: GithubUiSignals) {
    if ui.busy.get_untracked() {
        return;
    }
    ui.busy.set(true);
    ui.err.set(None);
    ui.client_id_feedback.set(None);
    apply_client_id_clear_result(loc, ui, clear_github_oauth_client_id());
    refresh_github_local_slots(loc, ui);
    ui.busy.set(false);
}

fn connection_label(loc: Locale, set: bool) -> &'static str {
    if set {
        i18n::settings_github_connected(loc)
    } else {
        i18n::settings_github_disconnected(loc)
    }
}

fn client_id_local_label(loc: Locale, set: bool) -> &'static str {
    if set {
        i18n::settings_github_client_id_set(loc)
    } else {
        i18n::settings_github_client_id_unset(loc)
    }
}

fn status_pill_class(ok: bool) -> &'static str {
    if ok {
        "settings-status-pill settings-status-pill--ok"
    } else {
        "settings-status-pill settings-status-pill--muted"
    }
}

/// `device_ui_active`：已展示 user_code / verify_url 时允许「断开」取消进行中的 Device Flow。
fn disconnect_disabled(busy: bool, connected: bool, device_ui_active: bool) -> bool {
    if device_ui_active {
        return false;
    }
    busy || !connected
}

fn reopen_disabled(busy: bool, verify_url: Option<String>) -> bool {
    let _ = busy;
    verify_url.is_none()
}

fn clear_client_id_disabled(busy: bool, set: bool) -> bool {
    busy || !set
}

fn client_id_input_placeholder(set: bool) -> &'static str {
    if set { "••••••••" } else { "" }
}

#[component]
fn SettingsGithubClientIdStatus(locale: RwSignal<Locale>, ui: GithubUiSignals) -> impl IntoView {
    view! {
        <div class="settings-status-line" data-testid="settings-github-client-id-status">
            <span
                class=move || status_pill_class(ui.client_id_set.get())
                role="status"
            >
                {move || client_id_local_label(locale.get(), ui.client_id_set.get())}
            </span>
        </div>
    }
}

#[component]
fn SettingsGithubClientIdActions(locale: RwSignal<Locale>, ui: GithubUiSignals) -> impl IntoView {
    view! {
        <div class="settings-row">
            <button
                type="button"
                class="btn btn-secondary btn-sm"
                data-testid="settings-github-client-id-save"
                prop:disabled=move || ui.busy.get()
                on:click=move |_| spawn_save_client_id(locale.get_untracked(), ui)
            >
                {move || i18n::settings_github_client_id_save(locale.get())}
            </button>
            <button
                type="button"
                class="btn btn-ghost btn-sm"
                data-testid="settings-github-client-id-clear"
                prop:disabled=move || {
                    clear_client_id_disabled(ui.busy.get(), ui.client_id_set.get())
                }
                on:click=move |_| spawn_clear_client_id(locale.get_untracked(), ui)
            >
                {move || i18n::settings_github_client_id_clear(locale.get())}
            </button>
        </div>
        <Show when=move || ui.client_id_feedback.get().is_some()>
            <p class="settings-hint" data-testid="settings-github-client-id-feedback">
                {move || ui.client_id_feedback.get().unwrap_or_default()}
            </p>
        </Show>
    }
}

#[component]
fn SettingsGithubClientIdBlock(
    locale: RwSignal<Locale>,
    ui: GithubUiSignals,
    input_id: &'static str,
) -> impl IntoView {
    view! {
        <div class="settings-field" data-testid="settings-github-client-id">
            <label class="settings-field-label" for=input_id>
                {move || i18n::settings_github_client_id_label(locale.get())}
            </label>
            <SettingsGithubClientIdStatus locale=locale ui=ui />
            <input
                id=input_id
                class="input"
                type="text"
                autocomplete="off"
                spellcheck="false"
                data-testid="settings-github-client-id-input"
                prop:value=move || ui.client_id_draft.get()
                prop:placeholder=move || client_id_input_placeholder(ui.client_id_set.get())
                prop:disabled=move || ui.busy.get()
                on:input=move |ev| ui.client_id_draft.set(event_target_value(&ev))
            />
            <SettingsGithubClientIdActions locale=locale ui=ui />
        </div>
    }
}

#[component]
fn SettingsGithubBlockActions(
    locale: RwSignal<Locale>,
    ui: GithubUiSignals,
    on_connect: Callback<()>,
    on_disconnect: Callback<()>,
    on_reopen: Callback<()>,
) -> impl IntoView {
    view! {
        <div class="settings-row">
            <button
                type="button"
                class="btn btn-primary btn-sm"
                data-testid="settings-github-connect"
                on:click=move |_| on_connect.run(())
            >
                {move || i18n::settings_github_connect(locale.get())}
            </button>
            <button
                type="button"
                class="btn btn-secondary btn-sm"
                data-testid="settings-github-disconnect"
                prop:disabled=move || {
                    disconnect_disabled(
                        ui.busy.get(),
                        ui.github_set.get(),
                        ui.user_code.get().is_some() || ui.verify_url.get().is_some(),
                    )
                }
                on:click=move |_| on_disconnect.run(())
            >
                {move || i18n::settings_github_disconnect(locale.get())}
            </button>
            <button
                type="button"
                class="btn btn-ghost btn-sm"
                data-testid="settings-github-reopen"
                prop:disabled=move || reopen_disabled(ui.busy.get(), ui.verify_url.get())
                on:click=move |_| on_reopen.run(())
            >
                {move || i18n::settings_github_reopen(locale.get())}
            </button>
        </div>
    }
}

#[component]
fn SettingsGithubBlockView(
    locale: RwSignal<Locale>,
    ui: GithubUiSignals,
    input_id: &'static str,
    show_title: bool,
    on_connect: Callback<()>,
    on_disconnect: Callback<()>,
    on_reopen: Callback<()>,
) -> impl IntoView {
    view! {
        <div class="settings-block" data-testid="settings-github-block">
            <Show when=move || show_title>
                <h3 class="settings-block-title">{move || i18n::settings_github_block_title(locale.get())}</h3>
            </Show>
            <SettingsGithubClientIdBlock locale=locale ui=ui input_id=input_id />
            <div class="settings-field" data-testid="settings-github-connection">
                <div class="settings-field-label">
                    {move || i18n::settings_github_connection_label(locale.get())}
                </div>
                <div class="settings-status-line">
                    <span
                        class=move || status_pill_class(ui.github_set.get())
                        role="status"
                        data-testid="settings-github-connection-status"
                    >
                        {move || connection_label(locale.get(), ui.github_set.get())}
                    </span>
                </div>
                <Show when=move || ui.user_code.get().is_some()>
                    <p class="settings-github-user-code" data-testid="settings-github-user-code">
                        {move || ui.user_code.get().unwrap_or_default()}
                    </p>
                </Show>
                <Show when=move || ui.status_line.get().is_some()>
                    <p class="settings-hint" data-testid="settings-github-status">
                        {move || ui.status_line.get().unwrap_or_default()}
                    </p>
                </Show>
                <Show when=move || ui.err.get().is_some()>
                    <p class="settings-error" data-testid="settings-github-error">
                        {move || ui.err.get().unwrap_or_default()}
                    </p>
                </Show>
                <SettingsGithubBlockActions
                    locale=locale
                    ui=ui
                    on_connect=on_connect
                    on_disconnect=on_disconnect
                    on_reopen=on_reopen
                />
            </div>
        </div>
    }
}

#[component]
pub(crate) fn SettingsGithubBlock(
    locale: RwSignal<Locale>,
    /// `<input id=…>`：设置页与弹窗可能同时挂载，须用不同 id。
    input_id: &'static str,
    /// 全屏设置页已有分区标题时可关掉块内标题，避免与侧栏「GitHub」重复。
    #[prop(default = true)]
    show_title: bool,
    /// Device Flow 成功或断开后递增，供壳层刷新 `GET /github/repo-context`。
    auth_refresh_nonce: RwSignal<u64>,
) -> impl IntoView {
    let ui = GithubUiSignals {
        github_set: RwSignal::new(false),
        client_id_set: RwSignal::new(false),
        client_id_draft: RwSignal::new(String::new()),
        client_id_feedback: RwSignal::new(None),
        user_code: RwSignal::new(None),
        verify_url: RwSignal::new(None),
        status_line: RwSignal::new(None),
        busy: RwSignal::new(false),
        err: RwSignal::new(None),
    };
    let flow_gen = RwSignal::new(0u64);

    Effect::new(move |_| {
        let loc = locale.get();
        refresh_github_local_slots(loc, ui);
    });

    // 卸载设置块：作废本地 poll，并尽力取消远端 Device Flow。
    on_cleanup(move || {
        bump_device_flow_generation(flow_gen);
        let loc = locale.get_untracked();
        spawn_local(async move {
            let _ = post_github_oauth_device_cancel(loc).await;
        });
    });

    let on_connect = Callback::new(move |_| {
        spawn_device_connect(locale.get_untracked(), ui, auth_refresh_nonce, flow_gen);
    });
    let on_disconnect = Callback::new(move |_| {
        spawn_device_disconnect(locale.get_untracked(), ui, auth_refresh_nonce, flow_gen);
    });
    let on_reopen = Callback::new(move |_| {
        if let Some(url) = ui.verify_url.get_untracked() {
            tauri_open_external_url(&url);
        }
    });

    view! {
        <SettingsGithubBlockView
            locale=locale
            ui=ui
            input_id=input_id
            show_title=show_title
            on_connect=on_connect
            on_disconnect=on_disconnect
            on_reopen=on_reopen
        />
    }
}

#[cfg(test)]
mod tests {
    use super::{
        disconnect_disabled, http_status_hint_in_error, is_device_flow_generation_current,
        is_device_terminal, is_transient_device_poll_error, is_transient_http_status,
        should_rollback_stale_device_token, should_stop_device_poll_on_error,
    };

    #[test]
    fn generation_matches_only_when_equal() {
        assert!(is_device_flow_generation_current(3, 3));
        assert!(!is_device_flow_generation_current(4, 3));
        assert!(!is_device_flow_generation_current(0, 1));
    }

    #[test]
    fn stale_generation_must_not_apply() {
        let current = 2u64;
        let mine = 1u64;
        assert!(
            !is_device_flow_generation_current(current, mine),
            "旧代际任务不得写 UI / 落 token"
        );
    }

    #[test]
    fn stale_persist_requires_token_rollback() {
        assert!(should_rollback_stale_device_token(true, false));
        assert!(!should_rollback_stale_device_token(true, true));
        assert!(!should_rollback_stale_device_token(false, false));
    }

    #[test]
    fn disconnect_enabled_during_device_ui() {
        assert!(!disconnect_disabled(true, false, true));
        assert!(disconnect_disabled(false, false, false));
        assert!(!disconnect_disabled(false, true, false));
        assert!(disconnect_disabled(true, true, false));
    }

    #[test]
    fn terminal_states_recognized() {
        assert!(is_device_terminal("success"));
        assert!(is_device_terminal("cancelled"));
        assert!(!is_device_terminal("pending"));
    }

    #[test]
    fn android_github_app_fetch_abort_is_transient() {
        let wasm_fetch = "fetch: JsValue(TypeError: Failed to fetch TypeError: Failed to fetch at __wbg_fetch_729fad2e5272298f (http://tauri.localhost/crabmate-web.js:424:30))";
        assert!(is_transient_device_poll_error(wasm_fetch));
        assert!(is_transient_device_poll_error(
            "fetch: JsValue(TypeError: NetworkError when attempting to fetch resource.)"
        ));
        assert!(is_transient_device_poll_error(
            "read body: JsValue(TypeError: Failed to fetch)"
        ));
        assert!(!is_transient_device_poll_error(
            "fetch: JsValue(TypeError: Failed to execute 'fetch' on 'Window': Illegal invocation)"
        ));
        assert!(!is_transient_device_poll_error("upload failed"));
        assert!(!is_transient_device_poll_error("invalid client_id"));
        assert!(!is_transient_device_poll_error("Request failed"));
    }

    #[test]
    fn transient_http_status_retries_5xx_not_401() {
        assert!(is_transient_http_status(502));
        assert!(is_transient_http_status(429));
        assert!(is_transient_http_status(408));
        assert!(!is_transient_http_status(401));
        assert!(!is_transient_http_status(400));
        assert!(!is_transient_http_status(404));
        assert_eq!(http_status_hint_in_error("Request failed (502)"), Some(502));
        assert_eq!(http_status_hint_in_error("请求失败 (503)：网关"), Some(503));
        assert_eq!(
            http_status_hint_in_error(wasm_stack_without_http_hint()),
            None
        );
        assert!(is_transient_device_poll_error("Request failed (502)"));
        assert!(is_transient_device_poll_error("请求失败 (503)"));
        assert!(!is_transient_device_poll_error(
            "Request failed (401): enter bearer"
        ));
        assert!(!is_transient_device_poll_error("请求失败 (400)"));
    }

    #[test]
    fn poll_error_stop_policy_matches_gate_and_transience() {
        let fetch_abort = "fetch: JsValue(TypeError: Failed to fetch)";
        assert!(!should_stop_device_poll_on_error(true, fetch_abort));
        assert!(!should_stop_device_poll_on_error(
            true,
            "Request failed (502)"
        ));
        assert!(should_stop_device_poll_on_error(
            true,
            "Request failed (401)"
        ));
        assert!(should_stop_device_poll_on_error(false, fetch_abort));
    }

    fn wasm_stack_without_http_hint() -> &'static str {
        "fetch: JsValue(TypeError: Failed to fetch at __wbg_fetch (http://tauri.localhost/x.js:1:1))"
    }
}
