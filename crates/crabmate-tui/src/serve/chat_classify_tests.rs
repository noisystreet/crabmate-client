//! `timeline_log` 的分类用例（`context_trim` → 水位回落）。
//!
//! 独立成文件的原因很具体：这两个用例的 JSON 字面量里带花括号，而 `lizard` 不识别 Rust
//! raw string 内部的括号 —— 把它放回 `chat_classify.rs` 的 `mod tests` 里，前一个测试函数
//! 的区间会被一路吞到文件末尾，CCN 报成 11（假阳性）。与 `status_tests.rs` 同一手法。

use crate::serve::chat_classify::{LineAction, classify_line};
use crabmate_client_api::sse_dispatch::ClarificationKind;

/// `CUSTOM timeline_log`（`kind=context_trim`）：`detail` 是**字符串化 JSON**（`{"after_tokens":N}`），
/// 不是嵌套对象 —— 当对象读会永远拿不到，现象就是「压缩后水位不回落」。
#[test]
fn context_trim_custom_event_yields_reset() {
    let line = concat!(
        r#"{"type":"CUSTOM","customType":"timeline_log","data":{"kind":"context_trim","#,
        r#""title":"已裁剪历史","detail":"{\"n_before\":3,\"after_tokens\":40000}"}}"#
    );
    match classify_line(line) {
        Ok(LineAction::ContextTrim { after }) => assert_eq!(after, 40_000),
        other => panic!("期望 ContextTrim，得到 {other:?}"),
    }
}

/// 其它 `timeline_log`（如 `context_inject`）不该动水位：它们不改上下文占用。
#[test]
fn other_timeline_log_kinds_do_not_reset_usage() {
    let line = r#"{"type":"CUSTOM","customType":"timeline_log","data":{"kind":"context_inject","title":"注入","detail":""}}"#;
    assert!(
        matches!(classify_line(line), Ok(LineAction::Skip)),
        "只有 context_trim 才能重置水位"
    );
}

/// `kind=context_inject` 且 `detail.skills` 非空：这一轮确实调用了 skill，要成一行技能行
/// （对齐 Desktop 的 skill 卡；路径 / `.md` 归一在共享层做）。
#[test]
fn context_inject_with_skills_yields_skill_line() {
    let line = concat!(
        r#"{"type":"CUSTOM","customType":"timeline_log","data":{"kind":"context_inject","#,
        r#""title":"本轮已注入上下文","#,
        r#""detail":"{\"skills\":[\"rust-style\",\".crabmate/skills/code-review.md\"],\"forced\":true}"}}"#
    );
    match classify_line(line) {
        Ok(LineAction::SkillInject(info)) => {
            assert_eq!(info.skills, vec!["rust-style", "code-review"]);
            assert!(info.forced);
        }
        other => panic!("期望 SkillInject，得到 {other:?}"),
    }
}

/// `CUSTOM clarification_questionnaire`：显式注册钩子后必须变成 `LineAction::Clarify`。
///
/// 这正是 issue #424 的根因回归点：以前 `SseClarifyTraceHooks::default()` 不注册钩子，
/// 事件一路落到 `Skip`，终端只看到工具行、题面整份凭据丢弃。
#[test]
fn clarification_questionnaire_yields_clarify_action() {
    let line = concat!(
        r#"{"type":"CUSTOM","customType":"clarification_questionnaire","#,
        r#""data":{"questionnaireId":"q-42","intro":"需要确认两件事","#,
        r#""questions":[{"id":"name","label":"项目名","required":true,"kind":"text"},"#,
        r#"{"id":"style","label":"风格","hint":"可留空","kind":"choice"}]}}"#
    );
    match classify_line(line) {
        Ok(LineAction::Clarify(info)) => {
            assert_eq!(info.questionnaire_id, "q-42");
            assert_eq!(info.fields.len(), 2);
            assert_eq!(info.fields[0].id, "name");
            assert!(info.fields[0].required);
            // 题型只解析、不在这里拼文案：标记由渲染侧（`tui_mode::clarify`）自己映射，
            // 共享层不替任何一端决定界面语言。
            assert_eq!(info.fields[0].kind, ClarificationKind::Text);
            assert_eq!(info.fields[1].kind, ClarificationKind::Choice);
            assert_eq!(info.fields[1].hint.as_deref(), Some("可留空"));
        }
        other => panic!("期望 Clarify，得到 {other:?}"),
    }
}
