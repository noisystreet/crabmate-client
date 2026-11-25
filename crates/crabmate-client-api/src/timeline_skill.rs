//! `timeline_log` 的 `context_inject` 旁注：提取本轮被调用的 skill。
//!
//! Desktop 主列（`frontend/src/timeline_scan.rs` 的 skill 卡）与 TUI 的技能行共用这一份口径。
//! 原先只有 WASM 侧有实现，TUI 只能「不支持」——两端各写一份迟早会按各自理解漂移。
//!
//! 三种落盘形态都要认（见 [`skill_inject_from_timeline_text`]）：水合 `{kind,title,detail}`
//! （`detail` 是 JSON **字符串**，不是嵌套对象）、流式 `title\n<detail>`、纯 detail JSON。
//!
//! **两代 `detail` 形状都要认**（Serve `cm_agent::context_timeline::detail`）：
//! - 新（Server #1502 起，方案 B）：`{"kinds":["memory"],"skills":[{"id":"rust-style","source":"forced"}]}`；
//!   技能-only 回合是 `{"kinds":[],"skills":[…]}`，`skills` 为空时**该键不出现**。
//! - 旧（#1478 之前，**含当前契约 pin 的 rev**）：`{"kinds":[…],"skills":["rust-style"],"forced":true}`。
//!
//! 两代都会出现在客户端：历史落盘会话存的是旧形状；而 Serve 侧两个 detail struct 都
//! `deny_unknown_fields`（只写不读），所以旧形状的读侧责任**全在 client**。宽容解析是长期
//! 约束，不是「等 Serve 升级完就删」的过渡代码。

use serde_json::Value;

/// 本轮 skill 调用信息（Serve `context_inject` 的 `detail.skills`）。
///
/// `forced` 是**推导值**，不是原样副本：新形状下由 `any(skills[].source == "forced")` 得出，
/// 旧形状下直接取 `detail.forced`。两代都只表达「用户显式 `/<id>` 选用」。
///
/// Serve 每回合至多写**一条**强制注入（`skill_refs_in_messages` 只取窗口内最后一条），所以
/// 真机上 `skills` 恒为单元素；这里仍按数组建模，是因为契约是数组、历史落盘也可能有旧值。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SkillInjectInfo {
    /// 归一化后的展示名（见 [`skill_display_name`]；保序去重）。
    pub skills: Vec<String>,
    /// 用户显式 `/<skill-id>` 选用（Serve 强制注入）。
    pub forced: bool,
}

/// 新形状 `skills[]` 元素里表示「强制调用」的 `source` 取值。
const FORCED_SOURCE: &str = "forced";

/// skill 的约定文件名：`<id>/SKILL.md` 是 skill 的标准布局。
const SKILL_FILE_NAME: &str = "SKILL.md";

/// skill 根目录名：就在它下面的 md 没有 id 目录可退。
const SKILLS_DIR_NAME: &str = "skills";

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

/// 路径的末两段：`(文件名, 上一层目录名)`；没有目录层级时第二项为 `None`。
///
/// 不做空段过滤：`dir/` 的末段是空串（「只有目录、没有文件名」），这与「取 basename」的老
/// 口径一致，改动它会让尾斜杠路径从「无法取名」变成「叫 `dir`」。
fn last_two_segments(s: &str) -> (&str, Option<&str>) {
    let mut parts = s.rsplit(['/', '\\']);
    let base = parts.next().unwrap_or("").trim();
    (base, parts.next().map(str::trim).filter(|p| !p.is_empty()))
}

/// skill 展示名：`rust-style`、`.crabmate/skills/rust-style.md`、
/// `.crabmate/skills/api-design/SKILL.md` 都归一成 skill 的 id。
///
/// `SKILL.md` 是 skill 的**约定文件名**，同一个名字每张卡都出现，看了不知道调用的是哪个
/// skill —— 所以这类路径取**所在目录名**（`api-design`）。唯一的例外是上一层目录本身就是
/// `skills`：那里没有 id 目录可退，只好用文件名本身（否则会显示成 `skills`）。
#[must_use]
pub fn skill_display_name(raw: &str) -> Option<String> {
    let t = raw.trim();
    if t.is_empty() {
        return None;
    }
    let (base, parent) = last_two_segments(t);
    if base.is_empty() {
        return None;
    }
    if base.eq_ignore_ascii_case(SKILL_FILE_NAME) {
        let id = parent.filter(|p| !p.eq_ignore_ascii_case(SKILLS_DIR_NAME));
        return Some(id.unwrap_or(base).to_string());
    }
    let base = strip_md_suffix(base).trim();
    if base.is_empty() {
        None
    } else {
        Some(base.to_string())
    }
}

/// `skills[]` 的单个元素 → `(原始名, 该元素是否表示强制调用)`。
///
/// 新形状是对象 `{ id, source }`，旧形状是裸字符串（历史落盘）。**类型不匹配的元素返回
/// `None`**：一个坏元素（数字 / `null` / 缺 `id` 的对象）不能把整条 detail 判成解析失败——
/// Serve 侧的兼容说明也是「读侧应忽略未知键与类型不匹配项」。
fn skill_ref_entry(item: &Value) -> Option<(&str, bool)> {
    if let Some(raw) = item.as_str() {
        return Some((raw, false));
    }
    let obj = item.as_object()?;
    let id = obj.get("id").and_then(Value::as_str)?;
    let forced = obj.get("source").and_then(Value::as_str) == Some(FORCED_SOURCE);
    Some((id, forced))
}

fn skill_inject_from_detail_value(v: &Value) -> Option<SkillInjectInfo> {
    // 键缺失（无 `/<id>` 的回合：Serve 用 `skip_serializing_if` 省掉整个 `skills`）→ 无技能。
    let arr = v.get("skills")?.as_array()?;
    // 旧形状把「显式选用」放在整条 detail 的 `forced: bool` 上；新形状摊到每个元素的 `source`。
    // 两者都要认，且**并集**：历史落盘可能是旧形状，未来也可能新旧混写。
    let mut forced = v.get("forced").and_then(Value::as_bool).unwrap_or(false);
    let mut skills: Vec<String> = Vec::new();
    for item in arr {
        let Some((raw, entry_forced)) = skill_ref_entry(item) else {
            continue;
        };
        forced |= entry_forced;
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
    Some(SkillInjectInfo { skills, forced })
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

/// 一个 detail（字符串化 JSON 对象）**是否长得像 `context_inject` 的 detail**。
///
/// 门控口径的**单一实现**：Desktop 的 `timeline_scan` 也用它（那里再并上 `context_trim` 的
/// 判定），避免两端各写一份规则后按各自理解漂移。
///
/// 判定按 Serve 的真实字节来定（`cm_agent::context_timeline::detail`）：
/// - `kinds` **恒序列化**（新旧两代都没有 `skip_serializing_if`），技能-only 回合是 `kinds: []`
///   —— 所以「含 `kinds`」覆盖新契约的全部形态；
/// - 旧形状（#1478 之前，含当前契约 pin 的 rev）是 `{kinds, skills:[字符串], forced}`，其
///   `skills` + `forced` 组合留着兜底；
/// - `context_trim` 的 detail struct **没有** `kinds` → 两个 kind 的判别互斥。
///
/// 为什么不能「含 `skills` 就算」：正文恰为 `{"skills":["rust-style"]}` 的普通消息不是旁注
/// （见 `plain_message_with_skills_json_is_not_an_inject_event`）——旧形状靠「同时有 `forced`」挡住。
#[must_use]
pub fn detail_looks_like_context_inject(text: &str) -> bool {
    let Ok(v) = serde_json::from_str::<Value>(text.trim()) else {
        return false;
    };
    let Some(obj) = v.as_object() else {
        return false;
    };
    obj.contains_key("kinds") || (obj.contains_key("skills") && obj.contains_key("forced"))
}

/// 门控：这条正文是不是一条 **skill 注入**旁注。
///
/// 比 `timeline_scan::is_turn_context_inject_timeline_text` 窄一档——那个还要认
/// `context_trim`（因为它同时供裁剪卡与「显示每轮注入」开关用），而裁剪事件里没有
/// `kinds` / `skills`，解析结果一样是 `None`，所以窄在这里不改变 skill 卡的可见性。
///
/// 门控存在的理由：正文恰为 `{"skills":[...]}` 的普通消息不得被误判成注入旁注。合法注入行
/// 必须带 `kind=context_inject`（水合形态），或 detail 通过
/// [`detail_looks_like_context_inject`] 的判定（流式形态的 detail 里没有 `kind`，只能靠它）。
fn looks_like_skill_inject_event(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() || !looks_like_context_event(t) {
        return false;
    }
    if json_object_kind_is_inject(t) || detail_looks_like_context_inject(t) {
        return true;
    }
    let Some((_, rest)) = t.split_once('\n') else {
        return false;
    };
    json_object_kind_is_inject(rest) || detail_looks_like_context_inject(rest)
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
        SkillInjectInfo, detail_looks_like_context_inject, skill_display_name,
        skill_inject_from_detail, skill_inject_from_timeline_text,
    };

    fn info(skills: &[&str], forced: bool) -> SkillInjectInfo {
        SkillInjectInfo {
            skills: skills.iter().map(|s| (*s).to_string()).collect(),
            forced,
        }
    }

    /// 旧形状（#1478 之前，含当前契约 pin 的 rev）：`skills` 是字符串数组 + `forced` 是 bool。
    ///
    /// 历史落盘会话就是这一形状，所以它必须一直能解析（Serve 侧 `deny_unknown_fields`，服务端
    /// 自己读不了旧值，读侧责任全在 client）。
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

    /// 新形状（Serve #1502 起）：`skills` 是对象数组 `{id, source}`，`forced` 由 `source` 推出。
    ///
    /// 三种落盘形态都要认；注意 detail 里**没有** `kind`，流式形态的门控只能靠 `kinds`。
    #[test]
    fn parses_new_structured_skill_refs() {
        let hydrated = r#"{"kind":"context_inject","title":"本轮已注入上下文","detail":"{\"kinds\":[],\"skills\":[{\"id\":\"rust-style\",\"source\":\"forced\"}]}"}"#;
        let streamed = "本轮已注入上下文\n{\"kinds\":[\"memory\"],\"skills\":[{\"id\":\"api-design\",\"source\":\"forced\"}]}";
        let bare = r#"{"kinds":[],"skills":[{"id":"code-review","source":"forced"}]}"#;
        assert_eq!(
            skill_inject_from_timeline_text(hydrated),
            Some(info(&["rust-style"], true))
        );
        assert_eq!(
            skill_inject_from_timeline_text(streamed),
            Some(info(&["api-design"], true))
        );
        assert_eq!(
            skill_inject_from_timeline_text(bare),
            Some(info(&["code-review"], true))
        );
    }

    /// `source` 是「显式选用」的唯一证据：别的来源不算强制调用，但技能仍要显示。
    ///
    /// 这条同时钉住「将来把模型主动 `skill_read` 纳入时间线」时的语义（`forced=false`，
    /// UI 不打「显式」标记），不需要届时代码改判。
    #[test]
    fn new_shape_takes_forced_from_source() {
        let raw = r#"{"kinds":[],"skills":[{"id":"rust-style","source":"auto"}]}"#;
        assert_eq!(
            skill_inject_from_timeline_text(raw),
            Some(info(&["rust-style"], false))
        );
    }

    /// 单个坏元素不得把整条 detail 判成解析失败（Serve 的兼容说明：忽略未知键与类型不匹配项）。
    #[test]
    fn malformed_skill_entries_are_skipped() {
        let raw = r#"{"kinds":[],"skills":[7,null,{"source":"forced"},{"id":"","source":"forced"},{"id":"code-review","source":"forced"}]}"#;
        assert_eq!(
            skill_inject_from_timeline_text(raw),
            Some(info(&["code-review"], true))
        );
    }

    /// 不带 `/<id>` 的回合：`detail` 只有 `kinds`（Serve 的 `context_timeline_detail_golden.json` 里那个
    /// `{"kinds":["memory"]}`），是注入旁注但**不得**出技能行（#593 验收项 2）。
    #[test]
    fn kinds_only_inject_has_no_skill_row() {
        assert!(detail_looks_like_context_inject(r#"{"kinds":["memory"]}"#));
        // 技能-only 回合是 `kinds: []`，键仍在。
        assert!(detail_looks_like_context_inject(r#"{"kinds":[]}"#));
        let hydrated = r#"{"kind":"context_inject","title":"本轮已注入上下文","detail":"{\"kinds\":[\"memory\"]}"}"#;
        let streamed = "本轮已注入上下文\n{\"kinds\":[\"memory\"]}";
        assert_eq!(skill_inject_from_timeline_text(hydrated), None);
        assert_eq!(skill_inject_from_timeline_text(streamed), None);
    }

    /// 实机字节（2026-11-25 从本地 `serve` 的一轮 `/<id>` 回合抓取，SSE `timeline_log` 与落盘 marker 同形）。
    ///
    /// 夹具照抄真实输出而不是手写，是因为「键顺序 / 是否带 `forced` / `skills` 元素形状」这三件事
    /// 只有真 serve 说了算；手写的夹具会让实现与自己对表。
    #[test]
    fn parses_the_live_wire_bytes_from_a_forced_skill_turn() {
        let sse = r#"{"type":"CUSTOM","customType":"timeline_log","data":{"kind":"context_inject","title":"本轮已注入上下文","detail":"{\"kinds\":[\"workspace_profile\"],\"skills\":[{\"id\":\"rust-style\",\"source\":\"forced\"}]}"}}"#;
        let hydrated = r#"{"detail":"{\"kinds\":[\"workspace_profile\"],\"skills\":[{\"id\":\"rust-style\",\"source\":\"forced\"}]}","kind":"context_inject","title":"本轮已注入上下文"}"#;
        let want = Some(info(&["rust-style"], true));
        // 流式：`data.detail` 是字符串化 JSON；水合 marker：整个正文是 `{detail,kind,title}`。
        let sse_detail =
            r#"{"kinds":["workspace_profile"],"skills":[{"id":"rust-style","source":"forced"}]}"#;
        assert_eq!(skill_inject_from_detail(sse_detail), want);
        assert_eq!(
            skill_inject_from_timeline_text(sse_detail),
            want,
            "流式 detail 形态（无 `kind`）也必须过门控"
        );
        assert!(detail_looks_like_context_inject(sse_detail));
        // 两条原始行都只用于形状对照：解析器吃的是它们里面的 detail 字符串。
        for raw in [sse, hydrated] {
            let v: serde_json::Value = serde_json::from_str(raw).expect("raw line is JSON");
            let detail = v
                .get("data")
                .and_then(|d| d.get("detail"))
                .or_else(|| v.get("detail"))
                .and_then(serde_json::Value::as_str)
                .expect("detail string");
            assert_eq!(skill_inject_from_detail(detail), want);
        }
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

    /// `SKILL.md` 是每个 skill 都一样的约定文件名：用所在目录当名字，否则卡片上全是 `SKILL`。
    #[test]
    fn a_skill_file_is_named_after_its_directory() {
        assert_eq!(
            skill_display_name(".crabmate/skills/api-design/SKILL.md").as_deref(),
            Some("api-design")
        );
        assert_eq!(
            skill_display_name("skills/api-design/SKILL.md").as_deref(),
            Some("api-design")
        );
        // 大小写不敏感 + Windows 分隔符
        assert_eq!(
            skill_display_name("skills\\api-design\\skill.MD").as_deref(),
            Some("api-design")
        );
        // 上一层目录就是 `skills`：没有 id 可退，退回文件名本身（不能显示成 `skills`）。
        assert_eq!(
            skill_display_name(".crabmate/skills/SKILL.md").as_deref(),
            Some("SKILL.md")
        );
        // 裸文件名，没有目录层级
        assert_eq!(skill_display_name("SKILL.md").as_deref(), Some("SKILL.md"));
        // 只有目录、没有文件名
        assert_eq!(skill_display_name(".crabmate/skills/"), None);
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
