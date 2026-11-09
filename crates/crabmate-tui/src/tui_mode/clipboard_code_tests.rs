//! `/copy code [n]` 的参数解析与选块（issue #428 第 1 条）。
//!
//! 只断言「解析到什么 / 选中哪一块 / 为什么选不到」：真正的投递会写终端剪贴板，
//! 单测不碰（同 `clipboard` 内既有用例的取舍）。

use super::*;

fn line(kind: LineKind, text: &str) -> LogLine {
    LogLine::replayed(kind, text, false, None)
}

#[test]
fn no_argument_targets_the_last_assistant_message() {
    assert_eq!(parse_target(None), Ok(Target::Message(LineKind::Assistant)));
    assert_eq!(
        parse_target(Some("")),
        Ok(Target::Message(LineKind::Assistant))
    );
    assert_eq!(
        parse_target(Some("assistant")),
        Ok(Target::Message(LineKind::Assistant))
    );
    assert_eq!(
        parse_target(Some("user")),
        Ok(Target::Message(LineKind::User))
    );
    assert_eq!(parse_target(Some("path")), Ok(Target::Path));
}

/// `code` 不带序号 = 最近一块；带序号按词切分（`parse_control` 交下来的是整段剩余）。
#[test]
fn code_target_defaults_to_one_and_reads_the_index() {
    assert_eq!(parse_target(Some("code")), Ok(Target::Code(1)));
    assert_eq!(parse_target(Some("code 2")), Ok(Target::Code(2)));
    // 多余空白不该让序号解析失败。
    assert_eq!(parse_target(Some("  code   3  ")), Ok(Target::Code(3)));
}

/// 序号写错时报错，**不**静默回落到最近一块 —— 否则用户以为选的是第 3 块。
#[test]
fn bad_code_index_is_an_error_not_a_fallback() {
    for bad in ["code 0", "code x", "code -1", "code 1 2"] {
        let err = parse_target(Some(bad)).expect_err(bad);
        assert!(err.contains("code"), "{bad} → {err}");
        assert!(err.contains('1'), "{bad} → {err}");
    }
}

#[test]
fn unknown_target_names_every_supported_one() {
    let err = parse_target(Some("zzz")).expect_err("should reject");
    for word in ["assistant", "user", "path", "code"] {
        assert!(err.contains(word), "{err}");
    }
}

#[test]
fn pick_returns_the_block_counted_from_the_end() {
    let text = "前置说明\n\n```rust\nlet a = 1;\n```\n\n```sh\nls\n```";
    let (block, total) = pick_code_block(text, 1).expect("最后一块");
    assert_eq!(total, 2);
    assert_eq!(block.lang, "sh");
    assert_eq!(block.code, "ls");

    let (block, _) = pick_code_block(text, 2).expect("倒数第二块");
    assert_eq!(block.lang, "rust");
    assert_eq!(block.code, "let a = 1;");
}

/// 粘连围栏（`依赖：```rust`）在渲染侧被 normalize 拆开；复制侧同源，
/// 否则屏幕上明明是一块代码却报「没有代码块」。
#[test]
fn pick_finds_glued_and_unclosed_fences() {
    let (block, total) = pick_code_block("依赖：```rust\nlet x = 1;\n```", 1).expect("粘连围栏");
    assert_eq!((block.lang.as_str(), total), ("rust", 1));

    // 流式中途（围栏未闭合）：此刻看得见的就该能复制。
    let (block, _) = pick_code_block("```rust\nlet a = 1;", 1).expect("未闭合围栏");
    assert_eq!(block.code, "let a = 1;");
}

#[test]
fn pick_reports_missing_and_out_of_range_with_the_real_total() {
    let err = pick_code_block("没有围栏的正文", 1).expect_err("没有代码块");
    assert!(err.contains("没有代码块"), "{err}");
    // 报错里要带上真实块数与具体序号，否则用户不知道该改哪个数字。
    let err = pick_code_block("```\na\n```\n```\nb\n```", 3).expect_err("越界");
    assert!(err.contains('2'), "{err}");
    assert!(err.contains('3'), "{err}");
}

/// 空围栏不投递空字符串：那会在剪贴板里留下一片空白，看起来像复制失败。
#[test]
fn pick_rejects_a_blank_block() {
    let err = pick_code_block("```\n\n```", 1).expect_err("空块");
    assert!(err.contains("空代码块"), "{err}");
}

/// `run` 的选块路径在**选不到**时直接出文案（不投递）：整条流程接得上。
#[test]
fn run_reports_code_errors_without_touching_the_clipboard() {
    let lines = vec![line(LineKind::Assistant, "只有正文")];
    let notice = run(&lines, Some("code"), None, false);
    assert!(notice.contains("没有代码块"), "{notice}");

    let lines = vec![line(LineKind::Assistant, "```\nfn a() {}\n```")];
    let notice = run(&lines, Some("code 2"), None, false);
    assert!(notice.contains("只有 1 个"), "{notice}");
}

/// 代码块只从助手回复取；没有助手回复时说清原因，而不是「没有代码块」。
#[test]
fn run_without_any_assistant_reply_says_so() {
    let lines = vec![line(LineKind::User, "```\nx\n```")];
    let notice = run(&lines, Some("code"), None, false);
    assert!(notice.contains("没有可复制的助手回复"), "{notice}");
}
