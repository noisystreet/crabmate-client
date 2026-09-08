//! 进程内 LLM 覆盖缓存（非机密：`/user-data/llm-overrides`；密钥：本机钥匙串/Keystore）。

use std::cell::RefCell;

use serde_json::Value;

thread_local! {
    static LLM_MEM: RefCell<LlmMem> = RefCell::new(LlmMem::default());
}

#[derive(Clone, Default)]
pub(crate) struct LlmMem {
    pub(crate) api_base: String,
    pub(crate) model: String,
    pub(crate) temperature: String,
    pub(crate) llm_context_tokens: String,
    pub(crate) llm_thinking_mode: String,
    pub(crate) api_key: String,
    pub(crate) executor_api_base: String,
    pub(crate) executor_model: String,
    pub(crate) executor_api_key: String,
    pub(crate) execution_mode: String,
    pub(crate) readonly_ttl_follow_server: bool,
    pub(crate) saved_models: Vec<Value>,
}

pub fn with_mem<R>(f: impl FnOnce(&LlmMem) -> R) -> R {
    LLM_MEM.with(|c| f(&c.borrow()))
}

pub fn with_mem_mut<R>(f: impl FnOnce(&mut LlmMem) -> R) -> R {
    LLM_MEM.with(|c| f(&mut c.borrow_mut()))
}

/// 思考模式本机两态（`on` / `off`）：仅非机密偏好，存 webview `localStorage`；
/// 服务端 `/user-data/llm-overrides` 不再存储该选项。
#[cfg(target_arch = "wasm32")]
const THINKING_MODE_LS_KEY: &str = "crabmate-llm-thinking-mode";

/// 规范化思考模式：仅接受 `on`，其余（空 / 旧版 `server` / 未知值）一律视为 `off`。
pub fn normalize_llm_thinking_mode(raw: &str) -> &'static str {
    if raw.trim() == "on" { "on" } else { "off" }
}

#[cfg(target_arch = "wasm32")]
fn read_thinking_mode_local_storage() -> Option<String> {
    let w = super::browser::window()?;
    let storage = w.local_storage().ok().flatten()?;
    storage
        .get_item(THINKING_MODE_LS_KEY)
        .ok()
        .flatten()
        .map(|v| v.trim().to_string())
}

#[cfg(target_arch = "wasm32")]
fn write_thinking_mode_local_storage(normalized: &str) {
    let Some(w) = super::browser::window() else {
        return;
    };
    let Ok(Some(storage)) = w.local_storage() else {
        return;
    };
    let _ = storage.set_item(THINKING_MODE_LS_KEY, normalized);
}

/// 同步把本机 `localStorage` 的思考模式恢复进进程内缓存（缺失 / 非法值视为 `off`）。
pub fn hydrate_llm_thinking_mode_from_local() {
    #[cfg(target_arch = "wasm32")]
    {
        let normalized = read_thinking_mode_local_storage()
            .as_deref()
            .map(normalize_llm_thinking_mode)
            .unwrap_or("off");
        with_mem_mut(|m| m.llm_thinking_mode = normalized.to_string());
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        with_mem_mut(|m| {
            if m.llm_thinking_mode.trim() != "on" {
                m.llm_thinking_mode = "off".to_string();
            }
        });
    }
}

/// 思考模式持久化（写侧已规范化为 `on` / `off`；仅 wasm 落 `localStorage`）。
pub(crate) fn persist_llm_thinking_mode_local(raw: &str) {
    let normalized = normalize_llm_thinking_mode(raw);
    #[cfg(target_arch = "wasm32")]
    write_thinking_mode_local_storage(normalized);
    #[cfg(not(target_arch = "wasm32"))]
    let _ = normalized;
}

/// signals 构造期同步恢复：`localStorage` → mem，并返回初始两态值（`on` / `off`）。
pub fn load_llm_thinking_mode_initial() -> String {
    hydrate_llm_thinking_mode_from_local();
    with_mem(|m| normalize_llm_thinking_mode(&m.llm_thinking_mode).to_string())
}

pub async fn hydrate_from_server(loc: crate::i18n::Locale) {
    use super::github_secrets_local::hydrate_github_secrets_from_secure_store;
    use super::llm_secrets_local::{
        client_llm_api_key, executor_llm_api_key, hydrate_llm_secrets_from_secure_store,
    };
    use super::saved_models::migrate_saved_models_secrets_to_local;
    use super::user_data::fetch_llm_overrides;

    hydrate_llm_secrets_from_secure_store().await;
    hydrate_github_secrets_from_secure_store().await;

    let file = fetch_llm_overrides(loc).await.unwrap_or_default();
    let client_key = client_llm_api_key();
    let executor_key = executor_llm_api_key();
    let saved = migrate_saved_models_secrets_to_local(file.saved_models).await;

    with_mem_mut(|m| {
        m.api_base = file.client_llm.api_base.unwrap_or_default();
        m.model = file.client_llm.model.unwrap_or_default();
        m.temperature = file.client_llm.temperature.unwrap_or_default();
        m.llm_context_tokens = file.client_llm.llm_context_tokens.unwrap_or_default();
        // `llm_thinking_mode` 已改为本机两态（`localStorage`），服务端不再存储，水合不覆盖。
        m.executor_api_base = file.executor_llm.api_base.unwrap_or_default();
        m.executor_model = file.executor_llm.model.unwrap_or_default();
        m.execution_mode = file.execution_mode.unwrap_or_default();
        m.saved_models = saved;
        m.api_key = client_key;
        m.executor_api_key = executor_key;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_accepts_on_only_and_falls_back_to_off() {
        assert_eq!(normalize_llm_thinking_mode("on"), "on");
        assert_eq!(normalize_llm_thinking_mode(" on "), "on");
        assert_eq!(normalize_llm_thinking_mode("off"), "off");
        assert_eq!(normalize_llm_thinking_mode(""), "off");
        assert_eq!(normalize_llm_thinking_mode("server"), "off");
        assert_eq!(normalize_llm_thinking_mode("ON"), "off");
    }

    #[test]
    fn hydrate_local_normalizes_legacy_values_but_keeps_on() {
        with_mem_mut(|m| m.llm_thinking_mode = "server".to_string());
        hydrate_llm_thinking_mode_from_local();
        with_mem(|m| assert_eq!(m.llm_thinking_mode, "off"));

        with_mem_mut(|m| m.llm_thinking_mode = "on".to_string());
        hydrate_llm_thinking_mode_from_local();
        with_mem(|m| assert_eq!(m.llm_thinking_mode, "on"));
    }

    #[test]
    fn load_initial_returns_two_state_value() {
        with_mem_mut(|m| m.llm_thinking_mode = "server".to_string());
        assert_eq!(load_llm_thinking_mode_initial(), "off");
    }
}
