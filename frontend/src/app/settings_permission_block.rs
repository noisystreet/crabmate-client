//! 设置页「权限等级」块：`GET /permission` 的策略投影 + 会话级**意图**选择器。
//!
//! 三条铁律（客户端不复制服务端判定）：
//! 1. 等级、上限、可选档位、收窄轴全部由 serve 算好，这里只渲染；
//! 2. 选择器**只列 `available`**——不提供「写入超上限」的入口（上限启动期冻结 + 热重载只降不升，
//!    预置一个更高档位没有意义）；
//! 3. 展示层必须能渲染「意图高于生效」（别的客户端写的、或 role/mode 收窄后持久值仍是高档）：
//!    如实显示两个值，并用 `clamped_by` 解释被谁收窄，**不要**在客户端把意图改写成生效值。
//!
//! 写路径走 `POST /conversation/state`（空闲态写入口），但只在**已绑定会话**时才发：
//! 没有会话就没有可写的行（服务端明确不做 upsert），此时只写本地意图信号，
//! 下一条消息由 `/chat` 带上——所以文案是「发送后生效」而不是假装成功。

use leptos::prelude::*;
use leptos::task::spawn_local;

use crabmate_client_api::conversation_state::{ConversationStateOutcome, ConversationStatePatch};
use crabmate_client_api::ordered_clamped_axes;
use crabmate_client_api::permission::{PermissionLevelSource, PermissionView, normalize_level};

use crate::api::{PermissionFetch, fetch_permission, post_conversation_state};
use crate::app::shell_runtime_context::try_chat_shell_ctx;
use crate::chat_session_state::ChatSessionSignals;
use crate::i18n::{self, Locale};

#[derive(Clone)]
enum PermissionState {
    Loading,
    Loaded(PermissionView),
    /// 404：该 serve 还没有 `/permission`。
    Unsupported,
    Err(String),
}

/// 选择器要用的信号与落点（避免把六个参数铺满每个函数）。
#[derive(Clone, Copy)]
struct LevelSelectCtx {
    locale: RwSignal<Locale>,
    /// 意图信号：与 `/chat` 发送路径读的是**同一个**（壳层 ctx）。
    selected: RwSignal<Option<String>>,
    /// 该意图归属哪个本地会话（切会话时由壳层 effect 清空）。
    owner: RwSignal<Option<String>>,
    /// 当前会话句柄；`None` = 无 ctx（单测 / Story），此时选择器只改本地意图。
    chat: Option<ChatSessionSignals>,
    state: RwSignal<PermissionState>,
    /// 与刷新共用代次：连点两档时只有最后一次落地。
    generation: RwSignal<u32>,
    /// 一行操作结果 / 提示（就地显示，不弹全局错误条）。
    note: RwSignal<Option<String>>,
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

/// 意图行：`意图 → 生效`，两者相同就只显示一个。
///
/// 服务端允许「意图高于生效」（`active_permission_level` 原样存，钳制发生在 min 链）。
/// 客户端**不能**把意图改写成生效值——那会让下一次 `/chat` 把被钳制后的低档写回，
/// 从此再也回不到用户原本选的高档。
fn intent_line(selected: Option<&str>, effective: &str, locale: Locale) -> String {
    let Some(intent) = selected.map(str::trim).filter(|s| !s.is_empty()) else {
        return i18n::settings_permission_intent_none(locale).to_string();
    };
    let intent_label = i18n::settings_permission_level_label(locale, intent);
    let effective_label = i18n::settings_permission_level_label(locale, effective);
    if intent == effective {
        intent_label
    } else {
        format!("{intent_label} → {effective_label}")
    }
}

/// 点一个档位：先写本地意图（单一意图源），再尝试空闲态写入。
fn apply_level_selection(level: &str, ctx: LevelSelectCtx) {
    let Some(level) = normalize_level(level) else {
        // 选择器只列 `available`；走到这里说明词表漂移，如实说，不猜一个档。
        ctx.note.set(Some(
            i18n::settings_permission_unknown_level(ctx.locale.get_untracked()).to_string(),
        ));
        return;
    };
    ctx.selected.set(Some(level.to_string()));
    // 归属记在**选择这一刻**的会话上：之后切走就该清掉，而不是跟着跑到别的会话。
    ctx.owner.set(ctx.chat.map(|c| c.active_id.get_untracked()));
    let loc = ctx.locale.get_untracked();
    let cid = ctx
        .chat
        .and_then(|c| c.session_sync.with(|s| s.stream_conversation_id()));
    let Some(cid) = cid else {
        // 没有已持久化会话：服务端不做 upsert，只能在下一句话里带上意图。
        ctx.note.set(Some(
            i18n::settings_permission_send_to_apply(loc).to_string(),
        ));
        return;
    };
    ctx.note.set(None);
    let patch = ConversationStatePatch {
        conversation_id: Some(cid),
        permission_level: Some(level.to_string()),
        // **只发改的这一列**：服务端是部分更新，回传 `session_mode` / `agent_role`
        // 会把过期值写回去，或送一个未确认的角色直接吃 400。
        session_mode: None,
        agent_role: None,
    };
    let mine = ctx.generation.get_untracked().wrapping_add(1);
    ctx.generation.set(mine);
    spawn_local(async move {
        let outcome = post_conversation_state(&patch, loc).await;
        if ctx.generation.get_untracked() != mine {
            return;
        }
        match outcome {
            Ok(ConversationStateOutcome::Ok(view)) => {
                // 响应自带写后投影：一次请求就刷新了选择器与显示，不必再打 `/permission`。
                ctx.note.set(None);
                ctx.state.set(PermissionState::Loaded(view.permission));
            }
            Ok(ConversationStateOutcome::NotFound) => {
                ctx.note.set(Some(
                    i18n::settings_permission_conversation_gone(loc).to_string(),
                ));
            }
            Ok(ConversationStateOutcome::Unsupported) => {
                // 与 `GET /permission` 的裸 404 是同一件事：共用同一个 capability 状态，
                // 否则会出现「只读块说不支持、选择器说会话不存在」的自相矛盾。
                ctx.note
                    .set(Some(i18n::settings_permission_unsupported(loc).to_string()));
                ctx.state.set(PermissionState::Unsupported);
            }
            Ok(ConversationStateOutcome::Failed { status, code }) => {
                // 500 `CONVERSATION_STORE_ERROR` 是幂等可重试的；不要把它说成会话不存在。
                let retryable = status == 500;
                let text = if retryable {
                    i18n::settings_permission_write_retry(loc)
                } else {
                    i18n::settings_permission_write_failed(loc)
                };
                ctx.note.set(Some(match code {
                    Some(code) => format!("{text} ({code})"),
                    None => format!("{text} (HTTP {status})"),
                }));
            }
            Err(e) => {
                ctx.note.set(Some(format!(
                    "{} {e}",
                    i18n::settings_permission_write_failed(loc)
                )));
            }
        }
    });
}

#[component]
pub(crate) fn SettingsPermissionBlock(locale: RwSignal<Locale>) -> impl IntoView {
    let state = RwSignal::new(PermissionState::Loading);
    // 连点「刷新」会有两个请求在飞，返回顺序不定；代次保证只有最后一次落地。
    let generation = RwSignal::new(0_u32);
    let note = RwSignal::<Option<String>>::new(None);
    // 意图信号与发送路径同源；无 ctx（单测 / Story）时退化为只改本地信号。
    let shell = try_chat_shell_ctx();
    let level_ctx = LevelSelectCtx {
        locale,
        selected: shell
            .map(|c| c.selected_permission_level)
            .unwrap_or_else(|| RwSignal::new(None)),
        owner: shell
            .map(|c| c.permission_intent_owner)
            .unwrap_or_else(|| RwSignal::new(None)),
        chat: shell.map(|c| c.chat),
        state,
        generation,
        note,
    };
    let reload = move || {
        let mine = generation.get_untracked().wrapping_add(1);
        generation.set(mine);
        state.set(PermissionState::Loading);
        note.set(None);
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
            {move || permission_body(state.get(), locale.get(), level_ctx)}
            <Show when=move || note.get().is_some()>
                <p class="settings-hint">{move || note.get().unwrap_or_default()}</p>
            </Show>
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

fn permission_body(state: PermissionState, locale: Locale, ctx: LevelSelectCtx) -> AnyView {
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
        PermissionState::Loaded(view) => permission_rows(&view, locale, ctx).into_any(),
    }
}

fn permission_rows(v: &PermissionView, locale: Locale, ctx: LevelSelectCtx) -> impl IntoView {
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
        {level_selector(v, ctx)}
        <p class="settings-hint">{i18n::settings_permission_readonly_hint(locale)}</p>
        // 标签写的是「当前等级」，但这里是静态投影；不说清会被当成会话内实际档位。
        <p class="settings-hint">{i18n::settings_permission_static_hint(locale)}</p>
    }
}

/// 档位选择器：只列 `available`，点一下写意图（有会话时立刻写入，无会话时随下一条消息）。
fn level_selector(v: &PermissionView, ctx: LevelSelectCtx) -> impl IntoView {
    let locale = ctx.locale;
    let effective = v.level.clone();
    let options: Vec<(String, String)> = v
        .available
        .iter()
        .map(|raw| {
            (
                raw.clone(),
                i18n::settings_permission_level_label(locale.get_untracked(), raw),
            )
        })
        .collect();
    view! {
        <div class="settings-field">
            <span class="settings-field-label">
                {move || i18n::settings_permission_selector(locale.get())}
            </span>
            <div class="settings-status-line">
                {options
                    .into_iter()
                    .map(|(raw, label)| {
                        let raw_for_active = raw.clone();
                        let raw_for_title = raw.clone();
                        view! {
                            <button
                                type="button"
                                class="btn btn-secondary btn-sm"
                                prop:aria-pressed=move || {
                                    (ctx.selected.get().as_deref() == Some(raw_for_active.as_str()))
                                        .to_string()
                                }
                                prop:title=move || {
                                    i18n::settings_permission_level_label(locale.get(), &raw_for_title)
                                }
                                on:click=move |_| apply_level_selection(raw.as_str(), ctx)
                            >
                                {label}
                            </button>
                        }
                    })
                    .collect_view()}
            </div>
            <p class="settings-hint">
                {move || {
                    intent_line(ctx.selected.get().as_deref(), &effective, locale.get())
                }}
            </p>
        </div>
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

    #[test]
    fn intent_line_shows_both_values_only_when_they_differ() {
        // 意图 = 生效：单值，与改动前的观感一致。
        assert_eq!(
            intent_line(Some("workspace"), "workspace", Locale::ZhHans),
            "工作区"
        );
        // 意图高于生效：两个值都要出现——否则用户会以为「我明明切到不受限，怎么还是只读」。
        assert_eq!(
            intent_line(Some("unrestricted"), "read", Locale::ZhHans),
            "不受限 → 只读"
        );
        // 没表达意图：沿用会话持久值，不猜一个档。
        assert_eq!(
            intent_line(None, "read", Locale::ZhHans),
            "未选择（沿用会话已保存的档位）"
        );
        // 空白等同没有。
        assert_eq!(
            intent_line(Some("  "), "read", Locale::ZhHans),
            "未选择（沿用会话已保存的档位）"
        );
    }
}
