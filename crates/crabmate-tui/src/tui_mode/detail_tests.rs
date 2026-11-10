//! `/detail` 浮层的用例：参数解析、选行、滚动夹取、关闭键。
//!
//! 这里不构造 `LaneApp`（它要整条 lane 的依赖）：`on_detail_key` 只是
//! 「关闭判定 + `step_scroll` + 夹上界」的胶水，三块都单独测到了。真渲染（`TestBackend`）
//! 不在单测范围，见 PR 的手动冒烟步骤。

use super::super::state::{DetailOverlay, ToolDetail};
use super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

/// 缺省 `.detail` = 最近一条；`<n>` 从底部数。
#[test]
fn open_picks_the_nth_from_the_bottom() {
    let mut st = UiState::new();
    st.push_line(LineKind::Tool, "✓ 第一条");
    st.push_line(LineKind::Assistant, "答案");
    st.push_line(LineKind::Tool, "✓ 第三条");
    for row in [0usize, 2] {
        st.tool_details.insert(
            row,
            ToolDetail {
                text: format!("out-{row}"),
                truncated: false,
            },
        );
    }

    st.open_detail_command(None);
    assert_eq!(
        st.detail.map(|d| d.line),
        Some(2),
        "缺省 = 最近一条带详情的"
    );

    st.open_detail_command(Some("2"));
    assert_eq!(st.detail.map(|d| d.line), Some(0), "从底部数第 2 条");
}

/// 打不开的三种情形各自都要**说话**（命令已经从输入框消失，静默等于没发生）。
#[test]
fn open_reports_each_way_of_failing() {
    let mut st = UiState::new();
    st.open_detail_command(None);
    assert!(st.detail.is_none());
    assert!(st.lines.last().unwrap().text().contains("没有可展开"));

    st.push_line(LineKind::Tool, "✓ exec");
    st.tool_details.insert(
        0,
        ToolDetail {
            text: "out".to_string(),
            truncated: false,
        },
    );
    st.open_detail_command(Some("2"));
    assert!(st.detail.is_none());
    assert!(st.lines.last().unwrap().text().contains("只有 1 条"));

    st.open_detail_command(Some("x"));
    assert!(st.lines.last().unwrap().text().contains("用法"));
}

/// 打开时释放搜索锚点：否则浮层开着时画面还被 `/find` 钉在命中行上。
#[test]
fn open_releases_the_search_anchor() {
    let mut st = UiState::new();
    st.push_line(LineKind::Tool, "✓ exec");
    st.tool_details.insert(
        0,
        ToolDetail {
            text: "out".to_string(),
            truncated: false,
        },
    );
    st.search_cursor = Some(0);

    st.open_detail_command(None);

    assert!(st.search_cursor.is_none());
}

/// 滚动键逐行 / 翻页 / 首尾，且一律夹在 `[0, max_scroll]`。
#[test]
fn scroll_keys_are_clamped() {
    let max = 10;
    assert_eq!(step_scroll(3, &key(KeyCode::Up), max), 2);
    assert_eq!(step_scroll(3, &key(KeyCode::Char('k')), max), 2);
    assert_eq!(step_scroll(3, &key(KeyCode::Down), max), 4);
    assert_eq!(step_scroll(3, &key(KeyCode::Char('j')), max), 4);
    assert_eq!(
        step_scroll(3, &key(KeyCode::PageDown), 100),
        3 + DETAIL_PAGE_STEP
    );
    assert_eq!(
        step_scroll(9, &key(KeyCode::PageDown), max),
        max,
        "翻页也要夹住"
    );
    assert_eq!(step_scroll(1, &key(KeyCode::Home), max), 0);
    assert_eq!(step_scroll(0, &key(KeyCode::End), max), max);
    assert_eq!(
        step_scroll(0, &key(KeyCode::Char('z')), max),
        0,
        "非滚动键不动"
    );
}

/// 带 `Ctrl` 的组合不算单键滚动（`Ctrl+E` / `Ctrl+G` 有各自语义）。
#[test]
fn control_modified_keys_do_not_scroll() {
    let ctrl_j = KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL);
    assert_eq!(step_scroll(5, &ctrl_j, 10), 5);
}

/// `Esc` 与裸 `q` 关，`Ctrl+Q` 不关（那是进程级退出）。
#[test]
fn close_keys() {
    assert!(detail_key_closes(&key(KeyCode::Esc)));
    assert!(detail_key_closes(&key(KeyCode::Char('q'))));
    assert!(!detail_key_closes(&KeyEvent::new(
        KeyCode::Char('q'),
        KeyModifiers::CONTROL
    )));
    assert!(!detail_key_closes(&key(KeyCode::Char('j'))));
}

/// 太窄 / 太矮时不铺浮层：返回 `(0, 0)`，渲染侧据此直接跳过。
#[test]
fn tiny_area_has_no_bounds() {
    let mut st = UiState::new();
    st.push_line(LineKind::Tool, "✓ exec");
    st.tool_details.insert(
        0,
        ToolDetail {
            text: "out".to_string(),
            truncated: false,
        },
    );
    st.detail = Some(DetailOverlay { line: 0, scroll: 0 });

    assert_eq!(detail_scroll_bounds(Rect::new(0, 0, 10, 5), &st), (0, 0));
    let (total, body) = detail_scroll_bounds(Rect::new(0, 0, 80, 24), &st);
    assert!(total >= 1, "正常尺寸下正文至少一行");
    assert!(body >= 1);
}

/// 侧表里没有这一行（回放 / 已被预算淘汰）：不报错、不画，bounds 为 0。
#[test]
fn missing_entry_yields_no_bounds() {
    let mut st = UiState::new();
    st.push_line(LineKind::Tool, "✓ exec");
    st.detail = Some(DetailOverlay { line: 0, scroll: 0 });
    assert_eq!(detail_scroll_bounds(Rect::new(0, 0, 80, 24), &st), (0, 0));
}
