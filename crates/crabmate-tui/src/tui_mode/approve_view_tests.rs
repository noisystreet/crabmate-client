//! 审批浮层「看完整命令」的单测（`#[path]` 子模块）。
//!
//! 与 `approve.rs` 里的 `tests` 分开：那个模块已被 lizard 解析成一大段函数体
//! （`decision_keys_map_like_repl_aliases` 单独一列就报 244 行），再往里加用例会直接
//! 顶穿 fn-nloc 门禁。与 `state_paste_tests.rs` 同一个理由。

use super::*;
use crate::serve::ApprovalDecision;
use crate::tui_mode::state::ApprovalOverlay;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::sync::mpsc;

fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

/// 展开态的浮层对象（只需要通道占位，不参与这些用例）。
fn overlay_with(command: &str) -> ApprovalOverlay {
    let (tx, _rx) = mpsc::channel();
    ApprovalOverlay {
        command: command.to_string(),
        args: String::new(),
        allowlist_key: None,
        answer: tx,
        expanded: false,
        scroll: 0,
    }
}

#[test]
fn ctrl_e_toggles_expanded_and_resets_scroll() {
    let mut ap = overlay_with("rm -rf build");
    ap.expanded = true;
    ap.scroll = 5;
    let ctrl_e = key(KeyCode::Char('e'), KeyModifiers::CONTROL);
    assert!(approval_view_key(&ctrl_e, &mut ap, (40, 10)));
    assert!(!ap.expanded, "Ctrl+E 应收起");
    assert_eq!(ap.scroll, 0, "收起要回到第一页");
    assert!(approval_view_key(&ctrl_e, &mut ap, (40, 10)));
    assert!(ap.expanded, "再按一次应展开");
}

#[test]
fn scroll_keys_are_inert_until_expanded() {
    let mut ap = overlay_with("ls");
    // 收起态：↑/↓ 不被消费，否则会把别的键从浮层漏给输入框。
    assert!(!approval_view_key(
        &key(KeyCode::Down, KeyModifiers::NONE),
        &mut ap,
        (40, 10)
    ));
    assert_eq!(ap.scroll, 0);
}

#[test]
fn scroll_clamps_to_the_last_page() {
    let mut ap = overlay_with("bash -c 'for i in 1 2 3; do echo $i; done'");
    ap.expanded = true;
    // 总 40 行、可见 10 行 → 最后可滚起点是 30。
    let bounds = (40, 10);
    assert!(approval_view_key(
        &key(KeyCode::End, KeyModifiers::NONE),
        &mut ap,
        bounds
    ));
    assert_eq!(ap.scroll, 30, "End 停在最后一页的起点");
    // 关键回归：End 之后按 ↑ 必须能往回走（若 scroll 停在越界值上，这里会失效）。
    assert!(approval_view_key(
        &key(KeyCode::Up, KeyModifiers::NONE),
        &mut ap,
        bounds
    ));
    assert_eq!(ap.scroll, 29);
    for _ in 0..20 {
        approval_view_key(&key(KeyCode::PageDown, KeyModifiers::NONE), &mut ap, bounds);
    }
    assert_eq!(ap.scroll, 30, "越界翻页要夹住");
    assert!(approval_view_key(
        &key(KeyCode::Char('k'), KeyModifiers::NONE),
        &mut ap,
        bounds
    ));
    assert_eq!(ap.scroll, 29, "vim 的 k 与 ↑ 同义");
    assert!(approval_view_key(
        &key(KeyCode::Home, KeyModifiers::NONE),
        &mut ap,
        bounds
    ));
    assert_eq!(ap.scroll, 0);
}

#[test]
fn decision_keys_still_decide_while_expanded() {
    let mut ap = overlay_with("ls");
    ap.expanded = true;
    let mut choice = ApprovalChoice::default();
    // 展开态下滚动键被 `approval_view_key` 吃掉，但 Enter / Esc 仍走决策。
    assert!(!approval_view_key(
        &key(KeyCode::Enter, KeyModifiers::NONE),
        &mut ap,
        (40, 10)
    ));
    assert_eq!(
        decision_for_key(&key(KeyCode::Enter, KeyModifiers::NONE), &mut choice),
        Some(ApprovalDecision::AllowOnce)
    );
    assert_eq!(
        decision_for_key(&key(KeyCode::Esc, KeyModifiers::NONE), &mut choice),
        Some(ApprovalDecision::Deny)
    );
}
