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
        m.llm_thinking_mode = file.client_llm.llm_thinking_mode.unwrap_or_default();
        m.executor_api_base = file.executor_llm.api_base.unwrap_or_default();
        m.executor_model = file.executor_llm.model.unwrap_or_default();
        m.execution_mode = file.execution_mode.unwrap_or_default();
        m.saved_models = saved;
        m.api_key = client_key;
        m.executor_api_key = executor_key;
    });
}
