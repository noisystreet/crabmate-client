//! `settings.rs` 纯逻辑层的单测（`#[path]` 子模块）：压住设置逻辑文件行数门禁。

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
fn temperature_validation_matches_desktop_range() {
    assert!(is_valid_temperature("0.5"));
    assert!(is_valid_temperature(" 0.7 "), "trim 后解析");
    assert!(is_valid_temperature("0"));
    assert!(is_valid_temperature("2.0"), "区间右闭");
    assert!(is_valid_temperature(""), "空 = 未设置");
    assert!(is_valid_temperature("   "));
    assert!(!is_valid_temperature("2.1"), "超过上限");
    assert!(!is_valid_temperature("-0.1"), "低于下限");
    assert!(!is_valid_temperature("abc"));
    assert!(!is_valid_temperature("inf"));
    assert!(!is_valid_temperature("nan"));
}

#[test]
fn thinking_mode_validation_accepts_server_and_blank() {
    assert!(is_valid_thinking_mode("on"));
    assert!(is_valid_thinking_mode("off"));
    assert!(is_valid_thinking_mode("server"));
    assert!(is_valid_thinking_mode(""));
    assert!(is_valid_thinking_mode("  on  "));
    assert!(!is_valid_thinking_mode("bogus"));
    assert!(!is_valid_thinking_mode("auto"));
    assert_eq!(THINKING_MODES, ["on", "off"]);
    assert_eq!(THINKING_SERVER, "server");
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

#[test]
fn turn_context_tokens_sends_positive_numbers_only() {
    assert_eq!(turn_context_tokens(Some("8000")), Some("8000".into()));
    assert_eq!(turn_context_tokens(Some(" 8000 ")), Some("8000".into()));
    assert_eq!(turn_context_tokens(Some("0")), None, "0 不发送");
    assert_eq!(turn_context_tokens(Some("abc")), None);
    assert_eq!(turn_context_tokens(Some("   ")), None);
    assert_eq!(turn_context_tokens(None), None);
}

#[test]
fn turn_tool_cache_secs_sends_zero_only_when_disabled() {
    assert_eq!(turn_tool_cache_secs(Some(true)), Some(0));
    assert_eq!(turn_tool_cache_secs(Some(false)), None, "显式跟随不发送");
    assert_eq!(turn_tool_cache_secs(None), None, "缺省跟随不发送");
}

#[test]
fn build_turn_client_llm_assembles_only_non_blank_fields() {
    let f = build_turn_client_llm(
        Some("gpt-x"),
        Some("https://x/v1"),
        Some(" sk-1 "),
        Some("off"),
        Some(" 8000 "),
    )
    .unwrap();
    assert_eq!(f.api_key.as_deref(), Some("sk-1"), "密钥 trim");
    assert_eq!(f.model.as_deref(), Some("gpt-x"));
    assert_eq!(f.api_base.as_deref(), Some("https://x/v1"));
    assert_eq!(f.llm_thinking_mode.as_deref(), Some("off"));
    assert_eq!(
        f.llm_context_tokens.as_deref(),
        Some("8000"),
        "context >0 规范化"
    );

    // 全空 / 无效值 → 整块省略
    assert!(build_turn_client_llm(None, None, Some("   "), Some("server"), Some("0")).is_none());
    assert!(build_turn_client_llm(None, None, None, None, Some("abc")).is_none());
    assert!(build_turn_client_llm(None, None, None, None, None).is_none());
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
        ..Default::default()
    };
    let out = merge_llm_save(llm_base(), &save);
    assert_eq!(out.client_llm.model.as_deref(), Some("deepseek-chat"));
    assert_eq!(
        out.client_llm.api_base.as_deref(),
        Some("https://x.example/v1")
    );
    // 未编辑（Skip）的管理字段与非管理字段原样保留
    assert_eq!(out.client_llm.temperature.as_deref(), Some("0.7"));
    assert_eq!(out.client_llm.llm_thinking_mode, None);
    assert_eq!(out.client_llm.llm_context_tokens.as_deref(), Some("8000"));
    assert_eq!(out.executor_llm.model.as_deref(), Some("exec"));
    assert_eq!(out.execution_mode.as_deref(), Some("autonomous"));
    assert_eq!(out.saved_models.len(), 1);
}

#[test]
fn merge_llm_save_writes_and_clears_temperature_and_thinking() {
    let save = LlmSave {
        temperature: FieldAction::Write(Some(" 1.25 ".into())),
        thinking: FieldAction::Write(Some("off".into())),
        ..Default::default()
    };
    let out = merge_llm_save(llm_base(), &save);
    assert_eq!(out.client_llm.temperature.as_deref(), Some("1.25"));
    assert_eq!(out.client_llm.llm_thinking_mode.as_deref(), Some("off"));
    assert_eq!(out.client_llm.model.as_deref(), Some("old"), "Skip 键不动");

    let save = LlmSave {
        temperature: FieldAction::Write(None),
        thinking: FieldAction::Write(Some("   ".into())),
        ..Default::default()
    };
    let out = merge_llm_save(out, &save);
    assert_eq!(out.client_llm.temperature, None, "Write(None) 清除温度");
    assert_eq!(
        out.client_llm.llm_thinking_mode, None,
        "空白 thinking 写入等价清除"
    );
}

#[test]
fn merge_llm_save_clears_to_null_and_skips() {
    let save = LlmSave {
        model: FieldAction::Write(None),
        api_base: FieldAction::Skip,
        ..Default::default()
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
        ..Default::default()
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
        tool_cache: FieldAction::Skip,
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
            temperature: Some(" 0.7 ".into()),
            llm_thinking_mode: Some("off".into()),
            ..Default::default()
        },
        ..Default::default()
    };
    let p = PersistedSettings::from_snapshot(&prefs, &llm);
    assert_eq!(p.role.as_deref(), Some("coder"), "cm_role 归一化后保留");
    assert_eq!(p.session_mode, None, "空白 session_mode 视为未设置");
    assert_eq!(p.model.as_deref(), Some("gpt-x"));
    assert_eq!(p.api_base, None, "空 api_base 视为未设置");
    assert_eq!(p.temperature.as_deref(), Some("0.7"), "温度 trim 后存原文");
    assert_eq!(p.thinking.as_deref(), Some("off"));
}

#[test]
fn snapshot_drops_invalid_temperature_and_thinking() {
    let llm = LlmOverridesDto {
        client_llm: LlmEndpointOverrideDto {
            temperature: Some("9.9".into()),
            llm_thinking_mode: Some("server".into()),
            ..Default::default()
        },
        ..Default::default()
    };
    let p = PersistedSettings::from_snapshot(&UserPrefsDto::default(), &llm);
    assert_eq!(p.temperature, None, "超区间温度回落 serve 默认");
    assert_eq!(p.thinking, None, "server/空思考模式 = 跟随 server");

    let llm = LlmOverridesDto {
        client_llm: LlmEndpointOverrideDto {
            temperature: Some("   ".into()),
            ..Default::default()
        },
        ..Default::default()
    };
    let p = PersistedSettings::from_snapshot(&UserPrefsDto::default(), &llm);
    assert_eq!(p.temperature, None, "空白温度视为未设置");
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
        temperature: FieldAction::Write(Some("1.1".into())),
        thinking: FieldAction::Write(None),
        context_tokens: FieldAction::Skip,
    });
    assert_eq!(p.model.as_deref(), Some("new"));
    assert_eq!(p.api_base, None);
    assert_eq!(p.temperature.as_deref(), Some("1.1"));
    assert_eq!(p.thinking, None);
    p.apply_prefs_saved(&PrefsSave {
        role: FieldAction::Skip,
        session_mode: FieldAction::Write(Some("plan".into())),
        tool_cache: FieldAction::Skip,
    });
    assert_eq!(p.role.as_deref(), Some("coder"), "Skip 键不动");
    assert_eq!(p.session_mode.as_deref(), Some("plan"));
}

#[test]
fn save_payload_any_and_clear() {
    let mut s = LlmSave::default();
    assert!(!s.any());
    s.temperature = FieldAction::Write(Some("1.0".into()));
    assert!(s.any());
    s.clear();
    assert!(!s.any());
    s.thinking = FieldAction::Write(None);
    assert!(s.any());
    s.clear();
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

#[test]
fn context_tokens_validation_matches_desktop_bound() {
    assert!(is_valid_context_tokens(""));
    assert!(is_valid_context_tokens("   "));
    assert!(is_valid_context_tokens("8000"));
    assert!(is_valid_context_tokens(" 8000 "), "trim 后解析");
    assert!(
        is_valid_context_tokens("0"),
        "0 可存（Desktop 同区间，随轮不发送）"
    );
    assert!(is_valid_context_tokens("10000000"), "上限闭区间");
    assert!(!is_valid_context_tokens("10000001"), "超过上限");
    assert!(!is_valid_context_tokens("-1"));
    assert!(!is_valid_context_tokens("1.5"), "仅整数");
    assert!(!is_valid_context_tokens("abc"));
}

#[test]
fn merge_llm_save_writes_and_clears_context_tokens() {
    let save = LlmSave {
        context_tokens: FieldAction::Write(Some(" 32000 ".into())),
        ..Default::default()
    };
    let out = merge_llm_save(llm_base(), &save);
    assert_eq!(out.client_llm.llm_context_tokens.as_deref(), Some("32000"));
    assert_eq!(out.client_llm.model.as_deref(), Some("old"), "Skip 键不动");

    let save = LlmSave {
        context_tokens: FieldAction::Write(None),
        ..Default::default()
    };
    let out = merge_llm_save(out, &save);
    assert_eq!(out.client_llm.llm_context_tokens, None, "Write(None) 清除");
}

#[test]
fn merge_prefs_save_writes_and_clears_tool_cache() {
    let base = UserPrefsDto::default();
    let save = PrefsSave {
        tool_cache: FieldAction::Write(Some(" off ".into())),
        ..Default::default()
    };
    let out = merge_prefs_save(base, &save);
    assert_eq!(
        out.disable_readonly_tool_ttl_cache,
        Some(true),
        "off → disable=true"
    );

    // 跟随 server：Write(None) 与空白写入都等价清除键。
    for clear in [
        FieldAction::Write(None),
        FieldAction::Write(Some("   ".into())),
    ] {
        let save = PrefsSave {
            tool_cache: clear,
            ..Default::default()
        };
        let out = merge_prefs_save(UserPrefsDto::default(), &save);
        assert_eq!(out.disable_readonly_tool_ttl_cache, None);
    }
}

#[test]
fn snapshot_picks_context_tokens_and_tool_cache() {
    let prefs = UserPrefsDto {
        disable_readonly_tool_ttl_cache: Some(true),
        ..Default::default()
    };
    let llm = LlmOverridesDto {
        client_llm: LlmEndpointOverrideDto {
            llm_context_tokens: Some(" 8000 ".into()),
            ..Default::default()
        },
        ..Default::default()
    };
    let p = PersistedSettings::from_snapshot(&prefs, &llm);
    assert_eq!(p.context_tokens.as_deref(), Some("8000"), "trim 后存原文");
    assert_eq!(p.tool_cache_disabled, Some(true), "镜像 disable 标记");

    // 非法上下文值丢弃回落 serve 默认；Some(false)/None = 跟随 server（非禁用）。
    let llm = LlmOverridesDto {
        client_llm: LlmEndpointOverrideDto {
            llm_context_tokens: Some("20000000".into()),
            ..Default::default()
        },
        ..Default::default()
    };
    let prefs = UserPrefsDto {
        disable_readonly_tool_ttl_cache: Some(false),
        ..Default::default()
    };
    let p = PersistedSettings::from_snapshot(&prefs, &llm);
    assert_eq!(p.context_tokens, None, "超上限丢弃");
    assert_eq!(p.tool_cache_disabled, Some(false), "显式跟随非禁用");
}

#[test]
fn persisted_apply_saved_tool_cache_and_context() {
    let mut p = PersistedSettings {
        tool_cache_disabled: Some(true),
        ..Default::default()
    };
    p.apply_prefs_saved(&PrefsSave {
        tool_cache: FieldAction::Write(None),
        ..Default::default()
    });
    assert_eq!(p.tool_cache_disabled, None, "清除键 = 跟随 server");

    p.apply_llm_saved(&LlmSave {
        context_tokens: FieldAction::Write(Some("16000".into())),
        ..Default::default()
    });
    assert_eq!(p.context_tokens.as_deref(), Some("16000"));
}

#[test]
fn save_payload_any_ignores_skip_new_fields() {
    let s = LlmSave {
        context_tokens: FieldAction::Skip,
        ..Default::default()
    };
    assert!(!s.any());
    let p = PrefsSave {
        tool_cache: FieldAction::Skip,
        ..Default::default()
    };
    assert!(!p.any());
}
