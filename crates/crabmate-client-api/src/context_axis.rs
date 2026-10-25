//! 上下文用量口径（S11）：`used / raw_cap` 轴线、固定开销拆分、会话内滚动水位、紧凑 token 文本。
//!
//! 原实现在 `frontend/src/app/status_context.rs`（轴线与文本）与 `frontend/src/chat_session_state.rs`
//! （`ContextUsedWatermark`）。抽出动机是 ADE 0005 要求各端状态栏「同口径 fill + 压缩后 drop」——
//! 口径一旦各端各写一份，就只在特定输入下才暴露漂移。纯逻辑，无 IO。
//!
//! **为什么分母不用快照里的 `max_input_tokens`**：分母取上下文窗口（本机设置「上下文 token 上限」
//! 优先 → `GET /status`），会话内恒定、可预期；`max_input_tokens`（= 窗口 − 输出预留 − 安全余量）
//! 只在窗口未知时兜底。若首轮前用窗口、首轮后切成 `max_input_tokens`，分母会在 1000k / 993k
//! 之间跳变。
//!
//! **为什么分子要拆出 `fixed`**：`F = used − (message_tokens + attachment_tokens)` 是每轮都付的
//! 固定开销（系统提示、工具 schema、技能正文等）。DTO 没有 `vendor_overhead_tokens`，故只能反推。
//! 用量条把 `F` 与「会话连续增长的内容」分色，但两者之和刻意与文本 `used / raw_cap` 同归一化。

use crate::prompt_tokens::TiktokenPromptTokensSnapshot;

/// 用量条进入告警档的百分比阈值（两端同一口径）。
pub const CONTEXT_WARN_PCT: f64 = 90.0;

/// 会话内服务端 `used_input_tokens` 的滚动水位，按 `key`（服务端 `conversation_id`）归属。
///
/// 服务端每轮按当前 user 消息关键词重选 L5 Skills 正文，`used` 会来回波动，直接显示会让数字
/// 反复跳动，故按会话取滚动最大。切会话或重发流换新 cid 时 `key` 变化即重置，旧峰值不会沿用。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContextUsedWatermark {
    key: String,
    max_used: u32,
}

impl ContextUsedWatermark {
    /// 记录一次观测：`key` 变化（切会话 / 重发流换 cid）则重置为 `used`，否则取滚动最大。
    pub fn observe(&mut self, key: &str, used: u32) {
        if self.key != key {
            self.key = key.to_string();
            self.max_used = used;
        } else if used > self.max_used {
            self.max_used = used;
        }
    }

    /// 压缩 / 裁剪后显式重置水位：`key` 与水位**无条件**置为本次观测值，**允许回落**。
    ///
    /// 与 [`Self::observe`] 的滚动最大不同，裁剪 / 压缩会令服务端 `used_input_tokens` 真实下降，
    /// 若继续取滚动最大，状态栏会停在裁剪前的旧峰值。`used` 取 `context_trim` 事件的
    /// `after_tokens`（见 `timeline_scan::context_trim_after_used_tokens`）。
    pub fn reset(&mut self, key: &str, used: u32) {
        self.key = key.to_string();
        self.max_used = used;
    }

    /// 当前水位（服务端 `used_input_tokens`；未观测过为 0）。
    #[must_use]
    pub fn used(&self) -> u32 {
        self.max_used
    }

    /// 水位归属的 `key`（服务端 `conversation_id`）；未观测过为空串。
    ///
    /// 读侧据此校验水位是否属于当前会话，避免切会话 / 重发流换 cid 的瞬间把旧峰值施加到新会话。
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }
}

/// 上下文轴线：显示口径 `used / raw_cap`（分子分母都含每轮固定开销），用量条再拆出灰段 `fixed`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContextAxis {
    /// 当前用量 `used_input_tokens`（含固定开销；未施水位）。
    pub used: u32,
    /// 固定开销 `F = used − (message_tokens + attachment_tokens)`（用量条灰段）。
    pub fixed: u32,
    /// 文本分母 + 用量条刻度（上下文窗口：本机设置上限优先，否则 `/status`）。
    pub raw_cap: u32,
    /// 无服务端实测、仅有 system 基线估算（文本带 `~`）。
    pub baseline_estimate: bool,
}

/// 紧凑 token 文本：≥1 万用 `k`（一位小数，如 `116.9k`），否则原样整数。
#[must_use]
pub fn format_compact_tokens(n: u32) -> String {
    if n >= 10_000 {
        format!("{:.1}k", f64::from(n) / 1000.0)
    } else {
        n.to_string()
    }
}

/// 上限紧凑文本：≥1 万用整 `k`、不带小数（如 `200k` / `1000k`），否则原样整数。
#[must_use]
pub fn format_compact_cap_tokens(n: u32) -> String {
    if n >= 10_000 {
        format!("{:.0}k", f64::from(n) / 1000.0)
    } else {
        n.to_string()
    }
}

/// 固定开销记 0（无拆分字段或基线估算时），分子分母同为 `used / raw_cap`。
#[must_use]
pub fn context_axis_whole(raw_cap: u32, used: u32, baseline_estimate: bool) -> ContextAxis {
    ContextAxis {
        used,
        fixed: 0,
        raw_cap,
        baseline_estimate,
    }
}

/// 从快照拆出 `(used, F)`；缺 `used_input_tokens` / `message_tokens` 返回 `None`（调用方回退为固定开销记 0）。
#[must_use]
pub fn context_axis_split(snap: &TiktokenPromptTokensSnapshot) -> Option<(u32, u32)> {
    let used = snap.used_input_tokens?;
    let message = snap.message_tokens?;
    let content = message.saturating_add(snap.attachment_tokens.unwrap_or(0));
    Some((used, used.saturating_sub(content)))
}

/// 快照 → 轴线：能拆则拆出固定开销灰段，否则固定开销记 0。
#[must_use]
pub fn context_axis_from_snapshot(
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

/// 施加用量水位（单调不减）：基线估算不施水位；水位归属 cid 与当前会话不一致时也不施，
/// 避免切会话 / 重发流换 cid 的瞬间把旧峰值施加到新会话上。
///
/// `current_conversation_id` 由调用方从会话列表取（共享层不碰会话状态，故显式传参）。
#[must_use]
pub fn context_axis_apply_watermark(
    axis: ContextAxis,
    current_conversation_id: Option<&str>,
    watermark: &ContextUsedWatermark,
) -> ContextAxis {
    if axis.baseline_estimate {
        return axis;
    }
    if current_conversation_id != Some(watermark.key()) {
        return axis;
    }
    ContextAxis {
        used: axis.used.max(watermark.used()),
        ..axis
    }
}

/// 用量占比（`used / raw_cap`，百分比）；上限为 0 时返回 0。
#[must_use]
pub fn context_axis_used_pct(axis: ContextAxis) -> f64 {
    if axis.raw_cap == 0 {
        0.0
    } else {
        (f64::from(axis.used) / f64::from(axis.raw_cap)) * 100.0
    }
}

/// 芯片文案：`[~]used_k / max_k (pct%)`；上限为 0 时省略百分比。
#[must_use]
pub fn context_value_text(axis: ContextAxis) -> String {
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
#[must_use]
pub fn context_placeholder_text(raw_cap: u32) -> String {
    if raw_cap > 0 {
        format!("— / {}", format_compact_cap_tokens(raw_cap))
    } else {
        "—".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ContextAxis, ContextUsedWatermark, context_axis_apply_watermark,
        context_axis_from_snapshot, context_axis_split, context_axis_used_pct, context_axis_whole,
        context_placeholder_text, context_value_text, format_compact_cap_tokens,
        format_compact_tokens,
    };

    fn snap(
        used: Option<u32>,
        message: Option<u32>,
        attachment: Option<u32>,
    ) -> crate::prompt_tokens::TiktokenPromptTokensSnapshot {
        crate::prompt_tokens::TiktokenPromptTokensSnapshot {
            prompt_tokens: used.unwrap_or(0),
            tiktoken_model: "gpt-4o".to_string(),
            used_input_tokens: used,
            max_input_tokens: Some(993_000),
            reserved_output_tokens: None,
            message_tokens: message,
            tool_schema_tokens: None,
            attachment_tokens: attachment,
            counting_source: None,
            provider_input_tokens: None,
        }
    }

    #[test]
    fn cap_tokens_drop_decimal_while_used_keeps_it() {
        // 上限用整 `k`：1000.0k → 1000k。
        assert_eq!(format_compact_cap_tokens(1_000_000), "1000k");
        assert_eq!(format_compact_cap_tokens(200_000), "200k");
        assert_eq!(format_compact_cap_tokens(9_999), "9999");
        // 已用仍保留一位小数。
        assert_eq!(format_compact_tokens(116_900), "116.9k");
        assert_eq!(format_compact_tokens(9_999), "9999");
    }

    #[test]
    fn value_text_renders_cap_without_decimal() {
        let axis = ContextAxis {
            used: 116_900,
            fixed: 0,
            raw_cap: 1_000_000,
            baseline_estimate: false,
        };
        assert_eq!(context_value_text(axis), "116.9k / 1000k (11.7%)");
    }

    #[test]
    fn value_text_marks_baseline_and_omits_pct_without_cap() {
        let estimate = ContextAxis {
            used: 12_000,
            fixed: 0,
            raw_cap: 1_000_000,
            baseline_estimate: true,
        };
        assert!(context_value_text(estimate).starts_with('~'));
        let no_cap = ContextAxis {
            used: 12_000,
            fixed: 0,
            raw_cap: 0,
            baseline_estimate: false,
        };
        assert_eq!(context_value_text(no_cap), "12.0k");
    }

    #[test]
    fn placeholder_shows_cap_or_dash() {
        assert_eq!(context_placeholder_text(200_000), "— / 200k");
        assert_eq!(context_placeholder_text(0), "—");
    }

    #[test]
    fn used_pct_clamps_to_zero_when_cap_missing() {
        let axis = context_axis_whole(0, 50, false);
        assert!((context_axis_used_pct(axis) - 0.0).abs() < f64::EPSILON);
        let axis = context_axis_whole(100, 50, false);
        assert!((context_axis_used_pct(axis) - 50.0).abs() < 1e-9);
    }

    #[test]
    fn split_subtracts_content_from_used() {
        // used 1000 = 内容 300（message 200 + attachment 100）+ 固定开销 700。
        let s = snap(Some(1_000), Some(200), Some(100));
        assert_eq!(context_axis_split(&s), Some((1_000, 700)));
        // attachment 缺失按 0。
        let s = snap(Some(1_000), Some(200), None);
        assert_eq!(context_axis_split(&s), Some((1_000, 800)));
    }

    #[test]
    fn split_returns_none_without_used_or_message() {
        assert_eq!(context_axis_split(&snap(None, Some(200), None)), None);
        assert_eq!(context_axis_split(&snap(Some(1_000), None, None)), None);
    }

    #[test]
    fn split_does_not_underflow_when_content_exceeds_used() {
        // message 比 used 还大（口径不一致的脏快照）→ fixed 记 0，不 panic。
        let s = snap(Some(100), Some(500), None);
        assert_eq!(context_axis_split(&s), Some((100, 0)));
    }

    #[test]
    fn from_snapshot_falls_back_to_whole_without_split_fields() {
        let axis = context_axis_from_snapshot(&snap(Some(1_000), None, None), 1_000_000, 1_000);
        assert_eq!(axis.fixed, 0);
        assert!(!axis.baseline_estimate);
        assert_eq!(axis.raw_cap, 1_000_000);
    }

    #[test]
    fn watermark_rolls_max_within_key_and_resets_on_key_change() {
        let mut wm = ContextUsedWatermark::default();
        assert_eq!(wm.used(), 0);
        assert_eq!(wm.key(), "");
        wm.observe("c1", 100);
        wm.observe("c1", 90); // 回落不记
        assert_eq!(wm.used(), 100);
        wm.observe("c1", 150);
        assert_eq!(wm.used(), 150);
        // 换 cid（重发流）→ 重置，旧峰值不沿用。
        wm.observe("c2", 20);
        assert_eq!(wm.key(), "c2");
        assert_eq!(wm.used(), 20);
    }

    #[test]
    fn watermark_reset_allows_falling_back() {
        // 压缩 / 裁剪后必须允许回落，否则状态栏停在旧峰值。
        let mut wm = ContextUsedWatermark::default();
        wm.observe("c1", 900);
        wm.reset("c1", 120);
        assert_eq!(wm.used(), 120);
    }

    #[test]
    fn apply_watermark_only_when_key_matches() {
        let mut wm = ContextUsedWatermark::default();
        wm.observe("c1", 900);
        let axis = context_axis_whole(1_000, 300, false);
        // 匹配 → 取水位。
        assert_eq!(
            context_axis_apply_watermark(axis, Some("c1"), &wm).used,
            900
        );
        // 不匹配（切会话）→ 原值。
        assert_eq!(
            context_axis_apply_watermark(axis, Some("c2"), &wm).used,
            300
        );
        assert_eq!(context_axis_apply_watermark(axis, None, &wm).used, 300);
    }

    #[test]
    fn apply_watermark_skips_baseline_estimate() {
        // 基线估算没有服务端实测，不参与水位。
        let mut wm = ContextUsedWatermark::default();
        wm.observe("c1", 900);
        let axis = context_axis_whole(1_000, 300, true);
        assert_eq!(
            context_axis_apply_watermark(axis, Some("c1"), &wm).used,
            300
        );
    }
}
