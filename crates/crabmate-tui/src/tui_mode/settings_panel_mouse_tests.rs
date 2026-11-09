//! 设置面板鼠标支持的单测（`settings_panel.rs` 的 `#[path]` 子模块）。
//!
//! 分两类：
//! - **行归属**（`RowOwner`）：必须与 `lines` 等长，且折行续行、主题候选各归其位。
//!   这是「点一行选中相邻那行」的唯一防线。
//! - **命中与动作**：坐标 → `PanelHit` → 与键盘同一批状态机动作，并守住门禁
//!   （确认关闭 / 只读 / 编辑中）。

use super::*;
use ratatui::layout::Rect;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctx<'a>(o: &'a SessionPrefs) -> PanelCtx<'a> {
    PanelCtx {
        overrides: o,
        persisted: None,
        serve_defaults: None,
        conn: None,
        pal: &Palette::DARK,
    }
}

/// 足够宽：左导航轨可见（`SETTINGS_NAV_MIN_WIDTH = 44`）。
const WIDE: Rect = Rect::new(0, 0, 100, 30);
/// 窄屏：不画导航轨，内容区从第 0 列开始。
const NARROW: Rect = Rect::new(0, 0, 42, 20);

/// 渲染一帧并把它记进面板（与 `settings_app::settings_content` 同一条路径）。
fn render(p: &mut SettingsPanel, o: &SessionPrefs, area: Rect) -> PanelContent {
    let width = crate::tui_mode::render::overlay::settings_panel_content_width(area);
    let content = p.content(&ctx(o), width);
    p.remember_frame(width, &content.row_owner);
    content
}

fn section_of(label: &str) -> Section {
    *Section::ALL
        .iter()
        .find(|s| s.label() == label)
        .expect("分区标签必须存在于 Section::ALL")
}

#[test]
fn row_owner_is_always_aligned_with_lines() {
    let o = SessionPrefs::from_options(None, None, None);
    for section in Section::ALL {
        let mut p = SettingsPanel::new(false);
        p.select_section(Section::ALL.iter().position(|s| *s == section).unwrap());
        let content = render(&mut p, &o, WIDE);
        assert_eq!(
            content.row_owner.len(),
            content.lines.len(),
            "分区「{}」的行归属与绘制行数不一致",
            section.label()
        );
        // 首行是分区说明，不归属任何字段。
        assert_eq!(content.row_owner[0], RowOwner::Prose);
    }
}

#[test]
fn wrapped_continuation_rows_stay_on_the_same_field() {
    let long = format!("https://gateway.example.com/{}", "x".repeat(60));
    let o = SessionPrefs::from_options(None, None, Some(long));
    let mut p = SettingsPanel::new(false);
    // 窄屏才折行：82 列的内容区放得下短值，长值在这里会折成多段。
    let content = render(&mut p, &o, NARROW);
    let parts: Vec<usize> = content
        .row_owner
        .iter()
        .filter_map(|owner| match owner {
            RowOwner::Field { field, part } if *field == FieldId::ApiBase => Some(*part),
            _ => None,
        })
        .collect();
    assert!(
        parts.len() > 1,
        "长 API Base 应当折行，实际只占 {} 行",
        parts.len()
    );
    assert_eq!(
        parts,
        (0..parts.len()).collect::<Vec<_>>(),
        "续行 part 应递增"
    );
}

#[test]
fn theme_dropdown_rows_are_clickable_options() {
    let o = SessionPrefs::from_options(None, None, None);
    let mut p = SettingsPanel::new(false);
    p.select_section(0); // 外观
    assert_eq!(p.section, section_of("外观"));
    p.handle_key(&key(KeyCode::Enter), &ctx(&o)); // 展开主题候选
    let content = render(&mut p, &o, WIDE);
    let picks: Vec<usize> = content
        .row_owner
        .iter()
        .filter_map(|owner| match owner {
            RowOwner::ThemeOption(pick) => Some(*pick),
            _ => None,
        })
        .collect();
    assert!(!picks.is_empty(), "主题编辑态应当摊开候选列表");
    assert_eq!(picks[0], 0, "候选从第 0 项开始编号");
    assert_eq!(
        content.row_owner.len(),
        content.lines.len(),
        "下拉插行后映射仍必须对齐"
    );
}

#[test]
fn clicking_nav_row_selects_that_section() {
    let o = SessionPrefs::from_options(None, None, None);
    let mut p = SettingsPanel::new(false);
    let content = render(&mut p, &o, WIDE);
    let layout = crate::tui_mode::render::overlay::settings_panel_layout(WIDE);
    // 导航轨从顶栏下面一行开始；第 2 行 = `Section::ALL[1]`。
    let hit = super::mouse::hit(WIDE, &content.row_owner, layout.nav.x, layout.nav.y + 1);
    assert_eq!(hit, Some(mouse::PanelHit::Nav(1)));
    p.click(hit.unwrap(), &ctx(&o));
    assert_eq!(p.section, Section::ALL[1]);
    assert_eq!(p.row, 0, "切分区要回到首行");
}

#[test]
fn clicking_label_focuses_row_and_clicking_value_opens_edit() {
    let o = SessionPrefs::from_options(None, None, None);
    let mut p = SettingsPanel::new(false);
    let content = render(&mut p, &o, WIDE);
    let layout = crate::tui_mode::render::overlay::settings_panel_layout(WIDE);
    let inner = crate::tui_mode::render::overlay::content_inner_rect(layout.content);
    // 说明行占内容区第 0 行，Api Base 是模型分区第 2 个字段 → 屏幕第 3 行。
    let row = inner.y + 2;
    let label_hit = super::mouse::hit(WIDE, &content.row_owner, inner.x + 1, row);
    assert_eq!(
        label_hit,
        Some(mouse::PanelHit::Field {
            field: FieldId::ApiBase,
            on_value: false
        })
    );
    p.click(label_hit.unwrap(), &ctx(&o));
    assert_eq!(p.row, 1, "点标签只落光标");

    let value_hit = super::mouse::hit(WIDE, &content.row_owner, inner.x + LABEL_PAD as u16, row);
    assert_eq!(
        value_hit,
        Some(mouse::PanelHit::Field {
            field: FieldId::ApiBase,
            on_value: true
        })
    );
    p.click(value_hit.unwrap(), &ctx(&o));
    // API Base 的编辑态是「网关预设循环」（`Editing::Gateway`），自定义 URL 才转文本。
    assert!(
        matches!(p.editing.as_ref(), Some(Editing::Gateway { .. })),
        "点值列要进入该字段的编辑态，实际 {:?}",
        p.editing
    );
}

#[test]
fn read_only_panel_only_warns_on_value_click() {
    let o = SessionPrefs::from_options(None, None, None);
    let mut p = SettingsPanel::new(true);
    let content = render(&mut p, &o, WIDE);
    let layout = crate::tui_mode::render::overlay::settings_panel_layout(WIDE);
    let inner = crate::tui_mode::render::overlay::content_inner_rect(layout.content);
    let hit = super::mouse::hit(
        WIDE,
        &content.row_owner,
        inner.x + LABEL_PAD as u16,
        inner.y + 1,
    );
    p.click(hit.unwrap(), &ctx(&o));
    assert!(p.editing.is_none(), "只读态不得进入编辑");
    assert!(p.note.is_some(), "只读态要给一句提示，而不是毫无反应");
}

#[test]
fn clicks_are_ignored_while_confirming_close() {
    let o = SessionPrefs::from_options(None, None, None);
    let mut p = SettingsPanel::new(false);
    let before = p.section;
    p.confirm_close = true;
    p.click(mouse::PanelHit::Nav(2), &ctx(&o));
    assert_eq!(p.section, before, "确认关闭时点击不该动状态");
}
#[test]
fn clicks_do_not_retarget_while_editing_text() {
    let o = SessionPrefs::from_options(None, None, None);
    let mut p = SettingsPanel::new(false);
    render(&mut p, &o, WIDE);
    p.handle_key(&key(KeyCode::Enter), &ctx(&o)); // 编辑「模型名」
    let editing = p.editing.clone();
    p.click(
        mouse::PanelHit::Field {
            field: FieldId::ApiBase,
            on_value: true,
        },
        &ctx(&o),
    );
    assert_eq!(p.editing, editing, "编辑中换目标会丢掉缓冲，必须挡住");
    assert!(
        p.note.is_some(),
        "挡住之后要给一句说明，否则用户读成「点了没反应」"
    );
}

#[test]
fn wheel_moves_the_row_cursor_within_the_section() {
    let mut p = SettingsPanel::new(false);
    let last = p.section.fields().len() - 1;
    for _ in 0..10 {
        p.scroll_row(false);
    }
    assert_eq!(p.row, last, "滚到底停在末行，不环绕");
    for _ in 0..10 {
        p.scroll_row(true);
    }
    assert_eq!(p.row, 0, "滚到顶停在首行");
}

#[test]
fn frame_rows_are_dropped_when_the_width_changes() {
    let o = SessionPrefs::from_options(None, None, None);
    let mut p = SettingsPanel::new(false);
    let width = crate::tui_mode::render::overlay::settings_panel_content_width(WIDE);
    render(&mut p, &o, WIDE);
    assert!(p.frame_rows_for(width).is_some());
    assert!(
        p.frame_rows_for(width - 1).is_none(),
        "宽度变了就不能再用上一屏的行归属"
    );
}

#[test]
fn narrow_layout_never_hits_the_nav_rail() {
    let o = SessionPrefs::from_options(None, None, None);
    let mut p = SettingsPanel::new(false);
    let content = render(&mut p, &o, NARROW);
    // 窄屏第 0 列原本是导航轨的位置：必须落到内容区（或不命中），不能是 Nav。
    let hit = super::mouse::hit(NARROW, &content.row_owner, 0, 2);
    assert!(!matches!(hit, Some(mouse::PanelHit::Nav(_))));
}

#[test]
fn left_and_right_arrows_switch_sections() {
    let o = SessionPrefs::from_options(None, None, None);
    let mut p = SettingsPanel::new(false);
    assert_eq!(p.section, Section::Llm);
    p.handle_key(&key(KeyCode::Right), &ctx(&o));
    assert_eq!(p.section, Section::ALL[2]);
    p.handle_key(&key(KeyCode::Left), &ctx(&o));
    assert_eq!(p.section, Section::Llm);
    // 左边界回绕到最后一个分区（与 Tab / BackTab 同语义）。
    p.select_section(0);
    p.handle_key(&key(KeyCode::Left), &ctx(&o));
    assert_eq!(p.section, *Section::ALL.last().unwrap());
}

#[test]
fn arrows_stay_within_the_text_editor() {
    let o = SessionPrefs::from_options(None, None, None);
    let mut p = SettingsPanel::new(false);
    p.handle_key(&key(KeyCode::Enter), &ctx(&o)); // 进入「模型名」文本编辑
    let before = p.section;
    p.handle_key(&key(KeyCode::Right), &ctx(&o));
    assert_eq!(p.section, before, "编辑态 ←→ 归光标，不切分区");
}

/// 内容区扣掉左右内缩后的可绘矩形（与绘制 / 命中共用同一份换算）。
fn inner_of(area: Rect) -> Rect {
    crate::tui_mode::render::overlay::content_inner_rect(
        crate::tui_mode::render::overlay::settings_panel_layout(area).content,
    )
}

#[test]
fn clicking_a_theme_candidate_stages_the_slug() {
    let o = SessionPrefs::from_options(None, None, None);
    let mut p = SettingsPanel::new(false);
    p.select_section(0); // 外观
    p.handle_key(&key(KeyCode::Enter), &ctx(&o)); // 展开候选
    let content = render(&mut p, &o, WIDE);
    let inner = inner_of(WIDE);
    let idx = content
        .row_owner
        .iter()
        .position(|owner| matches!(owner, RowOwner::ThemeOption(1)))
        .expect("候选 1 应当出现在下拉里");
    // 候选行与字段行一样从内容区左边缘起算。
    let hit = super::mouse::hit(WIDE, &content.row_owner, inner.x, inner.y + idx as u16);
    assert_eq!(hit, Some(mouse::PanelHit::ThemeOption(1)));
    p.click(hit.unwrap(), &ctx(&o));
    assert!(p.editing.is_none(), "点候选即提交，应退出编辑态");
    assert!(p.is_dirty(), "点候选要真的写进 staged");
    assert_eq!(
        p.staged(FieldId::Theme),
        &FieldAction::Write(Some(THEME_OPTIONS[1].unwrap().to_string())),
        "落地的 slug 必须是被点的那一项"
    );
}

#[test]
fn wheel_moves_the_theme_candidate_while_the_list_is_open() {
    let o = SessionPrefs::from_options(None, None, None);
    let mut p = SettingsPanel::new(false);
    p.select_section(0);
    p.handle_key(&key(KeyCode::Enter), &ctx(&o));
    assert!(matches!(p.editing, Some(Editing::Theme { pick: 0 })));
    p.scroll_row(false);
    assert!(
        matches!(p.editing, Some(Editing::Theme { pick: 1 })),
        "下拉展开时滚轮与 ↓ 同义"
    );
    p.scroll_row(true);
    p.scroll_row(true);
    assert!(
        matches!(p.editing, Some(Editing::Theme { pick: 0 })),
        "到顶停住，不环绕（与 pick_step 同一个 clamp）"
    );
}
