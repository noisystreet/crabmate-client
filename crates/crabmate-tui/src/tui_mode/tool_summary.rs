//! 工具行摘要文案（纯逻辑）：结束行补 `✓/✗/(done)`；当说明文案本身以工具名开头
//! （如 `read_file` + `read file: hello.cpp`）时省略工具名前缀，避免冗余重复。

/// `✓` / `✗` / `(done)` 标记。
fn mark(ok: Option<bool>) -> &'static str {
    match ok {
        Some(true) => "✓",
        Some(false) => "✗",
        None => "(done)",
    }
}

/// 归一化：小写 + 下划线转空格，便于比较「read_file」与「read file: …」。
fn normalized(s: &str) -> String {
    s.trim().to_lowercase().replace('_', " ")
}

/// note 是否以与工具名等价的短语开头（name 前缀 + 空格 / 半角冒号 / 全角冒号）。
#[must_use]
pub(super) fn note_repeats_name(name: &str, note: &str) -> bool {
    let name_n = normalized(name);
    if name_n.is_empty() {
        return false;
    }
    let note_n = normalized(note);
    note_n == name_n
        || note_n.starts_with(&format!("{name_n} "))
        || note_n.starts_with(&format!("{name_n}:"))
        || note_n.starts_with(&format!("{name_n}："))
}

/// 工具结束行文案：`name ✓ — note`；note 与 name 重复时省略 name（`✓ read file: hello.cpp`）。
#[must_use]
pub(super) fn tool_end_text(name: &str, ok: Option<bool>, note: Option<&str>) -> String {
    let m = mark(ok);
    match note.map(str::trim).filter(|s| !s.is_empty()) {
        Some(note) if note_repeats_name(name, note) => format!("{m} {note}"),
        Some(note) => format!("{name} {m} — {note}"),
        None => format!("{name} {m}"),
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

    #[test]
    fn distinct_note_keeps_name() {
        assert_eq!(
            tool_end_text("exec", Some(true), Some("exit 0")),
            "exec ✓ — exit 0"
        );
        assert_eq!(
            tool_end_text("exec", Some(false), Some("rejected")),
            "exec ✗ — rejected"
        );
    }

    #[test]
    fn no_note_just_mark() {
        assert_eq!(tool_end_text("exec", Some(true), None), "exec ✓");
        assert_eq!(tool_end_text("patch", None, None), "patch (done)");
    }
}
