//! 设置面板「外观 → 主题」字段的单测。
//!
//! 独立成文件（`settings_panel.rs` 的 `#[path]` 子模块）：除行数门禁外，也避免
//! 把该文件与 W2/W3 的字段单测混在一起后 lizard 的函数体解析跨度过大。

use super::*;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
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

/// 从默认分区（模型）用 BackTab 回到「外观」。
fn panel_on_appearance() -> SettingsPanel {
    let mut p = SettingsPanel::new(false);
    let o = SessionPrefs::from_options(None, None, None);
    p.handle_key(&key(KeyCode::BackTab), &ctx(&o, None));
    p
}

#[test]
fn appearance_section_leads_the_nav_and_holds_only_theme() {
    let o = SessionPrefs::from_options(None, None, None);
    let p = SettingsPanel::new(false);
    let content = p.content(&ctx(&o, None), 90);
    let labels: Vec<&str> = content.nav.iter().map(|(l, _)| l.as_str()).collect();
    assert_eq!(labels, vec!["外观", "模型", "会话"], "导航顺序对齐 Desktop");
    assert!(
        matches!(&content.nav[..], [(_, selected), ..] if !*selected),
        "默认停在模型分区（最常用），外观靠 Tab/BackTab 到达"
    );
}

#[test]
fn theme_is_the_only_field_in_appearance() {
    assert_eq!(Section::Appearance.fields(), &[FieldId::Theme]);
    let mut p = panel_on_appearance();
    assert_eq!(p.section, Section::Appearance);
    assert_eq!(p.current_field(), FieldId::Theme);
    // 单行分区：↓ 不越界。
    let o = SessionPrefs::from_options(None, None, None);
    p.handle_key(&key(KeyCode::Down), &ctx(&o, None));
    assert_eq!(p.row, 0);
}

#[test]
fn theme_row_says_default_when_unset() {
    let o = SessionPrefs::from_options(None, None, None);
    let p = panel_on_appearance();
    let text = lines_text(&p.content(&ctx(&o, None), 90));
    assert!(
        text.iter()
            .any(|l| l.contains("主题") && l.contains("默认（透明）")),
        "空 prefs.theme 显示产物默认，而不是「跟随 server」：{text:?}"
    );
}

#[test]
fn theme_row_shows_stored_slug_verbatim() {
    let stored = PersistedSettings {
        theme: Some("shadcn-light".into()),
        ..Default::default()
    };
    let o = SessionPrefs::from_options(None, None, None);
    let p = panel_on_appearance();
    let text = lines_text(&p.content(&ctx(&o, Some(&stored)), 90));
    assert!(
        text.iter()
            .any(|l| l.contains("主题") && l.contains("shadcn-light")),
        "已存 slug 原样展示：{text:?}"
    );
}

#[test]
fn theme_cycle_commits_the_selected_slug() {
    let o = SessionPrefs::from_options(None, None, None);
    let c = ctx(&o, None);
    let mut p = panel_on_appearance();
    p.handle_key(&key(KeyCode::Enter), &c);
    assert!(matches!(&p.editing, Some(Editing::Theme { pick: 0 })));
    // 0=默认 1=shadcn-transparent 2=default-dark 3=crabmate-dark
    for _ in 0..3 {
        p.handle_key(&key(KeyCode::Right), &c);
    }
    assert!(matches!(&p.editing, Some(Editing::Theme { pick: 3 })));
    p.handle_key(&key(KeyCode::Enter), &c);
    assert_eq!(
        p.staged(FieldId::Theme),
        &FieldAction::Write(Some("crabmate-dark".into()))
    );
    assert!(p.is_dirty());
}

#[test]
fn theme_cycle_back_to_default_clears_the_key() {
    let stored = PersistedSettings {
        theme: Some("material-dark".into()),
        ..Default::default()
    };
    let o = SessionPrefs::from_options(None, None, None);
    let c = ctx(&o, Some(&stored));
    let mut p = panel_on_appearance();
    p.handle_key(&key(KeyCode::Enter), &c);
    // 已存 material-dark 应定位到它在选项表里的下标，不是从头开始。
    assert!(matches!(&p.editing, Some(Editing::Theme { pick: 5 })));
    for _ in 0..5 {
        p.handle_key(&key(KeyCode::Left), &c);
    }
    assert!(matches!(&p.editing, Some(Editing::Theme { pick: 0 })));
    p.handle_key(&key(KeyCode::Enter), &c);
    assert_eq!(p.staged(FieldId::Theme), &FieldAction::Write(None));
}

#[test]
fn theme_esc_cancels_without_staging() {
    let o = SessionPrefs::from_options(None, None, None);
    let c = ctx(&o, None);
    let mut p = panel_on_appearance();
    p.handle_key(&key(KeyCode::Enter), &c);
    p.handle_key(&key(KeyCode::Right), &c);
    p.handle_key(&key(KeyCode::Esc), &c);
    assert!(p.editing.is_none());
    assert!(!p.is_dirty(), "Esc 取消编辑不应留下 staged");
}

#[test]
fn unknown_stored_slug_reads_through_and_opens_at_default() {
    // `system` 不在 TUI 选项表里（终端无 OS 明暗感知）：读取行照原样显示，
    // 编辑起点回落到下标 0，只有用户真的按 Enter 才会改写 Desktop 的值。
    let stored = PersistedSettings {
        theme: Some("system".into()),
        ..Default::default()
    };
    let o = SessionPrefs::from_options(None, None, None);
    let c = ctx(&o, Some(&stored));
    let p = panel_on_appearance();
    let text = lines_text(&p.content(&c, 90));
    assert!(
        text.iter()
            .any(|l| l.contains("主题") && l.contains("system")),
        "未知 slug 不被丢弃：{text:?}"
    );

    let mut p = panel_on_appearance();
    p.handle_key(&key(KeyCode::Enter), &c);
    assert!(matches!(&p.editing, Some(Editing::Theme { pick: 0 })));
}

#[test]
fn theme_save_goes_to_prefs_group_only() {
    let o = SessionPrefs::from_options(None, None, None);
    let c = ctx(&o, None);
    let mut p = panel_on_appearance();
    p.handle_key(&key(KeyCode::Enter), &c);
    p.handle_key(&key(KeyCode::Right), &c); // shadcn-transparent
    p.handle_key(&key(KeyCode::Enter), &c);
    let PanelEffect::Save { llm, prefs, secret } = p.handle_key(&key(KeyCode::Char('s')), &c)
    else {
        panic!("主题改动应产生保存请求");
    };
    assert_eq!(
        prefs.theme,
        FieldAction::Write(Some("shadcn-transparent".into()))
    );
    assert!(!llm.any(), "主题不写 llm-overrides");
    assert!(secret.is_skip(), "主题不碰钥匙串");
}

#[test]
fn theme_section_is_reachable_from_the_last_section_by_tab() {
    let o = SessionPrefs::from_options(None, None, None);
    let c = ctx(&o, None);
    let mut p = SettingsPanel::new(false);
    // 模型 → 会话 → 外观
    p.handle_key(&key(KeyCode::Tab), &c);
    assert_eq!(p.section, Section::Session);
    p.handle_key(&key(KeyCode::Tab), &c);
    assert_eq!(p.section, Section::Appearance);
    assert_eq!(p.row, 0, "切分区后行光标回到首行");
}

/// Enter 后主题行下方要摊开候选，且选中项一定在可见范围内。
///
/// 列表长于窗口时尾部用省略行示意，不是静默丢掉 —— 叠加 `render_settings_body`
/// 的 `take(height)` 硬裁，静默丢行会让选中项连同提示一起消失。
#[test]
fn enter_on_theme_opens_a_dropdown_that_keeps_the_pick_visible() {
    let o = SessionPrefs::from_options(None, None, None);
    let mut p = panel_on_appearance();
    let before = lines_text(&p.content(&ctx(&o, None), 90)).len();
    p.handle_key(&key(KeyCode::Enter), &ctx(&o, None));

    let text = lines_text(&p.content(&ctx(&o, None), 90));
    assert!(text.len() > before, "字段行之外还要多出候选行：{text:?}");
    assert!(
        text.iter().any(|l| l.contains("默认（透明）")),
        "当前选中项必须在列表里：{text:?}"
    );
    assert!(
        text.iter().any(|l| l.contains("还有") && l.contains("项")),
        "超出可见窗口的候选要有省略提示：{text:?}"
    );
}

/// 窗口跟着高亮走：走到最后一项时，它必须还看得见。
#[test]
fn dropdown_window_follows_the_highlight_to_the_tail() {
    let o = SessionPrefs::from_options(None, None, None);
    let mut p = panel_on_appearance();
    p.handle_key(&key(KeyCode::Enter), &ctx(&o, None));
    for _ in 0..(THEME_OPTIONS.len() + 2) {
        p.handle_key(&key(KeyCode::Down), &ctx(&o, None));
    }

    let text = lines_text(&p.content(&ctx(&o, None), 90));
    let last = THEME_OPTIONS[THEME_OPTIONS.len() - 1].unwrap();
    assert!(
        text.iter().any(|l| l.contains(last)),
        "走到底时最后一项 {last} 应可见：{text:?}"
    );
}

/// 下拉行数有固定上界：候选封顶 7 行，加首尾各一行省略提示，最多多出 9 行。
///
/// 不要求各位置精确相等——停在中间时首尾都要省略提示，比贴边多一行；这里要守的是
/// 「不会因为候选多而无限撑高」，那才是小终端里被 `take(height)` 裁掉的来源。
#[test]
fn dropdown_line_count_has_a_fixed_ceiling() {
    let o = SessionPrefs::from_options(None, None, None);
    let mut p = panel_on_appearance();
    p.handle_key(&key(KeyCode::Enter), &ctx(&o, None));
    let base = lines_text(&p.content(&ctx(&o, None), 90)).len();

    for _ in 0..(THEME_OPTIONS.len() + 2) {
        p.handle_key(&key(KeyCode::Down), &ctx(&o, None));
        let text = lines_text(&p.content(&ctx(&o, None), 90));
        assert!(
            text.len() - base <= 9,
            "展开后最多多出 9 行（7 候选 + 2 省略），实际多出 {}：{text:?}",
            text.len() - base
        );
    }
}

/// ↑↓ 逐项移动并 clamp 在两端（下拉不环绕）；Enter 提交高亮那一项。
#[test]
fn dropdown_walks_with_arrows_and_commits_the_highlighted_option() {
    let o = SessionPrefs::from_options(None, None, None);
    let mut p = panel_on_appearance();
    p.handle_key(&key(KeyCode::Enter), &ctx(&o, None));

    // 未设主题时停在 0（默认），↓ 两下到 index 2。
    p.handle_key(&key(KeyCode::Down), &ctx(&o, None));
    p.handle_key(&key(KeyCode::Down), &ctx(&o, None));
    p.handle_key(&key(KeyCode::Enter), &ctx(&o, None));

    let want = THEME_OPTIONS[2].map(str::to_string);
    assert_eq!(
        p.staged(FieldId::Theme),
        &FieldAction::Write(want),
        "提交的是高亮项，不是 index 0"
    );
}

/// 到顶/到底就停：连按 9 次 ↓ 仍停在最后一项，不会绕回默认。
#[test]
fn dropdown_clamps_at_both_ends() {
    let o = SessionPrefs::from_options(None, None, None);
    let mut p = panel_on_appearance();
    p.handle_key(&key(KeyCode::Enter), &ctx(&o, None));

    for _ in 0..(THEME_OPTIONS.len() + 3) {
        p.handle_key(&key(KeyCode::Down), &ctx(&o, None));
    }
    p.handle_key(&key(KeyCode::Enter), &ctx(&o, None));
    let last = THEME_OPTIONS[THEME_OPTIONS.len() - 1].map(str::to_string);
    assert_eq!(p.staged(FieldId::Theme), &FieldAction::Write(last));

    // 回到编辑态后再连按 ↑：停在 0（默认）。
    p.handle_key(&key(KeyCode::Enter), &ctx(&o, None));
    for _ in 0..(THEME_OPTIONS.len() + 3) {
        p.handle_key(&key(KeyCode::Up), &ctx(&o, None));
    }
    p.handle_key(&key(KeyCode::Enter), &ctx(&o, None));
    assert_eq!(p.staged(FieldId::Theme), &FieldAction::Write(None));
}

/// Esc 取消：列表收起，暂存值不变 —— 下拉开了又关不该留下半截改动。
#[test]
fn esc_closes_the_dropdown_without_staging_anything() {
    let o = SessionPrefs::from_options(None, None, None);
    let mut p = panel_on_appearance();
    p.handle_key(&key(KeyCode::Enter), &ctx(&o, None));
    p.handle_key(&key(KeyCode::Down), &ctx(&o, None));
    p.handle_key(&key(KeyCode::Esc), &ctx(&o, None));

    assert_eq!(p.staged(FieldId::Theme), &FieldAction::Skip, "取消不暂存");
    let text = lines_text(&p.content(&ctx(&o, None), 90));
    assert!(
        !text.iter().any(|l| l.contains("shadcn-transparent")),
        "取消后列表应已收起：{text:?}"
    );
}

/// 底栏第二行（按键说明）。`footer` 与字段行分开返回，不能从 `lines` 末尾取。
fn footer_hint_of(p: &SettingsPanel, o: &SessionPrefs) -> String {
    p.content(&ctx(o, None), 90)
        .footer
        .get(1)
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
        })
        .unwrap_or_default()
}

/// 编辑态的底栏提示跟着换：主题走 ↑↓，其余枚举仍是 ←→。
#[test]
fn footer_hint_switches_to_up_down_for_the_theme_dropdown() {
    let o = SessionPrefs::from_options(None, None, None);
    let mut p = panel_on_appearance();
    p.handle_key(&key(KeyCode::Enter), &ctx(&o, None));
    let hint = footer_hint_of(&p, &o);
    assert!(hint.contains("↑/↓"), "主题编辑态应提示上下选择：{hint}");

    p.handle_key(&key(KeyCode::Esc), &ctx(&o, None));
    p.handle_key(&key(KeyCode::Tab), &ctx(&o, None)); // 外观 → 模型
    p.handle_key(&key(KeyCode::Tab), &ctx(&o, None)); // 模型 → 会话
    p.handle_key(&key(KeyCode::Down), &ctx(&o, None)); // 角色 → 会话模式（枚举）
    p.handle_key(&key(KeyCode::Enter), &ctx(&o, None));
    let hint = footer_hint_of(&p, &o);
    assert!(hint.contains("←/→"), "其他枚举仍是左右循环：{hint}");
}
