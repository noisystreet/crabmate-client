//! 工具结果写入 UI 存储的 `(compact, detail)` 单入口。

use crate::ToolCardInput;
use crate::ToolCardLocale;
use crate::card::{tool_card_compact_text, tool_card_text};
use crate::parse::parse_tool_envelope;

/// 单行摘要的字符上限（`COMPACT_MAX_CHARS` 的唯一来源）。
///
/// 执行中（SSE `tool_call`）与完成后（`tool_result` / 水合）**共用**这一个值：两态若各用
/// 一份常量，改动一处就会出现「执行中截 180、完成后截别的」—— 同一行在两态被截在两个
/// 不同位置，读起来像换了张卡（issue #616 的 D）。
pub const COMPACT_MAX_CHARS: usize = 180;

/// SSE `on_tool_result` 与水合 `role=tool` 落盘时的 `(text, reasoning_text)` 形态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolStoredText {
    pub compact: String,
    pub detail: String,
}

/// 压成单行并截到 [`COMPACT_MAX_CHARS`]（空白归一 + 溢出加 `…`）。
///
/// 执行中与完成后的单行**必须走这一个函数**：frontend 曾各自实现一份同样的逻辑，改一处
/// 就会让同一行在两态截在不同位置（issue #616 的 D）。
pub fn compact_one_line(s: &str) -> String {
    let compact = s
        .split_whitespace()
        .filter(|seg| !seg.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if compact.chars().count() <= COMPACT_MAX_CHARS {
        return compact;
    }
    let mut out = String::new();
    for ch in compact.chars().take(COMPACT_MAX_CHARS.saturating_sub(1)) {
        out.push(ch);
    }
    out.push('…');
    out
}

/// 由 [`ToolCardInput`] 生成 compact + detail。
#[must_use]
pub fn tool_stored_text(input: &ToolCardInput, loc: ToolCardLocale) -> ToolStoredText {
    let detail = tool_card_text(input, loc);
    let compact = compact_one_line(&tool_card_compact_text(input, loc));
    ToolStoredText { compact, detail }
}

/// 由 API `role=tool` 信封 JSON 生成展示字段。
pub fn tool_stored_text_from_envelope(
    raw: &str,
    fallback_name: Option<&str>,
    loc: ToolCardLocale,
) -> Option<ToolStoredText> {
    let input = parse_tool_envelope(raw, fallback_name)?;
    Some(tool_stored_text(&input, loc))
}

#[cfg(test)]
mod hydrate_tool_card_golden {
    use std::fs;
    use std::path::PathBuf;

    use super::*;
    use crate::ToolCardLocale;

    fn assert_golden_needles(
        line_no: usize,
        label: &str,
        text: &str,
        needles_csv: &str,
        field: &str,
    ) {
        for needle in needles_csv
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            assert!(
                text.contains(needle),
                "line {} ({}): {} {:?} missing {:?}",
                line_no + 1,
                label,
                field,
                text,
                needle
            );
        }
    }

    fn run_one_hydrate_golden_line(line_no: usize, line: &str) {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            return;
        }
        let mut parts = t.splitn(4, '\t');
        let label = parts.next().unwrap_or("?");
        let envelope = parts
            .next()
            .unwrap_or_else(|| panic!("line {}: missing envelope", line_no + 1));
        let compact_needles = parts
            .next()
            .unwrap_or_else(|| panic!("line {}: missing compact needles", line_no + 1));
        let detail_needles = parts
            .next()
            .unwrap_or_else(|| panic!("line {}: missing detail needles", line_no + 1));
        let loc = ToolCardLocale::ZhHans;
        let sse = tool_stored_text_from_envelope(envelope, None, loc)
            .unwrap_or_else(|| panic!("line {} ({}): parse", line_no + 1, label));
        let input = crate::parse_tool_envelope(envelope, None)
            .unwrap_or_else(|| panic!("line {} ({}): input", line_no + 1, label));
        let direct = tool_stored_text(&input, loc);
        assert_eq!(
            sse,
            direct,
            "line {} ({}): envelope vs input",
            line_no + 1,
            label
        );
        assert_golden_needles(line_no, label, &sse.compact, compact_needles, "compact");
        assert_golden_needles(line_no, label, &sse.detail, detail_needles, "detail");
        assert!(!sse.compact.contains("crabmate_tool"));
        assert!(!sse.detail.contains("crabmate_tool"));
    }

    #[test]
    fn hydrate_tool_card_golden() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/hydrate_tool_card_golden.jsonl");
        let raw =
            fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        for (line_no, line) in raw.lines().enumerate() {
            run_one_hydrate_golden_line(line_no, line);
        }
    }
}
