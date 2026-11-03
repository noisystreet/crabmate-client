//! 只读三栏（连接 / 快捷键 / 本地行为）的单测。
//!
//! 这三栏的共同契约：**Enter 永远不改状态**（无编辑态、不变脏），值本身就是说明；
//! 拿不到 serve 事实时写「未提供」，不假装知道。

use super::*;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn overrides() -> SessionPrefs {
    SessionPrefs::from_options(None, None, None)
}

fn panel_on(section: Section) -> SettingsPanel {
    let mut p = SettingsPanel::new(false);
    p.section = section;
    p.row = 0;
    p
}

fn row_texts(p: &SettingsPanel, c: &PanelCtx<'_>) -> Vec<String> {
    p.content(c, 120)
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
fn connection_rows_read_serve_facts_or_say_unknown() {
    let o = overrides();
    let defaults = ServeDefaults::from_status(&serde_json::json!({ "model": "gpt-4o-mini" }));
    let c = PanelCtx {
        overrides: &o,
        persisted: None,
        serve_defaults: Some(&defaults),
        pal: &Palette::DARK,
    };
    let p = panel_on(Section::Connection);
    let lines = row_texts(&p, &c);
    assert!(
        lines.iter().any(|l| l.contains("gpt-4o-mini")),
        "应显示 serve 给的默认模型：{lines:?}"
    );
    // 没有 /permission 投影 → 权限档写「未提供」
    assert!(
        lines.iter().any(|l| l.contains("(该 serve 未提供)")),
        "权限档未知时要明说：{lines:?}"
    );

    let c = PanelCtx {
        overrides: &o,
        persisted: None,
        serve_defaults: None,
        pal: &Palette::DARK,
    };
    let unknown = row_texts(&p, &c);
    assert_eq!(
        unknown
            .iter()
            .filter(|l| l.contains("(该 serve 未提供)"))
            .count(),
        2,
        "两个未知行都要写清楚：{unknown:?}"
    );
}

#[test]
fn shortcut_and_local_rows_carry_their_copy() {
    let o = overrides();
    let c = PanelCtx {
        overrides: &o,
        persisted: None,
        serve_defaults: None,
        pal: &Palette::DARK,
    };
    let keys = row_texts(&panel_on(Section::Shortcuts), &c).join("\n");
    assert!(keys.contains("F6"), "全局键要提到设置键：{keys}");
    assert!(keys.contains("Tab"), "面板键要提到分区切换：{keys}");
    assert!(keys.contains("PgUp"), "编辑器键要提到翻页：{keys}");

    let local = row_texts(&panel_on(Section::Local), &c).join("\n");
    assert!(local.contains("钥匙串"), "要写清密钥落点：{local}");
    assert!(local.contains("先 GET"), "要写清保存语义：{local}");
}

#[test]
fn enter_never_edits_a_readonly_section() {
    let o = overrides();
    let c = PanelCtx {
        overrides: &o,
        persisted: None,
        serve_defaults: None,
        pal: &Palette::DARK,
    };
    for section in [Section::Connection, Section::Shortcuts, Section::Local] {
        let mut p = panel_on(section);
        assert!(matches!(
            p.handle_key(&key(KeyCode::Enter), &c),
            PanelEffect::None
        ));
        assert!(p.editing.is_none(), "{section:?} 不该进编辑态");
        assert!(!p.is_dirty(), "{section:?} 不该变脏");
        assert!(p.note.is_some(), "{section:?} 应给只读提示");
    }
}

#[test]
fn readonly_sections_keep_their_fields_non_empty() {
    // 空 `fields()` 会让 `current_field()` 越界 —— 只读靠 `is_readonly_section`
    // 提前返回，但字段表本身也必须非空（导航与行渲染都按它走）。
    for section in [Section::Connection, Section::Shortcuts, Section::Local] {
        assert!(!section.fields().is_empty(), "{section:?} 必须有可见行");
        assert!(is_readonly_section(section));
    }
    // 可写四栏不能被误标成只读。
    for section in [
        Section::Appearance,
        Section::Llm,
        Section::Session,
        Section::Mcp,
    ] {
        assert!(!is_readonly_section(section), "{section:?} 应可编辑");
    }
}
