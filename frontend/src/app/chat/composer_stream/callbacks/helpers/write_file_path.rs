//! 写盘工具卡「打开此文件」：从 SSE `arguments` / `arguments_preview` 提取目标文件路径。
//!
//! 仅在 `on_tool_call`（SSE 期）捕获，写入运行时 overlay（tool_call_id → 路径）；
//! 持久化信封不含 arguments，水合后无路径即无按钮（不做假按钮）。
//!
//! 路径规则（与 Server 契约对齐）：
//! - 单文件工具用 `path` 参数（`delete_files` 除外——文件已删除，无「此文件」可开）；
//! - `apply_patch` 解析 unified diff 头（`--- ` 优先，全新文件回退 `+++ `）；
//! - `copy_file` / `move_file` 打开落点 `to`。

use serde_json::Value;

/// 单文件 `path` 参数工具（写盘/文件元数据类；不含 `delete_files`）。
const SINGLE_PATH_TOOLS: [&str; 10] = [
    "create_file",
    "modify_file",
    "append_file",
    "search_replace",
    "chmod_file",
    "format_file",
    "format_check_file",
    "extract_in_file",
    "read_binary_meta",
    "hash_file",
];

/// 从 unified diff 头行取路径：剥 `a/`、`b/` 前缀与制表符时间戳，跳过 `/dev/null`。
fn diff_header_path(line: &str) -> Option<String> {
    let rest = line
        .strip_prefix("--- ")
        .or_else(|| line.strip_prefix("+++ "))?;
    let rest = rest.trim();
    let rest = rest.split('\t').next().unwrap_or("").trim();
    let rest = rest
        .strip_prefix("a/")
        .or_else(|| rest.strip_prefix("b/"))
        .unwrap_or(rest);
    let path = rest.trim();
    if path.is_empty() || path == "/dev/null" {
        return None;
    }
    Some(path.to_string())
}

/// 取 diff 中第一个可打开路径：`--- ` 行优先；全新文件（`--- /dev/null`）回退 `+++ ` 行。
/// rename diff（`--- a/old` + `+++ b/new`）打开旧路径（`--- ` 优先为契约；与
/// `move_file` 打开落点 `to` 的取舍不同，两行均为有效文件）。
fn first_patch_path(diff: &str) -> Option<String> {
    let mut fallback = None;
    for line in diff.lines() {
        let minus = line.starts_with("--- ");
        let plus = fallback.is_none() && line.starts_with("+++ ");
        if !minus && !plus {
            continue;
        }
        match diff_header_path(line) {
            Some(p) if minus => return Some(p),
            Some(p) => fallback = Some(p),
            None => {}
        }
    }
    fallback
}

/// `apply_patch` 参数：JSON `{"patch": "..."}` / `{"diff": "..."}`、裸 diff 字符串或纯 diff 文本。
fn path_from_patch_args(raw: &str) -> Option<String> {
    if let Ok(v) = serde_json::from_str::<Value>(raw) {
        let diff = match &v {
            Value::String(s) => Some(s.as_str()),
            Value::Object(o) => o
                .get("patch")
                .or_else(|| o.get("diff"))
                .and_then(Value::as_str),
            _ => None,
        };
        return diff.and_then(first_patch_path);
    }
    first_patch_path(raw)
}

/// 从 JSON 对象取非空字符串字段（`path` / `to`）；严格解析失败时对截断 JSON 容错
/// （`arguments_preview` 可能被截断，手扫 `"field":"value"`；路径值不含转义引号）。
fn path_from_json_field(raw: &str, field: &str) -> Option<String> {
    if let Ok(v) = serde_json::from_str::<Value>(raw) {
        let p = v.get(field)?.as_str()?.trim();
        if !p.is_empty() {
            return Some(p.to_string());
        }
        return None;
    }
    let needle = format!("\"{field}\"");
    let idx = raw.find(&needle)?;
    let rest = raw[idx + needle.len()..].trim_start().strip_prefix(':')?;
    let value = rest.trim_start().strip_prefix('"')?;
    let end = value.find('"')?;
    let p = value[..end].trim();
    if p.is_empty() {
        return None;
    }
    Some(p.to_string())
}

fn path_from_args(name: &str, raw: Option<&str>) -> Option<String> {
    let raw = raw.map(str::trim).filter(|s| !s.is_empty())?;
    if name == "apply_patch" {
        return path_from_patch_args(raw);
    }
    if name == "copy_file" || name == "move_file" {
        return path_from_json_field(raw, "to");
    }
    if SINGLE_PATH_TOOLS.contains(&name) {
        return path_from_json_field(raw, "path");
    }
    None
}

/// 优先完整 arguments，其次预览截断版；未识别工具返回 `None`（不猜路径）。
#[must_use]
pub(crate) fn write_tool_file_path(
    name: &str,
    preview: Option<&str>,
    full: Option<&str>,
) -> Option<String> {
    let name = name.trim();
    path_from_args(name, full).or_else(|| path_from_args(name, preview))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_path_tool_reads_path_field() {
        let full = r#"{"path":"src/lib.rs","content":"fn main() {}"}"#;
        assert_eq!(
            write_tool_file_path("create_file", None, Some(full)).as_deref(),
            Some("src/lib.rs")
        );
    }

    #[test]
    fn falls_back_to_preview_and_ignores_unknown_tools() {
        let preview = r#"{"path":"a/b.txt""#;
        assert_eq!(
            write_tool_file_path("modify_file", Some(preview), None).as_deref(),
            Some("a/b.txt")
        );
        assert!(write_tool_file_path("run_command", None, Some(preview)).is_none());
        assert!(write_tool_file_path("delete_files", None, Some(preview)).is_none());
    }

    #[test]
    fn apply_patch_parses_minus_header_and_strips_a_prefix() {
        let full = r#"{"patch":"*** Begin Patch\n--- a/src/main.rs\n+++ b/src/main.rs\n@@\n"}"#;
        assert_eq!(
            write_tool_file_path("apply_patch", None, Some(full)).as_deref(),
            Some("src/main.rs")
        );
    }

    #[test]
    fn apply_patch_new_file_falls_back_to_plus_header() {
        let diff = "--- /dev/null\n+++ b/docs/new.md\n@@\n";
        assert_eq!(
            write_tool_file_path("apply_patch", None, Some(diff)).as_deref(),
            Some("docs/new.md")
        );
    }

    #[test]
    fn copy_move_opens_destination_to() {
        let full = r#"{"from":"a.txt","to":"b/a.txt"}"#;
        assert_eq!(
            write_tool_file_path("move_file", None, Some(full)).as_deref(),
            Some("b/a.txt")
        );
        assert_eq!(
            write_tool_file_path("copy_file", None, Some(full)).as_deref(),
            Some("b/a.txt")
        );
    }

    #[test]
    fn diff_header_strips_timestamp_and_dev_null() {
        assert_eq!(
            diff_header_path("--- a/x/y.rs\t2026-01-01 00:00:00").as_deref(),
            Some("x/y.rs")
        );
        assert!(diff_header_path("--- /dev/null").is_none());
        assert!(diff_header_path("@@ -1 +1 @@").is_none());
    }
}
