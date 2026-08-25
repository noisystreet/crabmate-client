//! 设置页壳级 `Effect`（从 `view` 拆出以降低 `SettingsPageView` 的 nloc 棘轮）。

use std::cell::Cell;
use std::rc::Rc;

use leptos::prelude::*;

use super::dom_preview::{apply_bg_decor_preview_to_dom, apply_theme_preview_to_dom};
use super::form_snapshot::{SettingsPageDraftSignals, form_current_untracked};
use super::hash_routing::{
    SettingsSection, clear_settings_hash_if_present, write_settings_section_to_hash,
};
use super::page_actions::{DiscardToBaselinesCtx, discard_to_baselines};
use crate::app::settings_form_state::{SettingsDirtyBaselines, sync_appearance_drafts_from_shell};
use crate::i18n::Locale;

/// 打开设置页时刷新 baseline 与外观草稿所需的信号（[`wire_settings_page_open_snapshot_effect`] 形参打包）。
#[derive(Clone, Copy)]
pub(super) struct SettingsPageOpenBaselineWire {
    pub settings_page: RwSignal<bool>,
    pub locale: RwSignal<Locale>,
    pub theme: RwSignal<String>,
    pub bg_decor: RwSignal<bool>,
    pub appearance_locale: RwSignal<Locale>,
    pub appearance_theme: RwSignal<String>,
    pub appearance_bg_decor: RwSignal<bool>,
    pub drafts: SettingsPageDraftSignals,
    pub clear_client_key_intent: RwSignal<bool>,
    pub clear_executor_key_intent: RwSignal<bool>,
    pub llm_settings_feedback: RwSignal<Option<String>>,
    pub executor_llm_settings_feedback: RwSignal<Option<String>>,
}

pub(super) fn wire_settings_page_hash_section_effect(
    settings_page: RwSignal<bool>,
    active_section: RwSignal<SettingsSection>,
) {
    Effect::new(move |_| {
        if !settings_page.get() {
            clear_settings_hash_if_present();
            return;
        }
        write_settings_section_to_hash(active_section.get());
    });
}

pub(super) fn wire_settings_page_open_snapshot_effect(
    args: SettingsPageOpenBaselineWire,
    baselines: SettingsDirtyBaselines,
) {
    let prev_open = Rc::new(Cell::new(false));
    Effect::new({
        let args = args;
        let baselines = baselines;
        let prev_open = Rc::clone(&prev_open);
        move |_| {
            let open = args.settings_page.get();
            let was_open = prev_open.get();
            if was_open && !open {
                // 关闭设置页（Escape / 返回 / hash 导航）时放弃未保存草稿，
                // 避免遗留脏草稿在重开时被 `refresh_from_current` 吸收为新 baseline、
                // 之后点「保存全部」把「已放弃」的编辑一并持久化。
                discard_to_baselines(DiscardToBaselinesCtx {
                    baselines,
                    drafts: args.drafts,
                    llm_settings_feedback: args.llm_settings_feedback,
                    executor_llm_settings_feedback: args.executor_llm_settings_feedback,
                });
            }
            prev_open.set(open);
            if !open {
                return;
            }
            sync_appearance_drafts_from_shell(
                args.locale,
                args.theme,
                args.bg_decor,
                args.appearance_locale,
                args.appearance_theme,
                args.appearance_bg_decor,
            );
            baselines.refresh_from_current(&form_current_untracked(args.drafts));
            args.clear_client_key_intent.set(false);
            args.clear_executor_key_intent.set(false);
            args.llm_settings_feedback.set(None);
            args.executor_llm_settings_feedback.set(None);
        }
    });
}

pub(super) fn wire_settings_page_dom_preview_effect(
    settings_page: RwSignal<bool>,
    theme: RwSignal<String>,
    bg_decor: RwSignal<bool>,
    appearance_theme: RwSignal<String>,
    appearance_bg_decor: RwSignal<bool>,
) {
    Effect::new(move |_| {
        if !settings_page.get() {
            apply_theme_preview_to_dom(theme.get().as_str());
            apply_bg_decor_preview_to_dom(bg_decor.get());
            return;
        }
        apply_theme_preview_to_dom(appearance_theme.get().as_str());
        apply_bg_decor_preview_to_dom(appearance_bg_decor.get());
    });
}
