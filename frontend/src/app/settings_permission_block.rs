//! 设置页「权限等级」只读块：`GET /permission` 的策略投影。
//!
//! 只读：等级、上限与可选档位全部由 serve 判定（客户端不复制等级比较规则，
//! 见 `docs/design/client_shared_logic.md` §4.16）。
//! 老 serve 没有该路由（404）→ 显示「不支持」，既不报错，也不猜一个默认档。

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crabmate_client_api::ordered_clamped_axes;
use crabmate_client_api::permission::{PermissionLevelSource, PermissionView};

use crate::api::{PermissionFetch, fetch_permission};
use crate::i18n::{self, Locale};

#[derive(Clone)]
enum PermissionState {
    Loading,
    Loaded(PermissionView),
    /// 404：该 serve 还没有 `/permission`。
    Unsupported,
    Err(String),
}

/// 请求结果 → 界面状态。404 与「取到了但读不出来」必须是两个变体：
/// 合成一个，老 serve 就会一直挂着一条错误。
fn state_from_fetch(result: PermissionFetch) -> PermissionState {
    match result {
        PermissionFetch::Ok(v) => PermissionState::Loaded(v),
        PermissionFetch::Unsupported => PermissionState::Unsupported,
        PermissionFetch::Err(e) => PermissionState::Err(e),
    }
}

/// 收窄轴的展示串。顺序与「未知轴丢弃」都由共享层决定——未知轴没有文案，
/// 硬渲染只会露出一个英文标识符。
fn clamped_axes_text(clamped_by: &[String], locale: Locale) -> String {
    let axes: Vec<String> = ordered_clamped_axes(clamped_by)
        .into_iter()
        .map(|axis| i18n::settings_permission_axis_label(locale, axis))
        .collect();
    if axes.is_empty() {
        return i18n::settings_permission_clamped_none(locale).to_string();
    }
    axes.join(" · ")
}

/// 可选档位的显示名。未知档位由 [`i18n::settings_permission_level_label`] 原样透出。
fn available_labels(available: &[String], locale: Locale) -> Vec<String> {
    available
        .iter()
        .map(|raw| i18n::settings_permission_level_label(locale, raw))
        .collect()
}

#[component]
pub(crate) fn SettingsPermissionBlock(locale: RwSignal<Locale>) -> impl IntoView {
    let state = RwSignal::new(PermissionState::Loading);
    // 连点「刷新」会有两个请求在飞，返回顺序不定；代次保证只有最后一次落地。
    let generation = RwSignal::new(0_u32);
    let reload = move || {
        let mine = generation.get_untracked().wrapping_add(1);
        generation.set(mine);
        state.set(PermissionState::Loading);
        let loc = locale.get_untracked();
        spawn_local(async move {
            let result = state_from_fetch(fetch_permission(None, None, loc).await);
            if generation.get_untracked() == mine {
                state.set(result);
            }
        });
    };
    // 只取静态投影（不带 `agent_role` / `conversation_id`）：设置页没有「即将发送的角色」
    // 概念，按角色 / 会话收窄后的档位属于聊天现场，不在这里猜——界面上有对应说明。
    reload();

    view! {
        <div class="settings-block">
            <h3 class="settings-block-title">
                {move || i18n::settings_permission_title(locale.get())}
            </h3>
            {move || permission_body(state.get(), locale.get())}
            <div class="settings-row">
                <button
                    type="button"
                    class="btn btn-secondary btn-sm"
                    on:click=move |_| reload()
                >
                    {move || i18n::settings_permission_refresh(locale.get())}
                </button>
            </div>
        </div>
    }
}

fn permission_body(state: PermissionState, locale: Locale) -> AnyView {
    match state {
        PermissionState::Loading => view! {
            <p class="settings-hint">{i18n::settings_permission_loading(locale)}</p>
        }
        .into_any(),
        PermissionState::Unsupported => view! {
            <p class="settings-hint">{i18n::settings_permission_unsupported(locale)}</p>
        }
        .into_any(),
        PermissionState::Err(e) => view! {
            <p class="settings-error">
                {format!("{}{}", i18n::settings_permission_unavailable(locale), e)}
            </p>
        }
        .into_any(),
        PermissionState::Loaded(view) => permission_rows(&view, locale).into_any(),
    }
}

fn permission_rows(v: &PermissionView, locale: Locale) -> impl IntoView {
    let level = i18n::settings_permission_level_label(locale, &v.level);
    let ceiling = i18n::settings_permission_level_label(locale, &v.ceiling);
    let level_source = match v.level_source {
        PermissionLevelSource::Config => i18n::settings_permission_source_config(locale),
        PermissionLevelSource::Persisted => i18n::settings_permission_source_persisted(locale),
    };
    // 契约里 `ceiling_source` 是裸字符串；不本地化就会直接露出 `derived_non_loopback`。
    let ceiling_source = i18n::settings_permission_ceiling_source_label(locale, &v.ceiling_source);
    let clamped = clamped_axes_text(&v.clamped_by, locale);
    let available = available_labels(&v.available, locale);
    view! {
        <div class="settings-field">
            <span class="settings-field-label">
                {i18n::settings_permission_current(locale)}
            </span>
            <div class="settings-status-line">
                <span class="settings-status-pill settings-status-pill--ok">{level}</span>
                <span class="settings-hint">{format!("{} · {}", v.level, level_source)}</span>
            </div>
        </div>
        <div class="settings-field">
            <span class="settings-field-label">
                {i18n::settings_permission_ceiling(locale)}
            </span>
            <div class="settings-status-line">
                <span class="settings-status-pill settings-status-pill--muted">{ceiling}</span>
                <span class="settings-hint">{ceiling_source}</span>
            </div>
        </div>
        <div class="settings-field">
            <span class="settings-field-label">
                {i18n::settings_permission_available(locale)}
            </span>
            <div class="settings-status-line">
                {available
                    .into_iter()
                    .map(|label| {
                        view! {
                            <span class="settings-status-pill settings-status-pill--muted">
                                {label}
                            </span>
                        }
                    })
                    .collect_view()}
            </div>
        </div>
        <div class="settings-field">
            <span class="settings-field-label">
                {i18n::settings_permission_clamped(locale)}
            </span>
            <div class="settings-status-line">
                <span class="settings-hint">{clamped}</span>
            </div>
        </div>
        <p class="settings-hint">{i18n::settings_permission_readonly_hint(locale)}</p>
        // 标签写的是「当前等级」，但这里是静态投影；不说清会被当成会话内实际档位。
        <p class="settings-hint">{i18n::settings_permission_static_hint(locale)}</p>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_and_failure_map_to_different_states() {
        // 两者的区别就是「老 serve」与「读取失败」的区别；混成一个变体就会误报。
        assert!(matches!(
            state_from_fetch(PermissionFetch::Unsupported),
            PermissionState::Unsupported
        ));
        assert!(matches!(
            state_from_fetch(PermissionFetch::Err("boom".to_owned())),
            PermissionState::Err(_)
        ));
    }

    #[test]
    fn clamped_text_uses_the_shared_order_and_drops_unknown_axes() {
        let raw = vec![
            "session_mode".to_owned(),
            " Unknown ".to_owned(),
            "role".to_owned(),
        ];
        assert_eq!(
            clamped_axes_text(&raw, Locale::ZhHans),
            "受角色上限限制 · 受会话模式限制"
        );
        // 空数组 = 未被钳制，与服务端契约同义。
        assert_eq!(clamped_axes_text(&[], Locale::ZhHans), "未被收窄");
        assert_eq!(clamped_axes_text(&[], Locale::En), "not narrowed");
    }

    #[test]
    fn available_labels_keep_server_order() {
        let raw = vec!["read".to_owned(), "approved".to_owned()];
        assert_eq!(
            available_labels(&raw, Locale::ZhHans),
            vec!["只读".to_owned(), "已批准".to_owned()]
        );
        assert!(available_labels(&[], Locale::En).is_empty());
    }
}
