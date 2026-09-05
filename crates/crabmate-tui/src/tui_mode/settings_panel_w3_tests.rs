//! 设置面板 W3（W2 收尾：上下文 tokens / 只读工具缓存开关 / 网关预设）的单测。
//!
//! 独立成文件（`settings_panel.rs` 的 `#[path]` 子模块）：除行数门禁外，也避免
//! 与旧字段单测混在同一文件时把 lizard 的函数体解析合并得过大。

use super::*;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn overrides(
    model: Option<&str>,
    api_base: Option<&str>,
    role: Option<&str>,
    mode: Option<&str>,
) -> SessionPrefs {
    let mut o = SessionPrefs::from_options(
        None,
        model.map(str::to_string),
        api_base.map(str::to_string),
    );
    o.agent_role = role.map(str::to_string);
    o.session_mode = mode.map(str::to_string);
    o
}

fn ctx<'a>(o: &'a SessionPrefs, persisted: Option<&'a PersistedSettings>) -> PanelCtx<'a> {
    PanelCtx {
        overrides: o,
        persisted,
        serve_defaults: None,
    }
}

fn lines_text(content: &PanelContent) -> Vec<String> {
    content
        .lines
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
        })
        .collect()
}

#[test]
fn context_tokens_invalid_commit_keeps_editing_with_note() {
    let o = overrides(None, None, None, None);
    let c = ctx(&o, None);
    let mut p = SettingsPanel::new(false);
    for _ in 0..3 {
        p.handle_key(&key(KeyCode::Down), &c); // 上下文 tokens 行
    }
    assert_eq!(p.current_field(), FieldId::ContextTokens);
    p.handle_key(&key(KeyCode::Enter), &c);
    assert!(matches!(
        &p.editing,
        Some(Editing::Text {
            field: FieldId::ContextTokens,
            ..
        })
    ));
    for ch in "10000001".chars() {
        p.handle_key(&key(KeyCode::Char(ch)), &c);
    }
    p.handle_key(&key(KeyCode::Enter), &c);
    assert!(p.editing.is_some(), "越界提交留在编辑态");
    assert!(p.note.is_some());
    assert!(!p.is_dirty());
    for _ in 0..8 {
        p.handle_key(&key(KeyCode::Backspace), &c);
    }
    for ch in "8000".chars() {
        p.handle_key(&key(KeyCode::Char(ch)), &c);
    }
    p.handle_key(&key(KeyCode::Enter), &c);
    assert!(p.editing.is_none());
    assert!(matches!(
        p.staged(FieldId::ContextTokens),
        FieldAction::Write(Some(v)) if v == "8000"
    ));
    // 空提交 = 清除（Write(None)）
    p.handle_key(&key(KeyCode::Enter), &c);
    for _ in 0..4 {
        p.handle_key(&key(KeyCode::Backspace), &c);
    }
    p.handle_key(&key(KeyCode::Enter), &c);
    assert_eq!(p.staged(FieldId::ContextTokens), &FieldAction::Write(None));
}

#[test]
fn context_tokens_row_shows_stored_value() {
    let stored = PersistedSettings {
        context_tokens: Some("32000".into()),
        ..Default::default()
    };
    let o = overrides(None, None, None, None);
    let p = SettingsPanel::new(false);
    let text = lines_text(&p.content(&ctx(&o, Some(&stored)), 90));
    assert!(
        text.iter()
            .any(|l| l.contains("上下文 tokens") && l.contains("32000"))
    );
}

#[test]
fn gateway_preset_commit_writes_url_and_back_to_server_clears() {
    let o = overrides(None, None, None, None);
    let c = ctx(&o, None);
    let mut p = SettingsPanel::new(false);
    p.handle_key(&key(KeyCode::Down), &c); // API Base 行
    p.handle_key(&key(KeyCode::Enter), &c);
    assert!(matches!(&p.editing, Some(Editing::Gateway { pick: 0, .. })));
    // → → 到 deepseek（0=server 1=ollama 2=deepseek）提交
    p.handle_key(&key(KeyCode::Right), &c);
    p.handle_key(&key(KeyCode::Right), &c);
    assert!(matches!(&p.editing, Some(Editing::Gateway { pick: 2, .. })));
    p.handle_key(&key(KeyCode::Enter), &c);
    assert!(matches!(
        p.staged(FieldId::ApiBase),
        FieldAction::Write(Some(v)) if v == "https://api.deepseek.com/v1"
    ));
    // 行值显示为预设 id + staged 标记
    let text = lines_text(&p.content(&c, 90));
    assert!(text.iter().any(|l| l.contains("deepseek~")));
    // 重进（staged deepseek → pick2）←← 回 server 提交 = 清除
    p.handle_key(&key(KeyCode::Enter), &c);
    assert!(matches!(&p.editing, Some(Editing::Gateway { pick: 2, .. })));
    p.handle_key(&key(KeyCode::Left), &c);
    p.handle_key(&key(KeyCode::Left), &c);
    p.handle_key(&key(KeyCode::Enter), &c);
    assert_eq!(p.staged(FieldId::ApiBase), &FieldAction::Write(None));
}

#[test]
fn gateway_preset_cycle_matches_stored_url_pick() {
    // 存的是 ollama 预设 URL：重进循环应停在 ollama（pick1）。
    let stored = PersistedSettings {
        api_base: Some("http://127.0.0.1:11434/v1".into()),
        ..Default::default()
    };
    let o = overrides(None, None, None, None);
    let c = ctx(&o, Some(&stored));
    let mut p = SettingsPanel::new(false);
    p.handle_key(&key(KeyCode::Down), &c);
    p.handle_key(&key(KeyCode::Enter), &c);
    assert!(matches!(&p.editing, Some(Editing::Gateway { pick: 1, .. })));
}

#[test]
fn tool_cache_toggle_off_and_back_to_follow() {
    let o = overrides(None, None, None, None);
    let c = ctx(&o, None);
    let mut p = SettingsPanel::new(false);
    p.handle_key(&key(KeyCode::Tab), &c); // 会话分区
    p.handle_key(&key(KeyCode::Down), &c); // 会话模式
    p.handle_key(&key(KeyCode::Down), &c); // 只读工具缓存
    assert_eq!(p.current_field(), FieldId::ToolCache);
    p.handle_key(&key(KeyCode::Enter), &c);
    assert!(matches!(&p.editing, Some(Editing::Tool { pick: 0 })));
    p.handle_key(&key(KeyCode::Right), &c);
    assert!(matches!(&p.editing, Some(Editing::Tool { pick: 1 })));
    p.handle_key(&key(KeyCode::Enter), &c);
    assert!(matches!(
        p.staged(FieldId::ToolCache),
        FieldAction::Write(Some(v)) if v == TOOL_CACHE_DISABLED
    ));
    // Save 效果：prefs 组携带 tool_cache 动作
    let (llm, prefs) = match p.handle_key(&key(KeyCode::Char('s')), &c) {
        PanelEffect::Save { llm, prefs, .. } => (*llm, *prefs),
        _ => panic!("expected save"),
    };
    assert!(!llm.any());
    assert!(prefs.tool_cache.is_write());
    // 行值显示 staged
    let text = lines_text(&p.content(&c, 90));
    assert!(text.iter().any(|l| l.contains("关（禁用缓存）~")));
    // 落地回调：清 staged（保存中禁止编辑，结果到达后才解锁）
    assert!(p.is_saving());
    p.save_group_result(SaveGroup::Prefs, true);
    assert!(!p.is_saving());
    assert!(!p.is_dirty());
    // 未设存储时重进循环从 0（开/跟随 server）开始；再按 ←→ 到 off 提交可保留脏标记
    p.handle_key(&key(KeyCode::Enter), &c);
    assert!(matches!(&p.editing, Some(Editing::Tool { pick: 0 })));
    p.handle_key(&key(KeyCode::Right), &c);
    p.handle_key(&key(KeyCode::Enter), &c);
    assert!(matches!(
        p.staged(FieldId::ToolCache),
        FieldAction::Write(Some(v)) if v == TOOL_CACHE_DISABLED
    ));
    // 显式回到开/跟随 = 清除键（Write(None)）
    p.handle_key(&key(KeyCode::Enter), &c);
    assert!(matches!(&p.editing, Some(Editing::Tool { pick: 1 })));
    p.handle_key(&key(KeyCode::Left), &c);
    p.handle_key(&key(KeyCode::Enter), &c);
    assert_eq!(p.staged(FieldId::ToolCache), &FieldAction::Write(None));
}

#[test]
fn tool_cache_row_shows_state_when_stored() {
    let o = overrides(None, None, None, None);
    // disable=true → 会话分区显示「关（禁用缓存）」
    let stored_off = PersistedSettings {
        tool_cache_disabled: Some(true),
        ..Default::default()
    };
    let c_off = ctx(&o, Some(&stored_off));
    let mut p = SettingsPanel::new(false);
    p.handle_key(&key(KeyCode::Tab), &c_off);
    let text = lines_text(&p.content(&c_off, 90));
    assert!(
        text.iter()
            .any(|l| l.contains("只读工具缓存") && l.contains("关（禁用缓存）"))
    );
    // 显式跟随（Some(false)）与缺省（None）都显示「开（跟随 server）」
    let stored_follow = PersistedSettings {
        tool_cache_disabled: Some(false),
        ..Default::default()
    };
    for persisted in [None, Some(&stored_follow)] {
        let c = ctx(&o, persisted);
        let mut p = SettingsPanel::new(false);
        p.handle_key(&key(KeyCode::Tab), &c);
        let text = lines_text(&p.content(&c, 90));
        assert!(
            text.iter()
                .any(|l| l.contains("只读工具缓存") && l.contains("开（跟随 server）"))
        );
        assert!(!text.iter().any(|l| l.contains("关（禁用缓存）")));
    }
}
