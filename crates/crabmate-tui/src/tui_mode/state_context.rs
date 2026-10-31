//! 上下文用量：`tiktokenPromptTokens` 快照 → 状态行 `ctx` 段文本（ADE 0005 同口径）。
//!
//! 拆成子模块（同 `state_search` / `state_paste`）：`state.rs` 贴着 920 行门禁，而这段是
//! 自成一体的装配 —— 口径本身在 `crabmate_client_api::context_axis`，这里只负责把
//! 「本机设置上限 / 快照 / 会话水位」三者拼成一条轴线。

use crabmate_client_api::chat_body::llm_context_tokens_for_chat_body;
use crabmate_client_api::context_axis::ContextUsedWatermark;
use crabmate_client_api::context_axis::{
    ContextAxis, context_axis_apply_watermark, context_axis_from_snapshot, context_axis_used_pct,
    context_axis_whole, context_placeholder_text, context_value_text,
};
use crabmate_client_api::prompt_tokens::TiktokenPromptTokensSnapshot;

use super::UiState;

/// 上下文用量的两件状态：会话内水位 + 最近一次 `tiktokenPromptTokens` 快照。
///
/// 合成一个字段是为了 `state.rs` 的行数门禁 —— 那里每个字段都要配注释。
#[derive(Clone, Debug, Default)]
pub struct ContextUsage {
    /// 服务端 `used_input_tokens` 的滚动水位，按 `conversation_id` 归属。
    pub watermark: ContextUsedWatermark,
    /// 最近一次 `tiktokenPromptTokens` 的快照：固定开销拆分与上限兜底都用它。
    pub snapshot: Option<TiktokenPromptTokensSnapshot>,
}

impl UiState {
    /// 记录一次用量快照（`tiktokenPromptTokens`）。
    ///
    /// 水位按服务端 `conversation_id` 归属；cid 还没绑上（首轮终止帧可能早于 `TurnDone`
    /// 回填）时**只存快照、不记水位** —— 宁缺不猜：猜错会把旧会话的峰值挂到新会话上。
    pub fn set_prompt_tokens(&mut self, snap: TiktokenPromptTokensSnapshot) {
        if let (Some(used), Some(cid)) = (used_tokens(&snap), self.conversation_id.as_deref()) {
            self.context.watermark.observe(cid, used);
        }
        self.context.snapshot = Some(snap);
    }

    /// `context_trim`（历史压缩 / 工具输出压缩 / 摘要）后把会话内水位**回落**到服务端给的
    /// `after_tokens`。没有 cid 时不动：水位按会话归属，无主的水位没有意义。
    ///
    /// 这是 ADR 0005 里「会话内单调、压缩 / 裁剪后回落」的后半句 —— 少了它，水位只增不减，
    /// 压缩过的会话会一直显示压缩前的峰值。
    pub fn reset_used(&mut self, after: u32) {
        if let Some(cid) = self.conversation_id.clone() {
            self.context.watermark.reset(&cid, after);
        }
    }

    /// 换会话 / 新建会话：清掉用量快照与水位。
    ///
    /// 不清的话 `/conv new` 之后状态栏还在显示上一个会话的数字 —— `context_text` 只看
    /// `snapshot`、不看 cid，那个数字是别人会话的。
    pub fn clear_usage(&mut self) {
        self.context = ContextUsage::default();
    }

    /// 本会话当前是否已有实测快照（换会话的 `clear_usage` 之后、第一份实测到达之前为 `false`）。
    ///
    /// 回合末只有它为 `false` 时才去补拉用量：已实测的一轮没必要再拉一次整页历史。
    #[must_use]
    pub fn has_usage_snapshot(&self) -> bool {
        self.context.snapshot.is_some()
    }

    /// 状态行 `ctx` 段的文本：有实测时 `12.0k / 1000k (1.2%)`，没有时占位 `—`（`— / 1000k`）。
    ///
    /// **总是返回文本**（而不是「无数据就没有段」）：段整个消失时，用户无法区分
    /// 「这个功能没生效」和「这一轮还没产生实测」，排查时只剩猜。
    ///
    /// `cap_from_settings` 是本机设置里的上下文窗口（`context_tokens` 原文），**优先于**快照的
    /// `max_input_tokens`：后者是「窗口 − 输出预留 − 安全余量」，拿它当分母会让数字在首轮
    /// 前后于 1000k / 993k 之间跳变。
    /// `new_session_baseline` 是 `GET /status` 给的 system-only 粗估（按 agent role），
    /// 只在**还没有 cid**（即还没有任何服务端实测）时使用，显示时带 `~` 标明是估算 ——
    /// 与 Desktop 的 `status_bar_context_is_baseline_estimate` 同口径。
    pub fn context_text(
        &self,
        cap_from_settings: Option<&str>,
        new_session_baseline: Option<u32>,
    ) -> String {
        match self.context_axis(cap_from_settings, new_session_baseline) {
            Some(axis) => context_value_text(axis),
            // 没有实测也没有基线：占位分母仍按「有快照就取快照上限」的规则给 ——
            // 段整个消失会让人误判「功能没生效」。
            None => context_placeholder_text(self.placeholder_cap(cap_from_settings)),
        }
    }

    /// 状态行 `ctx` 段的**短形式**：`62%` / `~1%`。
    ///
    /// 左栏 lane 行只有十几列宽，塞不下 `12.0k / 1000k (62%)`；口径与 [`Self::context_text`]
    /// 完全一致（同一条轴线：settings 上限优先、会话内水位、估算带 `~`），只是把分子分母
    /// 省成占比。**没有任何用量信息时返回 `None`** —— 状态行有位置放 `—` 提醒「还没实测」，
    /// lane 行没有，画一个空占位只是噪声。
    pub fn context_pct_text(
        &self,
        cap_from_settings: Option<&str>,
        new_session_baseline: Option<u32>,
    ) -> Option<String> {
        let axis = self.context_axis(cap_from_settings, new_session_baseline)?;
        if axis.raw_cap == 0 {
            return None;
        }
        let prefix = if axis.baseline_estimate { "~" } else { "" };
        Some(format!(
            "{prefix}{:.0}%",
            context_axis_used_pct(axis).min(999.0)
        ))
    }

    /// 把「本机设置上限 / 快照 / 会话水位」拼成一条轴线；`None` = 既没有实测也没有基线。
    ///
    /// 状态行与 lane 行的两种文案都必须走这里：各拼一遍的后果是同一个数字在两处显示成
    /// 不同值（`ctx` 段带水位、lane 行不带），比不显示更难排查。
    fn context_axis(
        &self,
        cap_from_settings: Option<&str>,
        new_session_baseline: Option<u32>,
    ) -> Option<ContextAxis> {
        let Some(snap) = self.context.snapshot.as_ref() else {
            let used = new_session_baseline.filter(|_| self.conversation_id.is_none())?;
            return Some(context_axis_whole(
                settings_cap(cap_from_settings),
                used,
                true,
            ));
        };
        let used = used_tokens(snap)?;
        let cap = cap_tokens(cap_from_settings, snap);
        let axis = context_axis_from_snapshot(snap, cap, used);
        Some(context_axis_apply_watermark(
            axis,
            self.conversation_id.as_deref(),
            &self.context.watermark,
        ))
    }

    /// 占位文本的分母：有快照时用快照上限，否则用本机设置。
    fn placeholder_cap(&self, cap_from_settings: Option<&str>) -> u32 {
        match self.context.snapshot.as_ref() {
            Some(snap) => cap_tokens(cap_from_settings, snap),
            None => settings_cap(cap_from_settings),
        }
    }
}

/// 分子：服务端实测优先，缺 `used_input_tokens` 时退到 `prompt_tokens`（与 Web 同一顺序）。
fn used_tokens(snap: &TiktokenPromptTokensSnapshot) -> Option<u32> {
    Some(snap.used_input_tokens.unwrap_or(snap.prompt_tokens))
}

/// 本机设置里的窗口（`context_tokens` 原文）；空 / 非数字 / 0 一律视为未设置。
fn settings_cap(cap_from_settings: Option<&str>) -> u32 {
    llm_context_tokens_for_chat_body(cap_from_settings)
        .map(|n| u32::try_from(n).unwrap_or(u32::MAX))
        .unwrap_or(0)
}

/// 分母：本机设置上限优先 → 快照 `max_input_tokens` → `0`（表示未知，文本省略百分比）。
fn cap_tokens(cap_from_settings: Option<&str>, snap: &TiktokenPromptTokensSnapshot) -> u32 {
    match settings_cap(cap_from_settings) {
        0 => snap.max_input_tokens.unwrap_or(0),
        n => n,
    }
}

#[cfg(test)]
mod tests {
    use super::{UiState, cap_tokens, used_tokens};
    use crabmate_client_api::prompt_tokens::TiktokenPromptTokensSnapshot;

    fn snap(
        used: Option<u32>,
        prompt: u32,
        max_input: Option<u32>,
    ) -> TiktokenPromptTokensSnapshot {
        TiktokenPromptTokensSnapshot {
            prompt_tokens: prompt,
            tiktoken_model: "gpt-4o".to_string(),
            used_input_tokens: used,
            max_input_tokens: max_input,
            reserved_output_tokens: None,
            message_tokens: None,
            tool_schema_tokens: None,
            attachment_tokens: None,
            counting_source: None,
            provider_input_tokens: None,
        }
    }

    #[test]
    fn settings_cap_wins_over_snapshot_max_input() {
        // 本机设置 100 万优先，不被快照的 993k 顶掉（否则数字会在首轮前后跳变）。
        let s = snap(Some(1_000), 1_000, Some(993_000));
        assert_eq!(cap_tokens(Some("1000000"), &s), 1_000_000);
    }

    #[test]
    fn cap_falls_back_to_snapshot_then_zero() {
        let with_max = snap(Some(1_000), 1_000, Some(993_000));
        assert_eq!(cap_tokens(None, &with_max), 993_000);
        // 空串 / 非数字 / 0 都视为「未设置」，继续回退。
        assert_eq!(cap_tokens(Some("  "), &with_max), 993_000);
        assert_eq!(cap_tokens(Some("abc"), &with_max), 993_000);
        assert_eq!(cap_tokens(Some("0"), &with_max), 993_000);
        let no_max = snap(Some(1_000), 1_000, None);
        assert_eq!(cap_tokens(None, &no_max), 0);
    }

    #[test]
    fn used_prefers_server_count_and_falls_back_to_prompt_tokens() {
        assert_eq!(used_tokens(&snap(Some(500), 400, None)), Some(500));
        assert_eq!(used_tokens(&snap(None, 400, None)), Some(400));
    }

    /// lane 行的短形式与状态行的 `ctx` 段必须同一条轴线，只是省成占比。
    ///
    /// 各算一遍的后果是同一个数字在两处显示成不同值 —— 比不显示更难排查。
    #[test]
    fn pct_text_is_the_short_form_of_the_same_axis() {
        let mut st = UiState::new();
        st.conversation_id = Some("c1".to_string());
        st.set_prompt_tokens(snap(Some(620_000), 620_000, Some(993_000)));
        assert_eq!(
            st.context_pct_text(Some("1000000"), None).as_deref(),
            Some("62%")
        );
        assert!(
            st.context_text(Some("1000000"), None).contains("62.0%"),
            "长形式该是同一个占比：{}",
            st.context_text(Some("1000000"), None)
        );
    }

    /// 没有任何用量信息时短形式返回 `None`：lane 行没有位置放 `—` 这种占位。
    #[test]
    fn pct_text_is_absent_without_measurement_or_baseline() {
        assert_eq!(UiState::new().context_pct_text(Some("1000000"), None), None);
        // 只有基线（新会话）：带 `~`，与 `ctx` 段同口径。
        assert_eq!(
            UiState::new()
                .context_pct_text(Some("1000000"), Some(12_000))
                .as_deref(),
            Some("~1%")
        );
    }

    /// 实测优先于基线，且水位照常生效 —— 短形式不能漏掉这两条规则。
    #[test]
    fn pct_text_follows_watermark_and_prefers_measurement() {
        let mut st = UiState::new();
        st.conversation_id = Some("c1".to_string());
        st.set_prompt_tokens(snap(Some(120_000), 120_000, Some(993_000)));
        // 下一轮实测回落，水位仍显示本会话峰值。
        st.set_prompt_tokens(snap(Some(100_000), 100_000, Some(993_000)));
        assert_eq!(
            st.context_pct_text(Some("1000000"), Some(12_000))
                .as_deref(),
            Some("12%")
        );
    }

    #[test]
    fn snapshot_becomes_status_text_end_to_end() {
        let mut st = UiState::new();
        st.conversation_id = Some("conv-1".to_string());
        st.set_prompt_tokens(snap(Some(116_900), 116_900, Some(993_000)));
        assert_eq!(
            st.context_text(Some("1000000"), None),
            "116.9k / 1000k (11.7%)"
        );
        // 水位：下一轮实测回落时仍显示本会话峰值（`observe` 只取滚动最大）。
        st.set_prompt_tokens(snap(Some(100_000), 100_000, Some(993_000)));
        assert_eq!(
            st.context_text(Some("1000000"), None),
            "116.9k / 1000k (11.7%)"
        );
    }

    #[test]
    fn no_snapshot_renders_placeholder_not_nothing() {
        // 段消失会让用户以为功能没生效；没有实测时给占位（有已知上限就带上分母）。
        let st = UiState::new();
        assert_eq!(st.context_text(Some("1000000"), None), "— / 1000k");
        assert_eq!(st.context_text(None, None), "—");
    }

    /// 新会话（无 cid、无快照）用 `/status` 基线撑住，并带 `~` 标明是估算。
    #[test]
    fn new_session_shows_baseline_with_tilde() {
        let mut st = UiState::new();
        assert_eq!(
            st.context_text(Some("1000000"), Some(12_000)),
            "~12.0k / 1000k (1.2%)"
        );
        // 没有基线 → 仍是占位（不编数字）
        assert_eq!(st.context_text(Some("1000000"), None), "— / 1000k");
        // 有 cid（已有服务端会话）时基线不参与：等实测
        st.conversation_id = Some("c1".to_string());
        assert_eq!(st.context_text(Some("1000000"), Some(12_000)), "— / 1000k");
    }

    /// 实测优先于基线：有了快照就不该再显示估算值。
    #[test]
    fn measured_snapshot_wins_over_baseline() {
        let mut st = UiState::new();
        st.set_prompt_tokens(snap(Some(116_900), 116_900, Some(1_000_000)));
        assert_eq!(
            st.context_text(Some("1000000"), Some(12_000)),
            "116.9k / 1000k (11.7%)"
        );
    }
}
