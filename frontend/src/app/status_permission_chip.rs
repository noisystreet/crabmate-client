//! 底栏权限等级 chip：显示「意图 → 生效」，点一下就地切换。
//!
//! 为什么放底栏而不只在设置页：等级是**每个回合都生效**的东西，用户判断「这句话要不要
//! 让它动文件」发生在聊天现场；切个档要跳两次页面，实际结果就是没人切、一路用默认档。
//!
//! 显示口径与设置页一致（[`i18n::settings_permission_level_label`] 同一套词表）：
//! - 有意图且与生效相同 → 只显示该档；不同 → `意图 → 生效`（服务端允许意图高于生效，
//!   客户端**不得**把意图改写成生效值，否则下次 `/chat` 会把被钳制的低档写回）；
//! - 没表达意图 → 显示生效档（`GET /permission` 的 `level`）；两者都不知道 → 整段不渲染
//!   （老 serve 没有 `/permission`，此时**不能**拿 `/status` 的 `permission_level_default` 顶替，
//!   那是「没意图时会落到哪」，不是「此刻是哪」）。
//!
//! 写路径与设置页共用同一套规则：先写意图信号（`/chat` 发送时读它），已绑定会话时再
//! `POST /conversation/state`（只发等级这一列），响应自带的投影就地刷新本 chip。

use leptos::html;
use leptos::portal::Portal;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_dom::helpers::request_animation_frame;
use wasm_bindgen::JsCast;

use crabmate_client_api::conversation_state::{ConversationStateOutcome, ConversationStatePatch};
use crabmate_client_api::permission::{PermissionView, normalize_level};

use crate::api::{PermissionFetch, fetch_permission, post_conversation_state};
use crate::app::shell_runtime_context::try_chat_shell_ctx;
use crate::chat_session_state::ChatSessionSignals;
use crate::i18n::{self, Locale};

#[derive(Clone, Copy)]
struct PermissionMenuProps {
    locale: RwSignal<Locale>,
    level_select: LevelSelect,
    menu_open: RwSignal<bool>,
    menu_fixed_style: RwSignal<Option<String>>,
}

/// chip 要用的信号与落点。
#[derive(Clone, Copy)]
struct LevelSelect {
    locale: RwSignal<Locale>,
    selected: RwSignal<Option<String>>,
    owner: RwSignal<Option<String>>,
    chat: Option<ChatSessionSignals>,
    /// 生效投影（本 chip 自己的缓存，成功写入后用响应刷新）。
    view: RwSignal<Option<PermissionView>>,
    /// 丢弃过期响应（连点两档时只有最后一次落地）。
    generation: RwSignal<u32>,
    /// 就地错误提示（一行，显示在 chip 的 `title` 上，不占底栏宽度）。
    note: RwSignal<Option<String>>,
}

/// chip 上显示什么。`None` = 整段不渲染。
///
/// 三条与设置页共享的判定：意图优先于生效；两者不同都要显示；都不知道就不显示
/// （`.` 而不是猜一个档）。
#[must_use]
fn chip_text(intent: Option<&str>, effective: Option<&str>, loc: Locale) -> Option<String> {
    let intent = intent.map(str::trim).filter(|s| !s.is_empty());
    let effective = effective.map(str::trim).filter(|s| !s.is_empty());
    match (intent, effective) {
        (Some(i), Some(e)) if i == e => Some(i18n::settings_permission_level_label(loc, i)),
        (Some(i), Some(e)) => Some(format!(
            "{} → {}",
            i18n::settings_permission_level_label(loc, i),
            i18n::settings_permission_level_label(loc, e)
        )),
        (Some(i), None) => Some(i18n::settings_permission_level_label(loc, i)),
        (None, Some(e)) => Some(i18n::settings_permission_level_label(loc, e)),
        (None, None) => None,
    }
}

fn menu_fixed_style_for_trigger(trigger: &web_sys::HtmlElement) -> String {
    let el = trigger.unchecked_ref::<web_sys::Element>();
    let rect = el.get_bounding_client_rect();
    // 底栏贴窗口底部，菜单向上展开（`bottom` 从视口底算），否则会被视口裁掉。
    let viewport_h = window()
        .and_then(|w| w.inner_height().ok())
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    format!(
        "position:fixed;left:{}px;bottom:{}px;top:auto;min-width:{}px;z-index:201;",
        rect.left(),
        (viewport_h - rect.top() + 4.0).max(0.0),
        rect.width()
    )
}

fn window() -> Option<web_sys::Window> {
    web_sys::window()
}

fn close_menu(menu_open: RwSignal<bool>, menu_fixed_style: RwSignal<Option<String>>) {
    menu_open.set(false);
    menu_fixed_style.set(None);
}

fn anchor_menu(trigger_ref: NodeRef<html::Button>, menu_fixed_style: RwSignal<Option<String>>) {
    if let Some(t) = trigger_ref.get_untracked() {
        let el: web_sys::HtmlElement = t.unchecked_into();
        menu_fixed_style.set(Some(menu_fixed_style_for_trigger(&el)));
    }
}

/// 开合菜单并重新锚定；展开后再补一帧，避免底栏布局抖动导致错位。
fn toggle_menu(
    ev: web_sys::MouseEvent,
    menu_open: RwSignal<bool>,
    menu_fixed_style: RwSignal<Option<String>>,
    trigger_ref: NodeRef<html::Button>,
) {
    ev.stop_propagation();
    let next = !menu_open.get_untracked();
    if next {
        anchor_menu(trigger_ref, menu_fixed_style);
        request_animation_frame(move || anchor_menu(trigger_ref, menu_fixed_style));
    } else {
        menu_fixed_style.set(None);
    }
    menu_open.set(next);
}

/// 拉一次「此刻生效的档」。404 / 失败一律清空缓存：老 serve 上本段就该消失，
/// 而不是留着一个上次的读数继续骗人。
fn spawn_effective_refresh(sel: LevelSelect, loc: Locale) {
    let cid = sel
        .chat
        .and_then(|c| c.session_sync.with(|s| s.stream_conversation_id()));
    let mine = sel.generation.get_untracked().wrapping_add(1);
    sel.generation.set(mine);
    spawn_local(async move {
        let result = fetch_permission(None, cid.as_deref(), loc).await;
        if sel.generation.get_untracked() != mine {
            return;
        }
        match result {
            PermissionFetch::Ok(v) => sel.view.set(Some(v)),
            PermissionFetch::Unsupported | PermissionFetch::Err(_) => sel.view.set(None),
        }
    });
}

/// chip 的 `title`：基础说明 + 就地错误（有才拼）。
fn chip_title(note: Option<String>, loc: Locale) -> String {
    let base = i18n::status_perm_title_attr(loc);
    match note {
        Some(n) => format!("{base} · {n}"),
        None => base.to_string(),
    }
}

/// 注册「生效档」的拉取 effect：会话 / 意图 / 语言变化时重拉（不轮询）。
fn wire_refresh_effect(sel: LevelSelect, locale: RwSignal<Locale>) {
    Effect::new(move |_| {
        let _ = sel.chat.map(|c| c.active_id.get()).unwrap_or_default();
        let _ = sel.selected.get();
        let loc = locale.get();
        spawn_effective_refresh(sel, loc);
    });
}

/// 点一个档位：写意图（含归属）→ 有会话则落盘 → 用结局刷新本 chip 或记一行提示。
fn apply_level(level: &str, sel: LevelSelect) {
    let loc = sel.locale.get_untracked();
    let Some(level) = normalize_level(level) else {
        sel.note.set(Some(
            i18n::settings_permission_unknown_level(loc).to_string(),
        ));
        return;
    };
    sel.selected.set(Some(level.to_string()));
    sel.note.set(None);
    // 归属记在**选择这一刻**的会话上：之后切走就该清掉，而不是跟着跑到别的会话。
    sel.owner.set(sel.chat.map(|c| c.active_id.get_untracked()));
    let Some(chat) = sel.chat else {
        return;
    };
    let cid = chat.session_sync.with(|s| s.stream_conversation_id());
    let Some(cid) = cid else {
        // 没有已持久化会话：服务端不做 upsert，意图随下一条消息生效。
        sel.note.set(Some(
            i18n::settings_permission_send_to_apply(loc).to_string(),
        ));
        return;
    };
    let patch = ConversationStatePatch {
        conversation_id: Some(cid),
        permission_level: Some(level.to_string()),
        // 只发改的这一列（部分更新）。
        session_mode: None,
        agent_role: None,
    };
    let mine = sel.generation.get_untracked().wrapping_add(1);
    sel.generation.set(mine);
    spawn_local(async move {
        let outcome = post_conversation_state(&patch, loc).await;
        if sel.generation.get_untracked() != mine {
            return;
        }
        match outcome {
            Ok(ConversationStateOutcome::Ok(view)) => {
                sel.note.set(None);
                sel.view.set(Some(view.permission));
            }
            Ok(ConversationStateOutcome::NotFound) => {
                sel.note.set(Some(
                    i18n::settings_permission_conversation_gone(loc).to_string(),
                ));
            }
            Ok(ConversationStateOutcome::Unsupported) => {
                // 老 serve：连读都做不到，chip 本就不该显示；清空缓存让它消失。
                sel.note
                    .set(Some(i18n::settings_permission_unsupported(loc).to_string()));
                sel.view.set(None);
            }
            Ok(ConversationStateOutcome::Failed { status, code }) => {
                let base = if status == 500 {
                    i18n::settings_permission_write_retry(loc)
                } else {
                    i18n::settings_permission_write_failed(loc)
                };
                let detail = code.unwrap_or_else(|| format!("HTTP {status}"));
                sel.note.set(Some(format!("{base} ({detail})")));
            }
            Err(e) => sel.note.set(Some(format!(
                "{} {e}",
                i18n::settings_permission_write_failed(loc)
            ))),
        }
    });
}

#[component]
fn StatusPermissionMenu(props: PermissionMenuProps) -> impl IntoView {
    let PermissionMenuProps {
        locale,
        level_select,
        menu_open,
        menu_fixed_style,
    } = props;
    let menu_style = Memo::new(move |_| menu_fixed_style.get().unwrap_or_default());
    let aria_label = Memo::new(move |_| i18n::status_perm_menu_label(locale.get()).to_string());
    let options = Memo::new(move |_| {
        level_select
            .view
            .get()
            .map(|v| v.available)
            .unwrap_or_default()
    });
    view! {
        <Portal>
            <button
                type="button"
                class="status-mode-backdrop status-mode-backdrop--portal"
                tabindex="-1"
                aria-hidden="true"
                on:click=move |ev: web_sys::MouseEvent| {
                    ev.stop_propagation();
                    close_menu(menu_open, menu_fixed_style);
                }
            />
            <crate::app::focusable_menu::FocusableRoleMenu
                class="status-mode-menu status-mode-menu--fixed status-mode-menu--portal"
                menu_style=menu_style
                aria_label=aria_label
                on_escape=Callback::new(move |_| close_menu(menu_open, menu_fixed_style))
            >
                {move || {
                    options
                        .get()
                        .into_iter()
                        .map(|raw| {
                            let raw_for_active = raw.clone();
                            let raw_for_click = raw.clone();
                            let label = i18n::settings_permission_level_label(
                                locale.get_untracked(),
                                &raw,
                            );
                            view! {
                                <button
                                    type="button"
                                    class="status-mode-menu-item"
                                    role="menuitemradio"
                                    prop:aria-checked=move || {
                                        (level_select.selected.get().as_deref()
                                            == Some(raw_for_active.as_str()))
                                            .to_string()
                                    }
                                    on:click=move |_| {
                                        apply_level(raw_for_click.as_str(), level_select);
                                        close_menu(menu_open, menu_fixed_style);
                                    }
                                >
                                    {label}
                                </button>
                            }
                        })
                        .collect_view()
                }}
            </crate::app::focusable_menu::FocusableRoleMenu>
        </Portal>
    }
}

#[component]
pub fn StatusPermissionChip(locale: RwSignal<Locale>) -> impl IntoView {
    let shell = try_chat_shell_ctx();
    let view = RwSignal::new(None::<PermissionView>);
    let generation = RwSignal::new(0_u32);
    let note = RwSignal::<Option<String>>::new(None);
    let menu_open = RwSignal::new(false);
    let menu_fixed_style = RwSignal::<Option<String>>::new(None);
    let trigger_ref = NodeRef::<html::Button>::new();
    let level_select = LevelSelect {
        locale,
        selected: shell
            .map(|c| c.selected_permission_level)
            .unwrap_or_else(|| RwSignal::new(None)),
        owner: shell
            .map(|c| c.permission_intent_owner)
            .unwrap_or_else(|| RwSignal::new(None)),
        chat: shell.map(|c| c.chat),
        view,
        generation,
        note,
    };

    // 拉当前生效档：会话切换、意图变化、语言切换都重拉一次（不轮询）。
    // 404 / 失败把缓存清空——老 serve 上本段就该不存在。
    wire_refresh_effect(level_select, locale);

    let text = Memo::new(move |_| {
        let effective = view.get().map(|v| v.level);
        chip_text(
            level_select.selected.get().as_deref(),
            effective.as_deref(),
            locale.get(),
        )
    });

    view! {
        <Show when=move || text.get().is_some()>
            <span
                class="status-chip status-chip-perm"
                prop:title=move || chip_title(note.get(), locale.get())
            >
                <span class="status-chip-label">
                    {move || i18n::status_perm_label(locale.get())}
                </span>
                <button
                    type="button"
                    class="status-mode-trigger"
                    class:status-mode-trigger-open=move || menu_open.get()
                    data-testid="status-perm-trigger"
                    node_ref=trigger_ref
                    prop:aria-expanded=move || menu_open.get()
                    aria-haspopup="menu"
                    on:click=move |ev: web_sys::MouseEvent| {
                        toggle_menu(ev, menu_open, menu_fixed_style, trigger_ref)
                    }
                >
                    <span class="status-chip-value">{move || text.get().unwrap_or_default()}</span>
                </button>
            </span>
            <Show when=move || menu_open.get()>
                <StatusPermissionMenu props=PermissionMenuProps {
                    locale,
                    level_select,
                    menu_open,
                    menu_fixed_style,
                } />
            </Show>
        </Show>
    }
}

#[cfg(test)]
mod tests {
    use super::chip_text;
    use crate::i18n::Locale;

    #[test]
    fn intent_wins_and_both_values_show_when_they_differ() {
        // 一致：单值（与改动前的观感一致）。
        assert_eq!(
            chip_text(Some("workspace"), Some("workspace"), Locale::ZhHans),
            Some("工作区".to_string())
        );
        // 意图高于生效：两个值都要出现，否则用户以为切换没生效。
        assert_eq!(
            chip_text(Some("unrestricted"), Some("read"), Locale::ZhHans),
            Some("不受限 → 只读".to_string())
        );
    }

    #[test]
    fn no_intent_shows_the_effective_level_and_unknown_shows_nothing() {
        assert_eq!(
            chip_text(None, Some("approved"), Locale::ZhHans),
            Some("已批准".to_string())
        );
        // 两者都不知道 → 整段不渲染（不猜默认档）。
        assert_eq!(chip_text(None, None, Locale::ZhHans), None);
        // 空白意图等同没有。
        assert_eq!(
            chip_text(Some("  "), Some("read"), Locale::En),
            Some("Read".to_string())
        );
        // 只有意图、还没有投影（首次拉取未回）：仍显示意图，别让 chip 闪没。
        assert_eq!(
            chip_text(Some("read"), None, Locale::En),
            Some("Read".to_string())
        );
    }
}
