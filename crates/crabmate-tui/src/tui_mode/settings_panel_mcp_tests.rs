//! MCP 分区的单测：只读信息行、两个可编辑标量、以及与「该 serve 未提供」的区分。
//!
//! 断言走 `content()`（用户真正看到的那几行），不碰 `draw` 的私有项。

use super::*;
use crate::serve::{LlmOverridesDto, McpServersFileDto, UserPrefsDto};
use crate::tui_mode::settings::merge_mcp_save;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

/// 造一个「服务端有 MCP 配置」的持久层快照。
fn persisted_mcp(global_enabled: bool, timeout: u64, servers: usize) -> PersistedSettings {
    let list: Vec<serde_json::Value> = (0..servers)
        .map(|i| {
            serde_json::json!({
                "id": format!("srv-{i}"),
                "name": format!("srv-{i}"),
                "enabled": i == 0,
            })
        })
        .collect();
    let dto: McpServersFileDto = serde_json::from_value(serde_json::json!({
        "global_enabled": global_enabled,
        "tool_timeout_secs": timeout,
        "servers": list,
    }))
    .expect("mcp dto");
    PersistedSettings::from_snapshot(
        &UserPrefsDto::default(),
        &LlmOverridesDto::default(),
        Some(&dto),
    )
}

fn overrides() -> SessionPrefs {
    SessionPrefs::from_options(None, None, None)
}

fn panel_on_mcp(field: FieldId) -> SettingsPanel {
    let mut p = SettingsPanel::new(false);
    p.section = Section::Mcp;
    p.row = Section::Mcp
        .fields()
        .iter()
        .position(|f| *f == field)
        .expect("MCP 分区必须含该行");
    p
}

fn row_text(p: &SettingsPanel, c: &PanelCtx<'_>, field: FieldId) -> String {
    let label = field_label(field);
    p.content(c, 90)
        .lines
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
        })
        .find(|t| t.starts_with(label))
        .unwrap_or_default()
}

#[test]
fn rows_show_the_server_side_values() {
    let o = overrides();
    let persisted = persisted_mcp(true, 45, 3);
    let c = PanelCtx {
        overrides: &o,
        persisted: Some(&persisted),
        serve_defaults: None,
        pal: &Palette::DARK,
    };
    let p = panel_on_mcp(FieldId::McpGlobalEnabled);
    assert!(row_text(&p, &c, FieldId::McpGlobalEnabled).contains('开'));
    assert!(row_text(&p, &c, FieldId::McpToolTimeout).contains("45 秒"));
    let servers = row_text(&p, &c, FieldId::McpServers);
    assert!(servers.contains("3 个"), "实际：{servers}");
    assert!(
        servers.contains("1 启用"),
        "只数 enabled=true 的：{servers}"
    );
    assert!(servers.contains("Desktop/Web"), "要说清编辑入口：{servers}");
}

#[test]
fn missing_mcp_endpoint_is_flagged_not_assumed_off() {
    let o = overrides();
    let c = PanelCtx {
        overrides: &o,
        persisted: None,
        serve_defaults: None,
        pal: &Palette::DARK,
    };
    for field in [
        FieldId::McpGlobalEnabled,
        FieldId::McpToolTimeout,
        FieldId::McpServers,
    ] {
        let p = panel_on_mcp(field);
        assert!(
            row_text(&p, &c, field).contains("(该 serve 未提供)"),
            "{field:?} 应显示「未提供」而不是假装知道值"
        );
    }
}

#[test]
fn toggle_edit_commits_off_into_the_mcp_group() {
    let o = overrides();
    let persisted = persisted_mcp(true, 60, 1);
    let c = PanelCtx {
        overrides: &o,
        persisted: Some(&persisted),
        serve_defaults: None,
        pal: &Palette::DARK,
    };
    let mut p = panel_on_mcp(FieldId::McpGlobalEnabled);
    p.handle_key(&key(KeyCode::Enter), &c);
    assert!(matches!(
        p.editing,
        Some(Editing::Toggle {
            field: FieldId::McpGlobalEnabled,
            pick: 0
        })
    ));
    p.handle_key(&key(KeyCode::Right), &c);
    p.handle_key(&key(KeyCode::Enter), &c);
    assert!(p.is_dirty());
    let PanelEffect::Save { mcp, .. } = p.handle_key(&key(KeyCode::Char('s')), &c) else {
        panic!("S 应产生保存请求");
    };
    assert_eq!(
        mcp.global_enabled,
        FieldAction::Write(Some(STORE_OFF.to_string()))
    );
    assert!(!mcp.tool_timeout.is_write(), "只改开关时不该带上超时");
}

#[test]
fn toggle_without_a_known_value_stays_in_browse_mode() {
    let o = overrides();
    let c = PanelCtx {
        overrides: &o,
        persisted: None,
        serve_defaults: None,
        pal: &Palette::DARK,
    };
    let mut p = panel_on_mcp(FieldId::McpGlobalEnabled);
    assert!(matches!(
        p.handle_key(&key(KeyCode::Enter), &c),
        PanelEffect::None
    ));
    assert!(p.editing.is_none());
    assert!(p.note.is_some(), "应给出「未提供」提示");
}

#[test]
fn timeout_edit_prefills_and_rejects_zero() {
    let o = overrides();
    let persisted = persisted_mcp(true, 60, 0);
    let c = PanelCtx {
        overrides: &o,
        persisted: Some(&persisted),
        serve_defaults: None,
        pal: &Palette::DARK,
    };
    let mut p = panel_on_mcp(FieldId::McpToolTimeout);
    // 预填当前值
    p.handle_key(&key(KeyCode::Enter), &c);
    let Some(Editing::Text { buf, .. }) = &p.editing else {
        panic!("超时行应进文本编辑");
    };
    assert_eq!(buf.iter().collect::<String>(), "60");
    // 清空后输入 0：非法，留在编辑态并提示
    for _ in 0..2 {
        p.handle_key(&key(KeyCode::Backspace), &c);
    }
    p.handle_key(&key(KeyCode::Char('0')), &c);
    p.handle_key(&key(KeyCode::Enter), &c);
    assert!(p.editing.is_some(), "0 秒应被拒（会立刻超时）");
    assert!(!p.is_dirty());
    // 改成 30：合法
    p.handle_key(&key(KeyCode::Backspace), &c);
    p.handle_key(&key(KeyCode::Char('3')), &c);
    p.handle_key(&key(KeyCode::Char('0')), &c);
    p.handle_key(&key(KeyCode::Enter), &c);
    assert!(p.editing.is_none());
    assert_eq!(
        p.staged(FieldId::McpToolTimeout),
        &FieldAction::Write(Some("30".to_string()))
    );
}

#[test]
fn server_list_row_is_read_only() {
    let o = overrides();
    let persisted = persisted_mcp(true, 60, 2);
    let c = PanelCtx {
        overrides: &o,
        persisted: Some(&persisted),
        serve_defaults: None,
        pal: &Palette::DARK,
    };
    let mut p = panel_on_mcp(FieldId::McpServers);
    assert!(matches!(
        p.handle_key(&key(KeyCode::Enter), &c),
        PanelEffect::None
    ));
    assert!(p.editing.is_none(), "只读行不该进编辑态");
    assert!(!p.is_dirty());
    let (note, _) = p.note.clone().expect("应提示编辑入口");
    assert!(note.contains("Desktop"), "实际提示：{note}");
}

#[test]
fn merge_keeps_the_server_list_and_schema_version() {
    let base: McpServersFileDto = serde_json::from_value(serde_json::json!({
        "schema_version": 2,
        "global_enabled": true,
        "tool_timeout_secs": 60,
        "servers": [{"id": "a", "enabled": true, "env": {"TOKEN": "x"}}],
    }))
    .expect("dto");
    let save = McpSave {
        global_enabled: FieldAction::Write(Some(STORE_OFF.to_string())),
        tool_timeout: FieldAction::Write(Some("120".to_string())),
    };
    let merged = merge_mcp_save(base.clone(), &save);
    assert!(!merged.global_enabled);
    assert_eq!(merged.tool_timeout_secs, 120);
    assert_eq!(merged.schema_version, Some(2), "格式版本原样带回");
    assert_eq!(merged.servers, base.servers, "条目（含 env）必须一字不改");
}
