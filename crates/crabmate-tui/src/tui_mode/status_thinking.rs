//! 状态栏 `think` 段：取值归一 + 点击切换（对齐 Desktop 底栏的「思考」开关）。
//!
//! Desktop 那个 chip 是两态开关（`role="switch"`，见 `frontend/src/app/status_bar.rs`）：
//! 只有 `on` 算开，`server`（跟随）也读作关，点一下写回本机两态值。TUI 原来只有一个只读
//! 读数段，改档得绕到设置面板 —— 这里把同一段补成开关，判定口径仍是**同一条**（`on` 才算
//! 开），不另立一套，否则同一个设置会在两个界面里给出两个答案。
//!
//! 与本模块**无关**的另一件事：`Ctrl+E` 的 thinking 折叠（`state::toggle_thinking`）只改
//! 思考内容的展开/折叠，不动模型是否思考。两者名字相近，别混。

use super::LaneApp;
use super::settings::{FieldAction, LlmSave, PersistedSettings};
use super::state::LineKind;
use super::worker::WorkerJob;

/// 开 / 关的屏上文案。两处（段取值、切换提示）共用，避免一处写 `开` 另一处写 `on`。
const ON_LABEL: &str = "开";
const OFF_LABEL: &str = "关";

/// 线格式 → 屏上文案：**只有 `on` 算开**（`off` / `server` / 空都读作关，与 Desktop 一致）。
fn label_of(mode: &str) -> &'static str {
    if mode.trim() == "on" {
        ON_LABEL
    } else {
        OFF_LABEL
    }
}

/// `think` 段的取值：`开` / `关`；`None` = 还没拉到 user-data（→ 整段不画）。
///
/// 优先级与其余状态段一致：本端 override（刚点过开关）＞ user-data 已存值。
/// **user-data 到手后恒显**（没存 `llm_thinking_mode` ＝ 跟随 server ＝ 关）：段本身就是开关，
/// 时隐时现等于「想点的时候找不到」。还没拉到才返回 `None` —— 把「不知道」画成「关掉了」
/// 不行，与 `perm` 段的 `None` 同一套语义。
pub(super) fn switch_label(
    intent: Option<&str>,
    persisted: Option<&PersistedSettings>,
) -> Option<String> {
    if let Some(mode) = intent.map(str::trim).filter(|s| !s.is_empty()) {
        return Some(label_of(mode).to_string());
    }
    let persisted = persisted?;
    Some(label_of(persisted.thinking.as_deref().unwrap_or("")).to_string())
}

/// 点一下之后的目标值：两态、不落在 `server` 上（与 Desktop `next_thinking_mode` 同规则）。
///
/// `current` 传**屏上那个值**的线格式（intent 优先），否则连点两下会算出同一个目标值。
pub(super) fn next_mode(current: Option<&str>) -> &'static str {
    if current.map(str::trim) == Some("on") {
        "off"
    } else {
        "on"
    }
}

/// 落盘回包到了：只有「刚保存的值」仍是当前 override 时才清掉它。
///
/// 连点两下开关时，第一枪的回包会晚于第二枪的 override 落地；无脑清 override 会让状态行
/// 退回第一枪的旧值（看起来像「点两次只生效一次」）。值不相等 = 用户又改过了，留着。
pub(super) fn clear_intent_if_saved(intent: &mut Option<String>, save: &LlmSave) {
    let FieldAction::Write(saved) = &save.thinking else {
        return;
    };
    let Some(saved) = saved.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
        return;
    };
    if intent.as_deref() == Some(saved) {
        *intent = None;
    }
}

impl LaneApp<'_> {
    /// 状态栏 `think` 段被点击：切到另一端，**立刻**对本 lane 生效并落盘 user-data。
    ///
    /// 与 `/perm` 点击同一条思路（`permission_intent::apply_permission_level`）：先写本端
    /// override（状态行立刻翻、下一轮 `chat` 就带上），再 `SaveLlm` 落盘；回包成功才清
    /// override（那时 persisted 已是新值）。写失败时 override 留着 —— 本轮照新值发，
    /// 只是没落盘，下一条提示会把原因说清楚，比悄悄回退更容易理解。
    pub(super) fn toggle_thinking_mode(&mut self) {
        let current = self
            .overrides
            .thinking
            .as_deref()
            .or_else(|| self.persisted.as_ref().and_then(|p| p.thinking.as_deref()));
        let next = next_mode(current);
        self.overrides.thinking = Some(next.to_string());
        let label = label_of(next);
        let save = LlmSave {
            thinking: FieldAction::Write(Some(next.to_string())),
            ..Default::default()
        };
        if self.job_tx.send(WorkerJob::SaveLlm(save)).is_err() {
            self.st.push_line(
                LineKind::System,
                &format!("思考模式：{label}（未落盘：worker 已退出）"),
            );
            return;
        }
        self.st.push_line(
            LineKind::System,
            &format!("思考模式：{label}（正在保存到 serve user-data…）"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::super::settings::PersistedSettings;
    use super::{clear_intent_if_saved, next_mode, switch_label};

    fn persisted(thinking: Option<&str>) -> PersistedSettings {
        PersistedSettings {
            thinking: thinking.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn only_on_reads_as_on() {
        // `server` / `off` / 空都与 Desktop 的 `thinking_mode_is_on` 同口径：关。
        assert_eq!(switch_label(Some("on"), None).as_deref(), Some("开"));
        assert_eq!(switch_label(Some("off"), None).as_deref(), Some("关"));
        assert_eq!(switch_label(Some("server"), None).as_deref(), Some("关"));
    }

    /// user-data 到手后段恒显：没存值 = 跟随 server = 关（开关得先看得见才点得到）。
    #[test]
    fn known_persisted_value_always_yields_a_label() {
        assert_eq!(
            switch_label(None, Some(&persisted(Some("on")))).as_deref(),
            Some("开")
        );
        assert_eq!(
            switch_label(None, Some(&persisted(None))).as_deref(),
            Some("关")
        );
    }

    /// 没拉到 user-data 且没有本地 override：不画段（不知道 ≠ 关）。
    #[test]
    fn unknown_draws_nothing() {
        assert_eq!(switch_label(None, None), None);
        assert_eq!(switch_label(Some("  "), None), None);
    }

    /// override 优先于已存值：连点两下时第一枪的回包还没到，界面也得跟着第二下走。
    #[test]
    fn intent_wins_over_the_stored_value() {
        let p = persisted(Some("off"));
        assert_eq!(switch_label(Some("on"), Some(&p)).as_deref(), Some("开"));
    }

    #[test]
    fn toggle_flips_between_the_two_states() {
        assert_eq!(next_mode(Some("on")), "off");
        assert_eq!(next_mode(Some("off")), "on");
        // 跟随 server / 未知：屏上是「关」，点一下就是「开」。
        assert_eq!(next_mode(None), "on");
        assert_eq!(next_mode(Some("server")), "on");
    }

    /// 旧值的回包不能清掉新意图；同一值的回包才清。
    #[test]
    fn a_stale_save_keeps_the_newer_intent() {
        use super::super::settings::LlmSave;
        let save = |v: &str| LlmSave {
            thinking: super::super::settings::FieldAction::Write(Some(v.to_string())),
            ..Default::default()
        };
        let mut intent = Some("off".to_string());
        clear_intent_if_saved(&mut intent, &save("on"));
        assert_eq!(intent.as_deref(), Some("off"), "旧回包不许清掉新意图");

        let mut intent = Some("on".to_string());
        clear_intent_if_saved(&mut intent, &save("on"));
        assert_eq!(intent, None, "同值回包才清 override");
    }
}
