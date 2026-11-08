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
    // 确认框在途时按钮置灰并忽略重复点击。
    let remove_pending = RwSignal::new(false);

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
                prop:disabled=move || busy.get() || remove_pending.get()
                on:click=move |_| {
                    if remove_pending.get_untracked() {
                        return;
                    }
                    let sid = id_remove.clone();
                    // 按钮只改表单草案，但「保存」会把它写回服务端；误点无处撤销，先确认再删。
                    let loc = locale.get_untracked();
                    remove_pending.set(true);
                    spawn_local(async move {
                        let ok = crate::confirm_dialog::confirm_in_page(
                            &i18n::settings_mcp_remove_confirm(loc, &sid),
                            i18n::settings_mcp_remove(loc),
                            i18n::ide_confirm_cancel(loc),
                        )
                        .await;
                        remove_pending.set(false);
                        if ok {
                            set_file.update(|f| remove_server_by_id(f, &sid));
                        }
                    });
                }
            >
                {move || i18n::settings_mcp_remove(locale.get())}
            </button>
        </div>
    }
}

/// 确认后从草案里移除该 id 的服务器。
///
/// 抽成独立函数：组件没有测试框架，把「确认 → 移除」做成可直接单测的纯函数，
/// 才能钉住「取消则草案原样保留」这条语义（取消时根本不会调用它）。
fn remove_server_by_id(file: &mut crate::api::user_data::McpServersFileDto, id: &str) {
    file.servers.retain(|s| s.id != id);
}

#[cfg(test)]
mod tests {
    use super::remove_server_by_id;
    use crate::api::user_data::{McpServerEntryDto, McpServersFileDto};

    fn entry(id: &str) -> McpServerEntryDto {
        McpServerEntryDto {
            id: id.to_string(),
            ..Default::default()
        }
    }

    fn draft(ids: &[&str]) -> McpServersFileDto {
        McpServersFileDto {
            servers: ids.iter().map(|id| entry(id)).collect(),
            ..Default::default()
        }
    }

    /// 只删被确认的那一行，其余服务器与文件级配置都不得被连带改动。
    #[test]
    fn removes_only_the_confirmed_server() {
        let mut file = draft(&["alpha", "beta"]);
        file.tool_timeout_secs = 42;
        remove_server_by_id(&mut file, "alpha");
        assert_eq!(file.servers.len(), 1);
        assert_eq!(file.servers[0].id, "beta");
        assert_eq!(file.tool_timeout_secs, 42, "删除服务器不得改文件级配置");
    }

    /// id 对不上（确认期间草案被别处改过）时不得误删任何行。
    #[test]
    fn unknown_id_leaves_draft_untouched() {
        let mut file = draft(&["alpha"]);
        remove_server_by_id(&mut file, "missing");
        assert_eq!(file.servers.len(), 1);
        assert_eq!(file.servers[0].id, "alpha");
    }
}
