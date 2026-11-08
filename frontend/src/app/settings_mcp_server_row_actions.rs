//! MCP 设置页：单服务器行操作按钮。

use leptos::prelude::*;
use leptos::task::spawn_local;

use super::settings_mcp_status::{McpSettingsSignals, spawn_probe_mcp_server};
use crate::i18n::{self, Locale};

#[component]
pub(crate) fn SettingsMcpServerRowActions(
    locale: RwSignal<Locale>,
    server_id: String,
    ctx: McpSettingsSignals,
) -> impl IntoView {
    let McpSettingsSignals { set_file, busy, .. } = ctx;
    let id_remove = server_id.clone();
    let id_probe = server_id;

    view! {
        <div class="settings-mcp-row-actions">
            <button
                type="button"
                class="btn btn-secondary btn-sm"
                prop:disabled=move || busy.get()
                on:click=move |_| spawn_probe_mcp_server(ctx, id_probe.clone())
            >
                {move || i18n::settings_mcp_probe(locale.get())}
            </button>
            <button
                type="button"
                class="btn btn-danger btn-sm"
                data-testid="settings-mcp-remove"
                on:click=move |_| {
                    let sid = id_remove.clone();
                    // 按钮只改表单草案，但「保存」会把它写回服务端；误点无处撤销，先确认再删。
                    let loc = locale.get_untracked();
                    spawn_local(async move {
                        let ok = crate::confirm_dialog::confirm_in_page(
                            &i18n::settings_mcp_remove_confirm(loc, &sid),
                            i18n::settings_mcp_remove(loc),
                            i18n::ide_confirm_cancel(loc),
                        )
                        .await;
                        if ok {
                            set_file.update(|f| f.servers.retain(|s| s.id != sid));
                        }
                    });
                }
            >
                {move || i18n::settings_mcp_remove(locale.get())}
            </button>
        </div>
    }
}
