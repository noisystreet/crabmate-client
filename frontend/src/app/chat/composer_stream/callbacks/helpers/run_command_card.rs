//! `run_command` 工具卡：从 SSE `arguments` / `arguments_preview` 拼出可展示的调用行。

use serde_json::Value;

fn shell_quote_token(arg: &str) -> String {
    let arg = arg.trim();
    if arg.is_empty() {
        return "''".to_string();
    }
    let safe_unquoted = arg.chars().all(|c| {
        c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':' | '=' | ',' | '@')
    }) && !arg.contains('\'');
    if safe_unquoted {
        return arg.to_string();
    }
    let mut s = String::with_capacity(arg.len().saturating_add(2));
    s.push('"');
    for ch in arg.chars() {
        if ch == '"' || ch == '\\' {
            s.push('\\');
        }
        s.push(ch);
    }
    s.push('"');
    s
}

fn json_arg_token(v: &Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::String(s) => Some(shell_quote_token(s)),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        other => Some(shell_quote_token(&other.to_string())),
    }
}

fn args_tokens(args: Option<&Value>) -> Vec<String> {
    match args {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::String(s)) => {
            let t = s.trim();
            if t.is_empty() {
                Vec::new()
            } else {
                vec![shell_quote_token(t)]
            }
        }
        Some(Value::Array(items)) => items.iter().filter_map(json_arg_token).collect(),
        Some(other) => vec![shell_quote_token(&other.to_string())],
    }
}

fn invocation_from_json_object(v: &Value) -> Option<String> {
    let cmd = v.get("command")?.as_str()?.trim();
    if cmd.is_empty() {
        return None;
    }
    let tokens = args_tokens(v.get("args"));
    if tokens.is_empty() {
        Some(cmd.to_string())
    } else {
        Some(format!("{} {}", cmd, tokens.join(" ")))
    }
}

fn invocation_from_args_json(raw: Option<&str>) -> Option<String> {
    let raw = raw.map(str::trim).filter(|s| !s.is_empty())?;
    let v: Value = serde_json::from_str(raw).ok()?;
    invocation_from_json_object(&v)
}

fn summary_is_placeholder(summary: &str) -> bool {
    let s = summary.trim();
    s.is_empty()
        || s.eq_ignore_ascii_case("run_command")
        || s.eq_ignore_ascii_case("tool: run_command")
}

/// 优先完整/预览 JSON 里的 `command`+`args`，否则用已有 summary（占位串除外）。
#[must_use]
pub(crate) fn run_command_card_invocation_line(
    summary: &str,
    preview: Option<&str>,
    full: Option<&str>,
) -> String {
    if let Some(s) = invocation_from_args_json(full) {
        return s;
    }
    if let Some(s) = invocation_from_args_json(preview) {
        return s;
    }
    if !summary_is_placeholder(summary) {
        return summary.trim().to_string();
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_full_json_command_and_args() {
        let full = r#"{"command":"cargo","args":["test","--all"]}"#;
        let line = run_command_card_invocation_line("tool: run_command", None, Some(full));
        assert_eq!(line, "cargo test --all");
    }

    #[test]
    fn quotes_bash_c_script() {
        let full = r#"{"command":"bash","args":["-c","echo hello world"]}"#;
        let line = run_command_card_invocation_line("", None, Some(full));
        assert_eq!(line, r#"bash -c "echo hello world""#);
    }

    #[test]
    fn accepts_args_string_and_falls_back_to_summary() {
        let preview = r#"{"command":"git","args":"status -sb"}"#;
        let line = run_command_card_invocation_line("ignored", Some(preview), None);
        assert!(line.starts_with("git "), "{line}");
        assert!(line.contains("status -sb"), "{line}");
        let line = run_command_card_invocation_line("make -C build", None, None);
        assert_eq!(line, "make -C build");
    }
}
