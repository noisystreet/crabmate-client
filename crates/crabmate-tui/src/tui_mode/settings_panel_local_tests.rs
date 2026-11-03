//! 只读三栏（连接 / 快捷键 / 本地行为）的单测。
//!
//! 这三栏的共同契约：**Enter 永远不改状态**（无编辑态、不变脏），值本身就是说明；
//! 拿不到 serve 事实时写「未提供」，不假装知道。

use super::*;
use crate::serve::BearerSource;
use crate::tui_mode::hotkeys::{self, HotkeyGroup};
use crate::tui_mode::settings_panel_env::EnvSwitch;
use unicode_width::UnicodeWidthStr;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn overrides() -> SessionPrefs {
    SessionPrefs::from_options(None, None, None)
}

/// 不带连接事实、不带 serve 快照的上下文（三层合成与只读文案的基准情形）。
fn ctx(o: &SessionPrefs) -> PanelCtx<'_> {
    PanelCtx {
        overrides: o,
        persisted: None,
        serve_defaults: None,
        conn: None,
        pal: &Palette::DARK,
    }
}

/// 一行行拼起来的整栏文本（快捷键值会折行，断言要跨行看）。
fn section_text(section: Section, c: &PanelCtx<'_>) -> String {
    row_texts(&panel_on(section), c).join("\n")
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
        conn: None,
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
        conn: None,
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
        conn: None,
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
        conn: None,
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

/// 连接栏读的是**本连接**的事实：api_base / lane 名与序号 / Bearer 来源。
///
/// 「连错 serve」「以为带了 token 其实没带」正是这一栏要回答的两个问题，所以三者
/// 都必须来自真实对象，而不是面板自己再推断一遍。
#[test]
fn connection_rows_report_lane_and_bearer_source() {
    let o = overrides();
    let facts = ConnectionFacts {
        api_base: "http://127.0.0.1:8080".to_string(),
        label: "proj".to_string(),
        pos: LanePos { index: 2, total: 3 },
        bearer: BearerSource::Keyring,
    };
    let c = PanelCtx {
        overrides: &o,
        persisted: None,
        serve_defaults: None,
        conn: Some(&facts),
        pal: &Palette::DARK,
    };
    let text = section_text(Section::Connection, &c);
    assert!(
        text.contains("http://127.0.0.1:8080"),
        "要显示生效 api_base：{text}"
    );
    assert!(
        text.contains("proj") && text.contains("第 2/3 条"),
        "lane 名与序号要能对上左栏那一行：{text}"
    );
    assert!(text.contains("本机钥匙串"), "要报 Bearer 来源：{text}");
}

/// 三种来源各自说清；且**任何一种都不带值**（这一栏只报来源）。
#[test]
fn bearer_row_names_every_source_without_the_value() {
    let o = overrides();
    for (source, phrase) in [
        (BearerSource::Cli, "启动参数或环境"),
        (BearerSource::Keyring, "本机钥匙串"),
        (BearerSource::Missing, "未提供"),
    ] {
        let facts = ConnectionFacts {
            api_base: "http://127.0.0.1:8080".to_string(),
            label: "local".to_string(),
            pos: LanePos::default(),
            bearer: source,
        };
        let c = PanelCtx {
            overrides: &o,
            persisted: None,
            serve_defaults: None,
            conn: Some(&facts),
            pal: &Palette::DARK,
        };
        let text = section_text(Section::Connection, &c);
        assert!(
            text.contains(phrase),
            "{source:?} 应写成「{phrase}」：{text}"
        );
    }
}

/// 拿不到连接事实时不猜：三行各写一句「未提供」。
#[test]
fn connection_rows_say_unknown_without_facts() {
    let o = overrides();
    let text = section_text(Section::Connection, &ctx(&o));
    for what in ["lane", "api_base", "Bearer"] {
        assert!(
            text.contains(&format!("({what} 未提供)")),
            "缺少「{what} 未提供」：{text}"
        );
    }
}

/// 快捷键栏**由热键表生成**：表里每一项都出现在面板里。
///
/// 这就是 #433 的成功判据 —— 新增一个顶层热键只改 `hotkeys::TopHotkey::ALL`，面板与
/// `/help` 自动跟上；反过来，若谁把手写字符串塞回来，这条会立刻红。
#[test]
fn shortcut_rows_come_from_the_hotkey_table() {
    let o = overrides();
    let text = section_text(Section::Shortcuts, &ctx(&o));
    for group in HotkeyGroup::ALL {
        for item in hotkeys::group_items(group) {
            assert!(text.contains(&item), "快捷键栏缺少 `{item}`：{text}");
        }
    }
}

/// 80 列也不截断：值折行，每一项仍完整出现，且没有一行超宽。
///
/// 这条栏的值**就是**事实 —— 旧实现在 80 列下被 `truncate_display` 截掉尾几个键，
/// 用户看到的是一份不完整的键位表。
#[test]
fn shortcut_rows_wrap_instead_of_truncating_at_80_columns() {
    let o = overrides();
    let content = panel_on(Section::Shortcuts).content(&ctx(&o), 80);
    let texts: Vec<String> = content
        .lines
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
        .collect();
    let text = texts.join("\n");
    for group in HotkeyGroup::ALL {
        for item in hotkeys::group_items(group) {
            assert!(text.contains(&item), "80 列下丢了 `{item}`：{texts:#?}");
        }
    }
    for line in &texts {
        assert!(
            UnicodeWidthStr::width(line.as_str()) <= 80,
            "折行后仍超宽：{line:?}"
        );
    }
}

/// 本地行为栏展示 `CM_TUI_*` 开关的**生效值**，并把环境变量名与值写在同一行。
#[test]
fn local_rows_show_env_switch_values() {
    let o = overrides();
    let text = section_text(Section::Local, &ctx(&o));
    for switch in EnvSwitch::ALL {
        assert!(
            text.contains(switch.env_name()),
            "缺少 {}：{text}",
            switch.env_name()
        );
    }
    // 「改需重启」只交代一次（分区说明行），不逐行重复。
    assert!(text.contains("改需重启"), "要标出改需重启：{text}");
}

/// 折行内核：先按 ` · ` 分项装行，单项超宽再硬折，且**不丢字**。
#[test]
fn wrap_note_packs_items_then_hard_wraps() {
    assert_eq!(super::draw::wrap_note("a · b · c", 5), vec!["a · b", "c"]);
    // 单项超宽：硬折，拼回去仍是原文（不截断）。
    let long = "x".repeat(9);
    let chunks = super::draw::wrap_note(&long, 4);
    assert_eq!(chunks.concat(), long);
    assert!(
        chunks
            .iter()
            .all(|c| UnicodeWidthStr::width(c.as_str()) <= 4)
    );
}
