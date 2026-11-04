//! 技能行文案：`context_inject` 里的 skill 列表 → 一行标题（对齐 Desktop 的 skill 卡）。
//!
//! Desktop 把同样的 `SkillInjectInfo` 画成 `✦ 已调用 skill：code-review`，
//! 终端没有卡片，所以「卡片标题」直接就是这一行的正文。单独成文件是为了让
//! 「名称怎么排」只写一遍（前缀与配色在 `prefix.rs`，两者都不该长在这里）。

use crabmate_client_api::timeline_skill::SkillInjectInfo;

/// 技能行正文：`已调用 skill：code-review、rust-style`。
///
/// 用户显式 `/<skill-id>` 选用（Server 强制注入）时，尾部标「（显式）」——
/// 与 Desktop 卡上的 `显式` 标记同义，否则「为什么这轮会加载这个 skill」在终端里没有交代。
#[must_use]
pub fn skill_line_text(info: &SkillInjectInfo) -> String {
    let names = info.skills.join("、");
    if info.forced {
        format!("已调用 skill：{names}（显式）")
    } else {
        format!("已调用 skill：{names}")
    }
}

#[cfg(test)]
mod tests {
    use super::skill_line_text;
    use crabmate_client_api::timeline_skill::SkillInjectInfo;

    fn info(skills: &[&str], forced: bool) -> SkillInjectInfo {
        SkillInjectInfo {
            skills: skills.iter().map(|s| (*s).to_string()).collect(),
            forced,
        }
    }

    #[test]
    fn lists_names_in_injection_order() {
        assert_eq!(
            skill_line_text(&info(&["code-review", "rust-style"], false)),
            "已调用 skill：code-review、rust-style"
        );
    }

    #[test]
    fn marks_explicit_pick() {
        assert_eq!(
            skill_line_text(&info(&["code-review"], true)),
            "已调用 skill：code-review（显式）"
        );
    }
}
