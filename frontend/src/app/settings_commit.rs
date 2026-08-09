//! 设置「保存全部」：将外观草稿与 LLM 草稿一次性写入本机存储，并同步全局信号。

use leptos::prelude::*;

use crate::api::client_llm_storage::{
    client_llm_storage_has_api_key, executor_llm_storage_has_api_key, merge_llm_persist_kinds,
    persist_client_llm_to_storage_async, persist_executor_llm_to_storage_async,
    persist_readonly_tool_ttl_cache_follow_server,
};
use crate::api::{PersistKind, persist_saved_model_presets_to_storage_async};
use crate::i18n::{self, Locale, store_locale_slug};

fn validate_temperature_override(raw: &str, loc: Locale) -> Result<(), String> {
    let t = raw.trim();
    if t.is_empty() {
        return Ok(());
    }
    let parsed = t
        .parse::<f64>()
        .map_err(|_| crate::i18n::settings_err_temperature_invalid(loc).to_string())?;
    if !parsed.is_finite() || !(0.0..=2.0).contains(&parsed) {
        return Err(crate::i18n::settings_err_temperature_range(loc).to_string());
    }
    Ok(())
}

fn validate_llm_context_tokens_override(raw: &str, loc: Locale) -> Result<(), String> {
    let t = raw.trim();
    if t.is_empty() {
        return Ok(());
    }
    let parsed = t
        .parse::<u64>()
        .map_err(|_| crate::i18n::settings_err_context_tokens_invalid(loc).to_string())?;
    if parsed > 10_000_000 {
        return Err(crate::i18n::settings_err_context_tokens_range(loc).to_string());
    }
    Ok(())
}

fn validate_llm_thinking_mode_override(raw: &str, loc: Locale) -> Result<(), String> {
    let t = raw.trim();
    if t.is_empty() || t == "server" || t == "on" || t == "off" {
        return Ok(());
    }
    Err(crate::i18n::settings_err_thinking_mode_invalid(loc).to_string())
}

fn api_key_update_from_clear_and_draft(clear: bool, draft: &str) -> Option<&str> {
    match (clear, draft.trim().is_empty()) {
        (true, _) => Some(""),
        (false, true) => None,
        (false, false) => Some(draft),
    }
}

fn feedback_for_persist_kind(kind: Option<PersistKind>, loc: Locale) -> String {
    match kind {
        Some(PersistKind::BrowserInsecure) => {
            i18n::settings_save_ok_browser_insecure_key(loc).to_string()
        }
        _ => i18n::settings_save_all_ok(loc).to_string(),
    }
}

/// 校验并写入 LLM / 执行模式 / TTL / 已保存模型列表（不含外观信号）。
async fn persist_settings_storage_payload(
    p: &CommitAllSettingsInput<'_>,
) -> Result<Option<PersistKind>, String> {
    validate_temperature_override(p.client_temperature, p.ui_locale)?;
    validate_llm_context_tokens_override(p.client_llm_context_tokens, p.ui_locale)?;
    validate_llm_thinking_mode_override(p.client_llm_thinking_mode, p.ui_locale)?;

    let presets_kind =
        persist_saved_model_presets_to_storage_async(p.saved_model_presets, p.ui_locale).await?;

    let client_key_upd =
        api_key_update_from_clear_and_draft(p.clear_client_llm_key, p.client_api_key_draft);
    let client_kind = persist_client_llm_to_storage_async(
        p.client_base,
        p.client_model,
        p.client_temperature,
        p.client_llm_context_tokens,
        p.client_llm_thinking_mode,
        client_key_upd,
        p.ui_locale,
    )
    .await?;

    let executor_key_upd =
        api_key_update_from_clear_and_draft(p.clear_executor_llm_key, p.executor_api_key_draft);
    let executor_kind = persist_executor_llm_to_storage_async(
        p.executor_base,
        p.executor_model,
        executor_key_upd,
        p.ui_locale,
    )
    .await?;
    persist_readonly_tool_ttl_cache_follow_server(
        p.readonly_tool_ttl_cache_follow_server,
        p.ui_locale,
    );
    Ok(merge_llm_persist_kinds(
        client_kind,
        executor_kind,
        presets_kind,
    ))
}

fn sync_shell_ui_after_settings_commit(p: &CommitAllSettingsInput<'_>) {
    p.locale.set(p.appearance_locale);
    store_locale_slug(p.appearance_locale.storage_slug());
    p.theme.set(p.appearance_theme.clone());
    p.bg_decor.set(p.appearance_bg_decor);

    p.llm_api_key_draft.set(String::new());
    p.executor_llm_api_key_draft.set(String::new());
    p.llm_has_saved_key.set(client_llm_storage_has_api_key());
    p.executor_llm_has_saved_key
        .set(executor_llm_storage_has_api_key());
    p.client_llm_storage_tick.update(|n| *n = n.wrapping_add(1));
}

/// 一次「保存全部设置」所需的表单快照与 UI 信号（避免长参数列表）。
pub struct CommitAllSettingsInput<'a> {
    pub ui_locale: Locale,
    pub appearance_locale: Locale,
    pub appearance_theme: String,
    pub appearance_bg_decor: bool,
    pub locale: RwSignal<Locale>,
    pub theme: RwSignal<String>,
    pub bg_decor: RwSignal<bool>,
    pub client_base: &'a str,
    pub client_model: &'a str,
    pub client_temperature: &'a str,
    pub client_llm_context_tokens: &'a str,
    pub client_llm_thinking_mode: &'a str,
    pub client_api_key_draft: &'a str,
    pub executor_base: &'a str,
    pub executor_model: &'a str,
    pub executor_api_key_draft: &'a str,
    pub readonly_tool_ttl_cache_follow_server: bool,
    pub clear_client_llm_key: bool,
    pub clear_executor_llm_key: bool,
    pub llm_api_key_draft: RwSignal<String>,
    pub llm_has_saved_key: RwSignal<bool>,
    pub executor_llm_api_key_draft: RwSignal<String>,
    pub executor_llm_has_saved_key: RwSignal<bool>,
    pub client_llm_storage_tick: RwSignal<u64>,
    /// 与主/执行器草稿分离存储的「已保存模型」列表（服务端 llm-overrides + 本机密钥）。
    pub saved_model_presets: &'a [crate::api::SavedModelPreset],
}

/// 将语言 / 主题 / 背景与（可选）LLM 覆盖写入存储，并更新全局 UI 信号。
///
/// 模型密钥先确认本机钥匙串/Keystore（或浏览器弱持久化）写入成功，再更新「已保存」态。
pub async fn commit_all_settings(
    p: CommitAllSettingsInput<'_>,
) -> Result<Option<PersistKind>, String> {
    let kind = persist_settings_storage_payload(&p).await?;

    sync_shell_ui_after_settings_commit(&p);

    Ok(kind)
}

/// 保存成功文案（含浏览器弱持久化提示）。
#[must_use]
pub fn commit_settings_success_message(kind: Option<PersistKind>, loc: Locale) -> String {
    feedback_for_persist_kind(kind, loc)
}
