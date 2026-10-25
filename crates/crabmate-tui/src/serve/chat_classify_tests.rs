//! `timeline_log` 的分类用例（`context_trim` → 水位回落）。
//!
//! 独立成文件的原因很具体：这两个用例的 JSON 字面量里带花括号，而 `lizard` 不识别 Rust
//! raw string 内部的括号 —— 把它放回 `chat_classify.rs` 的 `mod tests` 里，前一个测试函数
//! 的区间会被一路吞到文件末尾，CCN 报成 11（假阳性）。与 `status_tests.rs` 同一手法。

use crate::serve::chat_classify::{LineAction, classify_line};

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
