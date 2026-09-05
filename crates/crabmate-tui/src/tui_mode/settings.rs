//! TUI 设置面板（W1）的纯逻辑层：三层合成 / 校验 / 保存动作 / 合并 PUT 构造。
//!
//! 本模块只做与 IO/UI 无关的推导（单测友好）：字段保存动作（[`LlmSave`] /
//! [`PrefsSave`]）、持久层快照（[`PersistedSettings`]）、单字段"override ＞
//! user-data ＞ serve 默认"的生效值合成、以及"先 GET 再改自己管理的键再全量 PUT"
//! 所需的 DTO 合并函数。UI 状态机见 [`super::settings_panel`]。

use crabmate_tui_core::{ClientLlmFields, LlmOverridesDto, UserPrefsDto};

/// 会话模式枚举的合法取值（与 `/mode` 斜杠一致；面板枚举与校验共用，避免漂移）。
pub const SESSION_MODES: [&str; 3] = ["ask", "plan", "act"];

/// 思考模式「跟随 server」的显式 user-data 值（Desktop 校验允许；TUI 持久层归一为
/// `None` = 跟随 server，随轮不发送该键）。
pub const THINKING_SERVER: &str = "server";

/// 思考模式除 `server`（跟随）外的显式取值（对齐 Desktop `client_llm.llm_thinking_mode`）。
pub const THINKING_MODES: [&str; 2] = ["on", "off"];

/// 上下文 tokens 上限（对齐前端 `settings_commit.rs` 的 `validate_llm_context_tokens_override`）。
pub const CONTEXT_TOKENS_MAX: u64 = 10_000_000;

/// 只读工具缓存"禁用"的 staged 值（写 prefs `disable_readonly_tool_ttl_cache = true`，
/// 随轮发 `readonly_tool_ttl_cache_secs: 0`）；空 / 其它值 = 清除键 = 跟随 server。
///
/// 注意持久化表示与 Desktop 的差异（读取语义等价）：Desktop「跟随」显式写
/// `Some(false)`；TUI 用删除键（`None`）表示，缺省即跟随，两端行为一致。
pub const TOOL_CACHE_DISABLED: &str = "off";

/// 一个键的保存动作（面板管理键：model / api_base / temperature / thinking /
/// context_tokens / cm_role / session_mode / tool_cache）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum FieldAction {
    /// 未编辑：合并时跳过该键（user-data 现值原样保留）。
    #[default]
    Skip,
    /// 已编辑：`Some(s)` 写入新值（空串按清除处理）；`None` 清除（键写回 null，
    /// 该字段回落到 serve 默认）。
    Write(Option<String>),
}

impl FieldAction {
    /// 是否"未编辑"（保存时跳过）。
    #[must_use]
    pub const fn is_skip(&self) -> bool {
        matches!(self, Self::Skip)
    }

    /// 是否"已编辑"（有保存动作）。
    #[must_use]
    pub const fn is_write(&self) -> bool {
        matches!(self, Self::Write(_))
    }
}

/// `/user-data/llm-overrides` 的 `client_llm.{model,api_base,temperature,llm_thinking_mode,
/// llm_context_tokens}` 保存动作。
/// （密钥不在此列：`api_key` 只写本机钥匙串，见面板与 TuiApp 接线。）
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LlmSave {
    pub model: FieldAction,
    pub api_base: FieldAction,
    /// 温度覆盖（`client_llm.temperature`；空/`Write(None)` = 清除，回落 serve 默认）。
    pub temperature: FieldAction,
    /// 思考模式覆盖（`client_llm.llm_thinking_mode`；`Write(None)` = server / 跟随）。
    pub thinking: FieldAction,
    /// 上下文 tokens 覆盖（`client_llm.llm_context_tokens`；空/`Write(None)` = 清除，回落 serve 默认）。
    pub context_tokens: FieldAction,
}

impl LlmSave {
    /// 各字段是否有任一待保存动作。
    #[must_use]
    pub fn any(&self) -> bool {
        self.model.is_write()
            || self.api_base.is_write()
            || self.temperature.is_write()
            || self.thinking.is_write()
            || self.context_tokens.is_write()
    }

    /// 清空全部动作（保存成功落地后调用）。
    pub fn clear(&mut self) {
        self.model = FieldAction::Skip;
        self.api_base = FieldAction::Skip;
        self.temperature = FieldAction::Skip;
        self.thinking = FieldAction::Skip;
        self.context_tokens = FieldAction::Skip;
    }
}

/// `/user-data/prefs` 的 `cm_role / session_mode / disable_readonly_tool_ttl_cache` 保存动作。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrefsSave {
    pub role: FieldAction,
    pub session_mode: FieldAction,
    /// 只读工具缓存：`Write(Some("off"))` = 禁用（disable=true，随轮发 ttl=0）；
    /// `Write(None)` / 空 = 清除键（跟随 server）。见 [`TOOL_CACHE_DISABLED`]。
    pub tool_cache: FieldAction,
}

impl PrefsSave {
    /// 各组字段是否有任一待保存动作。
    #[must_use]
    pub fn any(&self) -> bool {
        self.role.is_write() || self.session_mode.is_write() || self.tool_cache.is_write()
    }

    /// 清空全部动作（保存成功落地后调用）。
    pub fn clear(&mut self) {
        self.role = FieldAction::Skip;
        self.session_mode = FieldAction::Skip;
        self.tool_cache = FieldAction::Skip;
    }
}

/// user-data 持久层快照（启动拉取 / 保存成功后更新）。
///
/// 连同一 serve 时与 Desktop/Web 共享：`client_llm.{model,api_base,temperature,
/// llm_thinking_mode,llm_context_tokens}` + prefs 的 `cm_role/session_mode/
/// disable_readonly_tool_ttl_cache`。空白值一律归一为 `None`（= 未设置 / 跟随 server）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PersistedSettings {
    pub model: Option<String>,
    pub api_base: Option<String>,
    /// 温度（trim 后存原文，如 `"0.7"`；非法/空 → `None` = 跟随 server）。
    pub temperature: Option<String>,
    /// 思考模式（仅 `on`/`off`；`server`/空 → `None` = 跟随 server）。
    pub thinking: Option<String>,
    /// 上下文 tokens（trim 后存原文；非法/空 → `None` = 跟随 server；随轮仅 > 0 发送）。
    pub context_tokens: Option<String>,
    pub role: Option<String>,
    pub session_mode: Option<String>,
    /// 只读工具 TTL 缓存禁用标记（镜像 prefs `disable_readonly_tool_ttl_cache`：
    /// `Some(true)` = 禁用 / 随轮发 ttl=0；`None`/`Some(false)` = 跟随 server）。
    pub tool_cache_disabled: Option<bool>,
}

impl PersistedSettings {
    /// 从 user-data 快照（prefs + llm-overrides）合成持久层；空白视为未设置，
    /// 非法会话模式值（旧文件遗留）丢弃并回落 serve 默认。
    #[must_use]
    pub fn from_snapshot(prefs: &UserPrefsDto, llm: &LlmOverridesDto) -> Self {
        Self {
            model: normalize(&llm.client_llm.model),
            api_base: normalize(&llm.client_llm.api_base),
            temperature: normalize(&llm.client_llm.temperature).filter(|v| is_valid_temperature(v)),
            // 只保留显式 on/off；server（跟随）/空/非法旧值 → None = 回落 serve 默认。
            thinking: normalize(&llm.client_llm.llm_thinking_mode)
                .filter(|m| is_valid_thinking_mode(m) && m.as_str() != THINKING_SERVER),
            context_tokens: normalize(&llm.client_llm.llm_context_tokens)
                .filter(|v| is_valid_context_tokens(v)),
            role: normalize(&prefs.cm_role),
            session_mode: normalize(&prefs.session_mode)
                .filter(|m| is_valid_session_mode(m.as_str())),
            tool_cache_disabled: prefs.disable_readonly_tool_ttl_cache,
        }
    }

    /// llm 侧保存成功后在内存做同样更新（未编辑的键不动）。
    pub fn apply_llm_saved(&mut self, save: &LlmSave) {
        apply_slot(&mut self.model, &save.model);
        apply_slot(&mut self.api_base, &save.api_base);
        apply_slot(&mut self.temperature, &save.temperature);
        apply_slot(&mut self.thinking, &save.thinking);
        apply_slot(&mut self.context_tokens, &save.context_tokens);
    }

    /// prefs 侧保存成功后在内存做同样更新（未编辑的键不动）。
    pub fn apply_prefs_saved(&mut self, save: &PrefsSave) {
        apply_slot(&mut self.role, &save.role);
        apply_slot(&mut self.session_mode, &save.session_mode);
        apply_tool_cache_action(&mut self.tool_cache_disabled, &save.tool_cache);
    }
}

/// 归一化一个字符串值（trim 后为空 → `None`）。
#[must_use]
pub fn normalize_str(v: &str) -> Option<String> {
    let t = v.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

/// 归一化一个 `Option` 字段（空 / 空白 → `None`）。
#[must_use]
pub fn normalize(o: &Option<String>) -> Option<String> {
    o.as_deref().and_then(normalize_str)
}

/// 按动作改一个字段槽（Skip 不动；Write 写入归一化值 / 清除）。
fn apply_slot(slot: &mut Option<String>, action: &FieldAction) {
    if action.is_skip() {
        return;
    }
    if let FieldAction::Write(v) = action {
        *slot = v.as_deref().and_then(normalize_str);
    }
}

/// 把工具缓存动作写进 bool 槽（`Write(Some("off"))` → `true` = 禁用；
/// `Write(None)` / 空 / 其它 = 清除键 = 跟随 server）。
fn apply_tool_cache_action(slot: &mut Option<bool>, action: &FieldAction) {
    if action.is_skip() {
        return;
    }
    if let FieldAction::Write(v) = action
        && v.as_deref().map(str::trim) == Some(TOOL_CACHE_DISABLED)
    {
        *slot = Some(true);
    } else {
        *slot = None;
    }
}

/// API Base 校验（对齐 Desktop URL 校验）：空 = 清除（合法）；非空需 `http(s)://` 前缀。
#[must_use]
pub fn validate_api_base(v: &str) -> bool {
    let t = v.trim();
    if t.is_empty() {
        return true;
    }
    let lower = t.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// 会话模式是否合法取值（ask / plan / act；与 `/mode` 一致）。
#[must_use]
pub fn is_valid_session_mode(v: &str) -> bool {
    SESSION_MODES.contains(&v)
}

/// 温度是否合法（对齐 Desktop）：trim 后空 = 合法（未设置）；非空需 `parse::<f64>`
/// 成功、`is_finite` 且在 `0.0..=2.0` 区间内。
#[must_use]
pub fn is_valid_temperature(v: &str) -> bool {
    let t = v.trim();
    if t.is_empty() {
        return true;
    }
    matches!(t.parse::<f64>(), Ok(n) if n.is_finite() && (0.0..=2.0).contains(&n))
}

/// 思考模式是否合法取值（对齐 Desktop）：空（未设置）/ `server` / `on` / `off`。
#[must_use]
pub fn is_valid_thinking_mode(v: &str) -> bool {
    let t = v.trim();
    t.is_empty() || t == THINKING_SERVER || THINKING_MODES.contains(&t)
}

/// 上下文 tokens 是否合法（对齐 Desktop `settings_commit.rs`）：trim 后空 = 合法（未设置）；
/// 非空需解析为 `u64` 且 ≤ [`CONTEXT_TOKENS_MAX`]（`0` 可存但不随轮发送，同 Desktop）。
#[must_use]
pub fn is_valid_context_tokens(v: &str) -> bool {
    let t = v.trim();
    if t.is_empty() {
        return true;
    }
    matches!(t.parse::<u64>(), Ok(n) if n <= CONTEXT_TOKENS_MAX)
}

/// 单字段生效值的来源层（面板行显示用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    /// 本进程 override（SessionPrefs；随行带 `*`）。
    Override,
    /// serve user-data 已存值（跨端共享）。
    Stored,
    /// `/status?view=shell` serve 默认（override 与 user-data 都没有时只读提示）。
    Default,
    /// 三层都没有 → 跟随 server。
    Follow,
}

/// 单字段三层合成的显示结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveView {
    pub layer: Layer,
    /// 显示值（`Follow` 层为 `None`）。
    pub value: Option<String>,
}

/// 三层合成（override ＞ user-data ＞ serve 默认）；空白一律视为未设置。
#[must_use]
pub fn effective_value(
    local: Option<&str>,
    stored: Option<&str>,
    remote: Option<&str>,
) -> EffectiveView {
    if let Some(v) = local.and_then(normalize_str) {
        return EffectiveView {
            layer: Layer::Override,
            value: Some(v),
        };
    }
    if let Some(v) = stored.and_then(normalize_str) {
        return EffectiveView {
            layer: Layer::Stored,
            value: Some(v),
        };
    }
    match remote.and_then(normalize_str) {
        Some(v) => EffectiveView {
            layer: Layer::Default,
            value: Some(v),
        },
        None => EffectiveView {
            layer: Layer::Follow,
            value: None,
        },
    }
}

/// 随轮发送用的两层合成（override ＞ user-data）；空白不发送（回落 serve 默认）。
#[must_use]
pub fn merge_turn(local: &Option<String>, stored: Option<&str>) -> Option<String> {
    normalize(local).or_else(|| stored.and_then(normalize_str))
}

/// 随轮 `client_llm.llm_context_tokens` 值：持久层存原文，仅 > 0 时发送为规范化数字串
/// （对齐 Desktop 只发正数；空/0/非数字 → `None` 不发送该键）。
#[must_use]
pub fn turn_context_tokens(stored: Option<&str>) -> Option<String> {
    stored
        .and_then(normalize_str)
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|n| *n > 0)
        .map(|n| n.to_string())
}

/// 随轮顶层 `readonly_tool_ttl_cache_secs`：仅缓存禁用（disable=`Some(true)`）时发 `0`；
/// 跟随 server（`None` / `Some(false)`）不发送。
#[must_use]
pub fn turn_tool_cache_secs(disabled: Option<bool>) -> Option<u64> {
    (disabled == Some(true)).then_some(0)
}

/// 随轮 `client_llm` 装配：override 已归一（model / api_base / api_key）与持久层
/// user-data（thinking 仅 on/off、context_tokens 仅 > 0）合成；全空返回 `None`
/// （不发送整块，回落 serve 默认）。供全屏 `tui` 每轮 `POST /chat/stream` 使用。
#[must_use]
pub fn build_turn_client_llm(
    model: Option<&str>,
    api_base: Option<&str>,
    api_key: Option<&str>,
    thinking: Option<&str>,
    context_tokens: Option<&str>,
) -> Option<ClientLlmFields> {
    let api_key = api_key
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let thinking = thinking
        .map(str::trim)
        .filter(|m| *m == "on" || *m == "off")
        .map(str::to_string);
    let context_tokens = turn_context_tokens(context_tokens);
    (model.is_some()
        || api_base.is_some()
        || api_key.is_some()
        || thinking.is_some()
        || context_tokens.is_some())
    .then_some(ClientLlmFields {
        api_key,
        model: model.map(str::to_string),
        api_base: api_base.map(str::to_string),
        llm_thinking_mode: thinking,
        llm_context_tokens: context_tokens,
    })
}

/// 合并写回 `/user-data/llm-overrides`：只改 `client_llm.{model,api_base,temperature,
/// llm_thinking_mode,llm_context_tokens}`，`executor_llm / saved_models / execution_mode`
/// 原样保留（合并保真）。
#[must_use]
pub fn merge_llm_save(mut base: LlmOverridesDto, save: &LlmSave) -> LlmOverridesDto {
    apply_slot(&mut base.client_llm.model, &save.model);
    apply_slot(&mut base.client_llm.api_base, &save.api_base);
    apply_slot(&mut base.client_llm.temperature, &save.temperature);
    apply_slot(&mut base.client_llm.llm_thinking_mode, &save.thinking);
    apply_slot(
        &mut base.client_llm.llm_context_tokens,
        &save.context_tokens,
    );
    base
}

/// 合并写回 `/user-data/prefs`：只改 `cm_role / session_mode / disable_readonly_tool_ttl_cache`，
/// `locale / theme / 布局 / IDE` 等字段原样保留（合并保真）。
#[must_use]
pub fn merge_prefs_save(mut base: UserPrefsDto, save: &PrefsSave) -> UserPrefsDto {
    apply_slot(&mut base.cm_role, &save.role);
    apply_slot(&mut base.session_mode, &save.session_mode);
    apply_tool_cache_action(&mut base.disable_readonly_tool_ttl_cache, &save.tool_cache);
    base
}

#[cfg(test)]
#[path = "settings_tests.rs"]
mod tests;
