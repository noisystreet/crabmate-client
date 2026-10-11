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

/// 上下文用量条填充类名：估算基线优先于告警阈值（两者互斥，先判基线）。
fn status_context_meter_fill_class(is_baseline_estimate: bool, full_pct: f64) -> &'static str {
    if is_baseline_estimate {
        "status-context-meter-fill status-context-meter-fill--estimate"
    } else if full_pct >= 90.0 {
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

/// 上下文轴线：显示口径 `used / raw_cap`（分子分母都含每轮固定开销），用量条再拆出灰色底衬 `fixed`。
///
/// 分母 `raw_cap` 取上下文窗口（本机设置「上下文 token 上限」优先 → `GET /status`），会话内恒定、可预期，
/// 避免首轮前后在两个口径间跳变；仅当窗口未知时才退回服务端 `max_input_tokens`（= 窗口减输出预留与安全余量）。
/// 固定开销 `F = used − (message_tokens + attachment_tokens)` 仍单独拆出，仅用于用量条把「每轮都付的底衬」
/// 与「会话连续增长的内容」分色。服务端每轮按当前 user 消息关键词重选 L5 Skills 正文会让 `used` 波动，
/// 故对 `used` 取会话内滚动水位（见 `ContextUsedWatermark`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ContextAxis {
    /// 当前用量 `used_input_tokens`（含固定开销；未施水位）。
    pub(super) used: u32,
    /// 固定开销 F = `used − (message_tokens + attachment_tokens)`（用量条灰段）。
    pub(super) fixed: u32,
    /// 文本分母 + 用量条刻度（上下文窗口：本机设置上限优先，否则 `/status`）。
    pub(super) raw_cap: u32,
    /// 无服务端实测、仅有 system 基线估算（文本带 `~`）。
    pub(super) baseline_estimate: bool,
}

/// 紧凑 token 文本：≥1 万用 `k`（一位小数，如 `116.9k`），否则原样整数。
pub(super) fn format_compact_tokens(n: u32) -> String {
    if n >= 10_000 {
        format!("{:.1}k", f64::from(n) / 1000.0)
    } else {
        n.to_string()
    }
}

/// 上限（`max_input_tokens`）紧凑文本：≥1 万用整 `k`、不带小数（如 `200k` / `1000k`），否则原样整数。
pub(super) fn format_compact_cap_tokens(n: u32) -> String {
    if n >= 10_000 {
        format!("{:.0}k", f64::from(n) / 1000.0)
    } else {
        n.to_string()
    }
}

/// 固定开销记 0（无拆分字段或基线估算时），分子分母同为 `used / raw_cap`。
fn context_axis_whole(raw_cap: u32, used: u32, baseline_estimate: bool) -> ContextAxis {
    ContextAxis {
        used,
        fixed: 0,
        raw_cap,
        baseline_estimate,
    }
}

/// 从快照拆出 `(used, F)`；缺 `used_input_tokens` / `message_tokens` 返回 `None`（调用方回退为固定开销记 0）。
///
/// DTO 无 `vendor_overhead_tokens`，故 F 反推为 `used − (message + attachment)`（不硬编码常量）。
/// 刻度 `raw_cap` 由调用方给定（本机设置上限优先），不取快照自身的 `max_input_tokens`。
fn context_axis_split(snap: &TiktokenPromptTokensSnapshot) -> Option<(u32, u32)> {
    let used = snap.used_input_tokens?;
    let message = snap.message_tokens?;
    let content = message.saturating_add(snap.attachment_tokens.unwrap_or(0));
    Some((used, used.saturating_sub(content)))
}

/// 快照 → 轴线：能拆则拆出固定开销灰段，否则固定开销记 0。
fn context_axis_from_snapshot(
    snap: &TiktokenPromptTokensSnapshot,
    raw_cap: u32,
    used: u32,
) -> ContextAxis {
    match context_axis_split(snap) {
        Some((used, fixed)) => ContextAxis {
            used,
            fixed,
            raw_cap,
            baseline_estimate: false,
        },
        None => context_axis_whole(raw_cap, used, false),
    }
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
pub(super) fn context_axis_apply_watermark(
    axis: ContextAxis,
    chat: ChatSessionSignals,
) -> ContextAxis {
    if axis.baseline_estimate {
        return axis;
    }
    let watermark = chat.context_used_watermark.get();
    if active_session_server_conversation_id(chat).as_deref() != Some(watermark.key()) {
        return axis;
    }
    ContextAxis {
        used: axis.used.max(watermark.used()),
        ..axis
    }
}

/// 用量占比（`used / raw_cap`，百分比）；上限为 0 时返回 0。
pub(super) fn context_axis_used_pct(axis: ContextAxis) -> f64 {
    if axis.raw_cap == 0 {
        0.0
    } else {
        (f64::from(axis.used) / f64::from(axis.raw_cap)) * 100.0
    }
}

/// 底栏文案：`[~]used_k / max_k (pct%)`；上限为 0 时省略百分比。
pub(super) fn status_bar_context_value_text(axis: ContextAxis) -> String {
    let prefix = if axis.baseline_estimate { "~" } else { "" };
    if axis.raw_cap == 0 {
        return format!("{prefix}{}", format_compact_tokens(axis.used));
    }
    format!(
        "{prefix}{} / {} ({:.1}%)",
        format_compact_tokens(axis.used),
        format_compact_cap_tokens(axis.raw_cap),
        context_axis_used_pct(axis).min(999.9)
    )
}

/// 无轴线（无活动会话 / 无用量）时的占位文案：`— / B_k`（无上限则 `—`）。
pub(super) fn status_bar_context_placeholder_text(raw_cap: u32) -> String {
    if raw_cap > 0 {
        format!("— / {}", format_compact_cap_tokens(raw_cap))
    } else {
        "—".to_string()
    }
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
                                status_bar_context_value_text(axis)
                            }
                            None => {
                                let (raw_cap, _) = status_bar_context_cap_and_used(
                                    chat,
                                    st,
                                    client_llm_storage_tick,
                                    selected_agent_role,
                                );
                                status_bar_context_placeholder_text(raw_cap)
                            }
                        }
                    }}</span>
                </span>
                <StatusBarContextMeter
                    chat=chat
                    st=st
                    client_llm_storage_tick=client_llm_storage_tick
                    selected_agent_role=selected_agent_role
                />
            </span>
        </Show>
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ContextAxis, format_compact_cap_tokens, format_compact_tokens,
        status_bar_context_value_text, status_context_meter_fill_class,
    };

    #[test]
    fn cap_tokens_drop_decimal_while_used_keeps_it() {
        // 上限用整 `k`：1000.0k → 1000k。
        assert_eq!(format_compact_cap_tokens(1_000_000), "1000k");
        assert_eq!(format_compact_cap_tokens(200_000), "200k");
        assert_eq!(format_compact_cap_tokens(9_999), "9999");
        // 已用仍保留一位小数。
        assert_eq!(format_compact_tokens(116_900), "116.9k");
    }

    #[test]
    fn value_text_renders_cap_without_decimal() {
        let axis = ContextAxis {
            used: 116_900,
            fixed: 0,
            raw_cap: 1_000_000,
            baseline_estimate: false,
        };
        assert_eq!(
            status_bar_context_value_text(axis),
            "116.9k / 1000k (11.7%)"
        );
    }

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
