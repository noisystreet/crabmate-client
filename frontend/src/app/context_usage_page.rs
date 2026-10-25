//! 上下文窗口用量可视化全屏页（方案 B：矩形堆叠条 + 图例，纯 CSS、只读）。
//!
//! 复用设置页外壳（`.settings-page*`，非模态、无 `role`），数据与状态栏「会话」芯片同口径：
//! 轴线取自 `status_bar_context_axis` + `context_axis_apply_watermark`，不新增任何协议字段。
//! 方案见 `docs/design/context_usage_visualization.md`。

use leptos::prelude::*;

use crate::chat_session_state::ChatSessionSignals;
use crate::conversation_hydrate::TiktokenPromptTokensSnapshot;
use crate::i18n::{self, Locale};
use crate::icon::icon_chevron_left;

use super::status_context::{
    ContextAxis, context_axis_apply_watermark, context_axis_used_pct, context_value_text,
    format_compact_tokens, status_bar_context_axis, status_bar_context_snapshot_for_active_session,
};
use super::status_tasks_state::StatusTasksSignals;

/// 全屏页输入（与设置页同构，便于在 `app_shell_ctx` 统一装配）。
#[derive(Clone, Copy)]
pub struct ContextUsagePageViewInput {
    pub context_usage_page: RwSignal<bool>,
    pub st: StatusTasksSignals,
    pub chat: ChatSessionSignals,
    pub client_llm_storage_tick: RwSignal<u64>,
    pub selected_agent_role: RwSignal<Option<String>>,
    pub locale: RwSignal<Locale>,
}

/// 堆叠条分段类型；同一类型在三处（条段 / 图例色块 / 图例名）复用同一配色。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BucketKind {
    ToolSchema,
    OtherFixed,
    Message,
    Attachment,
    Used,
    Remaining,
}

#[derive(Clone, Copy)]
struct Bucket {
    kind: BucketKind,
    /// `None` = 服务端未给该项单独计数（图例标「无数据」，段宽按 0）。
    tokens: Option<u32>,
}

struct ContextUsageData {
    /// 条分段用：未施水位（段宽与 `fixed` 拆分自洽）。
    axis_current: ContextAxis,
    /// headline / aria / 告警 / 水位线用：与芯片逐位一致（施会话内水位）。
    axis_watermark: ContextAxis,
    buckets: Vec<Bucket>,
    reserved_output_tokens: Option<u32>,
}

/// 条段类名（`match` 返回完整静态串，供 CSS 契约门禁取证）。
fn context_usage_seg_class(kind: BucketKind) -> &'static str {
    match kind {
        BucketKind::ToolSchema => "context-usage-seg context-usage-fill--tool-schema",
        BucketKind::OtherFixed => "context-usage-seg context-usage-fill--other-fixed",
        BucketKind::Message => "context-usage-seg context-usage-fill--message",
        BucketKind::Attachment => "context-usage-seg context-usage-fill--attachment",
        BucketKind::Used => "context-usage-seg context-usage-fill--used",
        BucketKind::Remaining => "context-usage-seg context-usage-fill--remaining",
    }
}

/// 图例色块类名（与对应条段同配色）。
fn context_usage_swatch_class(kind: BucketKind) -> &'static str {
    match kind {
        BucketKind::ToolSchema => "context-usage-legend-swatch context-usage-fill--tool-schema",
        BucketKind::OtherFixed => "context-usage-legend-swatch context-usage-fill--other-fixed",
        BucketKind::Message => "context-usage-legend-swatch context-usage-fill--message",
        BucketKind::Attachment => "context-usage-legend-swatch context-usage-fill--attachment",
        BucketKind::Used => "context-usage-legend-swatch context-usage-fill--used",
        BucketKind::Remaining => "context-usage-legend-swatch context-usage-fill--remaining",
    }
}

/// 整条类名：估算态（虚线）优先于告警态（描红），两者互斥。
fn context_usage_bar_class(baseline_estimate: bool, warn: bool) -> &'static str {
    if baseline_estimate {
        "context-usage-bar context-usage-bar--estimate"
    } else if warn {
        "context-usage-bar context-usage-bar--warn"
    } else {
        "context-usage-bar"
    }
}

/// headline 数值类名：踩到告警阈值时转告警色（与条描红同源判据）。
fn context_usage_headline_class(warn: bool) -> &'static str {
    if warn {
        "context-usage-headline-value context-usage-headline-value--warn"
    } else {
        "context-usage-headline-value"
    }
}

/// 图例项类名：无数据项弱化。
fn context_usage_legend_item_class(missing: bool) -> &'static str {
    if missing {
        "context-usage-legend-item context-usage-legend-item--missing"
    } else {
        "context-usage-legend-item"
    }
}

/// 分段类型 → i18n key（非类名，仅用于查名称，不参与 CSS 契约）。
fn context_usage_bucket_key(kind: BucketKind) -> &'static str {
    match kind {
        BucketKind::ToolSchema => "tool-schema",
        BucketKind::OtherFixed => "other-fixed",
        BucketKind::Message => "message",
        BucketKind::Attachment => "attachment",
        BucketKind::Used => "used",
        BucketKind::Remaining => "remaining",
    }
}

/// 占比（`tokens / raw_cap`，百分比）；上限为 0 或无数值时返回 0。
fn context_usage_pct(tokens: Option<u32>, raw_cap: u32) -> f64 {
    match tokens {
        Some(t) if raw_cap > 0 => (f64::from(t.min(raw_cap)) / f64::from(raw_cap)) * 100.0,
        _ => 0.0,
    }
}

/// 回退分段：无法拆分固定开销时只出「已用 + 剩余」两段。
fn context_usage_buckets_whole(axis: &ContextAxis) -> Vec<Bucket> {
    let used = axis.used.min(axis.raw_cap);
    let remaining = axis.raw_cap.saturating_sub(axis.used);
    vec![
        Bucket {
            kind: BucketKind::Used,
            tokens: Some(used),
        },
        Bucket {
            kind: BucketKind::Remaining,
            tokens: Some(remaining),
        },
    ]
}

/// 拆出固定开销内部构成：`ToolSchema`（可缺）→ `OtherFixed`（网关等）→ 会话内容 → 剩余。
///
/// `fixed = used − (message + attachment)` 是固定开销总额，`tool_schema` 是其子集，
/// 故 `OtherFixed = fixed − tool_schema`，两段之和恒等于 `fixed`（不双计）。
fn context_usage_buckets_split(
    axis: &ContextAxis,
    snap: &TiktokenPromptTokensSnapshot,
) -> Vec<Bucket> {
    let fixed = axis.fixed;
    let tool = snap.tool_schema_tokens.map(|t| t.min(fixed));
    let other = fixed.saturating_sub(tool.unwrap_or(0));
    let remaining = axis.raw_cap.saturating_sub(axis.used);
    vec![
        Bucket {
            kind: BucketKind::ToolSchema,
            tokens: tool,
        },
        Bucket {
            kind: BucketKind::OtherFixed,
            tokens: Some(other),
        },
        Bucket {
            kind: BucketKind::Message,
            tokens: snap.message_tokens,
        },
        Bucket {
            kind: BucketKind::Attachment,
            tokens: snap.attachment_tokens,
        },
        Bucket {
            kind: BucketKind::Remaining,
            tokens: Some(remaining),
        },
    ]
}

/// 分段有据（非基线估算、有固定开销、快照可拆）时按构成分段，否则回退为「已用 + 剩余」。
fn context_usage_buckets(
    axis: &ContextAxis,
    snap: Option<&TiktokenPromptTokensSnapshot>,
) -> Vec<Bucket> {
    match snap {
        Some(s) if !axis.baseline_estimate && axis.fixed > 0 => {
            context_usage_buckets_split(axis, s)
        }
        _ => context_usage_buckets_whole(axis),
    }
}

/// 取当前活动会话的用量数据；无活动会话 / 无用量时返回 `None`。
fn context_usage_load(
    st: StatusTasksSignals,
    chat: ChatSessionSignals,
    tick: RwSignal<u64>,
    role: RwSignal<Option<String>>,
) -> Option<ContextUsageData> {
    let axis_current = status_bar_context_axis(chat, st, tick, role)?;
    let axis_watermark = context_axis_apply_watermark(axis_current, chat);
    let snap = status_bar_context_snapshot_for_active_session(chat);
    let buckets = context_usage_buckets(&axis_current, snap.as_ref());
    let reserved = snap.as_ref().and_then(|s| s.reserved_output_tokens);
    Some(ContextUsageData {
        axis_current,
        axis_watermark,
        buckets,
        reserved_output_tokens: reserved,
    })
}

/// 图例 / 段 `title`：`名称 · 绝对 token (百分比)`。
fn context_usage_bucket_title(bucket: Bucket, raw_cap: u32, locale: Locale) -> String {
    let name = i18n::context_usage_bucket_name(locale, context_usage_bucket_key(bucket.kind));
    let value = context_usage_bucket_value_text(bucket.tokens, raw_cap, locale);
    format!("{name} · {value}")
}

/// 图例数值文本：有值出 `绝对 token (百分比)`，无值出「无数据」。
fn context_usage_bucket_value_text(tokens: Option<u32>, raw_cap: u32, locale: Locale) -> String {
    match tokens {
        Some(t) => format!(
            "{} ({:.1}%)",
            format_compact_tokens(t),
            context_usage_pct(Some(t), raw_cap)
        ),
        None => i18n::context_usage_no_data(locale).to_string(),
    }
}

/// 单个堆叠条段：宽度按 `raw_cap` 归一（`flex-basis` 为普通 CSS 属性，非自定义属性）。
fn context_usage_segment(bucket: Bucket, raw_cap: u32, locale: Locale) -> AnyView {
    let pct = context_usage_pct(bucket.tokens, raw_cap);
    view! {
        <div
            class=context_usage_seg_class(bucket.kind)
            style=format!("flex-basis: {pct:.4}%;")
            title=context_usage_bucket_title(bucket, raw_cap, locale)
        ></div>
    }
    .into_any()
}

/// 拼装全部条段。
fn context_usage_segments(buckets: &[Bucket], raw_cap: u32, locale: Locale) -> AnyView {
    buckets
        .iter()
        .map(|b| context_usage_segment(*b, raw_cap, locale))
        .collect::<Vec<_>>()
        .into_any()
}

/// 会话内水位竖线：仅当水位高于当前用量时渲染（同轴 `left`）。
fn context_usage_watermark(current: ContextAxis, applied: ContextAxis, raw_cap: u32) -> AnyView {
    if applied.used <= current.used || raw_cap == 0 {
        return ().into_any();
    }
    let pct = (f64::from(applied.used) / f64::from(raw_cap) * 100.0).min(100.0);
    view! {
        <div
            class="context-usage-watermark"
            style=format!("left: {pct:.4}%;")
        ></div>
    }
    .into_any()
}

/// 单个图例项：矩形色块 · 名称 · 数值（不可下钻）。
fn context_usage_legend_item(bucket: Bucket, raw_cap: u32, locale: Locale) -> AnyView {
    let missing = bucket.tokens.is_none();
    let name = i18n::context_usage_bucket_name(locale, context_usage_bucket_key(bucket.kind));
    let value = context_usage_bucket_value_text(bucket.tokens, raw_cap, locale);
    let title = context_usage_bucket_title(bucket, raw_cap, locale);
    view! {
        <div class=context_usage_legend_item_class(missing) title=title>
            <span class=context_usage_swatch_class(bucket.kind)></span>
            <span class="context-usage-legend-name">{name}</span>
            <span class="context-usage-legend-value">{value}</span>
        </div>
    }
    .into_any()
}

/// 图例：缺失项也保留（标「无数据」），不隐藏。
fn context_usage_legend(buckets: &[Bucket], raw_cap: u32, locale: Locale) -> AnyView {
    view! {
        <div class="context-usage-legend">
            {buckets
                .iter()
                .map(|b| context_usage_legend_item(*b, raw_cap, locale))
                .collect::<Vec<_>>()}
        </div>
    }
    .into_any()
}

/// 旁注：刷新时机 / 估算态 / `reserved_output_tokens`（关系未确认）/ 口径方向。
fn context_usage_notes(baseline_estimate: bool, reserved: Option<u32>, locale: Locale) -> AnyView {
    let mut items: Vec<String> = Vec::new();
    if baseline_estimate {
        items.push(i18n::context_usage_note_estimate(locale).to_string());
    }
    items.push(i18n::context_usage_note_refresh(locale).to_string());
    if let Some(tokens) = reserved {
        items.push(i18n::context_usage_reserved_note(locale, tokens));
    }
    items.push(i18n::context_usage_note_reverse(locale).to_string());
    view! {
        <div class="context-usage-notes">
            {items
                .into_iter()
                .map(|t| view! { <p class="context-usage-note">{t}</p> })
                .collect::<Vec<_>>()}
        </div>
    }
    .into_any()
}

/// 有数据时的主体：headline（口径同芯片）+ 堆叠条 + 水位线 + 图例 + 旁注。
fn context_usage_loaded_view(data: &ContextUsageData, locale: Locale) -> AnyView {
    let axis = data.axis_watermark;
    let current = data.axis_current;
    let summary = context_value_text(axis);
    let current_summary = context_value_text(current);
    // 峰值虚线仅在「峰值高于当前」时渲染；aria 与之同步，避免描述不存在的虚线。
    let peak = (axis.used > current.used && axis.raw_cap != 0).then(|| summary.clone());
    let aria = i18n::context_usage_bar_aria(locale, &current_summary, peak.as_deref());
    let warn = context_axis_used_pct(axis) >= 90.0;
    view! {
        <div class="context-usage">
            <div class="context-usage-headline">
                <span class=context_usage_headline_class(warn)>{summary}</span>
            </div>
            <div class="context-usage-bar-wrap">
                <div
                    class=context_usage_bar_class(axis.baseline_estimate, warn)
                    role="img"
                    aria-label=aria
                >
                    {context_usage_segments(&data.buckets, axis.raw_cap, locale)}
                    {context_usage_watermark(current, axis, axis.raw_cap)}
                </div>
                <div class="context-usage-scale" aria-hidden="true">
                    <span>"0"</span>
                    <span>"50%"</span>
                    <span>"100%"</span>
                </div>
            </div>
            {context_usage_legend(&data.buckets, axis.raw_cap, locale)}
            {context_usage_notes(
                axis.baseline_estimate,
                data.reserved_output_tokens,
                locale,
            )}
        </div>
    }
    .into_any()
}

/// 页面主体：有数据出图，否则出空态。
fn context_usage_page_body(
    st: StatusTasksSignals,
    chat: ChatSessionSignals,
    tick: RwSignal<u64>,
    role: RwSignal<Option<String>>,
    locale: Locale,
) -> AnyView {
    match context_usage_load(st, chat, tick, role) {
        Some(data) => context_usage_loaded_view(&data, locale),
        None => view! {
            <div class="context-usage context-usage-empty">
                {i18n::context_usage_empty(locale)}
            </div>
        }
        .into_any(),
    }
}

/// 页头：复用设置页 header 外壳（返回按钮 + 标题 + 徽标），自绘以避开设置页的「丢弃 / 保存」三按钮。
#[component]
fn ContextUsagePageHeader(
    locale: RwSignal<Locale>,
    context_usage_page: RwSignal<bool>,
) -> impl IntoView {
    view! {
        <div class="settings-page-header">
            <button
                type="button"
                class="btn btn-ghost settings-page-back"
                data-testid="context-usage-back"
                prop:aria-label=move || i18n::context_usage_back(locale.get())
                on:click=move |_| context_usage_page.set(false)
            >
                {icon_chevron_left("")}
                <span>{move || i18n::context_usage_back(locale.get())}</span>
            </button>
            <h1 class="settings-page-title">{move || i18n::context_usage_title(locale.get())}</h1>
            <span class="settings-page-badge">
                {move || i18n::context_usage_badge(locale.get())}
            </span>
            <span class="settings-page-head-spacer"></span>
        </div>
    }
}

/// 上下文用量全屏页（非模态；由状态栏「会话」芯片点击进入，Esc / 返回按钮关闭）。
#[component]
pub fn ContextUsagePageView(input: ContextUsagePageViewInput) -> impl IntoView {
    let ContextUsagePageViewInput {
        context_usage_page,
        st,
        chat,
        client_llm_storage_tick,
        selected_agent_role,
        locale,
    } = input;
    view! {
        <div class="settings-page" class:settings-page-visible=move || context_usage_page.get()>
            <ContextUsagePageHeader locale=locale context_usage_page=context_usage_page />
            <div class="settings-page-body">
                {move || {
                    context_usage_page_body(
                        st,
                        chat,
                        client_llm_storage_tick,
                        selected_agent_role,
                        locale.get(),
                    )
                }}
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn axis(used: u32, fixed: u32, raw_cap: u32, baseline_estimate: bool) -> ContextAxis {
        ContextAxis {
            used,
            fixed,
            raw_cap,
            baseline_estimate,
        }
    }

    fn snap(
        message: Option<u32>,
        tool: Option<u32>,
        attachment: Option<u32>,
    ) -> TiktokenPromptTokensSnapshot {
        TiktokenPromptTokensSnapshot {
            prompt_tokens: 0,
            tiktoken_model: "test".to_string(),
            used_input_tokens: None,
            max_input_tokens: None,
            reserved_output_tokens: None,
            message_tokens: message,
            tool_schema_tokens: tool,
            attachment_tokens: attachment,
            counting_source: None,
            provider_input_tokens: None,
        }
    }

    fn tokens_of(buckets: &[Bucket], kind: BucketKind) -> Option<u32> {
        buckets
            .iter()
            .find(|b| b.kind == kind)
            .and_then(|b| b.tokens)
    }

    fn has_kind(buckets: &[Bucket], kind: BucketKind) -> bool {
        buckets.iter().any(|b| b.kind == kind)
    }

    #[test]
    fn pct_clamps_to_cap_and_defaults_to_zero() {
        assert!((context_usage_pct(Some(50), 100) - 50.0).abs() < 1e-9);
        // 超窗夹到 100%，不越界。
        assert!((context_usage_pct(Some(150), 100) - 100.0).abs() < 1e-9);
        // 分母为 0 或无数值都记 0。
        assert!(context_usage_pct(Some(50), 0).abs() < 1e-9);
        assert!(context_usage_pct(None, 100).abs() < 1e-9);
    }

    #[test]
    fn whole_buckets_are_used_plus_remaining() {
        let b = context_usage_buckets_whole(&axis(30, 0, 100, false));
        assert_eq!(tokens_of(&b, BucketKind::Used), Some(30));
        assert_eq!(tokens_of(&b, BucketKind::Remaining), Some(70));
    }

    #[test]
    fn whole_buckets_clamp_when_used_over_cap() {
        let b = context_usage_buckets_whole(&axis(130, 0, 100, false));
        assert_eq!(tokens_of(&b, BucketKind::Used), Some(100));
        assert_eq!(tokens_of(&b, BucketKind::Remaining), Some(0));
    }

    #[test]
    fn split_buckets_sum_to_raw_cap_without_double_count() {
        // used=80、fixed=30 → message+attachment=50（fixed=used−内容）；tool 属 fixed 子集。
        let s = snap(Some(40), Some(12), Some(10));
        let b = context_usage_buckets_split(&axis(80, 30, 100, false), &s);
        assert_eq!(tokens_of(&b, BucketKind::ToolSchema), Some(12));
        assert_eq!(tokens_of(&b, BucketKind::OtherFixed), Some(18));
        assert_eq!(tokens_of(&b, BucketKind::Message), Some(40));
        assert_eq!(tokens_of(&b, BucketKind::Attachment), Some(10));
        assert_eq!(tokens_of(&b, BucketKind::Remaining), Some(20));
        let total: u32 = b.iter().map(|x| x.tokens.unwrap_or(0)).sum();
        assert_eq!(total, 100);
    }

    #[test]
    fn split_buckets_fold_missing_tool_schema_into_other_fixed() {
        let s = snap(Some(40), None, None);
        let b = context_usage_buckets_split(&axis(70, 30, 100, false), &s);
        assert_eq!(tokens_of(&b, BucketKind::ToolSchema), None);
        assert_eq!(tokens_of(&b, BucketKind::OtherFixed), Some(30));
        assert_eq!(tokens_of(&b, BucketKind::Attachment), None);
    }

    #[test]
    fn buckets_fall_back_to_whole_without_split_evidence() {
        let s = snap(Some(40), Some(12), None);
        // 无快照 / 基线估算 / 固定开销为 0 三种情况都回退到「已用 + 剩余」。
        assert!(has_kind(
            &context_usage_buckets(&axis(80, 30, 100, false), None),
            BucketKind::Used
        ));
        assert!(has_kind(
            &context_usage_buckets(&axis(80, 30, 100, true), Some(&s)),
            BucketKind::Used
        ));
        assert!(has_kind(
            &context_usage_buckets(&axis(80, 0, 100, false), Some(&s)),
            BucketKind::Used
        ));
        // 有据则按构成拆分，不再出现合并的 `Used` 段。
        assert!(!has_kind(
            &context_usage_buckets(&axis(80, 30, 100, false), Some(&s)),
            BucketKind::Used
        ));
    }

    #[test]
    fn bar_class_prefers_estimate_over_warn() {
        assert_eq!(
            context_usage_bar_class(true, true),
            "context-usage-bar context-usage-bar--estimate"
        );
        assert_eq!(
            context_usage_bar_class(false, true),
            "context-usage-bar context-usage-bar--warn"
        );
        assert_eq!(context_usage_bar_class(false, false), "context-usage-bar");
    }

    #[test]
    fn headline_class_marks_warn_state() {
        assert_eq!(
            context_usage_headline_class(true),
            "context-usage-headline-value context-usage-headline-value--warn"
        );
        assert_eq!(
            context_usage_headline_class(false),
            "context-usage-headline-value"
        );
    }
}
