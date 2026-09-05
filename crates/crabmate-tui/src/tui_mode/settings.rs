//! TUI 设置面板（W1）的纯逻辑层：三层合成 / 校验 / 保存动作 / 合并 PUT 构造。
//!
//! 本模块只做与 IO/UI 无关的推导（单测友好）：字段保存动作（[`LlmSave`] /
//! [`PrefsSave`]）、持久层快照（[`PersistedSettings`]）、单字段"override ＞
//! user-data ＞ serve 默认"的生效值合成、以及"先 GET 再改自己管理的键再全量 PUT"
//! 所需的 DTO 合并函数。UI 状态机见 [`super::settings_panel`]。

use crabmate_tui_core::{LlmOverridesDto, UserPrefsDto};

/// 会话模式枚举的合法取值（与 `/mode` 斜杠一致；面板枚举与校验共用，避免漂移）。
pub const SESSION_MODES: [&str; 3] = ["ask", "plan", "act"];

/// 一个键的保存动作（W1 只管理 4 个键：model / api_base / cm_role / session_mode）。
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

/// `/user-data/llm-overrides` 的 `client_llm.{model,api_base}` 保存动作。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LlmSave {
    pub model: FieldAction,
    pub api_base: FieldAction,
}

impl LlmSave {
    /// 两组字段是否有任一待保存动作。
    #[must_use]
    pub fn any(&self) -> bool {
        self.model.is_write() || self.api_base.is_write()
    }

    /// 清空全部动作（保存成功落地后调用）。
    pub fn clear(&mut self) {
        self.model = FieldAction::Skip;
        self.api_base = FieldAction::Skip;
    }
}

/// `/user-data/prefs` 的 `cm_role / session_mode` 保存动作。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrefsSave {
    pub role: FieldAction,
    pub session_mode: FieldAction,
}

impl PrefsSave {
    /// 两组字段是否有任一待保存动作。
    #[must_use]
    pub fn any(&self) -> bool {
        self.role.is_write() || self.session_mode.is_write()
    }

    /// 清空全部动作（保存成功落地后调用）。
    pub fn clear(&mut self) {
        self.role = FieldAction::Skip;
        self.session_mode = FieldAction::Skip;
    }
}

/// user-data 持久层快照（启动拉取 / 保存成功后更新）。
///
/// 连同一 serve 时与 Desktop/Web 共享：`client_llm.{model,api_base}` +
/// prefs 的 `cm_role/session_mode`。空白值一律归一为 `None`（= 未设置）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PersistedSettings {
    pub model: Option<String>,
    pub api_base: Option<String>,
    pub role: Option<String>,
    pub session_mode: Option<String>,
}

impl PersistedSettings {
    /// 从 user-data 快照（prefs + llm-overrides）合成持久层；空白视为未设置，
    /// 非法会话模式值（旧文件遗留）丢弃并回落 serve 默认。
    #[must_use]
    pub fn from_snapshot(prefs: &UserPrefsDto, llm: &LlmOverridesDto) -> Self {
        Self {
            model: normalize(&llm.client_llm.model),
            api_base: normalize(&llm.client_llm.api_base),
            role: normalize(&prefs.cm_role),
            session_mode: normalize(&prefs.session_mode)
                .filter(|m| is_valid_session_mode(m.as_str())),
        }
    }

    /// llm 侧保存成功后在内存做同样更新（未编辑的键不动）。
    pub fn apply_llm_saved(&mut self, save: &LlmSave) {
        apply_slot(&mut self.model, &save.model);
        apply_slot(&mut self.api_base, &save.api_base);
    }

    /// prefs 侧保存成功后在内存做同样更新（未编辑的键不动）。
    pub fn apply_prefs_saved(&mut self, save: &PrefsSave) {
        apply_slot(&mut self.role, &save.role);
        apply_slot(&mut self.session_mode, &save.session_mode);
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

/// 合并写回 `/user-data/llm-overrides`：只改 `client_llm.{model,api_base}`，
/// `executor_llm / saved_models / execution_mode` 原样保留（合并保真）。
#[must_use]
pub fn merge_llm_save(mut base: LlmOverridesDto, save: &LlmSave) -> LlmOverridesDto {
    apply_slot(&mut base.client_llm.model, &save.model);
    apply_slot(&mut base.client_llm.api_base, &save.api_base);
    base
}

/// 合并写回 `/user-data/prefs`：只改 `cm_role / session_mode`，
/// `locale / theme / 布局 / IDE` 等字段原样保留（合并保真）。
#[must_use]
pub fn merge_prefs_save(mut base: UserPrefsDto, save: &PrefsSave) -> UserPrefsDto {
    apply_slot(&mut base.cm_role, &save.role);
    apply_slot(&mut base.session_mode, &save.session_mode);
    base
}

#[cfg(test)]
mod tests {
    use super::*;
    use crabmate_tui_core::LlmEndpointOverrideDto;
    use serde_json::json;

    #[test]
    fn normalize_trims_and_drops_blank() {
        assert_eq!(normalize(&Some("  x ".into())), Some("x".to_string()));
        assert_eq!(normalize(&Some("   ".into())), None);
        assert_eq!(normalize(&None), None);
    }

    #[test]
    fn effective_prefers_override_then_stored_then_default() {
        let local = Some("local".to_string());
        let stored = Some("stored".to_string());
        let remote = Some("remote".to_string());
        let v = effective_value(local.as_deref(), stored.as_deref(), remote.as_deref());
        assert_eq!(
            (v.layer, v.value.as_deref()),
            (Layer::Override, Some("local"))
        );
        let v = effective_value(None, stored.as_deref(), remote.as_deref());
        assert_eq!(
            (v.layer, v.value.as_deref()),
            (Layer::Stored, Some("stored"))
        );
        let v = effective_value(None, None, remote.as_deref());
        assert_eq!(
            (v.layer, v.value.as_deref()),
            (Layer::Default, Some("remote"))
        );
        let v = effective_value(None, None, None);
        assert_eq!((v.layer, v.value), (Layer::Follow, None));
    }

    #[test]
    fn effective_skips_blank_override_and_stored() {
        let local = Some("  ".to_string());
        let stored = Some("stored".to_string());
        let v = effective_value(local.as_deref(), stored.as_deref(), None);
        assert_eq!(
            (v.layer, v.value.as_deref()),
            (Layer::Stored, Some("stored"))
        );
        let stored = Some("  ".to_string());
        let remote = Some("d".to_string());
        let v = effective_value(None, stored.as_deref(), remote.as_deref());
        assert_eq!((v.layer, v.value.as_deref()), (Layer::Default, Some("d")));
    }

    #[test]
    fn api_base_validation_accepts_empty_and_http_schemes() {
        assert!(validate_api_base(""));
        assert!(validate_api_base("   "));
        assert!(validate_api_base("http://127.0.0.1:8080"));
        assert!(validate_api_base("https://api.example.com/v1"));
        assert!(validate_api_base("HTTPS://example.com"), "前缀大小写不敏感");
        assert!(!validate_api_base("ftp://x"));
        assert!(!validate_api_base("example.com/v1"), "缺少 http(s):// 前缀");
        assert!(!validate_api_base("localhost:8080"));
    }

    #[test]
    fn session_mode_validation_matches_slash_modes() {
        assert!(is_valid_session_mode("ask"));
        assert!(is_valid_session_mode("plan"));
        assert!(is_valid_session_mode("act"));
        assert!(!is_valid_session_mode("bogus"));
        assert!(!is_valid_session_mode(""));
    }

    #[test]
    fn merge_turn_prefers_override_and_drops_blank() {
        assert_eq!(
            merge_turn(&Some("local".into()), Some("stored")),
            Some("local".to_string())
        );
        assert_eq!(
            merge_turn(&None, Some("stored")),
            Some("stored".to_string())
        );
        assert_eq!(merge_turn(&None, None), None);
        assert_eq!(
            merge_turn(&Some("  ".into()), None),
            None,
            "空白 override 不发送"
        );
    }

    fn llm_base() -> LlmOverridesDto {
        LlmOverridesDto {
            client_llm: LlmEndpointOverrideDto {
                model: Some("old".into()),
                api_base: Some("http://old/v1".into()),
                temperature: Some("0.7".into()),
                llm_context_tokens: Some("8000".into()),
                ..Default::default()
            },
            executor_llm: LlmEndpointOverrideDto {
                model: Some("exec".into()),
                ..Default::default()
            },
            execution_mode: Some("autonomous".into()),
            saved_models: vec![json!({"label": "mine"})],
        }
    }

    #[test]
    fn merge_llm_save_rewrites_only_managed_keys() {
        let save = LlmSave {
            model: FieldAction::Write(Some("deepseek-chat".into())),
            api_base: FieldAction::Write(Some("https://x.example/v1".into())),
        };
        let out = merge_llm_save(llm_base(), &save);
        assert_eq!(out.client_llm.model.as_deref(), Some("deepseek-chat"));
        assert_eq!(
            out.client_llm.api_base.as_deref(),
            Some("https://x.example/v1")
        );
        // 非管理字段原样保留
        assert_eq!(out.client_llm.temperature.as_deref(), Some("0.7"));
        assert_eq!(out.client_llm.llm_context_tokens.as_deref(), Some("8000"));
        assert_eq!(out.executor_llm.model.as_deref(), Some("exec"));
        assert_eq!(out.execution_mode.as_deref(), Some("autonomous"));
        assert_eq!(out.saved_models.len(), 1);
    }

    #[test]
    fn merge_llm_save_clears_to_null_and_skips() {
        let save = LlmSave {
            model: FieldAction::Write(None),
            api_base: FieldAction::Skip,
        };
        let out = merge_llm_save(llm_base(), &save);
        assert_eq!(out.client_llm.model, None, "清除键写 null");
        assert_eq!(
            out.client_llm.api_base.as_deref(),
            Some("http://old/v1"),
            "Skip 键保留现值"
        );
    }

    #[test]
    fn merge_llm_save_normalizes_blank_write_to_clear() {
        let save = LlmSave {
            model: FieldAction::Write(Some("   ".into())),
            api_base: FieldAction::Write(Some("  deepseek  ".into())),
        };
        let out = merge_llm_save(llm_base(), &save);
        assert_eq!(out.client_llm.model, None, "空白写入等价清除");
        assert_eq!(out.client_llm.api_base.as_deref(), Some("deepseek"));
    }

    #[test]
    fn merge_prefs_save_preserves_unrelated_keys() {
        let base = UserPrefsDto {
            locale: Some("zh-CN".into()),
            theme: Some("dark".into()),
            cm_role: Some("coder".into()),
            session_mode: Some("ask".into()),
            disable_readonly_tool_ttl_cache: Some(true),
            ..Default::default()
        };
        let save = PrefsSave {
            role: FieldAction::Write(Some("architect".into())),
            session_mode: FieldAction::Write(None),
        };
        let out = merge_prefs_save(base, &save);
        assert_eq!(out.cm_role.as_deref(), Some("architect"));
        assert_eq!(out.session_mode, None, "Write(None) 清除该键");
        assert_eq!(out.locale.as_deref(), Some("zh-CN"), "locale 原样保留");
        assert_eq!(out.theme.as_deref(), Some("dark"));
        assert_eq!(out.disable_readonly_tool_ttl_cache, Some(true));
    }

    #[test]
    fn persisted_snapshot_picks_fields_and_trims_blanks() {
        let prefs = UserPrefsDto {
            cm_role: Some(" coder ".into()),
            session_mode: Some("   ".into()),
            locale: Some("zh-CN".into()),
            ..Default::default()
        };
        let llm = LlmOverridesDto {
            client_llm: LlmEndpointOverrideDto {
                model: Some("gpt-x".into()),
                api_base: Some("".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        let p = PersistedSettings::from_snapshot(&prefs, &llm);
        assert_eq!(p.role.as_deref(), Some("coder"), "cm_role 归一化后保留");
        assert_eq!(p.session_mode, None, "空白 session_mode 视为未设置");
        assert_eq!(p.model.as_deref(), Some("gpt-x"));
        assert_eq!(p.api_base, None, "空 api_base 视为未设置");
    }

    #[test]
    fn snapshot_drops_invalid_session_mode() {
        let prefs = UserPrefsDto {
            session_mode: Some("Bogus".into()),
            ..Default::default()
        };
        let p = PersistedSettings::from_snapshot(&prefs, &LlmOverridesDto::default());
        assert_eq!(p.session_mode, None, "非法 session_mode 回落 serve 默认");
    }

    #[test]
    fn persisted_apply_saved_updates_only_written_keys() {
        let mut p = PersistedSettings {
            model: Some("old".into()),
            role: Some("coder".into()),
            ..Default::default()
        };
        p.apply_llm_saved(&LlmSave {
            model: FieldAction::Write(Some("new".into())),
            api_base: FieldAction::Write(None),
        });
        assert_eq!(p.model.as_deref(), Some("new"));
        assert_eq!(p.api_base, None);
        p.apply_prefs_saved(&PrefsSave {
            role: FieldAction::Skip,
            session_mode: FieldAction::Write(Some("plan".into())),
        });
        assert_eq!(p.role.as_deref(), Some("coder"), "Skip 键不动");
        assert_eq!(p.session_mode.as_deref(), Some("plan"));
    }

    #[test]
    fn save_payload_any_and_clear() {
        let mut s = LlmSave::default();
        assert!(!s.any());
        s.model = FieldAction::Write(None);
        assert!(s.any());
        s.clear();
        assert!(!s.any());
        let mut p = PrefsSave::default();
        assert!(!p.any());
        p.session_mode = FieldAction::Write(Some("act".into()));
        assert!(p.any());
        p.clear();
        assert!(!p.any());
    }

    #[test]
    fn field_action_predicates() {
        assert!(FieldAction::Skip.is_skip());
        assert!(FieldAction::Write(Some("x".into())).is_write());
        assert!(FieldAction::Write(None).is_write());
        assert!(!FieldAction::Write(None).is_skip());
    }
}
