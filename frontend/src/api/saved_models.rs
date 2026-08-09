//! 侧栏「已保存模型」：非机密字段在 **`llm_overrides.saved_models`**；密钥在本机钥匙串/Keystore。

use leptos::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::client_llm_cache::{with_mem, with_mem_mut};
use super::client_llm_storage;
use super::llm_secrets_local::{
    PersistKind, get_saved_preset_api_key, saved_preset_api_key_is_set, set_client_llm_api_key,
    set_executor_llm_api_key, set_saved_preset_api_key_async,
};

fn default_preset_enabled() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedModelPreset {
    pub label: String,
    pub api_base: String,
    pub api_base_preset_select: String,
    pub model: String,
    pub temperature: String,
    pub llm_context_tokens: String,
    pub llm_thinking_mode: String,
    /// 仅本机会话草稿用；写入 `/user-data/llm-overrides` 前会清空。
    #[serde(default)]
    pub api_key: String,
    /// 本机是否已为该条目存过密钥（不回显明文）。
    #[serde(default)]
    pub has_api_key: bool,
    #[serde(default = "default_preset_enabled")]
    pub enabled: bool,
}

/// 旧版 llm-overrides 若含明文 `api_key`：迁入本机并清空 JSON 字段；`has_api_key` 只信本机。
pub async fn migrate_saved_models_secrets_to_local(mut saved: Vec<Value>) -> Vec<Value> {
    for v in &mut saved {
        let Ok(mut preset) = serde_json::from_value::<SavedModelPreset>(v.clone()) else {
            continue;
        };
        if !preset.api_key.trim().is_empty() {
            let _ = set_saved_preset_api_key_async(
                &preset.label,
                &preset.api_base,
                &preset.model,
                &preset.api_key,
            )
            .await;
            preset.api_key.clear();
        }
        preset.has_api_key =
            saved_preset_api_key_is_set(&preset.label, &preset.api_base, &preset.model);
        if let Ok(cleaned) = serde_json::to_value(&preset) {
            *v = cleaned;
        }
    }
    saved
}

fn preset_for_server_put(p: &SavedModelPreset) -> SavedModelPreset {
    let has = saved_preset_api_key_is_set(&p.label, &p.api_base, &p.model)
        || !p.api_key.trim().is_empty();
    SavedModelPreset {
        label: p.label.clone(),
        api_base: p.api_base.clone(),
        api_base_preset_select: p.api_base_preset_select.clone(),
        model: p.model.clone(),
        temperature: p.temperature.clone(),
        llm_context_tokens: p.llm_context_tokens.clone(),
        llm_thinking_mode: p.llm_thinking_mode.clone(),
        api_key: String::new(),
        has_api_key: has,
        enabled: p.enabled,
    }
}

#[must_use]
pub fn load_saved_model_presets_from_storage() -> Vec<SavedModelPreset> {
    with_mem(|m| {
        m.saved_models
            .iter()
            .filter_map(|v| serde_json::from_value::<SavedModelPreset>(v.clone()).ok())
            .map(|mut p| {
                p.has_api_key = saved_preset_api_key_is_set(&p.label, &p.api_base, &p.model);
                p.api_key.clear();
                p
            })
            .collect()
    })
}

pub fn persist_saved_model_presets_to_storage(
    presets: &[SavedModelPreset],
    loc: crate::i18n::Locale,
) -> Result<(), String> {
    // 同步路径：先更新非机密列表；密钥走后台异步（设置页应使用 async 变体）。
    let vals: Vec<Value> = presets
        .iter()
        .map(preset_for_server_put)
        .filter_map(|p| serde_json::to_value(p).ok())
        .collect();
    with_mem_mut(|m| m.saved_models = vals);
    client_llm_storage::flush_llm_overrides_to_server(loc);
    let owned = presets.to_vec();
    leptos::task::spawn_local(async move {
        let _ = persist_saved_model_preset_secrets_async(&owned).await;
    });
    Ok(())
}

async fn persist_saved_model_preset_secrets_async(
    presets: &[SavedModelPreset],
) -> Result<Option<PersistKind>, String> {
    let mut kind_acc: Option<PersistKind> = None;
    for p in presets {
        let key_upd = if !p.api_key.trim().is_empty() {
            Some(p.api_key.as_str())
        } else if !p.has_api_key {
            Some("")
        } else {
            None
        };
        if let Some(k) = key_upd {
            let kind = set_saved_preset_api_key_async(&p.label, &p.api_base, &p.model, k).await?;
            kind_acc = Some(match (kind_acc, kind) {
                (None, k) => k,
                (Some(PersistKind::BrowserInsecure), _) | (_, PersistKind::BrowserInsecure) => {
                    PersistKind::BrowserInsecure
                }
                (Some(PersistKind::Durable), PersistKind::Durable) => PersistKind::Durable,
            });
        }
    }
    Ok(kind_acc)
}

/// 设置保存：同步写 llm-overrides，并等待本机密钥落盘结果。
pub async fn persist_saved_model_presets_to_storage_async(
    presets: &[SavedModelPreset],
    loc: crate::i18n::Locale,
) -> Result<Option<PersistKind>, String> {
    let kind = persist_saved_model_preset_secrets_async(presets).await?;
    let vals: Vec<Value> = presets
        .iter()
        .map(preset_for_server_put)
        .filter_map(|p| serde_json::to_value(p).ok())
        .collect();
    with_mem_mut(|m| m.saved_models = vals);
    client_llm_storage::flush_llm_overrides_to_server(loc);
    Ok(kind)
}

/// 将一条已保存预设应用到「主 LLM」草稿；若本机有密钥则写入主模型密钥槽。
pub fn apply_saved_model_preset_to_main_fields(
    preset: &SavedModelPreset,
    drafts: MainLlmDraftSignals,
) {
    let MainLlmDraftSignals {
        llm_api_base_draft,
        llm_api_base_preset_select,
        llm_model_draft,
        llm_temperature_draft,
        llm_context_tokens_draft,
        llm_thinking_mode_draft,
    } = drafts;
    llm_api_base_draft.set(preset.api_base.clone());
    llm_api_base_preset_select.set(preset.api_base_preset_select.clone());
    llm_model_draft.set(preset.model.clone());
    llm_temperature_draft.set(preset.temperature.clone());
    llm_context_tokens_draft.set(preset.llm_context_tokens.clone());
    llm_thinking_mode_draft.set(preset.llm_thinking_mode.clone());

    let key = get_saved_preset_api_key(&preset.label, &preset.api_base, &preset.model);
    if !key.trim().is_empty() {
        with_mem_mut(|m| m.api_key = key.clone());
        set_client_llm_api_key(&key);
    }
}

#[derive(Clone, Copy)]
pub struct MainLlmDraftSignals {
    pub llm_api_base_draft: RwSignal<String>,
    pub llm_api_base_preset_select: RwSignal<String>,
    pub llm_model_draft: RwSignal<String>,
    pub llm_temperature_draft: RwSignal<String>,
    pub llm_context_tokens_draft: RwSignal<String>,
    pub llm_thinking_mode_draft: RwSignal<String>,
}

/// 将一条已保存预设应用到「执行器 LLM」草稿；若本机有密钥则写入执行器密钥槽。
pub fn apply_saved_model_preset_to_executor_fields(
    preset: &SavedModelPreset,
    drafts: ExecutorLlmDraftSignals,
) {
    let ExecutorLlmDraftSignals {
        executor_llm_api_base_draft,
        executor_llm_api_base_preset_select,
        executor_llm_model_draft,
    } = drafts;
    executor_llm_api_base_draft.set(preset.api_base.clone());
    executor_llm_api_base_preset_select.set(preset.api_base_preset_select.clone());
    executor_llm_model_draft.set(preset.model.clone());

    let key = get_saved_preset_api_key(&preset.label, &preset.api_base, &preset.model);
    if !key.trim().is_empty() {
        with_mem_mut(|m| m.executor_api_key = key.clone());
        set_executor_llm_api_key(&key);
    }
}

#[derive(Clone, Copy)]
pub struct ExecutorLlmDraftSignals {
    pub executor_llm_api_base_draft: RwSignal<String>,
    pub executor_llm_api_base_preset_select: RwSignal<String>,
    pub executor_llm_model_draft: RwSignal<String>,
}

/// 查找与给定 `api_base` / `api_base_preset_select` / `model` 完全一致的首条已保存预设下标。
#[must_use]
pub fn matching_saved_preset_index(
    presets: &[SavedModelPreset],
    api_base: &str,
    api_base_preset_select: &str,
    model: &str,
) -> Option<usize> {
    presets.iter().position(|p| {
        p.enabled
            && p.api_base.trim() == api_base.trim()
            && p.api_base_preset_select.trim() == api_base_preset_select.trim()
            && p.model.trim() == model.trim()
    })
}
