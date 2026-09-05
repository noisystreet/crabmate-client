//! 全屏 TUI 的本地控制斜杠解析与 override 回显文本。
//!
//! 拆到独立模块以压住 `mod.rs` 行数/CCN 门禁；`apply_control` 仍在事件循环层。

/// 控制斜杠（本地处理，不发给模型）。`/` 开头的未知命令视为普通消息。
pub enum Control {
    Quit,
    Help,
    Model(Option<String>),
    Mode(Option<String>),
    Role(Option<String>),
    Status,
    Find(Option<String>),
    ConvNew,
    ConvRefresh,
    ConvUse(String),
    ConvUnknown(String),
}

pub fn clear_word(arg: &str) -> bool {
    matches!(arg, "off" | "none" | "clear")
}

/// 设置/查询一个字符串型 override 槽，返回给用户看的回显。
pub fn set_override_field(slot: &mut Option<String>, arg: Option<String>, name: &str) -> String {
    match arg {
        Some(v) if clear_word(&v) => {
            *slot = None;
            format!("{name} override cleared")
        }
        Some(v) => {
            *slot = Some(v.clone());
            format!("{name} override: {v}")
        }
        None => match slot.as_deref() {
            Some(v) if !v.trim().is_empty() => format!("{name} override: {v}"),
            _ => format!("{name} override: (none — 使用 serve 默认；/{name} <值> 设置)"),
        },
    }
}

/// 设置 `/mode` override（校验 ask/plan/act），返回给用户看的回显。
pub fn set_mode_field(slot: &mut Option<String>, arg: Option<String>) -> String {
    match arg {
        Some(v) if clear_word(&v) => {
            *slot = None;
            "mode override cleared".to_string()
        }
        Some(v) if matches!(v.as_str(), "ask" | "plan" | "act") => {
            *slot = Some(v.clone());
            format!("mode override: {v}")
        }
        Some(v) => format!("invalid session mode '{v}'; 可选 ask / plan / act（off 清除）"),
        None => match slot.as_deref() {
            Some(v) if !v.trim().is_empty() => format!("mode override: {v}"),
            _ => "mode override: (serve 默认；/mode ask|plan|act 设置)".to_string(),
        },
    }
}

/// 解析控制斜杠：`/model [x]` `/mode [ask|plan|act]` `/role [id]` `/status`
/// `/find [词]` `/help` `/conv`(刷新) `/conv new` `/quit`。返回 `None` 表示按普通消息处理。
pub fn parse_control(text: &str) -> Option<Control> {
    let t = text.trim();
    let rest = t.strip_prefix('/')?.trim();
    let (head, arg) = match rest.split_once(char::is_whitespace) {
        Some((h, a)) => (h.trim(), a.trim().to_string()),
        None => (rest, String::new()),
    };
    let head = head.to_ascii_lowercase();
    let arg_opt = (!arg.is_empty()).then_some(arg);
    match head.as_str() {
        "quit" | "exit" | "q" => Some(Control::Quit),
        "help" | "h" => Some(Control::Help),
        "model" => Some(Control::Model(arg_opt)),
        "mode" => Some(Control::Mode(arg_opt)),
        "role" => Some(Control::Role(arg_opt)),
        "status" => Some(Control::Status),
        "find" | "search" | "grep" => Some(Control::Find(arg_opt)),
        "conv" => Some(match arg_opt {
            None => Control::ConvRefresh,
            Some(a) => match a.split_whitespace().next().unwrap_or("") {
                "new" | "clear" => Control::ConvNew,
                "list" | "ls" | "show" => Control::ConvRefresh,
                "use" | "switch" => match a.split_whitespace().nth(1) {
                    Some(id) if !id.is_empty() => Control::ConvUse(id.to_string()),
                    _ => Control::ConvUnknown(a),
                },
                _ => Control::ConvUnknown(a),
            },
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_control_known_slashes() {
        assert!(matches!(parse_control("/quit"), Some(Control::Quit)));
        assert!(matches!(parse_control("/EXIT"), Some(Control::Quit)));
        assert!(matches!(parse_control("/status"), Some(Control::Status)));
        assert!(matches!(parse_control("/conv"), Some(Control::ConvRefresh)));
        assert!(matches!(parse_control("/conv new"), Some(Control::ConvNew)));
        assert!(matches!(
            parse_control("/model gpt-x"),
            Some(Control::Model(Some(v))) if v == "gpt-x"
        ));
        assert!(matches!(
            parse_control("/model"),
            Some(Control::Model(None))
        ));
        assert!(matches!(
            parse_control("/mode act"),
            Some(Control::Mode(Some(v))) if v == "act"
        ));
        assert!(matches!(
            parse_control("/role coder"),
            Some(Control::Role(Some(v))) if v == "coder"
        ));
    }

    #[test]
    fn parse_control_conv_use_and_unknown() {
        assert!(matches!(
            parse_control("/conv use c1"),
            Some(Control::ConvUse(v)) if v == "c1"
        ));
        assert!(matches!(
            parse_control("/conv bogus"),
            Some(Control::ConvUnknown(_))
        ));
        assert!(matches!(
            parse_control("/conv use"),
            Some(Control::ConvUnknown(_))
        ));
    }

    #[test]
    fn parse_control_find_and_help() {
        assert!(matches!(parse_control("/help"), Some(Control::Help)));
        assert!(matches!(parse_control("/h"), Some(Control::Help)));
        assert!(matches!(parse_control("/find"), Some(Control::Find(None))));
        assert!(matches!(
            parse_control("/search x y"),
            Some(Control::Find(Some(v))) if v == "x y"
        ));
        assert!(matches!(
            parse_control("/grep off"),
            Some(Control::Find(Some(v))) if v == "off"
        ));
    }

    #[test]
    fn parse_control_passes_plain_through() {
        assert!(parse_control("hello").is_none());
        assert!(parse_control("/my-skill").is_none());
        assert!(parse_control(" /model ").is_some());
    }

    #[test]
    fn override_field_set_clear_query() {
        let mut slot = None;
        let echo = set_override_field(&mut slot, Some("gpt-x".into()), "model");
        assert!(echo.contains("gpt-x"));
        assert_eq!(slot.as_deref(), Some("gpt-x"));
        let echo = set_override_field(&mut slot, Some("off".into()), "model");
        assert!(echo.contains("cleared"));
        assert!(slot.is_none());
        let echo = set_override_field(&mut slot, None, "model");
        assert!(echo.contains("(none"));
    }

    #[test]
    fn mode_field_validates() {
        let mut slot = None;
        let bad = set_mode_field(&mut slot, Some("bogus".into()));
        assert!(bad.contains("invalid"));
        assert!(slot.is_none());
        let ok = set_mode_field(&mut slot, Some("plan".into()));
        assert!(ok.contains("plan"));
        assert_eq!(slot.as_deref(), Some("plan"));
    }
}
