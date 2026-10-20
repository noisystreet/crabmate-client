//! 工具行摘要文案（纯逻辑）：结束行补 `✓/✗/?`；当说明文案本身以工具名开头
//! （如 `read_file` + `read file: hello.cpp`）时省略工具名前缀，避免冗余重复。

/// `✓` / `✗` / `?` 标记。
///
/// `?` 表示「已结束，但服务端没给成败」——它与 `✓`/`✗` 同为 1 列，成败列因此
/// 始终对齐。曾经用 `(done)`（6 列）当这一档，既比另两个宽出一截，又容易被读成
/// 「完成」，而它实际说的是「结果不明」。
fn mark(ok: Option<bool>) -> &'static str {
    match ok {
        Some(true) => "✓",
        Some(false) => "✗",
        None => "?",
    }
}

/// 归一化：小写 + 下划线转空格，便于比较「read_file」与「read file: …」。
fn normalized(s: &str) -> String {
    s.trim().to_lowercase().replace('_', " ")
}

/// 工具名之后允许紧跟的分隔符。
///
/// 够宽才不会印成 `search_replace — search_replace: …`（名字出现两次）：
/// `search_replace: …`、`search_replace(a.rs)`、`search_replace（a.rs）`、
/// `read file. …` 都算同一回事。但也不能放宽成「以名字开头就算」——
/// `exec` 与 `execute` 不是一回事，必须由分隔符收尾。
const NAME_SEPARATORS: [&str; 12] = [
    " ", ":", "：", "(", "（", "[", "【", "-", "—", "/", ",", "，",
];

/// note 是否以与工具名等价的短语开头（name + 任一分隔符，或就是 name 本身）。
#[must_use]
pub(super) fn note_repeats_name(name: &str, note: &str) -> bool {
    let name_n = normalized(name);
    if name_n.is_empty() {
        return false;
    }
    let note_n = normalized(note);
    note_n == name_n
        || NAME_SEPARATORS
            .iter()
            .any(|sep| note_n.starts_with(&format!("{name_n}{sep}")))
}

/// 工具结束行文案：`mark name — note`。
///
/// 标记一律**前置**：工具行常常连着出现，成败列对齐在同一列上才好扫——一眼看出
/// 哪几次失败。曾经「有 note」时标记落在工具名右边、「note 重复名字」时又跑到左边，
/// 同一屏里两种形态并存，反而看不出规律。
/// note 与 name 重复时省略 name（`✓ read file: hello.cpp`）。
#[must_use]
pub(super) fn tool_end_text(name: &str, ok: Option<bool>, note: Option<&str>) -> String {
    let m = mark(ok);
    match note.map(str::trim).filter(|s| !s.is_empty()) {
        Some(note) if note_repeats_name(name, note) => format!("{m} {note}"),
        Some(note) => format!("{m} {name} — {note}"),
        None => format!("{m} {name}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redundant_note_drops_name() {
        assert_eq!(
            tool_end_text("read_file", Some(true), Some("read file: hello.cpp")),
            "✓ read file: hello.cpp"
        );
        assert_eq!(
            tool_end_text("read_file", Some(true), Some("read file：hello.cpp")),
            "✓ read file：hello.cpp"
        );
        assert_eq!(
            tool_end_text("read_file", Some(false), Some("read file: denied")),
            "✗ read file: denied"
        );
    }

    /// 名字后面跟括号、斜杠、逗号等各式分隔符时也要判为重复：否则印成
    /// `search_replace — search_replace: …`，工具名在一行里出现两次。
    #[test]
    fn name_prefix_recognises_common_separators() {
        for note in [
            "search_replace: 2 处替换",
            "search_replace（a.rs）",
            "search_replace(a.rs): 1",
            "search_replace, 2 处",
            "search_replace — 命中 3 行",
            "search_replace/说明",
        ] {
            assert!(
                note_repeats_name("search_replace", note),
                "应判为重复：{note:?}"
            );
        }
        // 不能只按前缀判断：`execute …` 不是 `exec` 的说明。
        assert!(!note_repeats_name("exec", "execute something"));

        // 端到端：工具名只出现一次。
        assert_eq!(
            tool_end_text(
                "search_replace",
                Some(true),
                Some("search_replace: 2 处替换")
            ),
            "✓ search_replace: 2 处替换"
        );
    }

    #[test]
    fn distinct_note_keeps_name() {
        assert_eq!(
            tool_end_text("exec", Some(true), Some("exit 0")),
            "✓ exec — exit 0"
        );
        assert_eq!(
            tool_end_text("exec", Some(false), Some("rejected")),
            "✗ exec — rejected"
        );
    }

    #[test]
    fn no_note_just_mark() {
        assert_eq!(tool_end_text("exec", Some(true), None), "✓ exec");
        assert_eq!(tool_end_text("patch", None, None), "? patch");
    }

    /// 三档标记都占 1 列，成败列才能对齐（`(done)` 是 6 列，会把工具名推右 5 列）。
    #[test]
    fn every_mark_is_one_column() {
        for ok in [Some(true), Some(false), None] {
            assert_eq!(mark(ok).chars().count(), 1, "标记应占 1 列：{:?}", mark(ok));
        }
    }

    /// 标记恒在行首：连续工具行的成败列必须对齐。曾经「有 note」时标记在工具名
    /// 右边、「note 重复名字」时又在左边，同一屏两种形态混排，看不出规律。
    #[test]
    fn mark_always_leads_the_line() {
        for (ok, note) in [
            (Some(true), None),
            (Some(false), Some("rejected")),
            (None, Some("read file: hello.cpp")),
            (Some(true), Some("exit 0")),
        ] {
            let text = tool_end_text("exec", ok, note);
            assert!(text.starts_with(mark(ok)), "标记应前置：{text:?}");
        }
    }
}
