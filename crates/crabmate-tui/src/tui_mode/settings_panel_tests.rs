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
fn clean_panel_esc_and_f2_close_immediately() {
    let o = overrides(None, None, None, None);
    let c = ctx(&o, None);
    let mut p = SettingsPanel::new(false);
    assert!(matches!(
        p.handle_key(&key(KeyCode::Esc), &c),
        PanelEffect::Close
    ));
    let mut p = SettingsPanel::new(false);
    assert!(matches!(
        p.handle_key(&key(KeyCode::F(2)), &c),
        PanelEffect::Close
    ));
}

#[test]
fn f2_detector() {
    let k = KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE);
    assert!(is_f2_key(&k));
    let k1 = KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE);
    assert!(!is_f2_key(&k1));
    let kc = KeyEvent::new(KeyCode::F(2), KeyModifiers::CONTROL);
    assert!(!is_f2_key(&kc));
}

#[test]
fn dirty_esc_confirms_and_y_closes_or_esc_returns() {
    let o = overrides(None, None, None, None);
    let c = ctx(&o, None);
    let mut p = SettingsPanel::new(false);
    // 编辑 model 为 gpt-5 产生 staged
    p.handle_key(&key(KeyCode::Enter), &c);
    for ch in "gpt-5".chars() {
        p.handle_key(&key(KeyCode::Char(ch)), &c);
    }
    p.handle_key(&key(KeyCode::Enter), &c);
    assert!(p.is_dirty());
    // Esc → 弹确认而非直接关闭
    assert!(matches!(
        p.handle_key(&key(KeyCode::Esc), &c),
        PanelEffect::None
    ));
    assert!(p.confirm_close);
    // Esc 取消确认，再按 y 关闭
    p.handle_key(&key(KeyCode::Esc), &c);
    assert!(!p.confirm_close);
    p.handle_key(&key(KeyCode::Esc), &c);
    assert!(p.confirm_close);
    assert!(matches!(
        p.handle_key(&key(KeyCode::Char('y')), &c),
        PanelEffect::Close
    ));
}

#[test]
fn tab_cycles_section_and_arrows_move_row() {
    let o = overrides(None, None, None, None);
    let c = ctx(&o, None);
    let mut p = SettingsPanel::new(false);
    assert_eq!(p.section, Section::Llm);
    assert_eq!(p.current_field(), FieldId::Model);
    for expected in [
        FieldId::ApiBase,
        FieldId::Temperature,
        FieldId::ThinkingMode,
        FieldId::ApiKey,
    ] {
        p.handle_key(&key(KeyCode::Down), &c);
        assert_eq!(p.current_field(), expected);
    }
    p.handle_key(&key(KeyCode::Down), &c);
    assert_eq!(p.current_field(), FieldId::ApiKey, "底行 clamp");
    p.handle_key(&key(KeyCode::Up), &c);
    assert_eq!(p.current_field(), FieldId::ThinkingMode);
    p.handle_key(&key(KeyCode::Tab), &c);
    assert_eq!(p.section, Section::Session);
    assert_eq!(p.current_field(), FieldId::Role);
    p.handle_key(&key(KeyCode::BackTab), &c);
    assert_eq!(p.section, Section::Llm);
}

#[test]
fn edit_text_prefills_override_and_save_payload() {
    let o = overrides(Some("deepseek-chat"), None, None, None);
    let c = ctx(&o, None);
    let mut p = SettingsPanel::new(false);
    p.handle_key(&key(KeyCode::Enter), &c);
    // 缓冲预填生效值（override）
    let (field, buf) = match &p.editing {
        Some(Editing::Text { field, buf, .. }) => (*field, buf),
        _ => panic!("expected text editing"),
    };
    assert_eq!(field, FieldId::Model);
    assert_eq!(buf.iter().collect::<String>(), "deepseek-chat");
    // 追加光标后输入并提交
    p.handle_key(&key(KeyCode::Char('x')), &c);
    p.handle_key(&key(KeyCode::Enter), &c);
    assert!(p.editing.is_none());
    assert!(matches!(
        p.staged(FieldId::Model),
        FieldAction::Write(Some(v)) if v == "deepseek-chatx"
    ));
    // S → Save 效果（llm 组含 model；prefs 组为空；secret 见 W2 测试）
    let (llm, prefs) = match p.handle_key(&key(KeyCode::Char('s')), &c) {
        PanelEffect::Save { llm, prefs, .. } => (llm, prefs),
        _ => panic!("expected save effect"),
    };
    assert!(matches!(llm.model, FieldAction::Write(Some(v)) if v == "deepseek-chatx"));
    assert!(!llm.api_base.is_write());
    assert!(!prefs.any());
}

#[test]
fn empty_text_commit_means_clear_and_s_noop_when_clean() {
    let o = overrides(None, None, None, None);
    let c = ctx(&o, None);
    let mut p = SettingsPanel::new(false);
    // S 在无改动时不应产生 Save
    assert!(matches!(
        p.handle_key(&key(KeyCode::Char('s')), &c),
        PanelEffect::None
    ));
    // 编辑后清空缓冲 → Write(None)（清除）
    p.handle_key(&key(KeyCode::Enter), &c);
    for _ in 0..4 {
        p.handle_key(&key(KeyCode::Backspace), &c);
    }
    p.handle_key(&key(KeyCode::Enter), &c);
    assert_eq!(p.staged(FieldId::Model), &FieldAction::Write(None));
}

#[test]
fn api_base_invalid_commit_keeps_editing_with_note() {
    let o = overrides(None, None, None, None);
    let c = ctx(&o, None);
    let mut p = SettingsPanel::new(false);
    p.handle_key(&key(KeyCode::Down), &c); // API Base 行
    p.handle_key(&key(KeyCode::Enter), &c);
    for ch in "localhost:1".chars() {
        p.handle_key(&key(KeyCode::Char(ch)), &c);
    }
    p.handle_key(&key(KeyCode::Enter), &c);
    assert!(p.editing.is_some(), "非法提交留在编辑态");
    assert!(p.note.is_some());
    assert!(!p.is_dirty());
    // 清空缓冲后输入合法 URL
    for _ in 0..12 {
        p.handle_key(&key(KeyCode::Backspace), &c);
    }
    for ch in "https://x.example/v1".chars() {
        p.handle_key(&key(KeyCode::Char(ch)), &c);
    }
    p.handle_key(&key(KeyCode::Enter), &c);
    assert!(p.editing.is_none());
    assert!(matches!(
        p.staged(FieldId::ApiBase),
        FieldAction::Write(Some(v)) if v == "https://x.example/v1"
    ));
}

#[test]
fn mode_enum_edit_cycles_and_commits() {
    let o = overrides(None, None, None, None);
    let c = ctx(&o, None);
    let mut p = SettingsPanel::new(false);
    p.handle_key(&key(KeyCode::Tab), &c); // 会话分区
    p.handle_key(&key(KeyCode::Down), &c); // 会话模式行
    p.handle_key(&key(KeyCode::Enter), &c);
    assert!(matches!(&p.editing, Some(Editing::Mode { pick: 0 })));
    p.handle_key(&key(KeyCode::Right), &c);
    p.handle_key(&key(KeyCode::Right), &c);
    assert!(matches!(&p.editing, Some(Editing::Mode { pick: 2 })));
    p.handle_key(&key(KeyCode::Enter), &c);
    assert!(matches!(
        p.staged(FieldId::SessionMode),
        FieldAction::Write(Some(v)) if v == "plan"
    ));
}

#[test]
fn read_only_disables_edit_and_save_but_esc_closes() {
    let o = overrides(None, None, None, None);
    let c = ctx(&o, None);
    let mut p = SettingsPanel::new(true);
    assert!(matches!(
        p.handle_key(&key(KeyCode::Enter), &c),
        PanelEffect::None
    ));
    assert!(p.editing.is_none(), "只读不能进入编辑");
    assert!(p.note.is_some());
    p.handle_key(&key(KeyCode::Char('s')), &c);
    assert!(!p.is_dirty());
    assert!(matches!(
        p.handle_key(&key(KeyCode::Esc), &c),
        PanelEffect::Close
    ));
}

#[test]
fn content_shows_three_layer_values() {
    // override（带 *）
    let o = overrides(Some("deepseek-chat"), None, None, None);
    let p = SettingsPanel::new(false);
    let content = p.content(&ctx(&o, None), 90);
    let text = lines_text(&content);
    assert!(
        text.iter().any(|l| l.contains("deepseek-chat*")),
        "override 带 *"
    );
    // persisted（不带标记）
    let stored = PersistedSettings {
        api_base: Some("https://user.example/v1".into()),
        ..Default::default()
    };
    let o = overrides(None, None, None, None);
    let content = p.content(&ctx(&o, Some(&stored)), 90);
    let text = lines_text(&content);
    assert!(
        text.iter().any(|l| l.contains("https://user.example/v1")),
        "user-data 已存值显示"
    );
    // 会话分区显示 role/mode
    let mut p = SettingsPanel::new(false);
    p.handle_key(&key(KeyCode::Tab), &ctx(&o, Some(&stored)));
    let content = p.content(&ctx(&o, Some(&stored)), 90);
    let text = lines_text(&content);
    assert!(text.iter().any(|l| l.contains("Agent role")));
    assert!(text.iter().any(|l| l.contains("会话模式")));
}

#[test]
fn content_shows_staged_marker() {
    let o = overrides(None, None, None, None);
    let mut p = SettingsPanel::new(false);
    let c = ctx(&o, None);
    p.handle_key(&key(KeyCode::Enter), &c);
    for ch in "gpt-7".chars() {
        p.handle_key(&key(KeyCode::Char(ch)), &c);
    }
    p.handle_key(&key(KeyCode::Enter), &c);
    let content = p.content(&c, 90);
    let text = lines_text(&content);
    assert!(text.iter().any(|l| l.contains("gpt-7~")), "staged 值带 ~");
}

#[test]
fn panel_footer_lines_contain_key_hints() {
    let o = overrides(None, None, None, None);
    let c = ctx(&o, None);
    let p = SettingsPanel::new(false);
    let content = p.content(&c, 90);
    let text = lines_text(&content);
    assert!(text.iter().any(|l| l.contains("[S] 保存")));
    assert!(text.iter().any(|l| l.contains("[Tab] 分区")));
}

#[test]
fn turn_done_unlocks_read_only_panel() {
    let o = overrides(None, None, None, None);
    let c = ctx(&o, None);
    let mut p = SettingsPanel::new(true);
    assert!(matches!(
        p.handle_key(&key(KeyCode::Enter), &c),
        PanelEffect::None
    ));
    assert!(p.editing.is_none(), "只读不能编辑");
    p.unlock_after_turn();
    assert!(matches!(
        p.handle_key(&key(KeyCode::Enter), &c),
        PanelEffect::None
    ));
    assert!(p.editing.is_some(), "解锁后可进入编辑");
    // 已可编辑的面板解锁后不覆盖既有提示。
    let mut p = SettingsPanel::new(false);
    p.unlock_after_turn();
    assert!(p.note.is_none(), "非只读面板解锁不应写提示");
}
