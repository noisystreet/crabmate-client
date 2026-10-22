//! `state.rs` 的单测（`#[path]` 子模块）：压住单文件行数门禁。

use super::*;

#[test]
fn stream_delta_merges_same_kind() {
    let mut s = UiState::new();
    s.stream_delta(LineKind::Assistant, "你");
    s.stream_delta(LineKind::Assistant, "好");
    assert_eq!(s.lines.len(), 1);
    assert_eq!(s.lines[0].text(), "你好");
}

#[test]
fn stream_delta_breaks_on_kind_change() {
    let mut s = UiState::new();
    s.stream_delta(LineKind::Thinking, "think");
    s.stream_delta(LineKind::Assistant, "hi");
    assert_eq!(s.lines.len(), 2);
    assert_eq!(s.lines[0].kind(), LineKind::Thinking);
    assert_eq!(s.lines[1].kind(), LineKind::Assistant);
}

#[test]
fn system_never_merges() {
    let mut s = UiState::new();
    s.push_line(LineKind::System, "a");
    s.push_line(LineKind::System, "b");
    assert_eq!(s.lines.len(), 2);
}

#[test]
fn input_insert_and_backspace() {
    let mut s = UiState::new();
    s.insert_char('a');
    s.insert_char('b');
    assert_eq!(s.current_input(), "ab");
    s.cursor_left();
    s.insert_char('X');
    assert_eq!(s.current_input(), "aXb");
    s.cursor_left();
    s.backspace();
    assert_eq!(s.current_input(), "Xb");
}

#[test]
fn insert_workspace_path_separates_and_appends_space() {
    let mut s = UiState::new();
    s.insert_workspace_path("src/main.rs");
    assert_eq!(s.current_input(), "src/main.rs ", "末尾补空格便于继续输入");
    // 已有文字且末尾无空白 → 先补一个分隔空格
    for ch in "看".chars() {
        s.insert_char(ch);
    }
    s.insert_workspace_path("README.md");
    assert_eq!(s.current_input(), "src/main.rs 看 README.md ");
}

#[test]
fn insert_workspace_path_keeps_space_on_both_sides() {
    let mut s = UiState::new();
    for ch in "看图".chars() {
        s.insert_char(ch);
    }
    // 光标在开头：右侧要补空格，避免路径与原文粘连
    s.cursor_home();
    s.insert_workspace_path("src/main.rs");
    assert_eq!(s.current_input(), "src/main.rs 看图");

    // 光标在词中间：两侧都补空格
    let mut s = UiState::new();
    for ch in "ab".chars() {
        s.insert_char(ch);
    }
    s.cursor_left();
    s.insert_workspace_path("x/y");
    assert_eq!(s.current_input(), "a x/y b");
}

#[test]
fn insert_workspace_path_empty_is_noop() {
    let mut s = UiState::new();
    s.insert_workspace_path("");
    assert_eq!(s.current_input(), "");
}

#[test]
fn multiline_insert_newline_and_lines() {
    let mut s = UiState::new();
    for ch in "ab".chars() {
        s.insert_char(ch);
    }
    s.insert_newline();
    for ch in "cd".chars() {
        s.insert_char(ch);
    }
    assert_eq!(s.current_input(), "ab\ncd");
    assert_eq!(s.input_line_count(), 2);
    assert_eq!(s.input_lines(), vec!["ab", "cd"]);
    assert_eq!(s.cursor_line_col(), (1, 2));
}

#[test]
fn multiline_home_end_per_line() {
    let mut s = UiState::new();
    for ch in "ab\ncd".chars() {
        s.insert_char(ch);
    }
    // 光标在第 1 行 "cd" 末尾
    s.cursor_home();
    assert_eq!(s.cursor_line_col(), (1, 0));
    s.cursor_end();
    assert_eq!(s.cursor_line_col(), (1, 2));
    s.cursor_left();
    assert_eq!(s.cursor_line_col(), (1, 1));
}

#[test]
fn multiline_cursor_vertical_moves_and_clamps() {
    let mut s = UiState::new();
    for ch in "abc\nde\nfgh".chars() {
        s.insert_char(ch);
    }
    // 从光标行（行 2）行首出发，一路 Home 到行 0
    s.cursor_home();
    assert_eq!(s.cursor_line_col(), (2, 0));
    s.move_cursor_vert(true);
    assert_eq!(s.cursor_line_col(), (1, 0));
    s.move_cursor_vert(true);
    assert_eq!(s.cursor_line_col(), (0, 0));
    // 顶部再上移不变
    s.move_cursor_vert(true);
    assert_eq!(s.cursor_line_col(), (0, 0));
    // 行尾(0,3)下移 → 行1 col min(3,2)=2；上移回到行0 col2
    s.cursor_end();
    s.move_cursor_vert(false);
    assert_eq!(s.cursor_line_col(), (1, 2));
    s.move_cursor_vert(true);
    assert_eq!(s.cursor_line_col(), (0, 2));
    // 一路下到行 2，行底再下移不变
    s.cursor_end();
    s.move_cursor_vert(false);
    assert_eq!(s.cursor_line_col(), (1, 2));
    s.move_cursor_vert(false);
    assert_eq!(s.cursor_line_col(), (2, 2));
    s.cursor_end();
    assert_eq!(s.cursor_line_col(), (2, 3));
    s.move_cursor_vert(false);
    assert_eq!(s.cursor_line_col(), (2, 3), "bottom clamps");
}

#[test]
fn take_input_clears() {
    let mut s = UiState::new();
    s.insert_char('h');
    assert_eq!(s.take_input(), "h");
    assert_eq!(s.current_input(), "");
}

fn session(id: &str, conv: Option<&str>) -> SessionListRow {
    SessionListRow {
        id: id.to_string(),
        title: String::new(),
        updated_at: 0,
        pinned: false,
        starred: false,
        server_conversation_id: conv.map(str::to_string),
        server_revision: None,
        workspace_root: None,
    }
}

#[test]
fn sessions_clamp_selection() {
    let mut s = UiState::new();
    s.selected = 5;
    let rows = [session("a", None), session("b", Some("c2"))];
    s.replace_sessions(rows.to_vec());
    assert_eq!(s.selected, 1);
}

#[test]
fn session_selection_moves() {
    let mut s = UiState::new();
    let rows = [session("a", None), session("b", None)];
    s.replace_sessions(rows.to_vec());
    s.move_selection(false);
    assert_eq!(s.selected, 1);
    s.move_selection(false);
    assert_eq!(s.selected, 1, "clamped at bottom");
    s.move_selection(true);
    assert_eq!(s.selected, 0);
    s.move_selection(true);
    assert_eq!(s.selected, 0, "clamped at top");
}

#[test]
fn selected_resume_ignores_missing() {
    let mut s = UiState::new();
    let rows = [session("a", None), session("b", Some(" c9 "))];
    s.replace_sessions(rows.to_vec());
    assert_eq!(s.selected_resume(), None, "row a has no conv");
    s.move_selection(false);
    assert_eq!(s.selected_resume(), Some("c9".to_string()));
}

#[test]
fn row_in_use_marks_current_conv() {
    let mut s = UiState::new();
    s.conversation_id = Some("c2".to_string());
    let rows = [session("a", Some("c1")), session("b", Some("c2"))];
    assert!(!s.row_in_use(&rows[0]));
    assert!(s.row_in_use(&rows[1]));
}

#[test]
fn reset_transcript_clears_lines() {
    let mut s = UiState::new();
    s.push_line(LineKind::User, "hi");
    s.view_offset = 3;
    s.tool_start("tc-1", "exec", None);
    s.start_search("hi");
    s.reset_transcript();
    assert!(s.lines.is_empty());
    assert_eq!(s.view_offset, 0);
    assert!(!s.search_active());
}

#[test]
fn thinking_collapsed_by_default_and_toggles() {
    let mut s = UiState::new();
    s.push_line(LineKind::Thinking, "deep thought");
    assert!(s.lines[0].is_collapsed());
    assert!(!s.thinking_visible());
    s.toggle_thinking();
    assert!(s.thinking_visible());
    assert!(!s.lines[0].is_collapsed());
    s.toggle_thinking();
    assert!(s.lines[0].is_collapsed());
}

#[test]
fn new_thinking_rows_follow_global_expand_state() {
    let mut s = UiState::new();
    s.push_line(LineKind::User, "q");
    // 先展开（此时还没有 thinking 行）
    s.toggle_thinking();
    // User 行之后新开的 thinking 块应保持展开
    s.stream_delta(LineKind::Thinking, "思考a");
    assert!(!s.lines.last().unwrap().is_collapsed());
    // 同块续接仍展开
    s.stream_delta(LineKind::Thinking, "思考b");
    assert!(!s.lines.last().unwrap().is_collapsed());
    // 跨 Tool 行后再次出现的 thinking 块也应展开
    s.push_line(LineKind::Tool, "exec");
    s.stream_delta(LineKind::Thinking, "思考c");
    assert!(!s.lines.last().unwrap().is_collapsed());
    // 折叠后新建的 thinking 块恢复默认折叠
    s.toggle_thinking();
    s.push_line(LineKind::System, "x");
    s.stream_delta(LineKind::Thinking, "思考d");
    assert!(s.lines.last().unwrap().is_collapsed());
}

/// 开始态就该看到「要执行什么」，不必等结果回来才补。
///
/// `run_command` 连名字一起省掉：note 就是那条命令，前缀工具名只是噪音。
#[test]
fn tool_start_shows_note_immediately() {
    let mut s = UiState::new();
    s.tool_start("tc-2", "run_command", Some("git status"));
    assert_eq!(s.lines.last().unwrap().text(), "git status");
    s.tool_end("tc-2", "run_command", Some(true), Some("git status"));
    assert_eq!(s.lines.last().unwrap().text(), "✓ git status");
}

#[test]
fn tool_row_start_then_end_updates_in_place() {
    let mut s = UiState::new();
    s.tool_start("tc-9", "exec", None);
    assert_eq!(s.lines.last().unwrap().kind(), LineKind::Tool);
    assert_eq!(s.lines.last().unwrap().text(), "exec");
    s.tool_end("tc-9", "exec", Some(true), Some("exit 0"));
    assert_eq!(s.lines.last().unwrap().text(), "✓ exec — exit 0");
    assert_eq!(s.lines.len(), 1);
}

#[test]
fn tool_end_without_start_pushes_fallback_row() {
    let mut s = UiState::new();
    s.tool_end("tc-miss", "patch", Some(false), Some("rejected"));
    assert_eq!(s.lines.last().unwrap().text(), "✗ patch — rejected");
}

#[test]
fn search_start_finds_and_highlights_roundtrip() {
    let mut s = UiState::new();
    s.push_line(LineKind::User, "alpha");
    s.push_line(LineKind::Assistant, "Beta beta");
    s.push_line(LineKind::System, "gamma");
    assert_eq!(s.start_search("beta"), 1);
    assert_eq!(s.search_cursor, Some(1));
    assert_eq!(s.search_total, 1);
    // 下一个命中循环回第一个
    assert_eq!(s.next_search_hit(), Some(1));
    s.clear_search();
    assert!(!s.search_active());
}

#[test]
fn search_skips_non_matching_lines() {
    let mut s = UiState::new();
    s.push_line(LineKind::User, "x");
    s.push_line(LineKind::Assistant, "你好世界");
    assert_eq!(s.start_search("世界"), 1);
    assert_eq!(s.search_cursor, Some(1));
}

#[test]
fn search_total_refreshes_on_new_content() {
    let mut s = UiState::new();
    s.push_line(LineKind::User, "x");
    assert_eq!(s.start_search("boom"), 0);
    assert_eq!(s.search_total, 0);
    // 流式新内容命中：状态行计数应立刻刷新（跳转仍需用户 /find）
    s.stream_delta(LineKind::Assistant, "boom now");
    assert_eq!(s.search_total, 1);
    assert_eq!(s.search_cursor, None);
    // 同块续接命中次数不变
    s.stream_delta(LineKind::Assistant, " boom");
    assert_eq!(s.search_total, 1);
    // 工具行摘要更新为命中（原位文本变更也刷新）
    s.push_line(LineKind::Tool, "exec");
    assert_eq!(s.search_total, 1);
    s.tool_end("tc-1", "exec", Some(true), Some("boom done"));
    assert_eq!(s.search_total, 2);
}

#[test]
fn serve_defaults_parse_status() {
    let v = serde_json::json!({
        "model": "gpt-x",
        "default_agent_role_id": "coder",
        "default_session_mode": "plan",
        "api_base": "http://x",
    });
    let d = ServeDefaults::from_status(&v);
    assert_eq!(d.model.as_deref(), Some("gpt-x"));
    assert_eq!(d.role.as_deref(), Some("coder"));
    assert_eq!(d.mode.as_deref(), Some("plan"));
}

#[test]
fn serve_defaults_skip_blank() {
    let v = serde_json::json!({"model": "  "});
    let d = ServeDefaults::from_status(&v);
    assert_eq!(d.model, None);
}
