//! MCP 设置页：全局开关、超时与保存/探测操作栏。

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use super::settings_mcp_status::{
    McpSaveJob, McpSettingsSignals, spawn_probe_all_mcp, spawn_save_mcp,
};
use super::settings_toggle_switch::SettingsToggleSwitch;
use crate::api::user_data::McpServersFileDto;
use crate::i18n::{self, Locale};

/// 超时输入框 id（整个 MCP 块只渲染一次，用静态 id 即可让 label 关联到控件）。
const TIMEOUT_INPUT_ID: &str = "settings-mcp-timeout-input";

fn event_input_value(ev: &leptos::ev::Event) -> Option<String> {
    ev.target()
        .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
        .map(|el| el.value())
}

#[component]
fn SettingsMcpTimeoutField(
    locale: RwSignal<Locale>,
    file: ReadSignal<McpServersFileDto>,
    set_file: WriteSignal<McpServersFileDto>,
) -> impl IntoView {
    let invalid_hint = RwSignal::new(None::<String>);
    view! {
        <div class="settings-field">
            <label class="settings-field-label" for=TIMEOUT_INPUT_ID>
                {move || i18n::settings_mcp_timeout_label(locale.get())}
            </label>
            <input
                id=TIMEOUT_INPUT_ID
                type="number"
                min="1"
                class="settings-text-input"
                data-testid="settings-mcp-timeout"
                prop:value=move || file.get().tool_timeout_secs.to_string()
                on:input=move |ev| {
                    // 非法输入（空 / 非整数 / < 1）不再静默保留旧值：提示并说明当前生效值未变。
                    let current = file.get_untracked().tool_timeout_secs;
                    match event_input_value(&ev).and_then(|v| v.trim().parse::<u64>().ok()) {
                        Some(n) if n >= 1 => {
                            invalid_hint.set(None);
                            set_file.update(|f| f.tool_timeout_secs = n);
                        }
                        _ => invalid_hint.set(Some(i18n::settings_mcp_timeout_invalid(
                            locale.get_untracked(),
                            current,
                        ))),
                    }
                }
            />
        </div>
        <Show when=move || invalid_hint.get().is_some()>
            <p class="settings-hint" role="status">
                {move || invalid_hint.get().unwrap_or_default()}
            </p>
        </Show>
    }
}

#[component]
fn SettingsMcpToolbarActions(
    locale: RwSignal<Locale>,
    import_json: RwSignal<String>,
    busy: ReadSignal<bool>,
    feedback: ReadSignal<Option<String>>,
    set_feedback: WriteSignal<Option<String>>,
    row_ctx: McpSettingsSignals,
) -> impl IntoView {
    view! {
        <div class="settings-mcp-actions">
            <button
                type="button"
                class="btn btn-primary btn-sm"
                data-testid="settings-mcp-save"
                prop:disabled=move || busy.get()
                on:click=move |_| {
                    set_feedback.set(None);
                    spawn_save_mcp(McpSaveJob {
                        loc: locale.get_untracked(),
                        pending_import: import_json.get_untracked(),
                        import_json,
                        ctx: row_ctx,
                        set_feedback,
                    });
                }
            >
                {move || i18n::settings_mcp_save(locale.get())}
            </button>
            <button
                type="button"
                class="btn btn-secondary btn-sm"
                data-testid="settings-mcp-probe-all"
                prop:disabled=move || busy.get()
                on:click=move |_| spawn_probe_all_mcp(row_ctx)
            >
                {move || i18n::settings_mcp_probe_all(locale.get())}
            </button>
        </div>
        <Show when=move || feedback.get().is_some()>
            <p class="settings-intro settings-mcp-feedback">{move || feedback.get().unwrap_or_default()}</p>
        </Show>
    }
}

#[component]
pub(crate) fn SettingsMcpBlockToolbar(
    locale: RwSignal<Locale>,
    file: ReadSignal<McpServersFileDto>,
    set_file: WriteSignal<McpServersFileDto>,
    import_json: RwSignal<String>,
    busy: ReadSignal<bool>,
    feedback: ReadSignal<Option<String>>,
    set_feedback: WriteSignal<Option<String>>,
    row_ctx: McpSettingsSignals,
) -> impl IntoView {
    view! {
        <SettingsToggleSwitch
            test_id="settings-mcp-global-enabled"
            checked=Signal::derive(move || file.get().global_enabled)
            label=Signal::derive(move || {
                i18n::settings_mcp_global_enabled_label(locale.get()).to_string()
            })
            on_toggle=move || set_file.update(|f| f.global_enabled = !f.global_enabled)
        />
        <SettingsMcpTimeoutField locale file set_file />
        <SettingsMcpToolbarActions
            locale
            import_json
            busy
            feedback
            set_feedback
            row_ctx
        />
    }
}
