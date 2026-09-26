//! IDE 编辑器设置全屏视图。

use std::rc::Rc;

use leptos::prelude::*;

use super::draft::wire_ide_settings_draft;
use super::section_editor::IdeSettingsEditorBlock;
use crate::app::app_signals::IdeEditorSignals;
use crate::app::settings_page::{
    SettingsContentIntro, SettingsNavItem, SettingsPageHeader, SettingsPageHeaderSpec,
};
use crate::i18n::{self, Locale};

/// IDE 设置页顶栏与常规设置页结构一致，仅措辞与测试钩子不同。
const IDE_SETTINGS_HEADER_SPEC: SettingsPageHeaderSpec = SettingsPageHeaderSpec {
    title: i18n::ide_settings_title,
    back: i18n::ide_settings_back,
    // IDE 页没有单独的返回 aria 文案，复用可见文案（“返回编辑器”）。
    back_aria: i18n::ide_settings_back,
    badge_local: i18n::ide_settings_badge_local,
    unsaved_badge: i18n::ide_settings_unsaved_badge,
    discard_changes: i18n::ide_settings_discard_changes,
    save_all: i18n::ide_settings_save_all,
    back_testid: "ide-settings-back",
    save_testid: "ide-settings-save-all",
};

#[derive(Clone, Copy)]
pub struct IdeSettingsPageViewInput {
    pub ide_settings_page: RwSignal<bool>,
    pub locale: RwSignal<Locale>,
    pub editor: IdeEditorSignals,
}

#[component]
pub fn IdeSettingsPageView(input: IdeSettingsPageViewInput) -> impl IntoView {
    let IdeSettingsPageViewInput {
        ide_settings_page,
        locale,
        editor,
    } = input;

    let draft = wire_ide_settings_draft(ide_settings_page, editor);
    // IDE 设置保存是同步的（`persist_from_signals` 直接写 localStorage），没有 busy 相位。
    let save_busy = RwSignal::new(false);
    let section_title = Memo::new(move |_| i18n::ide_settings_section_editor_title(locale.get()));
    let section_desc = Memo::new(|_| "");

    view! {
        <div class="settings-page" class:settings-page-visible=move || ide_settings_page.get()>
            <SettingsPageHeader
                appearance_locale=locale
                dirty=draft.dirty
                save_busy=save_busy
                spec=IDE_SETTINGS_HEADER_SPEC
                on_back=draft.on_back
                on_discard=draft.discard_rc
                on_save=draft.save_rc
            />
            <div class="settings-page-body">
                <div class="settings-layout">
                    <nav class="settings-nav" prop:aria-label=move || i18n::ide_settings_nav_aria(locale.get())>
                        <SettingsNavItem
                            active=Memo::new(|_| true)
                            testid=None
                            on_click=Rc::new(|| {})
                        >
                            {move || i18n::ide_settings_section_editor_title(locale.get())}
                        </SettingsNavItem>
                    </nav>
                    <section class="settings-content">
                        <SettingsContentIntro title=section_title desc=section_desc />
                        <IdeSettingsEditorBlock locale=locale editor=draft.draft_editor />
                    </section>
                </div>
            </div>
        </div>
    }
}
