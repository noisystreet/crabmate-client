//! 设置页顶栏（常规设置页与 IDE 设置页共用；从 `view` 拆出以降低 `SettingsPageView` 的 nloc 棘轮）。

use std::rc::Rc;

use leptos::prelude::*;

use crate::i18n::{self, Locale};
use crate::icon::icon_chevron_left;

/// 设置页顶栏的页级差异：文案 + 两个操作按钮的 `data-testid`。
///
/// 两页顶栏结构完全一致，只有措辞（标题 / 返回 / 保存）与测试钩子不同；`data-testid` 也按页区分，
/// 因为两页在 DOM 中同时挂载，同一 testid 会重复而让选择器取到另一页的按钮。
#[derive(Clone, Copy)]
pub(crate) struct SettingsPageHeaderSpec {
    pub title: fn(Locale) -> &'static str,
    pub back: fn(Locale) -> &'static str,
    pub back_aria: fn(Locale) -> &'static str,
    pub badge_local: fn(Locale) -> &'static str,
    pub unsaved_badge: fn(Locale) -> &'static str,
    pub discard_changes: fn(Locale) -> &'static str,
    pub save_all: fn(Locale) -> &'static str,
    pub back_testid: &'static str,
    pub save_testid: &'static str,
}

/// 常规设置页顶栏差异。
pub(crate) const SETTINGS_PAGE_HEADER_SPEC: SettingsPageHeaderSpec = SettingsPageHeaderSpec {
    title: i18n::settings_title,
    back: i18n::settings_back,
    back_aria: i18n::settings_back_aria,
    badge_local: i18n::settings_badge_local,
    unsaved_badge: i18n::settings_unsaved_badge,
    discard_changes: i18n::settings_discard_changes,
    save_all: i18n::settings_save_all,
    back_testid: "settings-back",
    save_testid: "settings-save-all",
};

/// 顶栏右侧操作（丢弃 / 保存，含 dirty/busy 禁用态），独立以降低 header CCN。
#[component]
fn SettingsPageHeaderActions(
    appearance_locale: RwSignal<Locale>,
    dirty: Memo<bool>,
    save_busy: RwSignal<bool>,
    spec: SettingsPageHeaderSpec,
    on_discard: Rc<dyn Fn()>,
    on_save: Rc<dyn Fn()>,
) -> impl IntoView {
    view! {
        <div class="settings-page-header-actions">
            <button
                type="button"
                class="btn btn-secondary btn-sm"
                prop:disabled=move || !dirty.get()
                on:click=move |_| on_discard()
            >
                {move || (spec.discard_changes)(appearance_locale.get())}
            </button>
            <button
                type="button"
                class="btn btn-primary btn-sm"
                data-testid=spec.save_testid
                prop:disabled=move || !dirty.get() || save_busy.get()
                on:click=move |_| on_save()
            >
                {move || (spec.save_all)(appearance_locale.get())}
            </button>
        </div>
    }
}

#[component]
pub(crate) fn SettingsPageHeader(
    appearance_locale: RwSignal<Locale>,
    dirty: Memo<bool>,
    save_busy: RwSignal<bool>,
    spec: SettingsPageHeaderSpec,
    on_back: Rc<dyn Fn()>,
    on_discard: Rc<dyn Fn()>,
    on_save: Rc<dyn Fn()>,
) -> impl IntoView {
    view! {
        <div class="settings-page-header">
            <button
                type="button"
                class="btn btn-ghost settings-page-back"
                data-testid=spec.back_testid
                prop:aria-label=move || (spec.back_aria)(appearance_locale.get())
                on:click=move |_| on_back()
            >
                {icon_chevron_left("")}
                <span>{move || (spec.back)(appearance_locale.get())}</span>
            </button>
            <h1 class="settings-page-title">{move || (spec.title)(appearance_locale.get())}</h1>
            <span class="settings-page-badge">{move || (spec.badge_local)(appearance_locale.get())}</span>
            <Show when=move || dirty.get()>
                <span class="settings-unsaved-pill">
                    {move || (spec.unsaved_badge)(appearance_locale.get())}
                </span>
            </Show>
            <span class="settings-page-head-spacer"></span>
            <SettingsPageHeaderActions
                appearance_locale=appearance_locale
                dirty=dirty
                save_busy=save_busy
                spec=spec
                on_discard=on_discard
                on_save=on_save
            />
        </div>
    }
}
