//! 工具调用 → 一行展示文本（TUI / Web / Android 共用纯逻辑）。
//!
//! 背景（详见 `agent_space/tool-card-display-plan.md`）：Server 在摘要推导失败时会下发
//! `tool: <name>` 这种**占位串**当 `summary`，各端此前各自打补丁——Web 认出占位后改从
//! args JSON 解析，TUI 只是把「像 JSON 的串」丢掉、于是照单显示占位串。两份实现必然漂移，
//! 这里收敛成一份推导规则：
//!
//! 1. `summary` 非空且非占位 → 用它；
//! 2. `args` 是 JSON 对象 → 按语义槽（shell / path / query / url / ident）取标量渲染；
//! 3. `args` 是 JSON 残片（被截断）→ 尽力取出 `command`，不让整段 JSON 进正文；
//! 4. 其余非 JSON 文本 → 单行原样（旧式预览，如 `path=/etc/hosts`）；
//! 5. 都不行 → `None`，由调用方用自己的 i18n 兜底。
//!
//! 本模块不触碰 IO、Tauri、`web-sys`、`reqwest`、`tokio`（`crabmate-client-api` 边界）。

use serde_json::Value;

/// 一行展示文本的字符上限（超出截断并补 `…`）。
const LINE_MAX_CHARS: usize = 160;
/// 单个参数值的字符上限。
const VALUE_MAX_CHARS: usize = 120;
/// 多值槽最多展示几个值，其余折成 `+k`。
const MULTI_VALUE_MAX: usize = 3;
/// 未知参数（如 MCP 工具）最多展示几组 `key=value`。
const GENERIC_MAX_PAIRS: usize = 2;

/// 展示推导的输入。
#[derive(Debug, Clone, Copy)]
pub struct ToolDisplayInput<'a> {
    /// 工具名（`run_command` 等），用于识别占位串。
    pub name: &'a str,
    /// `TOOL_CALL_START.summary`；可能是 Server 的占位兜底文案。
    pub summary: Option<&'a str>,
    /// `TOOL_CALL_START.arguments` / `argsPreview`；可能是被截断的非法 JSON。
    pub args: Option<&'a str>,
}

/// 参数语义槽：决定对象参数如何渲染成一行。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Shell,
    Path,
    Query,
    Url,
    Ident,
}

/// 按优先级排列的非 shell 槽（shell 单独处理，因为要拼 `args`）。
///
/// `Query` 排在 `Path` 之前：`search_in_files` 同时有 `pattern` 与 `root`，
/// 前者才是这次调用「在找什么」，`root=.` 几乎没有信息量。
const VALUE_SLOTS: [Slot; 4] = [Slot::Query, Slot::Path, Slot::Url, Slot::Ident];

impl Slot {
    fn keys(self) -> &'static [&'static str] {
        match self {
            Slot::Shell => &["command", "cmd", "script"],
            Slot::Path => &["path", "paths", "file", "target", "root"],
            Slot::Query => &["pattern", "query", "regex", "symbol"],
            Slot::Url => &["url", "endpoint"],
            Slot::Ident => &["id", "name", "session_id", "action"],
        }
    }
}

/// 是否是「没有信息量」的占位摘要：空串、裸工具名、`tool: <name>`、`工具: <name>`。
///
/// Server 的 `tool_call_summary_from_args` 在摘要推导失败时正是下发 `tool: <name>`；
/// 各端都必须在拿它当最终文案前先问一次本函数。
pub fn is_placeholder_summary(name: &str, summary: &str) -> bool {
    let s = summary.trim();
    if s.is_empty() {
        return true;
    }
    let n = name.trim();
    if n.is_empty() {
        return false;
    }
    s.eq_ignore_ascii_case(n)
        || s.eq_ignore_ascii_case(&format!("tool: {n}"))
        || s == format!("工具: {n}")
        || s == format!("工具：{n}")
}

/// 推导工具调用的单行展示文本；无法推导时返回 `None`。
pub fn tool_display_line(input: &ToolDisplayInput<'_>) -> Option<String> {
    if let Some(summary) = input.summary {
        let s = summary.trim();
        if !s.is_empty() && !is_placeholder_summary(input.name, s) {
            let line = single_line(s);
            if !line.is_empty() {
                return Some(truncate_chars(&line, LINE_MAX_CHARS));
            }
        }
    }
    let raw = input.args.map(str::trim).filter(|s| !s.is_empty())?;
    if let Ok(v) = serde_json::from_str::<Value>(raw) {
        return from_json_value(&v);
    }
    if looks_like_json(raw) {
        // 被 240 字符截断/折叠过的 JSON 残片：只尽力捞 command，不把半截 JSON 打进正文。
        return command_from_broken_json(raw).map(|c| truncate_chars(&c, LINE_MAX_CHARS));
    }
    let line = single_line(raw);
    (!line.is_empty()).then(|| truncate_chars(&line, LINE_MAX_CHARS))
}

fn from_json_value(v: &Value) -> Option<String> {
    if v.is_object() {
        let line = object_line(v)?;
        return (!line.is_empty()).then(|| truncate_chars(&line, LINE_MAX_CHARS));
    }
    // 非对象的 JSON（数组/标量）：标量可直接显示，数组拼前几项。
    let tokens = value_tokens(v);
    (!tokens.is_empty()).then(|| truncate_chars(&join_limited(&tokens), LINE_MAX_CHARS))
}

fn object_line(v: &Value) -> Option<String> {
    if let Some(s) = shell_line(v) {
        return Some(s);
    }
    for slot in VALUE_SLOTS {
        if let Some(s) = slot_line(v, slot) {
            return Some(s);
        }
    }
    generic_line(v)
}

/// `command`/`cmd`/`script` + `args`：`git status -sb`、`bash -c "echo hi"`。
///
/// `command` 允许是字符串、数组（模型常写成 `["git","status"]`）或其它标量。
fn shell_line(v: &Value) -> Option<String> {
    let cmd = Slot::Shell.keys().iter().find_map(|k| v.get(*k))?;
    let mut tokens = shell_command_tokens(cmd);
    // `args` 无条件拼接：模型既可能写 `command: "git"` + `args: [...]`，
    // 也可能把程序名单独放进数组（`command: ["git"]` + `args: ["status"]`），
    // 后者若只在 `command` 为字符串时才拼参数，`status` 就丢了。
    for key in ["args", "arguments"] {
        let extra = args_tokens(v.get(key));
        if !extra.is_empty() {
            tokens.extend(extra);
            break;
        }
    }
    (!tokens.is_empty()).then(|| tokens.join(" "))
}

fn shell_command_tokens(cmd: &Value) -> Vec<String> {
    match cmd {
        Value::String(s) => keep_tokens(vec![shell_quote_token(s)]),
        Value::Array(items) => keep_tokens(items.iter().filter_map(json_arg_token).collect()),
        other => keep_tokens(
            value_tokens(other)
                .iter()
                .map(|t| shell_quote_token(t))
                .collect(),
        ),
    }
}

fn keep_tokens(tokens: Vec<String>) -> Vec<String> {
    tokens
        .into_iter()
        .filter(|t| !t.trim().is_empty() && t != "''")
        .collect()
}

fn slot_line(v: &Value, slot: Slot) -> Option<String> {
    for key in slot.keys() {
        let Some(val) = v.get(*key).filter(|x| !x.is_null()) else {
            continue;
        };
        let tokens = value_tokens(val);
        if tokens.is_empty() {
            continue;
        }
        return Some(format!("{key}={}", join_limited(&tokens)));
    }
    None
}

/// 未知参数兜底：取前两组 `key=value`，让 MCP 这类新工具也有可读的一行。
fn generic_line(v: &Value) -> Option<String> {
    let obj = v.as_object()?;
    let mut parts = Vec::new();
    for (k, val) in obj.iter().take(GENERIC_MAX_PAIRS) {
        let tokens = value_tokens(val);
        if tokens.is_empty() {
            continue;
        }
        parts.push(format!("{k}={}", join_limited(&tokens)));
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// 任意 JSON 值 → 可展示的标量片段（数组递归摊平，对象不展开）。
fn value_tokens(v: &Value) -> Vec<String> {
    match v {
        Value::Array(items) => items.iter().flat_map(value_tokens).collect(),
        other => scalar_text(other)
            .map(|s| {
                let line = single_line(&s);
                if line.is_empty() {
                    Vec::new()
                } else {
                    vec![line]
                }
            })
            .unwrap_or_default(),
    }
}

fn scalar_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => {
            let t = s.trim();
            (!t.is_empty()).then(|| t.to_string())
        }
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}

/// 多值槽用 `, ` 分隔：槽值常是路径，空格分隔在「路径含空格」时无法分辨边界。
fn join_limited(tokens: &[String]) -> String {
    let head: Vec<String> = tokens
        .iter()
        .take(MULTI_VALUE_MAX)
        .map(|t| truncate_chars(t, VALUE_MAX_CHARS))
        .collect();
    let rest = tokens.len().saturating_sub(MULTI_VALUE_MAX);
    if rest > 0 {
        format!("{} +{rest}", head.join(", "))
    } else {
        head.join(", ")
    }
}

/// 单个 shell 参数的显示形式：安全字符原样，其余加双引号并转义。
fn shell_quote_token(arg: &str) -> String {
    let arg = arg.trim();
    if arg.is_empty() {
        return "''".to_string();
    }
    let safe_unquoted = arg.chars().all(|c| {
        c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':' | '=' | ',' | '@')
    });
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
        Some(Value::String(s)) => keep_tokens(vec![shell_quote_token(s)]),
        Some(Value::Array(items)) => keep_tokens(items.iter().filter_map(json_arg_token).collect()),
        Some(other) => keep_tokens(vec![shell_quote_token(&other.to_string())]),
    }
}

fn looks_like_json(s: &str) -> bool {
    let t = s.trim_start();
    t.starts_with('{') || t.starts_with('[')
}

/// JSON 残片（截断）里的 `command`/`cmd`：不做完整解析，只按引号扫描取值。
///
/// 只在 `serde_json` 解析失败后调用；取不到就返回 `None`（宁缺毋滥，绝不吐半截 JSON）。
fn command_from_broken_json(raw: &str) -> Option<String> {
    for key in Slot::Shell.keys() {
        let Some(idx) = find_key_outside_string(raw, key) else {
            continue;
        };
        let after = &raw[idx + key.len() + 2..];
        let Some(s) = value_after_key(after) else {
            continue;
        };
        let quoted = shell_quote_token(&s);
        if !quoted.is_empty() && quoted != "''" {
            return Some(quoted);
        }
    }
    None
}

/// 找 `"key"` 第一次出现且**不在字符串值内部**的位置。
///
/// 截断的 args 里常有 `echo "command"` 这类字面量；用 `find` 会命中值里的同名文本，
/// 捞出一条错命令。这里按 JSON 的最小词法走一遍：`in_string` + `escaped` 两个状态，
/// 转义引号不会把状态翻回来。
fn find_key_outside_string(raw: &str, key: &str) -> Option<usize> {
    let pat = format!("\"{key}\"");
    let bytes = raw.as_bytes();
    let (mut i, mut in_string, mut escaped) = (0usize, false, false);
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if in_string => escaped = !escaped,
            b'"' if in_string && !escaped => {
                in_string = false;
                escaped = false;
            }
            b'"' if !in_string => {
                if raw[i..].starts_with(&pat) {
                    return Some(i);
                }
                in_string = true;
                escaped = false;
            }
            _ => escaped = false,
        }
        i += 1;
    }
    None
}

/// `: "…"` 里的字符串值（到闭合引号或残片末尾为止）。
fn value_after_key(rest: &str) -> Option<String> {
    let rest = rest.trim_start().strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => break,
            '\\' => {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            }
            _ => out.push(c),
        }
    }
    let t = out.trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// 折叠为单行：控制字符变空格，连续空格压一个，去首尾空白。
fn single_line(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for ch in s.chars() {
        let c = if ch.is_control() { ' ' } else { ch };
        if c == ' ' {
            if prev_space || out.is_empty() {
                continue;
            }
            prev_space = true;
        } else {
            prev_space = false;
        }
        out.push(c);
    }
    out.trim_end().to_string()
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(summary: &str, args: Option<&str>) -> Option<String> {
        tool_display_line(&ToolDisplayInput {
            name: "run_command",
            summary: Some(summary),
            args,
        })
    }

    #[test]
    fn placeholder_summaries_are_recognized() {
        for s in [
            "",
            "run_command",
            "tool: run_command",
            "Tool: run_command",
            // Web `i18n::tool_card_prefix(ZhHans)` 是 `工具：`（全角冒号），半角也一并认。
            "工具：run_command",
            "工具: run_command",
        ] {
            assert!(is_placeholder_summary("run_command", s), "{s:?}");
        }
        assert!(!is_placeholder_summary("run_command", "git status -sb"));
    }

    #[test]
    fn real_summary_wins_over_args() {
        let got = line("git status", Some(r#"{"command":"rm","args":["-rf","/"]}"#));
        assert_eq!(got.as_deref(), Some("git status"));
    }

    #[test]
    fn placeholder_summary_falls_back_to_args() {
        // 本 bug 的核心回归：占位串 + 合法 args 必须给出真实命令。
        let got = line(
            "tool: run_command",
            Some(r#"{"command":"git","args":["status","-sb"]}"#),
        );
        assert_eq!(got.as_deref(), Some("git status -sb"));
    }

    #[test]
    fn command_as_array_and_alias_key() {
        assert_eq!(
            line("", Some(r#"{"command":["git","status"]}"#)).as_deref(),
            Some("git status")
        );
        assert_eq!(line("", Some(r#"{"cmd":"ls"}"#)).as_deref(), Some("ls"));
    }

    /// `command` 只放了程序名、参数另起 `args`：两者都必须出现（否则丢 `status`）。
    #[test]
    fn array_command_with_separate_args_keeps_args() {
        assert_eq!(
            line("", Some(r#"{"command":["git"],"args":["status","-sb"]}"#)).as_deref(),
            Some("git status -sb")
        );
    }

    #[test]
    fn shell_args_are_quoted() {
        assert_eq!(
            line(
                "",
                Some(r#"{"command":"bash","args":["-c","echo hello world"]}"#)
            )
            .as_deref(),
            Some("bash -c \"echo hello world\"")
        );
    }

    #[test]
    fn non_shell_slots_render_key_value() {
        assert_eq!(
            line("", Some(r#"{"path":"src/main.rs"}"#)).as_deref(),
            Some("path=src/main.rs")
        );
        assert_eq!(
            line("", Some(r#"{"pattern":"TODO","root":"."}"#)).as_deref(),
            Some("pattern=TODO")
        );
        assert_eq!(
            line("", Some(r#"{"paths":["a","b","c","d"]}"#)).as_deref(),
            Some("paths=a, b, c +1")
        );
    }

    #[test]
    fn unknown_object_args_get_generic_pairs() {
        // 键序取决于 serde_json 的 map 实现（默认 BTreeMap 按字母序），只断言内容。
        let got = line("", Some(r#"{"foo":"bar","baz":42}"#)).expect("generic line");
        assert!(got.contains("foo=bar"), "{got}");
        assert!(got.contains("baz=42"), "{got}");
    }

    #[test]
    fn truncated_json_fragment_salvages_command() {
        let broken = r#"{"command":"bash","args":["-c","echo hel"#;
        assert_eq!(
            line("tool: run_command", Some(broken)).as_deref(),
            Some("bash")
        );
    }

    #[test]
    fn json_like_garbage_never_leaks_raw() {
        // 解析不出 command 的 JSON 残片：返回 None，绝不把半截 JSON 打进正文。
        let broken = r#"{"foo":"bar","args":["-c","echo hel"#;
        assert_eq!(line("tool: run_command", Some(broken)), None);
    }

    /// 唯一的 `"command"` 落在字符串值里（`echo \"command\"`）：不得当成真的键。
    #[test]
    fn broken_fragment_ignores_key_inside_string_value() {
        let broken = r#"{"args":["-c","echo \"command\" hi"#;
        assert_eq!(line("tool: run_command", Some(broken)), None);
    }

    /// 键名出现在某个值**之后**、但确实在对象层：状态机必须能从字符串里正常退出来。
    #[test]
    fn broken_fragment_finds_key_after_string_values() {
        let broken = r#"{"args":["-c","echo hi"],"command":"bash"#;
        assert_eq!(
            line("tool: run_command", Some(broken)).as_deref(),
            Some("bash")
        );
    }

    #[test]
    fn plain_text_args_pass_through() {
        assert_eq!(
            line("", Some("path=/etc/hosts")).as_deref(),
            Some("path=/etc/hosts")
        );
    }

    #[test]
    fn nothing_usable_returns_none() {
        assert_eq!(line("tool: run_command", None), None);
        assert_eq!(line("", None), None);
        assert_eq!(line("", Some("   ")), None);
    }

    #[test]
    fn long_line_is_truncated_with_ellipsis() {
        let long = "x".repeat(500);
        let got = line(&long, None).expect("line");
        assert!(got.chars().count() <= LINE_MAX_CHARS);
        assert!(got.ends_with('…'));
    }
}
