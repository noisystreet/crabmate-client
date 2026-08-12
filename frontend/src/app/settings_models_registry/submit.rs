//! 「+」/编辑弹窗提交：从草稿构建预设、合并列表并持久化。

use std::sync::Arc;

use leptos::prelude::*;

use crate::api::SavedModelPreset;
use crate::i18n::{self, Locale};
use crate::settings_llm_fields::LlmSavedPresetApplyTarget;

use super::persist::try_persist_saved_presets_with_feedback;
use super::{ManualPresetDraft, RegistryPresetDialogKind, try_build_manual_saved_preset};

#[derive(Clone)]
pub(super) struct RegistryAddFormActionSignals {
    pub locale: RwSignal<Locale>,
    pub saved_model_presets: RwSignal<Vec<SavedModelPreset>>,
    pub dialog_mode: RwSignal<Option<RegistryPresetDialogKind>>,
    pub form_error: RwSignal<Option<String>>,
    pub new_api_base: RwSignal<String>,
    pub new_label: RwSignal<String>,
    pub new_model_id: RwSignal<String>,
    pub new_api_key: RwSignal<String>,
    pub new_ctx_tokens: RwSignal<String>,
    pub new_temperature: RwSignal<String>,
    pub new_thinking_mode: RwSignal<String>,
    pub sync_saved_presets_baseline: Arc<dyn Fn() + Send + Sync>,
    pub llm_settings_feedback: RwSignal<Option<String>>,
    pub apply_on_save: Option<LlmSavedPresetApplyTarget>,
}

fn draft_from_form_signals(s: &RegistryAddFormActionSignals) -> ManualPresetDraft {
    ManualPresetDraft {
        api_base: s.new_api_base.get_untracked(),
        label: s.new_label.get_untracked(),
        model_id: s.new_model_id.get_untracked(),
        api_key: s.new_api_key.get_untracked(),
        ctx_tokens: s.new_ctx_tokens.get_untracked(),
        temperature: s.new_temperature.get_untracked(),
        thinking_mode: s.new_thinking_mode.get_untracked(),
    }
}

fn enabled_for_dialog_mode(
    mode: Option<RegistryPresetDialogKind>,
    presets: RwSignal<Vec<SavedModelPreset>>,
) -> bool {
    match mode {
        Some(RegistryPresetDialogKind::Edit(i)) => {
            presets.with_untracked(|v| v.get(i).map(|p| p.enabled).unwrap_or(true))
        }
        Some(RegistryPresetDialogKind::Add) | None => true,
    }
}

fn preserve_edit_has_api_key_if_blank(
    mode: Option<RegistryPresetDialogKind>,
    presets: RwSignal<Vec<SavedModelPreset>>,
    preset: &mut SavedModelPreset,
) {
    if let Some(RegistryPresetDialogKind::Edit(i)) = mode
        && preset.api_key.trim().is_empty()
    {
        preset.has_api_key =
            presets.with_untracked(|v| v.get(i).is_some_and(|old| old.has_api_key));
    }
}

fn should_apply_saved_preset(
    mode: Option<RegistryPresetDialogKind>,
    apply_on_save: Option<LlmSavedPresetApplyTarget>,
    presets: RwSignal<Vec<SavedModelPreset>>,
) -> bool {
    match (mode, apply_on_save) {
        (Some(RegistryPresetDialogKind::Add), Some(_)) => true,
        (Some(RegistryPresetDialogKind::Edit(i)), Some(target)) => presets.with_untracked(|v| {
            v.get(i).is_some_and(|old| {
                crate::settings_llm_fields::llm_drafts_match_saved_preset(target, old)
            })
        }),
        _ => false,
    }
}

/// 将 `preset` 写入 `next`；模式无效时返回 `false`。
fn merge_preset_into_list(
    mode: Option<RegistryPresetDialogKind>,
    next: &mut Vec<SavedModelPreset>,
    preset: SavedModelPreset,
) -> bool {
    match mode {
        Some(RegistryPresetDialogKind::Add) => {
            next.push(preset);
            true
        }
        Some(RegistryPresetDialogKind::Edit(i)) if i < next.len() => {
            next[i] = preset;
            true
        }
        Some(RegistryPresetDialogKind::Edit(_)) | None => false,
    }
}

/// 校验并持久化；成功返回 `true`（调用方关闭弹窗并重置字段）。
pub(super) fn submit_registry_add_form(s: &RegistryAddFormActionSignals) -> bool {
    let d = draft_from_form_signals(s);
    let mode = s.dialog_mode.get_untracked();
    let enabled = enabled_for_dialog_mode(mode, s.saved_model_presets);
    let mut preset = match try_build_manual_saved_preset(&d, enabled) {
        Ok(p) => p,
        Err(()) => {
            s.form_error.set(Some(
                i18n::settings_models_validation_required(s.locale.get()).to_string(),
            ));
            return false;
        }
    };
    preserve_edit_has_api_key_if_blank(mode, s.saved_model_presets, &mut preset);
    s.form_error.set(None);
    let loc = s.locale.get_untracked();
    let mut next = s.saved_model_presets.with_untracked(|v| v.clone());
    let applied = preset.clone();
    let should_apply = should_apply_saved_preset(mode, s.apply_on_save, s.saved_model_presets);
    if !merge_preset_into_list(mode, &mut next, preset) {
        return false;
    }
    if !try_persist_saved_presets_with_feedback(
        next,
        loc,
        s.saved_model_presets,
        &s.sync_saved_presets_baseline,
        s.llm_settings_feedback,
    ) {
        return false;
    }
    if should_apply && let Some(target) = s.apply_on_save {
        crate::settings_llm_fields::apply_llm_saved_preset_pick(target, &applied);
    }
    true
}
