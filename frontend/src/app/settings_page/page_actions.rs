//! 设置页「放弃更改 / 保存全部」副作用（从 `SettingsPageView` 拆出以降低 nloc 棘轮）。

use leptos::prelude::*;

use super::form_snapshot::{SettingsPageDraftSignals, form_current_untracked};
use crate::app::settings_commit::{
    CommitAllSettingsInput, commit_all_settings, commit_settings_success_message,
};
use crate::app::settings_form_state::SettingsDirtyBaselines;
use crate::app::settings_mcp_status::{McpSaveJob, McpSettingsPageState, spawn_save_mcp};
use crate::i18n::{self, Locale};

/// `discard_to_baselines` 入参（单结构体以满足 `fn-param` 棘轮）。
#[derive(Clone, Copy)]
pub(crate) struct DiscardToBaselinesCtx {
    pub baselines: SettingsDirtyBaselines,
    pub drafts: SettingsPageDraftSignals,
    pub llm_settings_feedback: RwSignal<Option<String>>,
    pub executor_llm_settings_feedback: RwSignal<Option<String>>,
}

pub(crate) fn discard_to_baselines(ctx: DiscardToBaselinesCtx) {
    let DiscardToBaselinesCtx {
        baselines,
        drafts,
        llm_settings_feedback,
        executor_llm_settings_feedback,
    } = ctx;

    let (bl, bt, bbd) = baselines.appearance.get_value();
    drafts.appearance_locale.set(bl);
    drafts.appearance_theme.set(bt);
    drafts.appearance_bg_decor.set(bbd);

    let (bb, bp, bm, btemp, bct, btm, bh) = baselines.llm.get_value();
    drafts.llm_api_base_draft.set(bb);
    drafts.llm_api_base_preset_select.set(bp);
    drafts.llm_model_draft.set(bm);
    drafts.llm_temperature_draft.set(btemp);
    drafts.llm_context_tokens_draft.set(bct);
    drafts.llm_thinking_mode_draft.set(btm);
    drafts.llm_has_saved_key.set(bh);
    drafts.llm_api_key_draft.set(String::new());

    let (eb, ep, em, eh) = baselines.executor.get_value();
    drafts.executor_llm_api_base_draft.set(eb);
    drafts.executor_llm_api_base_preset_select.set(ep);
    drafts.executor_llm_model_draft.set(em);
    drafts.executor_llm_has_saved_key.set(eh);
    drafts.executor_llm_api_key_draft.set(String::new());

    drafts
        .readonly_tool_ttl_cache_follow_server
        .set(baselines.readonly_tool_ttl_cache_follow_server.get_value());

    drafts
        .saved_model_presets
        .set(baselines.saved_model_presets.get_value());

    drafts.clear_client_key_intent.set(false);
    drafts.clear_executor_key_intent.set(false);
    llm_settings_feedback.set(None);
    executor_llm_settings_feedback.set(None);
}

/// `try_save_all_settings` 入参（单结构体以满足 `fn-param` 棘轮）。
#[derive(Clone, Copy)]
pub(crate) struct SaveAllSettingsCtx {
    pub dirty: Memo<bool>,
    /// 保存进行中：防止并发重复提交，并驱动「保存中」禁用态。
    pub save_busy: RwSignal<bool>,
    pub appearance_locale: RwSignal<Locale>,
    pub locale: RwSignal<Locale>,
    pub theme: RwSignal<String>,
    pub bg_decor: RwSignal<bool>,
    pub drafts: SettingsPageDraftSignals,
    pub llm_settings_feedback: RwSignal<Option<String>>,
    pub executor_llm_settings_feedback: RwSignal<Option<String>>,
    pub client_llm_storage_tick: RwSignal<u64>,
    pub baselines: SettingsDirtyBaselines,
    pub mcp: McpSettingsPageState,
}

pub(crate) fn try_save_all_settings(ctx: SaveAllSettingsCtx) {
    let SaveAllSettingsCtx {
        dirty,
        save_busy,
        appearance_locale,
        locale,
        theme,
        bg_decor,
        drafts,
        llm_settings_feedback,
        executor_llm_settings_feedback,
        client_llm_storage_tick,
        baselines,
        mcp,
    } = ctx;

    if save_busy.get_untracked() {
        return;
    }
    llm_settings_feedback.set(None);
    executor_llm_settings_feedback.set(None);
    if !dirty.get() {
        llm_settings_feedback.set(Some(
            i18n::settings_nothing_to_save(appearance_locale.get()).to_string(),
        ));
        return;
    }
    let form_current = form_current_untracked(drafts);
    let form_dirty = baselines.is_dirty(&form_current);
    let mcp_dirty = mcp.is_dirty_untracked();
    let ui_locale = appearance_locale.get();

    if form_dirty {
        let appearance_locale_v = drafts.appearance_locale.get();
        let appearance_theme_v = drafts.appearance_theme.get();
        let appearance_bg_decor_v = drafts.appearance_bg_decor.get();
        let client_base = drafts.llm_api_base_draft.get();
        let client_model = drafts.llm_model_draft.get();
        let client_temperature = drafts.llm_temperature_draft.get();
        let client_ctx = drafts.llm_context_tokens_draft.get();
        let client_think = drafts.llm_thinking_mode_draft.get();
        let client_key = drafts.llm_api_key_draft.get();
        let executor_base = drafts.executor_llm_api_base_draft.get();
        let executor_model = drafts.executor_llm_model_draft.get();
        let executor_key = drafts.executor_llm_api_key_draft.get();
        let readonly_ttl = drafts.readonly_tool_ttl_cache_follow_server.get();
        let clear_client = drafts.clear_client_key_intent.get();
        let clear_executor = drafts.clear_executor_key_intent.get();
        let saved_presets_owned = drafts.saved_model_presets.get();
        save_busy.set(true);
        leptos::task::spawn_local(async move {
            let result = commit_all_settings(CommitAllSettingsInput {
                ui_locale,
                appearance_locale: appearance_locale_v,
                appearance_theme: appearance_theme_v,
                appearance_bg_decor: appearance_bg_decor_v,
                locale,
                theme,
                bg_decor,
                client_base: client_base.as_str(),
                client_model: client_model.as_str(),
                client_temperature: client_temperature.as_str(),
                client_llm_context_tokens: client_ctx.as_str(),
                client_llm_thinking_mode: client_think.as_str(),
                client_api_key_draft: client_key.as_str(),
                executor_base: executor_base.as_str(),
                executor_model: executor_model.as_str(),
                executor_api_key_draft: executor_key.as_str(),
                readonly_tool_ttl_cache_follow_server: readonly_ttl,
                clear_client_llm_key: clear_client,
                clear_executor_llm_key: clear_executor,
                llm_api_key_draft: drafts.llm_api_key_draft,
                llm_has_saved_key: drafts.llm_has_saved_key,
                executor_llm_api_key_draft: drafts.executor_llm_api_key_draft,
                executor_llm_has_saved_key: drafts.executor_llm_has_saved_key,
                client_llm_storage_tick,
                saved_model_presets: saved_presets_owned.as_slice(),
            })
            .await;
            match result {
                Ok(kind) => {
                    baselines.refresh_from_current(&form_current_untracked(drafts));
                    drafts.clear_client_key_intent.set(false);
                    drafts.clear_executor_key_intent.set(false);
                    if mcp_dirty {
                        mcp.set_feedback.set(None);
                        spawn_save_mcp(McpSaveJob {
                            loc: ui_locale,
                            pending_import: mcp.import_json.get_untracked(),
                            import_json: mcp.import_json,
                            ctx: mcp.as_row_ctx(appearance_locale),
                            set_feedback: mcp.set_feedback,
                        });
                    }
                    llm_settings_feedback
                        .set(Some(commit_settings_success_message(kind, ui_locale)));
                }
                Err(e) => {
                    llm_settings_feedback.set(Some(e));
                }
            }
            save_busy.set(false);
        });
        return;
    }

    if mcp_dirty {
        mcp.set_feedback.set(None);
        spawn_save_mcp(McpSaveJob {
            loc: ui_locale,
            pending_import: mcp.import_json.get_untracked(),
            import_json: mcp.import_json,
            ctx: mcp.as_row_ctx(appearance_locale),
            set_feedback: mcp.set_feedback,
        });
        llm_settings_feedback.set(Some(i18n::settings_save_all_ok(ui_locale).to_string()));
    }
}
