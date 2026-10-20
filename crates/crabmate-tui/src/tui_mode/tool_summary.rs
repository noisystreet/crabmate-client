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

/// 归一化：小写 + `_` / `-` 转空格，便于比较 `read_file` 与 `read file: …`。
///
/// 连字符必须一起转：serve 的 summary 会把工具名写成 `search-replace (preview): …`，
/// 而工具名本身是 `search_replace`。只处理下划线时两者永远对不上，行里就会印成
/// `search_replace — search-replace (preview): …`——名字看着写了两遍。
fn normalized(s: &str) -> String {
    s.trim().to_lowercase().replace(['_', '-'], " ")
}

/// 工具名之后允许紧跟的分隔符。
///
/// 语义是「紧跟 name 的**那一个字符**」，所以用字符集合而不是拼字符串比较。
/// 够宽才不会印成 `search_replace — search_replace: …`（名字出现两次）：
/// `search_replace: …`、`search_replace(a.rs)`、`search_replace（a.rs）`、
/// `read file. …` 都算同一回事。但也不能放宽成「以名字开头就算」——
/// `exec` 与 `execute` 不是一回事，必须由分隔符收尾。
const NAME_SEPARATORS: [char; 13] = [
    ' ', ':', '：', '(', '（', '[', '【', '-', '—', '/', ',', '，', '.',
];

/// note 是否以与工具名等价的短语开头（name + 任一分隔符，或就是 name 本身）。
///
/// 只比较前缀：工具行常连着出现，而 `note` 可能是很长的 summary——没必要为了
/// 看前几个字符就把整段小写复制一遍。
#[must_use]
pub(super) fn note_repeats_name(name: &str, note: &str) -> bool {
    let name_n = normalized(name);
    if name_n.is_empty() {
        return false;
    }
    // 多取一个字符就够：要么恰好等于名字，要么是名字 + 一个分隔符。
    let head: String = note
        .trim()
        .chars()
        .take(name_n.chars().count() + 1)
        .collect();
    match normalized(&head).strip_prefix(name_n.as_str()) {
        Some("") => true,
        Some(rest) => rest
            .chars()
            .next()
            .is_some_and(|c| NAME_SEPARATORS.contains(&c)),
        None => false,
    }
}

/// 把多行 note 压成一行：丢掉空行、连续重复行，以及**只写了工具名**的行。
///
/// 服务端的 summary 常见形态是「工具名 \n 工具名: 详情」——首行没有信息量，留着
/// 就会印成 `search_replace — search_replace: …`，看着像名字写了两遍；而按前缀判重
/// 也判不出来，因为名字后面跟的是换行符而不是分隔符。
#[must_use]
pub(super) fn flatten_note(name: &str, note: &str) -> Option<String> {
    let name_n = normalized(name);
    let mut kept: Vec<&str> = Vec::new();
    for line in note.lines() {
        let t = line.trim();
        if t.is_empty() || normalized(t) == name_n {
            continue;
        }
        // 连续重复行去掉（`search_replace: 2 处` 出现两次只留一次）。
        if kept.last() == Some(&t) {
            continue;
        }
        kept.push(t);
    }
    (!kept.is_empty()).then(|| kept.join(" · "))
}

/// 名字只是壳、note 本身已经说明「在做什么」的工具。
///
/// `run_command` 的 note 就是那条命令，再前缀一遍等于说「跑命令：git add -A」——
/// 工具名不带来任何额外信息。`read_file` / `write_file` 则不同：note 里只有一个
/// 路径，读还是写得靠名字区分，省掉就看不出来了。
const NAME_IMPLIED_BY_NOTE: [&str; 1] = ["run_command"];

/// 该工具名对读者是否多余（note 已完整表达语义）。
fn name_implied_by_note(name: &str) -> bool {
    let name_n = normalized(name);
    NAME_IMPLIED_BY_NOTE.iter().any(|t| normalized(t) == name_n)
}

/// 去掉控制字符（含 `\x1b`）。
///
/// note 来自命令文本 / 路径，可能夹带 ANSI 转义；原样进缓冲区会被终端当成控制序列
/// 执行（清屏、移动光标、伪造输出）。`ratatui` 不会替调用方拦这一层，只能在这里滤。
fn strip_control_chars(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect()
}

/// 工具行正文（不含标记）：`name — note`；note 与 name 重复时只留 note。
///
/// **开始态与结束态共用**：工具一开始跑就该看到「要执行什么」
/// （`run_command — git status`），不必等结果回来再补。
#[must_use]
pub(super) fn tool_row_text(name: &str, note: Option<&str>) -> String {
    let body = match note.and_then(|n| flatten_note(name, n)) {
        Some(note) if note_repeats_name(name, &note) || name_implied_by_note(name) => note,
        Some(note) => format!("{name} — {note}"),
        None => name.to_string(),
    };
    strip_control_chars(&body)
}

/// 工具结束行文案：`mark name — note`。
///
/// 标记一律**前置**：工具行常常连着出现，成败列对齐在同一列上才好扫——一眼看出
/// 哪几次失败。曾经「有 note」时标记落在工具名右边、「note 重复名字」时又跑到左边，
/// 同一屏里两种形态并存，反而看不出规律。
/// note 与 name 重复时省略 name（`✓ read file: hello.cpp`）。
#[must_use]
pub(super) fn tool_end_text(name: &str, ok: Option<bool>, note: Option<&str>) -> String {
    format!("{} {}", mark(ok), tool_row_text(name, note))
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

    /// `run_command` 的 note 就是命令本身，工具名不带来信息：只留命令。
    /// 其它工具（note 只是一条路径）仍保留名字——读还是写得靠它区分。
    ///
    /// 注：serve 的 `summary_run_command` 返回的就是裸命令（`git add -A`），
    /// 不带 `命令：` 前缀；`命令：…` 只出现在工具**输出正文**的多行格式里。
    #[test]
    fn command_tool_drops_its_own_name() {
        assert_eq!(
            tool_row_text("run_command", Some("git add -A")),
            "git add -A"
        );
        assert_eq!(
            tool_end_text("run_command", Some(true), Some("git add -A")),
            "✓ git add -A"
        );
        assert_eq!(tool_row_text("read_file", Some("a.rs")), "read_file — a.rs");
    }

    /// note 可能夹带 ANSI 转义（命令文本原样透传）：必须滤掉，否则终端会当控制序列执行。
    #[test]
    fn note_control_chars_are_stripped() {
        assert_eq!(
            tool_row_text("run_command", Some("git status\u{1b}[2J\u{7}")),
            "git status[2J"
        );
        assert_eq!(tool_row_text("run_command", Some("a\tb")), "ab");
    }

    /// serve 的 summary 可能用连字符写工具名（`search-replace (preview): …`），
    /// 而工具名本身是下划线（`search_replace`）：两者要视为同一回事。
    #[test]
    fn hyphen_and_underscore_are_equivalent() {
        assert_eq!(
            tool_end_text(
                "search_replace",
                Some(true),
                Some("search-replace (preview): 找到 1 处匹配")
            ),
            "✓ search-replace (preview): 找到 1 处匹配"
        );
        assert!(note_repeats_name("read_file", "read-file: a.rs"));
        assert!(note_repeats_name("read-file", "read_file: a.rs"));
    }

    /// 多行 summary（首行只有工具名）压平后再判重：否则名字会印两遍。
    #[test]
    fn multi_line_note_drops_name_only_line() {
        assert_eq!(
            tool_end_text(
                "search_replace",
                Some(true),
                Some("search_replace\nsearch_replace: 2 处替换")
            ),
            "✓ search_replace: 2 处替换"
        );
        // 首行是名字、次行是别的说明：只留说明。
        assert_eq!(
            tool_end_text("exec", Some(true), Some("exec\n2 files changed")),
            "✓ exec — 2 files changed"
        );
        // 连续重复行只留一份，多段用 ` · ` 连接。
        assert_eq!(
            tool_end_text("exec", Some(true), Some("exec\nexit 0\nexit 0\n耗时 3ms")),
            "✓ exec — exit 0 · 耗时 3ms"
        );
    }

    /// note 恰好就是工具名（没有分隔符、没有说明）时同样省掉重复的名字。
    #[test]
    fn note_equal_to_name_drops_name_too() {
        assert_eq!(
            tool_end_text("read_file", Some(true), Some("read_file")),
            "✓ read_file"
        );
    }

    /// 只比较前缀：很长的 summary 也要判对（且不该为看前几个字符复制整段）。
    #[test]
    fn long_note_is_compared_by_prefix_only() {
        let redundant = format!("search_replace: {}", "x".repeat(4096));
        assert!(note_repeats_name("search_replace", &redundant));
        let distinct = format!("{}: …", "y".repeat(4096));
        assert!(!note_repeats_name("search_replace", &distinct));
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
