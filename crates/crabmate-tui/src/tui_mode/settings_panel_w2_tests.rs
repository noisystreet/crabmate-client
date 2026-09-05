//! 设置面板 W2 新增字段（温度 / 思考模式 / API 密钥）的单测。
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

/// 当前是否在 API 密钥文本编辑态且缓冲为空。
fn buf_is_empty(p: &SettingsPanel) -> bool {
    matches!(
        &p.editing,
        Some(Editing::Text { field: FieldId::ApiKey, buf, .. }) if buf.is_empty()
    )
}

#[test]
fn temperature_invalid_commit_keeps_editing_with_note() {
    let o = overrides(None, None, None, None);
    let c = ctx(&o, None);
    let mut p = SettingsPanel::new(false);
    for _ in 0..2 {
        p.handle_key(&key(KeyCode::Down), &c); // 温度行
    }
    assert_eq!(p.current_field(), FieldId::Temperature);
    p.handle_key(&key(KeyCode::Enter), &c);
    assert!(matches!(
        &p.editing,
        Some(Editing::Text {
            field: FieldId::Temperature,
            ..
        })
    ));
    for ch in "2.5".chars() {
        p.handle_key(&key(KeyCode::Char(ch)), &c);
    }
    p.handle_key(&key(KeyCode::Enter), &c);
    assert!(p.editing.is_some(), "越界温度非法提交留在编辑态");
    assert!(p.note.is_some());
    assert!(!p.is_dirty());
    // 清空后输入合法温度并提交
    for _ in 0..3 {
        p.handle_key(&key(KeyCode::Backspace), &c);
    }
    for ch in "1.5".chars() {
        p.handle_key(&key(KeyCode::Char(ch)), &c);
    }
    p.handle_key(&key(KeyCode::Enter), &c);
    assert!(p.editing.is_none());
    assert!(matches!(
        p.staged(FieldId::Temperature),
        FieldAction::Write(Some(v)) if v == "1.5"
    ));
    // 空提交 = 清除（Write(None)）：重新进入会预填 staged 值，需先清空缓冲。
    p.handle_key(&key(KeyCode::Enter), &c);
    assert!(matches!(
        &p.editing,
        Some(Editing::Text { field: FieldId::Temperature, buf, .. })
            if buf.iter().collect::<String>() == "1.5"
    ));
    for _ in 0..3 {
        p.handle_key(&key(KeyCode::Backspace), &c);
    }
    p.handle_key(&key(KeyCode::Enter), &c);
    assert_eq!(p.staged(FieldId::Temperature), &FieldAction::Write(None));
}

#[test]
fn thinking_enum_cycles_and_commits_on_off_clear() {
    let o = overrides(None, None, None, None);
    let c = ctx(&o, None);
    let mut p = SettingsPanel::new(false);
    for _ in 0..3 {
        p.handle_key(&key(KeyCode::Down), &c); // 思考模式行
    }
    assert_eq!(p.current_field(), FieldId::ThinkingMode);
    p.handle_key(&key(KeyCode::Enter), &c);
    assert!(matches!(&p.editing, Some(Editing::Think { pick: 0 })));
    // → 到 on 提交
    p.handle_key(&key(KeyCode::Right), &c);
    assert!(matches!(&p.editing, Some(Editing::Think { pick: 1 })));
    p.handle_key(&key(KeyCode::Enter), &c);
    assert!(matches!(
        p.staged(FieldId::ThinkingMode),
        FieldAction::Write(Some(v)) if v == "on"
    ));
    // 重新进入：staged on → pick1；→ 到 off 提交
    p.handle_key(&key(KeyCode::Enter), &c);
    assert!(matches!(&p.editing, Some(Editing::Think { pick: 1 })));
    p.handle_key(&key(KeyCode::Right), &c);
    p.handle_key(&key(KeyCode::Enter), &c);
    assert!(matches!(
        p.staged(FieldId::ThinkingMode),
        FieldAction::Write(Some(v)) if v == "off"
    ));
    // ←←← 回到 0（跟随 server = 清除）提交
    p.handle_key(&key(KeyCode::Enter), &c);
    assert!(matches!(&p.editing, Some(Editing::Think { pick: 2 })));
    p.handle_key(&key(KeyCode::Left), &c);
    p.handle_key(&key(KeyCode::Left), &c);
    p.handle_key(&key(KeyCode::Left), &c);
    assert!(matches!(&p.editing, Some(Editing::Think { pick: 0 })));
    p.handle_key(&key(KeyCode::Enter), &c);
    assert_eq!(p.staged(FieldId::ThinkingMode), &FieldAction::Write(None));
    assert!(p.is_dirty());
}

#[test]
fn api_key_staging_and_save_effect_carries_secret() {
    let o = overrides(None, None, None, None);
    let c = ctx(&o, None);
    let mut p = SettingsPanel::new(false);
    for _ in 0..4 {
        p.handle_key(&key(KeyCode::Down), &c); // API 密钥行
    }
    assert_eq!(p.current_field(), FieldId::ApiKey);
    p.handle_key(&key(KeyCode::Enter), &c);
    let (field, buf) = match &p.editing {
        Some(Editing::Text { field, buf, .. }) => (*field, buf),
        _ => panic!("expected api key text editing"),
    };
    assert_eq!(field, FieldId::ApiKey);
    assert!(buf.is_empty(), "密钥编辑从空开始，不回显已设值");
    for ch in "sk-12345".chars() {
        p.handle_key(&key(KeyCode::Char(ch)), &c);
    }
    p.handle_key(&key(KeyCode::Enter), &c);
    assert!(matches!(
        p.staged(FieldId::ApiKey),
        FieldAction::Write(Some(v)) if v == "sk-12345"
    ));
    assert!(p.is_dirty());
    // S → Save 效果带 secret（llm/prefs 均空）
    let (llm, prefs, secret) = match p.handle_key(&key(KeyCode::Char('s')), &c) {
        PanelEffect::Save { llm, prefs, secret } => (llm, prefs, secret),
        _ => panic!("expected save"),
    };
    assert!(!llm.any());
    assert!(!prefs.any());
    assert!(matches!(secret, FieldAction::Write(Some(v)) if v == "sk-12345"));
    // staged 在 Save 后保留（等待调用方落地回调）
    assert!(p.is_dirty());
    // 空提交 = 清除
    p.handle_key(&key(KeyCode::Enter), &c);
    assert!(buf_is_empty(&p));
    p.handle_key(&key(KeyCode::Enter), &c);
    assert_eq!(p.staged(FieldId::ApiKey), &FieldAction::Write(None));
}

#[test]
fn secret_set_display_text_and_secret_saved_callback() {
    let o = overrides(None, None, None, None);
    let c = ctx(&o, None);
    // 未设：灰字"未设（跟随 serve）"
    let mut p = SettingsPanel::new(false);
    p.set_secret_set(false);
    let text = lines_text(&p.content(&c, 90));
    assert!(
        text.iter().any(|l| l.contains("未设（跟随 serve）")),
        "secret_set=false 显示未设"
    );
    // 已设：白字"已设（钥匙串）"
    p.set_secret_set(true);
    let text = lines_text(&p.content(&c, 90));
    assert!(text.iter().any(|l| l.contains("已设（钥匙串）")));
    assert!(!text.iter().any(|l| l.contains("未设（跟随 serve）")));
    // secret_saved(true)：清 staged 且已设
    p.handle_key(&key(KeyCode::Down), &c);
    p.handle_key(&key(KeyCode::Down), &c);
    p.handle_key(&key(KeyCode::Down), &c);
    p.handle_key(&key(KeyCode::Down), &c);
    p.handle_key(&key(KeyCode::Enter), &c);
    for ch in "k2".chars() {
        p.handle_key(&key(KeyCode::Char(ch)), &c);
    }
    p.handle_key(&key(KeyCode::Enter), &c);
    assert!(p.is_dirty());
    p.secret_saved(true);
    assert!(!p.is_dirty(), "落地后清 staged");
    assert!(p.staged(FieldId::ApiKey).is_skip());
    let text = lines_text(&p.content(&c, 90));
    assert!(text.iter().any(|l| l.contains("已设（钥匙串）")));
}

#[test]
fn llm_fields_content_shows_temperature_and_thinking_rows() {
    let stored = PersistedSettings {
        temperature: Some("0.9".into()),
        thinking: Some("off".into()),
        ..Default::default()
    };
    let o = overrides(None, None, None, None);
    let p = SettingsPanel::new(false);
    let text = lines_text(&p.content(&ctx(&o, Some(&stored)), 90));
    assert!(text.iter().any(|l| l.contains("温度") && l.contains("0.9")));
    assert!(
        text.iter()
            .any(|l| l.contains("思考模式") && l.contains("off"))
    );
    assert!(text.iter().any(|l| l.contains("API 密钥")));
}

#[test]
fn save_group_result_clears_stage_on_ok_and_keeps_on_error() {
    let o = overrides(None, None, None, None);
    let c = ctx(&o, None);
    let mut p = SettingsPanel::new(false);
    p.handle_key(&key(KeyCode::Enter), &c);
    for ch in "gpt-5".chars() {
        p.handle_key(&key(KeyCode::Char(ch)), &c);
    }
    p.handle_key(&key(KeyCode::Enter), &c);
    assert!(p.is_dirty());
    let (llm, prefs) = match p.handle_key(&key(KeyCode::Char('s')), &c) {
        PanelEffect::Save { llm, prefs, .. } => (llm, prefs),
        _ => panic!("expected save"),
    };
    assert!(p.is_saving());
    assert!(llm.any(), "model staged → llm 组有动作");
    assert!(!prefs.any());
    // 失败：保留 staged 与脏标记
    p.save_group_result(SaveGroup::Llm, false);
    assert!(!p.is_saving());
    assert!(p.is_dirty());
    // 重试成功：staged 清空
    assert!(matches!(
        p.handle_key(&key(KeyCode::Char('S')), &c),
        PanelEffect::Save { .. }
    ));
    assert!(p.save_group_result(SaveGroup::Llm, true));
    assert!(!p.is_dirty());
}
