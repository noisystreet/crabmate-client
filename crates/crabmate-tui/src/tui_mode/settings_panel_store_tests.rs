//! 会话存储（SQLite）开关的单测：字段行文案、编辑态、保存动作与三组结果复位。
//!
//! 与 `tests_w3` 分开：那个文件覆盖 W2 收尾字段，这里覆盖「不在 user-data 快照里」
//! 的第三组保存（`POST /config/session/conversation-store`），语义差异值得单独一组。
//!
//! 断言一律走 `content()`（公开构建入口）而不是 `value_cell` 这类 `draw` 模块的私有项：
//! 测试模块不是 `draw` 的后代，看不到它们 —— 而且用户看到的本来就是拼好的那一行。

use super::*;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

/// 一个可控的 serve 默认值快照（只填会话存储那两个字段）。
fn serve(sqlite_active: Option<bool>, configured: Option<bool>) -> ServeDefaults {
    ServeDefaults {
        conversation_store_sqlite_active: sqlite_active,
        conversation_store_sqlite_path_configured: configured,
        ..Default::default()
    }
}

fn overrides() -> SessionPrefs {
    SessionPrefs::from_options(None, None, None)
}

fn ctx<'a>(o: &'a SessionPrefs, defaults: Option<&'a ServeDefaults>) -> PanelCtx<'a> {
    PanelCtx {
        overrides: o,
        persisted: None,
        serve_defaults: defaults,
        pal: &Palette::DARK,
    }
}

fn line_text(line: &Line<'_>) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

/// 字段行（以标签开头的第一行）文本。
fn row_text(content: &PanelContent, field: FieldId) -> String {
    let label = field_label(field);
    content
        .lines
        .iter()
        .map(line_text)
        .find(|t| t.starts_with(label))
        .unwrap_or_default()
}

fn footer_text(content: &PanelContent) -> String {
    content
        .footer
        .iter()
        .map(line_text)
        .collect::<Vec<_>>()
        .join("\n")
}

/// 打开面板并停到「会话 → 会话存储（SQLite）」行。
fn panel_on_store_row() -> SettingsPanel {
    let mut p = SettingsPanel::new(false);
    p.section = Section::Session;
    p.row = Section::Session
        .fields()
        .iter()
        .position(|f| *f == FieldId::SessionSqlite)
        .expect("会话分区必须包含会话存储行");
    p
}

#[test]
fn store_row_reads_the_serve_state() {
    let o = overrides();
    let file_store = serve(Some(false), Some(true));
    let c = ctx(&o, Some(&file_store));
    let p = panel_on_store_row();
    let row = row_text(&p.content(&c, 80), FieldId::SessionSqlite);
    assert!(
        row.contains("关（文件存储）"),
        "serve 报 false 应是文件存储：{row}"
    );

    let sqlite = serve(Some(true), Some(true));
    let c = ctx(&o, Some(&sqlite));
    let row = row_text(&p.content(&c, 80), FieldId::SessionSqlite);
    assert!(
        row.contains("开（SQLite）"),
        "serve 报 true 应是 SQLite：{row}"
    );
}

#[test]
fn unknown_serve_state_is_not_shown_as_off() {
    let o = overrides();
    let c = ctx(&o, None);
    let p = panel_on_store_row();
    let row = row_text(&p.content(&c, 80), FieldId::SessionSqlite);
    assert!(
        row.contains("(该 serve 未提供)"),
        "不知道当前态时不能显示成「关」：{row}"
    );
}

#[test]
fn enter_on_unsupported_serve_stays_in_browse_mode() {
    let o = overrides();
    // 路径未配置：服务端必然拒绝，面板不该让用户按进去。
    let d = serve(Some(false), Some(false));
    let c = ctx(&o, Some(&d));
    let mut p = panel_on_store_row();
    assert!(matches!(
        p.handle_key(&key(KeyCode::Enter), &c),
        PanelEffect::None
    ));
    assert!(p.editing.is_none(), "不支持时不该进入编辑态");
    assert!(!p.is_dirty());
    let (note, level) = p.note.clone().expect("应有提示");
    assert!(note.contains("不支持切换会话存储"), "实际提示：{note}");
    assert_eq!(level, NoticeLevel::Warn);
}

#[test]
fn commit_off_then_save_emits_store_action() {
    let o = overrides();
    let d = serve(Some(true), Some(true));
    let c = ctx(&o, Some(&d));
    let mut p = panel_on_store_row();
    // 当前是「开」→ 进编辑态 pick 0
    p.handle_key(&key(KeyCode::Enter), &c);
    assert!(
        matches!(
            p.editing,
            Some(Editing::Toggle {
                field: FieldId::SessionSqlite,
                pick: 0
            })
        ),
        "编辑起点应落在当前生效值上"
    );
    // → 挪到「关」，Enter 提交
    p.handle_key(&key(KeyCode::Right), &c);
    p.handle_key(&key(KeyCode::Enter), &c);
    assert!(p.editing.is_none());
    assert!(p.is_dirty(), "staged 改动应让面板变脏");
    let row = row_text(&p.content(&c, 80), FieldId::SessionSqlite);
    assert!(row.contains("关（文件存储）~"), "staged 值要带 `~`：{row}");

    let PanelEffect::Save { store, .. } = p.handle_key(&key(KeyCode::Char('s')), &c) else {
        panic!("S 应产生保存请求");
    };
    assert_eq!(store, FieldAction::Write(Some(STORE_OFF.to_string())));
    assert!(p.is_saving(), "请求已发出，面板应进入「保存中」");
}

#[test]
fn save_result_clears_or_keeps_the_staged_action() {
    let o = overrides();
    let d = serve(Some(true), Some(true));
    let c = ctx(&o, Some(&d));
    let mut p = panel_on_store_row();
    // 提交「开」：与当前生效值相同也允许（幂等动作，不做无改动判断）
    p.handle_key(&key(KeyCode::Enter), &c);
    p.handle_key(&key(KeyCode::Enter), &c);
    assert!(p.is_dirty());
    assert!(!p.is_saving(), "还没按 S，不该有在途标记");

    // 按 S：请求发出，面板进入「保存中」
    let PanelEffect::Save { .. } = p.handle_key(&key(KeyCode::Char('s')), &c) else {
        panic!("S 应产生保存请求");
    };
    assert!(p.is_saving());

    // 失败：保留 staged 供重试，且这是最后一组在途 → 返回「全部落地」
    assert!(p.save_group_result(SaveGroup::Store, false));
    assert!(p.is_dirty(), "失败不该清掉 staged");
    assert!(!p.is_saving());

    // 成功：清空 staged 与在途标记
    let _ = p.handle_key(&key(KeyCode::Char('s')), &c);
    assert!(p.save_group_result(SaveGroup::Store, true));
    assert!(!p.is_dirty());
    assert!(!p.is_saving());
}

#[test]
fn footer_preview_uses_the_same_wording_as_the_row() {
    let o = overrides();
    let d = serve(Some(false), Some(true));
    let c = ctx(&o, Some(&d));
    let mut p = panel_on_store_row();
    p.handle_key(&key(KeyCode::Enter), &c);
    let footer = footer_text(&p.content(&c, 80));
    assert!(
        footer.contains("会话存储：") && footer.contains("关（文件存储）"),
        "底栏循环预览应与行同一份措辞：{footer}"
    );
}
