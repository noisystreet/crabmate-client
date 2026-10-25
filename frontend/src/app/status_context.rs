//! 底栏「会话」上下文芯片：`used / 上限` 用量、固定开销灰段、会话内滚动水位。
//!
//! 从 `status_bar` 抽出，避免单文件越过行数门禁；契约面仅 `StatusBarContextChip` 与
//! `observe_context_watermark`。

use leptos::prelude::*;

use crate::api::load_client_llm_text_fields_from_storage;
use crate::app_prefs::{
    status_bar_context_used_tokens_without_hydrate, status_bar_effective_llm_context_tokens,
};
use crate::chat_session_state::ChatSessionSignals;
use crate::conversation_hydrate::TiktokenPromptTokensSnapshot;
use crate::i18n::{self, Locale};

use super::status_tasks_state::StatusTasksSignals;

use crabmate_client_api::context_axis::{
    CONTEXT_WARN_PCT, context_axis_apply_watermark as shared_axis_apply_watermark,
    context_axis_from_snapshot, context_axis_whole, context_placeholder_text,
};
/// 轴线与紧凑 token 文本已下沉到 `crabmate_client_api::context_axis`（S11，与 TUI 共用同一口径）。
/// 这里 re-export，保住 `super::status_context::…` 的既有引用路径（`context_usage_page`、
/// `chat::tui_body_chunks` 等）。
pub(super) use crabmate_client_api::context_axis::{
    ContextAxis, context_axis_used_pct, context_value_text, format_compact_cap_tokens,
    format_compact_tokens,
};

/// 上下文用量条填充类名：估算基线优先于告警阈值（两者互斥，先判基线）。
fn status_context_meter_fill_class(is_baseline_estimate: bool, full_pct: f64) -> &'static str {
    if is_baseline_estimate {
        "status-context-meter-fill status-context-meter-fill--estimate"
    } else if full_pct >= CONTEXT_WARN_PCT {
        "status-context-meter-fill status-context-meter-fill--warn"
    } else {
        "status-context-meter-fill"
    }
}

/// 有活动会话即展示「会话」芯片；用量在水合完成前显示 `— / 上限`。
fn status_bar_context_chip_visible(chat: ChatSessionSignals) -> bool {
    !chat.active_id.get().is_empty()
}

pub(super) fn status_bar_context_snapshot_for_active_session(
    chat: ChatSessionSignals,
) -> Option<TiktokenPromptTokensSnapshot> {
    let snap = chat.conversation_prompt_tokens.get()?;
    let aid = chat.active_id.get_untracked();
    let cid_matches = chat.sessions.with_untracked(|list| {
        list.iter().find(|s| s.id == aid).and_then(|s| {
            s.trimmed_server_conversation_id()
                .map(|c| c == snap.conversation_id.as_str())
        })
    });
    if cid_matches != Some(true) {
        return None;
    }
    snap.tiktoken
}

fn status_bar_context_used_for_active_session(chat: ChatSessionSignals) -> Option<u32> {
    let snap = status_bar_context_snapshot_for_active_session(chat)?;
    snap.used_input_tokens.or(Some(snap.prompt_tokens))
}

fn active_session_server_conversation_id(chat: ChatSessionSignals) -> Option<String> {
    let aid = chat.active_id.get_untracked();
    chat.sessions.with_untracked(|list| {
        list.iter()
            .find(|s| s.id == aid)
            .and_then(|s| s.trimmed_server_conversation_id())
            .map(str::to_string)
    })
}

fn active_session_has_server_conversation_id(chat: ChatSessionSignals) -> bool {
    active_session_server_conversation_id(chat).is_some()
}

fn status_bar_context_effective_used(
    chat: ChatSessionSignals,
    st: StatusTasksSignals,
    selected_agent_role: Option<&str>,
) -> Option<u32> {
    if let Some(n) = status_bar_context_used_for_active_session(chat) {
        return Some(n);
    }
    let role = selected_agent_role.map(str::trim).filter(|s| !s.is_empty());
    status_bar_context_used_tokens_without_hydrate(
        active_session_has_server_conversation_id(chat),
        st.status_data.get().as_ref(),
        role,
    )
}

/// 新会话、尚无服务端实测 tiktoken 时，展示 `GET /status` 的 system 基线（带 `~` 前缀）。
fn status_bar_context_is_baseline_estimate(chat: ChatSessionSignals, used: Option<u32>) -> bool {
    used.is_some()
        && status_bar_context_used_for_active_session(chat).is_none()
        && !active_session_has_server_conversation_id(chat)
}

pub(super) fn status_bar_context_cap_and_used(
    chat: ChatSessionSignals,
    st: StatusTasksSignals,
    client_llm_storage_tick: RwSignal<u64>,
    selected_agent_role: RwSignal<Option<String>>,
) -> (u32, Option<u32>) {
    let _tick = client_llm_storage_tick.get();
    let sd = st.status_data.get();
    let (_, _, _, stored_ctx, _) = load_client_llm_text_fields_from_storage();
    // 分母口径：上下文窗口（本机设置上限优先 → `/status`），会话内恒定、可预期。
    // 若首轮前用窗口、首轮后切成服务端 `max_input_tokens`（= 窗口 − 输出预留 − 安全余量），
    // 分母会在 1000k / 993k 之间跳变、引起困惑，故 `max_input_tokens` 仅作上限未知时的最后回退。
    let window = status_bar_effective_llm_context_tokens(sd.as_ref(), stored_ctx.as_str());
    let cap = if window > 0 {
        window
    } else {
        status_bar_context_snapshot_for_active_session(chat)
            .and_then(|snapshot| snapshot.max_input_tokens)
            .unwrap_or(0)
    };
    let role_sel = selected_agent_role.get();
    let role = role_sel.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let used = status_bar_context_effective_used(chat, st, role);
    (cap, used)
}

/// 计算当前活动会话的上下文轴线（`None` = 无活动会话 / 无用量）。
pub(super) fn status_bar_context_axis(
    chat: ChatSessionSignals,
    st: StatusTasksSignals,
    client_llm_storage_tick: RwSignal<u64>,
    selected_agent_role: RwSignal<Option<String>>,
) -> Option<ContextAxis> {
    let (raw_cap, used) =
        status_bar_context_cap_and_used(chat, st, client_llm_storage_tick, selected_agent_role);
    let used = used?;
    if raw_cap == 0 {
        return None;
    }
    if status_bar_context_is_baseline_estimate(chat, Some(used)) {
        return Some(context_axis_whole(raw_cap, used, true));
    }
    Some(match status_bar_context_snapshot_for_active_session(chat) {
        Some(snap) => context_axis_from_snapshot(&snap, raw_cap, used),
        None => context_axis_whole(raw_cap, used, false),
    })
}

/// 观测用量水位：随快照刷新把服务端 `used_input_tokens` 写入 [`ChatSessionSignals::context_used_watermark`]。
///
/// 基线估算（无服务端实测）不参与水位；水位按服务端 `conversation_id` 归属（`ContextUsedWatermark`
/// 内部按 key 重置），故重发流换新 cid 时旧峰值不会沿用。
pub(super) fn observe_context_watermark(
    chat: ChatSessionSignals,
    st: StatusTasksSignals,
    client_llm_storage_tick: RwSignal<u64>,
    selected_agent_role: RwSignal<Option<String>>,
) {
    let Some(axis) =
        status_bar_context_axis(chat, st, client_llm_storage_tick, selected_agent_role)
    else {
        return;
    };
    if axis.baseline_estimate {
        return;
    }
    let Some(key) = active_session_server_conversation_id(chat) else {
        return;
    };
    chat.context_used_watermark
        .update(|w| w.observe(&key, axis.used));
}

/// 施加用量水位（单调不减）：基线估算不施水位；水位归属 cid 与当前会话不一致时也不施，
/// 避免切会话 / 重发流换 cid 的瞬间把旧峰值施加到新会话上。
///
/// 口径实现在 `crabmate_client_api::context_axis`；这里只负责从会话信号取出当前 cid 与水位。
pub(super) fn context_axis_apply_watermark(
    axis: ContextAxis,
    chat: ChatSessionSignals,
) -> ContextAxis {
    if axis.baseline_estimate {
        return axis;
    }
    let watermark = chat.context_used_watermark.get();
    shared_axis_apply_watermark(
        axis,
        active_session_server_conversation_id(chat).as_deref(),
        &watermark,
    )
}

/// 上下文用量条：灰段=固定开销 F，brand 段=会话内容 `used − F`（含水位）。
///
/// 两段同轴叠加，刻度为同一 `raw_cap`（上下文窗口：本机设置上限优先，否则 `/status`）；
/// 两段之和 `used / raw_cap` 与文本刻意同归一化；
/// 仅在有用量时渲染。
#[component]
fn StatusBarContextMeter(
    chat: ChatSessionSignals,
    st: StatusTasksSignals,
    client_llm_storage_tick: RwSignal<u64>,
    selected_agent_role: RwSignal<Option<String>>,
) -> impl IntoView {
    move || {
        let Some(axis) =
            status_bar_context_axis(chat, st, client_llm_storage_tick, selected_agent_role)
        else {
            return ().into_any();
        };
        let axis = context_axis_apply_watermark(axis, chat);
        let raw = f64::from(axis.raw_cap.max(1));
        let fixed_pct = (f64::from(axis.fixed) / raw * 100.0).min(100.0);
        let content_pct =
            (f64::from(axis.used.saturating_sub(axis.fixed)) / raw * 100.0).min(100.0);
        let fill_class =
            status_context_meter_fill_class(axis.baseline_estimate, context_axis_used_pct(axis));
        view! {
            <div
                class="status-context-meter"
                style=format!(
                    "--status-context-fixed-pct: {fixed_pct:.2}%; --status-context-pct: {content_pct:.2}%",
                )
            >
                <div class="status-context-meter-fixed"></div>
                <div class=fill_class></div>
            </div>
        }
        .into_any()
    }
}

/// 「会话」芯片的键盘激活：Enter / Space 等价于点击（打开用量可视化页）。
fn status_context_chip_activate_key(ev: &web_sys::KeyboardEvent) -> bool {
    ev.key() == "Enter" || ev.key() == " "
}

#[component]
pub fn StatusBarContextChip(
    st: StatusTasksSignals,
    chat: ChatSessionSignals,
    client_llm_storage_tick: RwSignal<u64>,
    selected_agent_role: RwSignal<Option<String>>,
    locale: RwSignal<Locale>,
    context_usage_page: RwSignal<bool>,
) -> impl IntoView {
    let open_page = move || context_usage_page.set(true);
    view! {
        <Show when=move || status_bar_context_chip_visible(chat)>
            <span
                class="status-chip status-chip-context"
                role="button"
                tabindex="0"
                prop:title=move || i18n::status_chip_context_tooltip(locale.get())
                prop:aria-label=move || i18n::status_chip_context_tooltip(locale.get())
                on:click=move |_| open_page()
                on:keydown=move |ev: web_sys::KeyboardEvent| {
                    if status_context_chip_activate_key(&ev) {
                        ev.prevent_default();
                        open_page();
                    }
                }
            >
                <span class="status-chip-context-row">
                    <span class="status-chip-label">
                        {move || i18n::status_chip_context(locale.get())}
                    </span>
                    <span class="status-chip-value">{move || {
                        match status_bar_context_axis(
                            chat,
                            st,
                            client_llm_storage_tick,
                            selected_agent_role,
                        ) {
                            Some(axis) => {
                                let axis = context_axis_apply_watermark(axis, chat);
                                context_value_text(axis)
                            }
                            None => {
                                let (raw_cap, _) = status_bar_context_cap_and_used(
                                    chat,
                                    st,
                                    client_llm_storage_tick,
                                    selected_agent_role,
                                );
                                context_placeholder_text(raw_cap)
                            }
                        }
                    }}</span>
                    <StatusBarContextMeter
                        chat=chat
                        st=st
                        client_llm_storage_tick=client_llm_storage_tick
                        selected_agent_role=selected_agent_role
                    />
                </span>
            </span>
        </Show>
    }
}

#[cfg(test)]
mod tests {
    use super::status_context_meter_fill_class;

    #[test]
    fn context_meter_fill_prefers_estimate_over_warn() {
        // 估算基线即使已过 90% 也只出 `--estimate`，两个修饰位互斥。
        assert_eq!(
            status_context_meter_fill_class(true, 95.0),
            "status-context-meter-fill status-context-meter-fill--estimate"
        );
    }

    #[test]
    fn context_meter_fill_warns_at_threshold() {
        assert_eq!(
            status_context_meter_fill_class(false, 90.0),
            "status-context-meter-fill status-context-meter-fill--warn"
        );
        assert_eq!(
            status_context_meter_fill_class(false, 89.9),
            "status-context-meter-fill"
        );
    }
}
