//! 设置页「GitHub」分区：本机 Client ID + Device Flow（壳钥匙串 / 浏览器 Cookie）。

use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_dom::helpers::event_target_value;

use crate::api::github_secrets_local::{
    clear_github_connection_local, clear_github_oauth_client_id, github_oauth_client_id,
    github_oauth_client_id_is_set, on_device_flow_success, persist_github_oauth_client_id,
    reconcile_github_connection_status,
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

fn apply_terminal_device_state(
    loc: Locale,
    state: &str,
    error: Option<String>,
    access_token: Option<String>,
    login: Option<String>,
    ui: GithubUiSignals,
    auth_refresh_nonce: RwSignal<u64>,
) {
    match state {
        "success" => {
            spawn_local(async move {
                match on_device_flow_success(access_token.as_deref(), login.as_deref()).await {
                    Ok(()) => {
                        ui.err.set(None);
                        refresh_github_local_slots(loc, ui);
                        bump_auth_refresh_nonce(auth_refresh_nonce);
                    }
                    Err(e) => ui.err.set(Some(e)),
                }
                ui.busy.set(false);
            });
        }
        "denied" | "expired" | "cancelled" | "error" => {
            ui.err.set(Some(
                error.unwrap_or_else(|| i18n::settings_github_device_state(loc, state)),
            ));
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

async fn poll_until_device_done(
    loc: Locale,
    start: GithubDeviceStartDto,
    ui: GithubUiSignals,
    auth_refresh_nonce: RwSignal<u64>,
) {
    let interval_ms = u32::try_from(start.interval.max(1).saturating_mul(1000)).unwrap_or(5000);
    let expires = start.expires_in.max(60);
    let mut waited = 0u64;
    loop {
        TimeoutFuture::new(interval_ms).await;
        waited = waited.saturating_add(start.interval.max(1));
        match fetch_github_oauth_device_status(loc).await {
            Ok(st) => {
                ui.status_line
                    .set(Some(i18n::settings_github_device_state(loc, &st.state)));
                if is_device_terminal(&st.state) {
                    apply_terminal_device_state(
                        loc,
                        &st.state,
                        st.error,
                        st.access_token,
                        st.login,
                        ui,
                        auth_refresh_nonce,
                    );
                    return;
                }
            }
            Err(e) => {
                ui.err.set(Some(e));
                ui.busy.set(false);
                return;
            }
        }
        if waited >= expires {
            ui.err
                .set(Some(i18n::settings_github_device_expired(loc).to_string()));
            ui.busy.set(false);
            return;
        }
    }
}

fn spawn_device_connect(loc: Locale, ui: GithubUiSignals, auth_refresh_nonce: RwSignal<u64>) {
    if ui.busy.get_untracked() {
        return;
    }
    let client_id = github_oauth_client_id();
    if client_id.trim().is_empty() {
        ui.err
            .set(Some(i18n::settings_github_client_id_required(loc).into()));
        return;
    }
    ui.busy.set(true);
    ui.err.set(None);
    ui.status_line.set(None);
    ui.user_code.set(None);
    ui.verify_url.set(None);
    spawn_local(async move {
        match post_github_oauth_device_start(&client_id, loc).await {
            Ok(start) => {
                ui.user_code.set(Some(start.user_code.clone()));
                ui.verify_url
                    .set(Some(start.verification_uri_complete.clone()));
                tauri_open_external_url(&start.verification_uri_complete);
                poll_until_device_done(loc, start, ui, auth_refresh_nonce).await;
            }
            Err(e) => {
                ui.err.set(Some(e));
                ui.busy.set(false);
            }
        }
    });
}

fn spawn_device_disconnect(loc: Locale, ui: GithubUiSignals, auth_refresh_nonce: RwSignal<u64>) {
    if ui.busy.get_untracked() {
        return;
    }
    ui.busy.set(true);
    ui.err.set(None);
    spawn_local(async move {
        // 进行中的 Device Flow：尽力取消，失败不阻断断开。
        let _ = post_github_oauth_device_cancel(loc).await;

        let mut remote_errs: Vec<String> = Vec::new();
        // 先清 Cookie（依赖 credentials），再清本机态。
        if let Err(e) = post_github_oauth_device_logout(loc).await {
            remote_errs.push(e);
        }
        if let Err(e) = clear_github_connection_local().await {
            remote_errs.push(e);
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
        clear_github_oauth_client_id();
        ui.client_id_draft.set(String::new());
        ui.client_id_set.set(false);
        ui.client_id_feedback
            .set(Some(i18n::settings_github_client_id_cleared(loc).into()));
    } else {
        persist_github_oauth_client_id(&trimmed);
        ui.client_id_draft.set(String::new());
        ui.client_id_set.set(true);
        ui.client_id_feedback
            .set(Some(i18n::settings_github_client_id_saved(loc).into()));
    }
    refresh_github_local_slots(loc, ui);
    ui.busy.set(false);
}

fn spawn_clear_client_id(loc: Locale, ui: GithubUiSignals) {
    if ui.busy.get_untracked() {
        return;
    }
    ui.busy.set(true);
    ui.err.set(None);
    ui.client_id_feedback.set(None);
    clear_github_oauth_client_id();
    ui.client_id_draft.set(String::new());
    ui.client_id_set.set(false);
    ui.client_id_feedback
        .set(Some(i18n::settings_github_client_id_cleared(loc).into()));
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

fn disconnect_disabled(busy: bool, connected: bool) -> bool {
    busy || !connected
}

fn reopen_disabled(busy: bool, verify_url: Option<String>) -> bool {
    busy || verify_url.is_none()
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
            <p class="settings-hint" data-testid="settings-github-client-id-hint">
                {move || i18n::settings_github_client_id_hint(locale.get())}
            </p>
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
                prop:disabled=move || ui.busy.get()
                on:click=move |_| on_connect.run(())
            >
                {move || i18n::settings_github_connect(locale.get())}
            </button>
            <button
                type="button"
                class="btn btn-secondary btn-sm"
                data-testid="settings-github-disconnect"
                prop:disabled=move || disconnect_disabled(ui.busy.get(), ui.github_set.get())
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
                <p class="settings-hint" data-testid="settings-github-token-storage-hint">
                    {move || i18n::settings_github_token_storage_hint(locale.get())}
                </p>
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

    Effect::new(move |_| {
        let loc = locale.get();
        refresh_github_local_slots(loc, ui);
    });

    let on_connect = Callback::new(move |_| {
        spawn_device_connect(locale.get_untracked(), ui, auth_refresh_nonce);
    });
    let on_disconnect = Callback::new(move |_| {
        spawn_device_disconnect(locale.get_untracked(), ui, auth_refresh_nonce);
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
