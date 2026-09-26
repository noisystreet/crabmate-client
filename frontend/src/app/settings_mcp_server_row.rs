//! MCP 设置页：单条服务器编辑行。

use super::settings_mcp_server_row_actions::SettingsMcpServerRowActions;
use super::settings_mcp_status::McpSettingsSignals;
use super::settings_mcp_tools_list::SettingsMcpServerToolsList;
use super::settings_toggle_switch::SettingsToggleSwitch;
use crate::api::user_data::McpServersFileDto;
use crate::i18n::{self, Locale};
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

fn event_input_value(ev: &leptos::ev::Event) -> Option<String> {
    ev.target()
        .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
        .map(|el| el.value())
}

fn server_field<F>(file: &McpServersFileDto, id: &str, pick: F) -> String
where
    F: Fn(&crate::api::user_data::McpServerEntryDto) -> String,
{
    file.servers
        .iter()
        .find(|s| s.id == id)
        .map(pick)
        .unwrap_or_default()
}

fn apply_has_bearer(file: &mut McpServersFileDto, server_id: &str, has_bearer: bool) {
    if let Some(row) = file.servers.iter_mut().find(|s| s.id == server_id) {
        row.has_bearer = has_bearer;
    }
}

fn server_has_bearer(file: &McpServersFileDto, server_id: &str) -> bool {
    file.servers
        .iter()
        .find(|s| s.id == server_id)
        .is_some_and(|s| s.has_bearer)
}

/// 掩码仅在确已保存时显示；未保存留空（否则会让人误以为已配置）。
fn server_bearer_placeholder(file: &McpServersFileDto, server_id: &str) -> &'static str {
    if server_has_bearer(file, server_id) {
        i18n::SECRET_MASK_PLACEHOLDER
    } else {
        ""
    }
}

fn server_bearer_hint(loc: Locale, file: &McpServersFileDto, server_id: &str) -> &'static str {
    if server_has_bearer(file, server_id) {
        i18n::settings_mcp_bearer_hint_set(loc)
    } else {
        i18n::settings_mcp_bearer_hint_unset(loc)
    }
}

/// 保存远端 bearer：成功后把 `has_bearer` 标记同步进草稿与基线，并清空输入框。
fn mcp_bearer_save(
    sid: String,
    locale: RwSignal<Locale>,
    draft: RwSignal<String>,
    feedback: RwSignal<Option<String>>,
    file: WriteSignal<McpServersFileDto>,
    baseline: RwSignal<McpServersFileDto>,
    busy: WriteSignal<bool>,
) {
    let loc = locale.get_untracked();
    let token = draft.get_untracked();
    busy.set(true);
    spawn_local(async move {
        match crate::api::user_data::put_mcp_server_remote_auth(&sid, &token, loc).await {
            Ok(()) => {
                let has_bearer = !token.trim().is_empty();
                feedback.set(Some(if has_bearer {
                    i18n::settings_mcp_bearer_saved(loc).to_string()
                } else {
                    i18n::settings_mcp_bearer_cleared(loc).to_string()
                }));
                draft.set(String::new());
                file.update(|f| apply_has_bearer(f, &sid, has_bearer));
                baseline.update(|f| apply_has_bearer(f, &sid, has_bearer));
            }
            Err(e) => feedback.set(Some(e)),
        }
        busy.set(false);
    });
}

#[component]
fn SettingsMcpRemoteBearer(
    server_id: String,
    locale: RwSignal<crate::i18n::Locale>,
    file: ReadSignal<McpServersFileDto>,
    set_file: WriteSignal<McpServersFileDto>,
    baseline: RwSignal<McpServersFileDto>,
    busy: ReadSignal<bool>,
    set_busy: WriteSignal<bool>,
) -> impl IntoView {
    let bearer_draft = RwSignal::new(String::new());
    let bearer_feedback = RwSignal::new(None::<String>);
    let id_hint = server_id.clone();
    let id_placeholder = server_id.clone();
    let id_save = server_id;
    let bearer_input_id = format!("settings-mcp-bearer-{id_save}");

    view! {
        <div class="settings-field">
            <label class="settings-field-label" for=bearer_input_id.clone()>
                {move || i18n::settings_mcp_bearer_label(locale.get())}
            </label>
            <input
                id=bearer_input_id
                type="password"
                class="settings-text-input"
                autocomplete="off"
                data-testid="settings-mcp-bearer-input"
                prop:value=move || bearer_draft.get()
                placeholder=move || {
                    let sid = id_placeholder.clone();
                    server_bearer_placeholder(&file.get(), &sid)
                }
                on:input=move |ev| {
                    bearer_draft.set(event_input_value(&ev).unwrap_or_default());
                }
            />
        </div>
        <p class="settings-hint" data-testid="settings-mcp-bearer-hint">
            {move || {
                let sid = id_hint.clone();
                server_bearer_hint(locale.get(), &file.get(), &sid)
            }}
        </p>
        <button
            type="button"
            class="btn btn-secondary btn-sm"
            data-testid="settings-mcp-bearer-save"
            prop:disabled=move || busy.get()
            on:click={
                let sid = id_save.clone();
                move |_| {
                    mcp_bearer_save(
                        sid.clone(),
                        locale,
                        bearer_draft,
                        bearer_feedback,
                        set_file,
                        baseline,
                        set_busy,
                    );
                }
            }
        >
            {move || i18n::settings_mcp_bearer_save(locale.get())}
        </button>
        <p class="settings-hint" data-testid="settings-mcp-bearer-feedback">
            {move || bearer_feedback.get().unwrap_or_default()}
        </p>
    }
}

/// 传输提示文案：远程 URL / stdio 命令。
fn mcp_row_transport_hint(
    loc: Locale,
    f: &crate::api::user_data::McpServersFileDto,
    sid: &str,
) -> String {
    match f.servers.iter().find(|s| s.id == sid) {
        Some(s) if s.has_url => i18n::settings_mcp_transport_remote(loc).to_string(),
        Some(s) if s.has_command => i18n::settings_mcp_transport_stdio(loc).to_string(),
        _ => String::new(),
    }
}

/// 行内改名（写入草稿文件）。
fn mcp_row_set_name(
    set_file: WriteSignal<crate::api::user_data::McpServersFileDto>,
    sid: String,
    v: String,
) {
    set_file.update(|f| {
        if let Some(row) = f.servers.iter_mut().find(|s| s.id == sid) {
            row.name = v;
        }
    });
}

/// 行内启用开关。
fn mcp_row_toggle_enabled(
    set_file: WriteSignal<crate::api::user_data::McpServersFileDto>,
    sid: &str,
) {
    set_file.update(|f| {
        if let Some(row) = f.servers.iter_mut().find(|s| s.id == sid) {
            row.enabled = !row.enabled;
        }
    });
}

#[component]
pub(crate) fn SettingsMcpServerRow(server_id: String, ctx: McpSettingsSignals) -> impl IntoView {
    let McpSettingsSignals {
        locale,
        file,
        set_file,
        baseline,
        status,
        probing,
        busy,
        set_busy,
        ..
    } = ctx;
    let id_row = server_id.clone();
    let id_hint = server_id.clone();
    let id_name_val = server_id.clone();
    let id_name_in = server_id.clone();
    let id_enabled_val = server_id.clone();
    let id_enabled_in = server_id.clone();
    let id_tools = server_id.clone();
    let id_remote = server_id.clone();
    let id_bearer = server_id;
    let id_name_input = format!("settings-mcp-name-{id_bearer}");
    let tools_expanded = RwSignal::new(false);
    let show_bearer = Memo::new(move |_| {
        let sid = id_remote.clone();
        file.get()
            .servers
            .iter()
            .find(|s| s.id == sid)
            .is_some_and(|s| s.has_url)
    });

    view! {
        <div
            class="settings-mcp-server-row"
            data-testid=format!("mcp-server-row-{}", id_row)
        >
            <div class="settings-field">
                <label class="settings-field-label" for=id_name_input.clone()>
                    {move || i18n::settings_mcp_name_label(locale.get())}
                </label>
                <input
                    id=id_name_input
                    type="text"
                    class="settings-text-input"
                    prop:value=move || server_field(&file.get(), &id_name_val, |s| s.name.clone())
                    on:input=move |ev| {
                        let v = event_input_value(&ev).unwrap_or_default();
                        mcp_row_set_name(set_file, id_name_in.clone(), v);
                    }
                />
            </div>
            <SettingsMcpServerToolsList
                locale=locale
                server_id=id_tools.clone()
                status=status
                probing=probing
                expanded=tools_expanded
            />
            <p class="settings-hint" data-testid="mcp-server-transport-hint">
                {move || mcp_row_transport_hint(locale.get(), &file.get(), &id_hint)}
            </p>
            <Show when=move || show_bearer.get()>
                <SettingsMcpRemoteBearer
                    server_id=id_bearer.clone()
                    locale=locale
                    file=file
                    set_file=set_file
                    baseline=baseline
                    busy=busy
                    set_busy=set_busy
                />
            </Show>
            <SettingsToggleSwitch
                test_id="settings-mcp-server-enabled"
                checked=Signal::derive(move || {
                    file.get()
                        .servers
                        .iter()
                        .find(|s| s.id == id_enabled_val)
                        .is_some_and(|s| s.enabled)
                })
                label=Signal::derive(move || {
                    i18n::settings_mcp_enabled_label(locale.get()).to_string()
                })
                on_toggle={
                    let sid = id_enabled_in.clone();
                    move || mcp_row_toggle_enabled(set_file, &sid)
                }
            />
            <SettingsMcpServerRowActions locale=locale server_id=id_tools ctx=ctx />
        </div>
    }
}
