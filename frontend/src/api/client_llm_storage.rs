//! 侧栏「本机模型」：进程内缓存 + 非机密覆盖文件；API 密钥存本机钥匙串/Keystore。

use serde_json::Value;

use crate::i18n::Locale;

use super::client_llm_cache::{self, with_mem, with_mem_mut};
use super::llm_secrets_local::{
    PersistKind, client_llm_api_key_is_set, executor_llm_api_key_is_set,
    set_client_llm_api_key_async, set_executor_llm_api_key_async,
};
use super::user_data::{LlmOverridesDto, put_llm_overrides};

pub async fn hydrate_client_llm_from_server(loc: Locale) {
    client_llm_cache::hydrate_from_server(loc).await;
}

fn opt_trim(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

/// 将当前进程内 LLM 缓存写入 `/user-data/llm-overrides`。
pub fn flush_llm_overrides_to_server(loc: Locale) {
    sync_llm_to_server_async(loc);
}

fn sync_llm_to_server_async(loc: Locale) {
    leptos::task::spawn_local(async move {
        let snap = with_mem(|m| {
            (
                m.api_base.clone(),
                m.model.clone(),
                m.temperature.clone(),
                m.llm_context_tokens.clone(),
                m.llm_thinking_mode.clone(),
                m.executor_api_base.clone(),
                m.executor_model.clone(),
                m.execution_mode.clone(),
                m.saved_models.clone(),
            )
        });
        let mut file = LlmOverridesDto::default();
        file.client_llm.api_base = opt_trim(&snap.0);
        file.client_llm.model = opt_trim(&snap.1);
        file.client_llm.temperature = opt_trim(&snap.2);
        file.client_llm.llm_context_tokens = opt_trim(&snap.3);
        file.client_llm.llm_thinking_mode = opt_trim(&snap.4);
        file.executor_llm.api_base = opt_trim(&snap.5);
        file.executor_llm.model = opt_trim(&snap.6);
        file.execution_mode = opt_trim(&snap.7);
        file.saved_models = snap.8;
        let _ = put_llm_overrides(&file, loc).await;
    });
}

/// 是否已配置主模型 API Key（本机安全存储水合后的内存）。
pub fn client_llm_storage_has_api_key() -> bool {
    with_mem(|m| !m.api_key.trim().is_empty()) || client_llm_api_key_is_set()
}

pub fn load_client_llm_text_fields_from_storage() -> (String, String, String, String, String) {
    with_mem(|m| {
        (
            m.api_base.clone(),
            m.model.clone(),
            m.temperature.clone(),
            m.llm_context_tokens.clone(),
            m.llm_thinking_mode.clone(),
        )
    })
}

fn merge_persist_kind(acc: &mut Option<PersistKind>, kind: PersistKind) {
    match (*acc, kind) {
        (None, k) => *acc = Some(k),
        (Some(PersistKind::Durable), PersistKind::BrowserInsecure)
        | (Some(PersistKind::BrowserInsecure), PersistKind::Durable) => {
            *acc = Some(PersistKind::BrowserInsecure);
        }
        _ => {}
    }
}

/// 持久化非机密字段 + 可选密钥；密钥写入失败时返回 Err（不假装已保存）。
pub async fn persist_client_llm_to_storage_async(
    api_base: &str,
    model: &str,
    temperature: &str,
    llm_context_tokens: &str,
    llm_thinking_mode: &str,
    api_key_update: Option<&str>,
    loc: Locale,
) -> Result<Option<PersistKind>, String> {
    with_mem_mut(|m| {
        m.api_base = api_base.trim().to_string();
        m.model = model.trim().to_string();
        m.temperature = temperature.trim().to_string();
        m.llm_context_tokens = llm_context_tokens.trim().to_string();
        m.llm_thinking_mode = llm_thinking_mode.trim().to_string();
    });
    sync_llm_to_server_async(loc);
    if let Some(k) = api_key_update {
        let kind = set_client_llm_api_key_async(k).await?;
        with_mem_mut(|m| m.api_key = k.trim().to_string());
        return Ok(Some(kind));
    }
    Ok(None)
}

/// 同步入口（slash 等）：密钥路径内部 spawn；非密钥字段立即写入。
pub fn persist_client_llm_to_storage(
    api_base: &str,
    model: &str,
    temperature: &str,
    llm_context_tokens: &str,
    llm_thinking_mode: &str,
    api_key_update: Option<&str>,
    loc: Locale,
) -> Result<(), String> {
    with_mem_mut(|m| {
        m.api_base = api_base.trim().to_string();
        m.model = model.trim().to_string();
        m.temperature = temperature.trim().to_string();
        m.llm_context_tokens = llm_context_tokens.trim().to_string();
        m.llm_thinking_mode = llm_thinking_mode.trim().to_string();
        if let Some(k) = api_key_update {
            m.api_key = k.trim().to_string();
        }
    });
    sync_llm_to_server_async(loc);
    if let Some(k) = api_key_update {
        let key = k.to_string();
        leptos::task::spawn_local(async move {
            let _ = set_client_llm_api_key_async(&key).await;
        });
    }
    Ok(())
}

pub async fn clear_client_llm_api_key_storage_async(_loc: Locale) -> Result<PersistKind, String> {
    let kind = set_client_llm_api_key_async("").await?;
    with_mem_mut(|m| m.api_key.clear());
    Ok(kind)
}

#[allow(dead_code)] // 同步 slash / 旧调用保留；设置页走 async
pub fn clear_client_llm_api_key_storage(loc: Locale) -> Result<(), String> {
    // 同步路径：乐观清内存，后台确认钥匙串（失败时下次水合可能恢复——slash 应改用 async）。
    with_mem_mut(|m| m.api_key.clear());
    leptos::task::spawn_local(async move {
        let _ = clear_client_llm_api_key_storage_async(loc).await;
    });
    Ok(())
}

pub fn client_llm_json_for_chat_body() -> Option<Value> {
    with_mem(|m| {
        let mut map = serde_json::Map::new();
        if !m.api_base.trim().is_empty() {
            map.insert("api_base".into(), Value::String(m.api_base.clone()));
        }
        if !m.model.trim().is_empty() {
            map.insert("model".into(), Value::String(m.model.clone()));
        }
        if let Ok(n) = m.llm_context_tokens.trim().parse::<u64>() {
            if n > 0 {
                map.insert("llm_context_tokens".into(), Value::Number(n.into()));
            }
        }
        let tm = m.llm_thinking_mode.trim();
        if tm == "on" || tm == "off" {
            map.insert("llm_thinking_mode".into(), Value::String(tm.to_string()));
        }
        let key = if m.api_key.trim().is_empty() {
            super::llm_secrets_local::client_llm_api_key()
        } else {
            m.api_key.clone()
        };
        if !key.trim().is_empty() {
            map.insert("api_key".into(), Value::String(key));
        }
        if map.is_empty() {
            None
        } else {
            Some(Value::Object(map))
        }
    })
}

pub fn chat_temperature_override_from_storage() -> Option<f64> {
    let raw = with_mem(|m| m.temperature.clone());
    let parsed = raw.trim().parse::<f64>().ok()?;
    if !parsed.is_finite() || !(0.0..=2.0).contains(&parsed) {
        return None;
    }
    Some(parsed)
}

pub fn executor_llm_json_for_chat_body() -> Option<Value> {
    with_mem(|m| {
        let mut map = serde_json::Map::new();
        // 官方 Client 的执行轮与主轮强制共用同一模型身份，避免工具后切换到陈旧的
        // executor 密钥或端点；服务端仍保留兼容字段供其它调用方使用。
        if !m.api_base.trim().is_empty() {
            map.insert("api_base".into(), Value::String(m.api_base.clone()));
        }
        if !m.model.trim().is_empty() {
            map.insert("model".into(), Value::String(m.model.clone()));
        }
        let key = if m.api_key.trim().is_empty() {
            super::llm_secrets_local::client_llm_api_key()
        } else {
            m.api_key.clone()
        };
        if !key.trim().is_empty() {
            map.insert("api_key".into(), Value::String(key));
        }
        if map.is_empty() {
            None
        } else {
            Some(Value::Object(map))
        }
    })
}

pub fn executor_llm_storage_has_api_key() -> bool {
    with_mem(|m| !m.executor_api_key.trim().is_empty()) || executor_llm_api_key_is_set()
}

pub fn load_executor_llm_text_fields_from_storage() -> (String, String) {
    with_mem(|m| (m.executor_api_base.clone(), m.executor_model.clone()))
}

pub async fn persist_executor_llm_to_storage_async(
    api_base: &str,
    model: &str,
    api_key_update: Option<&str>,
    loc: Locale,
) -> Result<Option<PersistKind>, String> {
    with_mem_mut(|m| {
        m.executor_api_base = api_base.trim().to_string();
        m.executor_model = model.trim().to_string();
    });
    sync_llm_to_server_async(loc);
    if let Some(k) = api_key_update {
        let kind = set_executor_llm_api_key_async(k).await?;
        with_mem_mut(|m| m.executor_api_key = k.trim().to_string());
        return Ok(Some(kind));
    }
    Ok(None)
}

#[allow(dead_code)] // 同步路径保留；设置页走 async
pub fn persist_executor_llm_to_storage(
    api_base: &str,
    model: &str,
    api_key_update: Option<&str>,
    loc: Locale,
) -> Result<(), String> {
    with_mem_mut(|m| {
        m.executor_api_base = api_base.trim().to_string();
        m.executor_model = model.trim().to_string();
        if let Some(k) = api_key_update {
            m.executor_api_key = k.trim().to_string();
        }
    });
    sync_llm_to_server_async(loc);
    if let Some(k) = api_key_update {
        let key = k.to_string();
        leptos::task::spawn_local(async move {
            let _ = set_executor_llm_api_key_async(&key).await;
        });
    }
    Ok(())
}

#[allow(dead_code)]
pub async fn clear_executor_llm_api_key_storage_async(_loc: Locale) -> Result<PersistKind, String> {
    let kind = set_executor_llm_api_key_async("").await?;
    with_mem_mut(|m| m.executor_api_key.clear());
    Ok(kind)
}

#[allow(dead_code)]
pub fn clear_executor_llm_api_key_storage(loc: Locale) -> Result<(), String> {
    with_mem_mut(|m| m.executor_api_key.clear());
    leptos::task::spawn_local(async move {
        let _ = clear_executor_llm_api_key_storage_async(loc).await;
    });
    Ok(())
}

pub fn load_readonly_tool_ttl_cache_follow_server_from_memory() -> bool {
    with_mem(|m| m.readonly_ttl_follow_server)
}

pub fn set_readonly_tool_ttl_cache_follow_server_in_memory(follow: bool) {
    with_mem_mut(|m| m.readonly_ttl_follow_server = follow);
}

pub fn persist_readonly_tool_ttl_cache_follow_server(follow: bool, loc: Locale) {
    set_readonly_tool_ttl_cache_follow_server_in_memory(follow);
    leptos::task::spawn_local(async move {
        let Ok(mut prefs) = super::user_data::fetch_user_data_prefs(loc).await else {
            return;
        };
        prefs.disable_readonly_tool_ttl_cache = Some(!follow);
        let _ = super::user_data::put_user_data_prefs(&prefs, loc).await;
    });
}

/// 兼容旧名。
pub fn load_readonly_tool_ttl_cache_follow_server_from_storage() -> bool {
    load_readonly_tool_ttl_cache_follow_server_from_memory()
}

/// 合并进 `/chat/stream` 的 `readonly_tool_ttl_cache_secs`（关闭时返回 `Some(0)`）。
pub fn readonly_tool_ttl_cache_secs_for_chat_body() -> Option<u64> {
    if load_readonly_tool_ttl_cache_follow_server_from_memory() {
        None
    } else {
        Some(0)
    }
}

/// 供设置「保存全部」汇总密钥落盘结果。
pub fn merge_llm_persist_kinds(
    client: Option<PersistKind>,
    executor: Option<PersistKind>,
    presets: Option<PersistKind>,
) -> Option<PersistKind> {
    let mut acc = None;
    if let Some(k) = client {
        merge_persist_kind(&mut acc, k);
    }
    if let Some(k) = executor {
        merge_persist_kind(&mut acc, k);
    }
    if let Some(k) = presets {
        merge_persist_kind(&mut acc, k);
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executor_chat_body_uses_primary_model_identity() {
        with_mem_mut(|m| {
            m.api_base = "https://primary.example/v1".to_string();
            m.model = "primary-model".to_string();
            m.api_key = "sk-primary".to_string();
            m.executor_api_base = "https://stale.example/v1".to_string();
            m.executor_model = "stale-model".to_string();
            m.executor_api_key = "Bearer sk-stale".to_string();
        });

        let body = executor_llm_json_for_chat_body().expect("primary identity should be present");
        assert_eq!(body["api_base"], "https://primary.example/v1");
        assert_eq!(body["model"], "primary-model");
        assert_eq!(body["api_key"], "sk-primary");
        assert!(!body.to_string().contains("stale"));
        with_mem_mut(|m| *m = Default::default());
    }
}
