//! `timeline_log` 的 `context_inject` 旁注：提取本轮被调用的 skill。
//!
//! Desktop 主列（`frontend/src/timeline_scan.rs` 的 skill 卡）与 TUI 的技能行共用这一份口径。
//! 原先只有 WASM 侧有实现，TUI 只能「不支持」——两端各写一份迟早会按各自理解漂移。
//!
//! 三种落盘形态都要认（见 [`skill_inject_from_timeline_text`]）：水合 `{kind,title,detail}`
//! （`detail` 是 JSON **字符串**，不是嵌套对象）、流式 `title\n<detail>`、纯 detail JSON。

use serde_json::Value;

/// 本轮 skill 调用信息（Server `context_inject` 的 `detail.skills` / `detail.forced`）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SkillInjectInfo {
    /// 归一化后的展示名（取路径 basename、去 `.md` 后缀，保序去重）。
    pub skills: Vec<String>,
    /// 用户显式 `/<skill-id>` 选用（Server 强制注入）。
    pub forced: bool,
}

/// 去 `.md` 后缀（ASCII 大小写不敏感）。
fn strip_md_suffix(s: &str) -> &str {
    let Some(i) = s.len().checked_sub(3) else {
        return s;
    };
    if s.is_char_boundary(i) && s[i..].eq_ignore_ascii_case(".md") {
        &s[..i]
    } else {
        s
    }
}

/// skill 展示名：`rust-style` 与 `.crabmate/skills/rust-style.md` 归一为同一 id。
#[must_use]
pub fn skill_display_name(raw: &str) -> Option<String> {
    let t = raw.trim();
    if t.is_empty() {
        return None;
    }
    let base = t.rsplit(['/', '\\']).next().unwrap_or(t).trim();
    let base = strip_md_suffix(base).trim();
    if base.is_empty() {
        None
    } else {
        Some(base.to_string())
    }
}

fn skill_inject_from_detail_value(v: &Value) -> Option<SkillInjectInfo> {
    let arr = v.get("skills")?.as_array()?;
    let mut skills: Vec<String> = Vec::new();
    for raw in arr.iter().filter_map(|s| s.as_str()) {
        let Some(name) = skill_display_name(raw) else {
            continue;
        };
        if !skills.contains(&name) {
            skills.push(name);
        }
    }
    if skills.is_empty() {
        return None;
    }
    Some(SkillInjectInfo {
        skills,
        forced: v.get("forced").and_then(Value::as_bool).unwrap_or(false),
    })
}

/// 水合外层 JSON 的 `detail` 是字符串；流式正文是 `title\n<detail>`；也接受纯 detail JSON。
fn detail_value(text: &str) -> Option<Value> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    if let Ok(v) = serde_json::from_str::<Value>(t) {
        if let Some(detail) = v.get("detail").and_then(|d| d.as_str()) {
            return serde_json::from_str(detail.trim()).ok();
        }
        return v.is_object().then_some(v);
    }
    let (_, rest) = t.split_once('\n')?;
    let v: Value = serde_json::from_str(rest.trim()).ok()?;
    if let Some(detail) = v.get("detail").and_then(|d| d.as_str()) {
        return serde_json::from_str(detail.trim()).ok();
    }
    v.is_object().then_some(v)
}

/// 从 `context_inject` 的 detail（detail JSON 或「title\n<detail>」）提取 skill。
///
/// **不做**注入门控：调用方已确认这条消息就是 `context_inject` 旁注时用它；
/// 对任意正文请用 [`skill_inject_from_timeline_text`]。
#[must_use]
pub fn skill_inject_from_detail(text: &str) -> Option<SkillInjectInfo> {
    skill_inject_from_detail_value(&detail_value(text)?)
}

/// 旁注只可能是 JSON 对象，或「标题行 + JSON 对象」；先做廉价前缀判断，
/// 避免对普通正文（Markdown / 纯文本）反复尝试整串解析。
fn looks_like_context_event(text: &str) -> bool {
    text.starts_with('{')
        || text
            .split_once('\n')
            .is_some_and(|(_, rest)| rest.trim_start().starts_with('{'))
}

fn json_object_kind_is_inject(text: &str) -> bool {
    let Ok(v) = serde_json::from_str::<Value>(text.trim()) else {
        return false;
    };
    v.get("kind")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|k| k == "context_inject")
}

fn json_looks_like_context_inject_detail(text: &str) -> bool {
    let Ok(v) = serde_json::from_str::<Value>(text.trim()) else {
        return false;
    };
    let Some(obj) = v.as_object() else {
        return false;
    };
    obj.contains_key("skills") && obj.contains_key("forced")
}

/// 门控：这条正文是不是一条 **skill 注入**旁注。
///
/// 比 `timeline_scan::is_turn_context_inject_timeline_text` 窄一档——那个还要认
/// `context_trim`（因为它同时供裁剪卡与「显示每轮注入」开关用），而裁剪事件里没有
/// `skills` 数组，解析结果一样是 `None`，所以窄在这里不改变 skill 卡的可见性。
///
/// 门控存在的理由：正文恰为 `{"skills":[...]}` 的普通消息不得被误判成注入旁注，
/// 合法注入行必须带 `kind=context_inject`，或 detail 同时含 `skills` + `forced`
/// （Server 0.6.0 起恒定输出 `forced`）。
fn looks_like_skill_inject_event(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() || !looks_like_context_event(t) {
        return false;
    }
    if json_object_kind_is_inject(t) || json_looks_like_context_inject_detail(t) {
        return true;
    }
    let Some((_, rest)) = t.split_once('\n') else {
        return false;
    };
    json_object_kind_is_inject(rest) || json_looks_like_context_inject_detail(rest)
}

/// 从任意消息正文提取 skill 调用，先过注入门控。
#[must_use]
pub fn skill_inject_from_timeline_text(text: &str) -> Option<SkillInjectInfo> {
    if !looks_like_skill_inject_event(text) {
        return None;
    }
    skill_inject_from_detail(text)
}

#[cfg(test)]
mod tests {
    use super::{
        SkillInjectInfo, skill_display_name, skill_inject_from_detail,
        skill_inject_from_timeline_text,
    };

    fn info(skills: &[&str], forced: bool) -> SkillInjectInfo {
        SkillInjectInfo {
            skills: skills.iter().map(|s| (*s).to_string()).collect(),
            forced,
        }
    }

    #[test]
    fn parses_three_shapes_and_dedupes() {
        let hydrated = r#"{"kind":"context_inject","title":"本轮注入","detail":"{\"skills\":[\"rust-style\",\"code-review\",\"rust-style\"],\"forced\":true}"}"#;
        let streamed = "本轮注入\n{\"skills\":[\"rust-style\",\"code-review\"],\"forced\":true}";
        let bare = r#"{"skills":["rust-style","code-review"],"forced":true}"#;
        let want = Some(info(&["rust-style", "code-review"], true));
        assert_eq!(skill_inject_from_timeline_text(hydrated), want);
        assert_eq!(skill_inject_from_timeline_text(streamed), want);
        assert_eq!(skill_inject_from_timeline_text(bare), want);
    }

    #[test]
    fn normalises_paths_and_md_suffix() {
        assert_eq!(
            skill_display_name(".crabmate/skills/rust-style.md").as_deref(),
            Some("rust-style")
        );
        assert_eq!(
            skill_display_name("code-review.MD").as_deref(),
            Some("code-review")
        );
        assert_eq!(
            skill_display_name("  /a/b/tui.md  ").as_deref(),
            Some("tui")
        );
        assert_eq!(skill_display_name(""), None);
        assert_eq!(skill_display_name("   "), None);
    }

    #[test]
    fn plain_message_with_skills_json_is_not_an_inject_event() {
        let raw = r#"{"skills":["rust-style"]}"#;
        assert_eq!(
            skill_inject_from_detail(raw),
            Some(info(&["rust-style"], false))
        );
        assert_eq!(skill_inject_from_timeline_text(raw), None);
        assert_eq!(skill_inject_from_timeline_text("普通问题"), None);
        assert_eq!(skill_inject_from_timeline_text(""), None);
    }

    #[test]
    fn context_trim_and_empty_skills_are_not_skill_injects() {
        let trim =
            r#"{"kind":"context_trim","title":"已裁剪","detail":"{\"after_tokens\":40000}"}"#;
        assert_eq!(skill_inject_from_timeline_text(trim), None);
        let empty = r#"{"kind":"context_inject","title":"注入","detail":"{\"skills\":[],\"forced\":false}"}"#;
        assert_eq!(skill_inject_from_timeline_text(empty), None);
    }
}
