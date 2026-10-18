//! 从当前会话消息中抽取「规划 / 工具」时间线条目（供侧栏式面板跳转）。

use serde::Deserialize;
use serde_json::json;

use crate::storage::{StoredMessage, StoredMessageState};

pub use crate::storage::TIMELINE_UI_STATE_KEY;

fn is_planner_tool_call_rejected_timeline_text(text: &str) -> bool {
    text.contains("planner_tool_call_rejected")
        || text.contains("PLANNER_TOOL_CALL_REJECTED")
        || text.contains("规划轮工具调用已拒绝")
}

/// 编排路由决议（`timeline_log` `kind=orchestration_route`）仅供 tracing / replay，不进聊天气泡。
fn is_orchestration_route_timeline_text(text: &str) -> bool {
    text.trim_start().starts_with("编排路由：")
}

/// Server `timeline_log.kind`：本轮注入 / 窗口裁剪摘要（软字段；默认不进主列）。
#[must_use]
pub fn is_turn_context_inject_kind(kind: &str) -> bool {
    matches!(kind.trim(), "context_inject" | "context_trim")
}

/// 从 `timeline_log` `context_trim` 的 `detail` JSON 取裁剪 / 压缩后服务端 `used_input_tokens`
/// （字段 `after_tokens`，见 Server `context_timeline::trim_event`）。
///
/// 缺失 / 空 / 非 JSON / `null` / 非无符号数 → `None`（此时不重置
/// [`ContextUsedWatermark`](crate::chat_session_state::ContextUsedWatermark) 水位）。
#[must_use]
pub fn context_trim_after_used_tokens(detail: Option<&str>) -> Option<u32> {
    let raw = detail?.trim();
    if raw.is_empty() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    u32::try_from(v.get("after_tokens")?.as_u64()?).ok()
}

/// 判定 `context_trim` 事件是否应重置当前会话的上下文水位，并返回归属 `(conversation_id, after_tokens)`。
///
/// 纯函数（仅依赖入参，便于单测）；三条件缺一不可，任一缺失 → `None`：
/// - `bound_active`：本轮流正被查看——后台流不得改全局水位（与 apply 侧 key 同口径）；
/// - `after`：能从 `detail` 解析出裁剪后 `used_input_tokens`（[`context_trim_after_used_tokens`]）；
/// - `cid`：存在服务端 `conversation_id`，水位以它归属。
///
/// 水位「会话内单调、压缩 / 裁剪后回落、回落延迟到回合末快照」的语义与理由见
/// `docs/adr/0005-status-bar-context-watermark.md`。
#[must_use]
pub fn context_trim_watermark_reset(
    bound_active: bool,
    after: Option<u32>,
    cid: Option<&str>,
) -> Option<(&str, u32)> {
    if !bound_active {
        return None;
    }
    Some((cid?, after?))
}

fn json_object_kind_is_inject(text: &str) -> bool {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text.trim()) else {
        return false;
    };
    v.get("kind")
        .and_then(|k| k.as_str())
        .is_some_and(is_turn_context_inject_kind)
}

fn json_looks_like_context_inject_detail(text: &str) -> bool {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text.trim()) else {
        return false;
    };
    let Some(obj) = v.as_object() else {
        return false;
    };
    (obj.contains_key("skills") && obj.contains_key("forced"))
        || (obj.contains_key("count_hit") && obj.contains_key("n_before"))
}

/// 本轮上下文压缩 / 裁剪信息（Server `context_trim` 的 `detail`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextTrimInfo {
    /// 服务端事件标题（如「已裁剪历史」/「已压缩工具输出」）；纯 detail 形态下为空。
    pub title: String,
    /// 压缩前 token 数（`detail.before_tokens`）。
    pub before_tokens: Option<u32>,
    /// 压缩后服务端 `used_input_tokens`（`detail.after_tokens`）。
    pub after_tokens: Option<u32>,
    /// 模型上下文窗口（`detail.max_input_tokens`）。
    pub max_input_tokens: Option<u32>,
    /// 为输出预留的 token 数（`detail.reserved_output_tokens`）。
    pub reserved_output_tokens: Option<u32>,
    /// 消息正文占用（`detail.message_tokens`）。
    pub message_tokens: Option<u32>,
    /// 工具 schema 占用（`detail.tool_schema_tokens`）。
    pub tool_schema_tokens: Option<u32>,
    /// 附件占用（`detail.attachment_tokens`）。
    pub attachment_tokens: Option<u32>,
    /// 裁剪前消息条数（`detail.n_before`）。
    pub n_before: Option<u32>,
    /// 裁剪后消息条数（`detail.n_after`）。
    pub n_after: Option<u32>,
    /// 按条数阈值命中的裁剪（`detail.count_hit`）。
    pub count_hit: bool,
    /// 按字符阈值命中的裁剪（`detail.char_hit`）。
    pub char_hit: bool,
    /// 工具输出压缩次数（`detail.compress_hits`）。
    pub compress_hits: u32,
    /// 是否由 LLM 摘要替换前缀（`detail.summarized`）。
    pub summarized: bool,
    /// 摘要后保留的末尾消息数（`detail.tail_kept`）。
    pub tail_kept: Option<u32>,
    /// 是否由 token 预算触发（`detail.token_triggered`）。
    pub token_triggered: bool,
    /// 移除的完整轮次（`detail.removed_turn_groups`）。
    pub removed_turn_groups: u32,
    /// 移除的消息条数（`detail.removed_messages`）。
    pub removed_messages: u32,
    /// token 计数来源（`detail.counting_source`）。
    pub counting_source: Option<String>,
    /// 压缩原因标识（`detail.compaction_reason`）。
    pub compaction_reason: Option<String>,
    /// 归一化（pretty）后的 detail JSON，供原始 JSON 折叠区展示。
    pub detail: String,
}

/// detail 形状是否为 `context_trim`（`skills` / `kinds` 存在时优先按 `context_inject` 处理）。
fn detail_looks_like_context_trim(v: &serde_json::Value) -> bool {
    let Some(obj) = v.as_object() else {
        return false;
    };
    if obj.contains_key("skills") || obj.contains_key("kinds") {
        return false;
    }
    obj.contains_key("after_tokens") || obj.contains_key("n_after") || obj.contains_key("n_before")
}

/// 旁注只可能是 JSON 对象，或「标题行 + JSON 对象」；先做廉价前缀判断，
/// 避免对普通正文（Markdown / 纯文本）反复尝试整串解析。
fn looks_like_context_event(text: &str) -> bool {
    text.starts_with('{')
        || text
            .split_once('\n')
            .is_some_and(|(_, rest)| rest.trim_start().starts_with('{'))
}

/// 一次解析拆出 `(kind 是否为 context_trim, title, detail)`：水合 `{kind,title,detail}`
/// （`detail` 为字符串）、流式 `title\n<detail>`、纯 detail JSON 三种形态。
fn context_event_parts(text: &str) -> Option<(bool, String, serde_json::Value)> {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(text) {
        let kind_is_trim = v.get("kind").and_then(|k| k.as_str()) == Some("context_trim");
        let title = v
            .get("title")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        return Some((kind_is_trim, title, nested_timeline_detail(&v)?));
    }
    let (title, rest) = text.split_once('\n')?;
    let v: serde_json::Value = serde_json::from_str(rest.trim()).ok()?;
    Some((false, title.trim().to_string(), nested_timeline_detail(&v)?))
}

/// 外层 `detail` 为 JSON 字符串时再解析一层；已是对象时直接取该对象。
fn nested_timeline_detail(v: &serde_json::Value) -> Option<serde_json::Value> {
    match v.get("detail") {
        Some(serde_json::Value::String(raw)) => serde_json::from_str(raw.trim()).ok(),
        Some(other @ serde_json::Value::Object(_)) => Some(other.clone()),
        Some(_) => None,
        None => v.is_object().then(|| v.clone()),
    }
}

fn json_u32(v: &serde_json::Value, key: &str) -> Option<u32> {
    u32::try_from(v.get(key)?.as_u64()?).ok()
}

fn json_bool(v: &serde_json::Value, key: &str) -> Option<bool> {
    v.get(key)?.as_bool()
}

fn json_string(v: &serde_json::Value, key: &str) -> Option<String> {
    v.get(key)?
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// 从任意消息正文提取上下文压缩 / 裁剪信息；非 trim 旁注 → `None`。
///
/// 水合形态以 `kind=context_trim` 为准；流式 / 纯 detail 形态按
/// [`detail_looks_like_context_trim`] 判定（Server `trim_event` 至少含 `after_tokens` /
/// `n_before` / `n_after` 之一，E2E 也存在仅 `after_tokens` 的形态）。
/// 卡片渲染与默认隐藏开关共用此判定。
#[must_use]
pub fn context_trim_from_timeline_text(text: &str) -> Option<ContextTrimInfo> {
    let t = text.trim();
    if t.is_empty() || !looks_like_context_event(t) {
        return None;
    }
    let (kind_is_trim, title, detail) = context_event_parts(t)?;
    if !kind_is_trim && !detail_looks_like_context_trim(&detail) {
        return None;
    }
    let detail_text = serde_json::to_string_pretty(&detail).unwrap_or_else(|_| detail.to_string());
    Some(ContextTrimInfo {
        title,
        before_tokens: json_u32(&detail, "before_tokens"),
        after_tokens: json_u32(&detail, "after_tokens"),
        max_input_tokens: json_u32(&detail, "max_input_tokens"),
        reserved_output_tokens: json_u32(&detail, "reserved_output_tokens"),
        message_tokens: json_u32(&detail, "message_tokens"),
        tool_schema_tokens: json_u32(&detail, "tool_schema_tokens"),
        attachment_tokens: json_u32(&detail, "attachment_tokens"),
        n_before: json_u32(&detail, "n_before"),
        n_after: json_u32(&detail, "n_after"),
        count_hit: json_bool(&detail, "count_hit").unwrap_or(false),
        char_hit: json_bool(&detail, "char_hit").unwrap_or(false),
        compress_hits: json_u32(&detail, "compress_hits").unwrap_or(0),
        summarized: json_bool(&detail, "summarized").unwrap_or(false),
        tail_kept: json_u32(&detail, "tail_kept"),
        token_triggered: json_bool(&detail, "token_triggered").unwrap_or(false),
        removed_turn_groups: json_u32(&detail, "removed_turn_groups").unwrap_or(0),
        removed_messages: json_u32(&detail, "removed_messages").unwrap_or(0),
        counting_source: json_string(&detail, "counting_source"),
        compaction_reason: json_string(&detail, "compaction_reason"),
        detail: detail_text,
    })
}

/// 上下文压缩 / 裁剪旁注（system 水合行或 assistant 本地 snapshot）。
#[must_use]
pub fn is_context_trim_timeline_message(m: &StoredMessage) -> bool {
    !m.is_tool && context_trim_from_timeline_text(&m.text).is_some()
}

/// 水合 `crabmate_timeline` 正文或流式旁注拼接（title + `\n` + detail JSON）。
#[must_use]
pub fn is_turn_context_inject_timeline_text(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() || !looks_like_context_event(t) {
        return false;
    }
    if context_trim_from_timeline_text(t).is_some() {
        return true;
    }
    if json_object_kind_is_inject(t) {
        return true;
    }
    if json_looks_like_context_inject_detail(t) {
        return true;
    }
    let Some((_, rest)) = t.split_once('\n') else {
        return false;
    };
    json_object_kind_is_inject(rest) || json_looks_like_context_inject_detail(rest)
}

/// 主列 / 导出：本轮注入或裁剪旁注（system 水合行或 assistant 本地 snapshot）。
#[must_use]
pub fn is_turn_context_inject_timeline_message(m: &StoredMessage) -> bool {
    if m.is_tool {
        return false;
    }
    is_turn_context_inject_timeline_text(&m.text)
}

/// 本轮 skill 调用信息（Server 0.6.0 `context_inject` 的 `detail.skills` / `detail.forced`）。
#[derive(Debug, Clone, PartialEq, Eq)]
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
fn skill_display_name(raw: &str) -> Option<String> {
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

fn skill_inject_from_detail_value(v: &serde_json::Value) -> Option<SkillInjectInfo> {
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
        forced: v
            .get("forced")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
    })
}

/// 水合外层 JSON 的 `detail` 是字符串；流式正文是 `title\n<detail>`；也接受纯 detail JSON。
fn context_inject_detail_value(text: &str) -> Option<serde_json::Value> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(t) {
        if let Some(detail) = v.get("detail").and_then(|d| d.as_str()) {
            return serde_json::from_str(detail.trim()).ok();
        }
        return v.is_object().then_some(v);
    }
    let (_, rest) = t.split_once('\n')?;
    let v: serde_json::Value = serde_json::from_str(rest.trim()).ok()?;
    if let Some(detail) = v.get("detail").and_then(|d| d.as_str()) {
        return serde_json::from_str(detail.trim()).ok();
    }
    v.is_object().then_some(v)
}

/// 从已确认是 `context_inject` 的 detail 中提取 skill 调用信息；无 skill 或非法输入 → `None`。
///
/// 三种落盘形态均支持：水合 `{kind,title,detail}`（`detail` 为 JSON 字符串）、流式
/// `title\n<detail>`、纯 detail JSON。展示层据此画独立 skill 卡（不得伪造成 `tool_call`）。
///
/// 对任意消息正文请改用 [`skill_inject_from_timeline_text`]：本函数不做注入门控。
#[must_use]
pub fn context_inject_skills(text: &str) -> Option<SkillInjectInfo> {
    skill_inject_from_detail_value(&context_inject_detail_value(text)?)
}

/// 从任意消息正文提取 skill 调用，先过 `context_inject` 门控。
///
/// 门控避免把正文恰为 `{"skills":[...]}` 的普通消息误判为注入旁注：合法注入行必须带
/// `kind=context_inject`，或 detail 同时含 `skills` + `forced`（Server 0.6.0 恒定输出）。
#[must_use]
pub fn skill_inject_from_timeline_text(text: &str) -> Option<SkillInjectInfo> {
    if !is_turn_context_inject_timeline_text(text) {
        return None;
    }
    context_inject_skills(text)
}

/// 含 skill 调用的上下文注入旁注（主列默认可见）。
#[must_use]
pub fn is_skill_inject_timeline_message(m: &StoredMessage) -> bool {
    !m.is_tool && skill_inject_from_timeline_text(&m.text).is_some()
}

/// 服务端已持久化等价 skill 注入（水合后丢弃流式本地快照，避免双卡）。
fn server_has_equivalent_skill_inject(server_msgs: &[StoredMessage], local_text: &str) -> bool {
    let Some(local) = skill_inject_from_timeline_text(local_text) else {
        return false;
    };
    server_msgs
        .iter()
        .filter(|s| !s.is_tool)
        .filter(|s| {
            !s.state
                .as_ref()
                .is_some_and(|st| st.is_local_timeline_snapshot_row())
        })
        .filter_map(|s| skill_inject_from_timeline_text(&s.text))
        .any(|info| info.skills == local.skills && info.forced == local.forced)
}

/// 两条 trim 事件是否指向同一轮：`title` 相同，且至少有一个可比的稳定键相等。
///
/// `SSE` 首次事件与落盘最终事件的计数会继续累加，因此 `n_before` 或 `after_tokens`
/// 任一对齐即视为同一事件；两侧没有共同键时保守地视为不同，避免误删另一轮卡片。
fn context_trim_rows_match(local: &ContextTrimInfo, server: &ContextTrimInfo) -> bool {
    if server.title != local.title {
        return false;
    }
    let same = |a: Option<u32>, b: Option<u32>| matches!((a, b), (Some(x), Some(y)) if x == y);
    same(local.n_before, server.n_before) || same(local.after_tokens, server.after_tokens)
}

/// 服务端已持久化等价压缩 / 裁剪旁注（水合后丢弃流式本地快照，避免双卡）。
fn server_has_equivalent_context_trim(server_msgs: &[StoredMessage], local_text: &str) -> bool {
    let Some(local) = context_trim_from_timeline_text(local_text) else {
        return false;
    };
    server_msgs
        .iter()
        .filter(|s| !s.is_tool)
        .filter(|s| {
            !s.state
                .as_ref()
                .is_some_and(|st| st.is_local_timeline_snapshot_row())
        })
        .filter_map(|s| context_trim_from_timeline_text(&s.text))
        .any(|info| context_trim_rows_match(&local, &info))
}

/// 已落盘的编排路由旁注：导出与聊天列均跳过。
pub fn is_orchestration_route_timeline_message(m: &StoredMessage) -> bool {
    !m.is_tool && is_orchestration_route_timeline_text(&m.text)
}

/// 工具轮次前的 commentary 旁注（`commentary_before_tools`）：不进主气泡/导出。
pub fn is_commentary_before_tools_assistant(m: &StoredMessage) -> bool {
    m.role == "assistant"
        && !m.is_tool
        && m.state
            .as_ref()
            .is_some_and(|s| matches!(s, crate::storage::StoredMessageState::CommentaryBeforeTools))
}

fn json_value_looks_like_tool_args(v: &serde_json::Value) -> bool {
    match v {
        serde_json::Value::Array(a) => {
            let mut saw_arg_like = false;
            for x in a {
                let Some(s) = x.as_str() else {
                    return false;
                };
                let t = s.trim();
                saw_arg_like |= t.starts_with('-')
                    || t.contains('=')
                    || t.contains('/')
                    || t.ends_with(".gz")
                    || t.ends_with(".tar");
            }
            !a.is_empty() && saw_arg_like
        }
        serde_json::Value::Object(o) => o.keys().any(|k| {
            matches!(
                k.as_str(),
                "command" | "args" | "path" | "content" | "cmd" | "pattern" | "file" | "files"
            )
        }),
        _ => false,
    }
}

fn is_tool_argument_residue_assistant_text(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return false;
    }
    serde_json::from_str::<serde_json::Value>(t).is_ok_and(|v| json_value_looks_like_tool_args(&v))
}

fn is_bare_shell_command_residue(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() || t.lines().count() != 1 {
        return false;
    }
    let lower = t.to_lowercase();
    let command_like = [
        "tar ", "make ", "cmake ", "bash ", "sh ", "ls ", "cat ", "python ", "python3 ", "./",
        "cargo ",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix));
    command_like
        && (lower.contains(" -")
            || lower.contains(" --")
            || lower.contains('/')
            || lower.contains(".tar")
            || lower.contains(".gz")
            || lower.contains('='))
}

/// 常见的压缩包 / 归档扩展名（`.tar.gz` 等）。
fn bare_archive_or_compressed_suffix(lower: &str) -> bool {
    lower.ends_with(".tar.gz")
        || lower.ends_with(".tgz")
        || lower.ends_with(".tar")
        || lower.ends_with(".zip")
        || lower.ends_with(".gz")
}

fn is_bare_tool_path_arg_residue(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() || t.lines().count() != 1 || t.contains(char::is_whitespace) {
        return false;
    }
    if t.contains("```") || t.contains('：') || t.contains(':') {
        return false;
    }
    let lower = t.to_lowercase();
    bare_archive_or_compressed_suffix(&lower) || is_bare_single_line_path_residue(t)
}

/// 单行、无扩展名或疑似截断的文件路径（如 `.../INSTALLe`）——多为工具参数残留。
fn is_bare_single_line_path_residue(text: &str) -> bool {
    if !text.contains('/') {
        return false;
    }
    let basename = text.rsplit('/').next().unwrap_or(text);
    if basename.eq_ignore_ascii_case("Makefile") || basename.eq_ignore_ascii_case("README") {
        return false;
    }
    if basename.contains('.') {
        return false;
    }
    basename.len() <= 12
}
#[derive(Debug, Deserialize)]
struct TimelineUiState {
    k: String,
    t: String,
}

pub fn timeline_state_tool(msg_id: &str, ok: bool) -> StoredMessageState {
    StoredMessageState::TimelineUiJson(
        json!({
            "k": TIMELINE_UI_STATE_KEY,
            "t": "tool",
            "msg": msg_id,
            "ok": ok,
        })
        .to_string(),
    )
}

/// 工具时间线状态中的 `ok`（非 tool 快照或缺字段 → [`None`]）。
#[must_use]
pub fn timeline_tool_ok(state: &StoredMessageState) -> Option<bool> {
    let raw = state.as_timeline_parse_candidate()?;
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    if v.get("k").and_then(|x| x.as_str()) != Some(TIMELINE_UI_STATE_KEY) {
        return None;
    }
    if v.get("t").and_then(|x| x.as_str()) != Some("tool") {
        return None;
    }
    v.get("ok").and_then(|x| x.as_bool())
}

/// 本地时间线快照（仅用于 hydrate 保留；不会进入侧栏时间线条目）。
pub fn timeline_state_local_snapshot() -> StoredMessageState {
    StoredMessageState::TimelineUiJson(
        json!({
            "k": TIMELINE_UI_STATE_KEY,
            "t": "local_snapshot",
        })
        .to_string(),
    )
}

/// `final_response` 时间线补偿旁注：Phase 4 起不再 push 新行；保留供导出/水合单测与旧会话 state 解析。
#[allow(dead_code)]
pub fn timeline_state_final_response_snapshot() -> StoredMessageState {
    StoredMessageState::TimelineUiJson(
        json!({
            "k": TIMELINE_UI_STATE_KEY,
            "t": "final_response_snapshot",
            "msg": "",
        })
        .to_string(),
    )
}

fn parse_timeline_ui_snapshot_type(raw: &str) -> Option<String> {
    let v: TimelineUiState = serde_json::from_str(raw).ok()?;
    if v.k != TIMELINE_UI_STATE_KEY {
        return None;
    }
    Some(v.t)
}

/// 从 [`StoredMessageState`] 读取时间线快照 `t`（如 `final_response_snapshot`、`local_snapshot`）。
pub fn timeline_ui_snapshot_type(state: &StoredMessageState) -> Option<String> {
    state
        .as_timeline_parse_candidate()
        .and_then(parse_timeline_ui_snapshot_type)
}

fn server_assistant_has_trimmed_text(server_msgs: &[StoredMessage], text: &str) -> bool {
    let needle = text.trim();
    if needle.is_empty() {
        return false;
    }
    server_msgs
        .iter()
        .any(|m| m.role == "assistant" && !m.is_tool && m.text.trim() == needle)
}

/// 本地 timeline 旁注是否与同会话内「正式」助手行正文重复（兼容旧 `local_snapshot`）。
pub fn is_timeline_snapshot_duplicate_of_canonical_assistant(
    m: &StoredMessage,
    session_messages: &[StoredMessage],
) -> bool {
    if m.role != "assistant" || m.is_tool {
        return false;
    }
    let Some(state) = m.state.as_ref() else {
        return false;
    };
    if !state.is_local_timeline_snapshot_row() {
        return false;
    }
    let needle = m.text.trim();
    if needle.is_empty() {
        return false;
    }
    session_messages.iter().any(|other| {
        other.id != m.id
            && other.role == "assistant"
            && !other.is_tool
            && !other
                .state
                .as_ref()
                .is_some_and(|s| s.is_local_timeline_snapshot_row())
            && other.text.trim() == needle
    })
}

/// 水合合并时：`final_response` 补偿旁注若与服务端助手正文重复则丢弃。
pub fn should_preserve_local_timeline_on_hydrate(
    m: &StoredMessage,
    server_msgs: &[StoredMessage],
) -> bool {
    let Some(state) = m.state.as_ref() else {
        return false;
    };
    if !state.is_local_timeline_snapshot_row() {
        return false;
    }
    match timeline_ui_snapshot_type(state).as_deref() {
        Some("final_response_snapshot") => !server_assistant_has_trimmed_text(server_msgs, &m.text),
        Some("local_snapshot") | None => {
            if is_skill_inject_timeline_message(m)
                && server_has_equivalent_skill_inject(server_msgs, &m.text)
            {
                return false;
            }
            if is_context_trim_timeline_message(m)
                && server_has_equivalent_context_trim(server_msgs, &m.text)
            {
                return false;
            }
            !server_assistant_has_trimmed_text(server_msgs, &m.text)
                && !is_timeline_snapshot_duplicate_of_canonical_assistant(m, server_msgs)
        }
        _ => true,
    }
}

/// 主列（TUI）应跳过的助手旁注：编排路由、工具前旁注状态、终答 snapshot、规划拒绝旁注、
/// 与正式助手行重复的本地 snapshot。**不含**导出专用的规划轮 JSON / 工具参数残留启发式。
#[must_use]
pub fn is_ephemeral_timeline_assistant_for_chat_ui(
    m: &StoredMessage,
    session_messages: &[StoredMessage],
) -> bool {
    if m.role != "assistant" || m.is_tool {
        return false;
    }
    if m.state
        .as_ref()
        .and_then(timeline_ui_snapshot_type)
        .is_some_and(|t| t == "final_response_snapshot")
    {
        return true;
    }
    if is_planner_tool_call_rejected_timeline_text(&m.text) {
        return true;
    }
    if is_orchestration_route_timeline_message(m) {
        return true;
    }
    if is_commentary_before_tools_assistant(m) {
        return true;
    }
    is_timeline_snapshot_duplicate_of_canonical_assistant(m, session_messages)
}

/// 会话导出时跳过仅用于流式 UI 的助手旁注（在主列过滤之上，另含规划轮 JSON 与工具参数残留）。
pub fn is_ephemeral_timeline_assistant_for_export(
    m: &StoredMessage,
    session_messages: &[StoredMessage],
) -> bool {
    if is_ephemeral_timeline_assistant_for_chat_ui(m, session_messages) {
        return true;
    }
    if m.role != "assistant" || m.is_tool {
        return false;
    }
    if crate::message_format::stored_message_is_staged_planner_round(m) {
        return true;
    }
    if is_tool_argument_residue_assistant_text(&m.text) {
        return true;
    }
    if is_bare_shell_command_residue(&m.text) {
        return true;
    }
    is_bare_tool_path_arg_residue(&m.text)
}

#[cfg(test)]
mod tests;
