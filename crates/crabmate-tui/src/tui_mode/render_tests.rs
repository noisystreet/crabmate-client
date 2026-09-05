use super::*;
use crabmate_tui_core::SessionListItem;

#[test]
fn truncate_appends_ellipsis() {
    assert_eq!(truncate_display("abcdef", 4), "abcd…");
    assert_eq!(truncate_display("你好", 3), "你…");
    assert_eq!(truncate_display("", 4), "");
}

#[test]
fn display_lines_prefix_only_first_physical() {
    let mut st = UiState::new();
    st.push_line(LineKind::Assistant, "一二三四五六七八九十");
    let lines = display_lines(&st, 4);
    assert!(lines.len() >= 2);
    assert_eq!(lines[0].spans.len(), 2);
    assert_eq!(lines[1].spans.len(), 1);
}

#[test]
fn thinking_collapsed_shows_single_fold_row() {
    let mut st = UiState::new();
    st.stream_delta(LineKind::Thinking, "第一行思考\n第二行很长很长的思考内容");
    let rows = body_rows(&st, 60);
    assert_eq!(rows.len(), 1, "折叠后仅一行预览");
    assert!(rows[0].line.to_string().contains('…'));
}

#[test]
fn thinking_expanded_wraps_multirow() {
    let mut st = UiState::new();
    st.stream_delta(LineKind::Thinking, "第一行思考\n第二行思考");
    st.toggle_thinking();
    let rows = body_rows(&st, 60);
    assert!(rows.len() >= 2, "展开后至少两行");
}

#[test]
fn search_marks_and_anchors_rows() {
    let mut st = UiState::new();
    st.push_line(LineKind::Assistant, "hello world");
    st.push_line(LineKind::User, "bye");
    st.start_search("hello");
    let rows = body_rows(&st, 60);
    // 锚定行第一物理行以黄底反色突出
    assert!(
        rows[0]
            .line
            .spans
            .iter()
            .any(|s| s.style.bg == Some(Color::Yellow))
    );
    // 非命中行不带黄色前景
    assert!(
        rows[1]
            .line
            .spans
            .iter()
            .all(|s| s.style.fg != Some(Color::Yellow))
    );
}

#[test]
fn chat_body_width_matches_layout() {
    assert_eq!(chat_body_width(false, 100), 99);
    // 宽屏：左右各一列（会话 + 工作区），再留 1 列正文间距。
    assert_eq!(
        chat_body_width(true, 100),
        100 - SIDEBAR_WIDTH as usize * 2 - 1
    );
}

#[test]
fn thinking_and_answer_do_not_add_blank_between() {
    let mut st = UiState::new();
    st.push_line(LineKind::User, "q");
    st.stream_delta(LineKind::Thinking, "想");
    st.stream_delta(LineKind::Assistant, "答");
    let rows = body_rows(&st, 80);
    let all_filled = rows.iter().all(|r| !r.line.to_string().is_empty());
    assert!(all_filled, "思考行与正文同回合不应插入空行");
}

#[test]
fn workspace_basename_derives_name() {
    assert_eq!(workspace_basename("/data/proj"), "proj");
    assert_eq!(workspace_basename("/data/proj/"), "proj");
    assert_eq!(workspace_basename("/"), "/");
    assert_eq!(workspace_basename(""), "");
}

#[test]
fn top_text_shows_name_without_prefix_or_path() {
    let mut st = UiState::new();
    st.workspace_path = Some("/data/proj".into());
    let s = top_text(&st, 80);
    assert_eq!(s, "proj");
    assert!(!s.contains("工作区:"), "顶栏不带“工作区: ”前缀");
    let st2 = UiState::new();
    assert!(top_text(&st2, 80).contains("未获取"));
}

#[test]
fn status_line_lists_overrides() {
    let info = StatusInfo {
        api_base: "http://127.0.0.1:8080".into(),
        model: Some("gpt-x*".into()),
        role: Some("coder".into()),
        mode: Some("plan".into()),
        running: false,
        cancel_sent: false,
        view_offset: 0,
        search_term: None,
        search_total: 0,
    };
    let s = status_text(&info, Some("c1"), 200);
    assert!(s.contains("serve http://127.0.0.1:8080"));
    assert!(s.contains("model gpt-x*"));
    assert!(s.contains("role coder"));
    assert!(s.contains("mode plan"));
    assert!(s.contains("conv c1"));
    assert!(s.contains("○ 空闲"));
}

#[test]
fn status_running_indicator_right_aligned() {
    let base = StatusInfo {
        api_base: "http://x".into(),
        model: None,
        role: None,
        mode: None,
        running: false,
        cancel_sent: false,
        view_offset: 0,
        search_term: None,
        search_total: 0,
    };
    let idle = status_text(&base, None, 40);
    assert!(idle.ends_with("○ 空闲"), "空闲指示贴右缘");
    assert_eq!(
        UnicodeWidthStr::width(idle.as_str()),
        40,
        "整行铺满状态栏宽"
    );
    let generating = StatusInfo {
        running: true,
        ..base.clone()
    };
    assert!(
        status_text(&generating, None, 40).ends_with("● 生成中"),
        "生成中指示贴右缘"
    );
    let cancelling = StatusInfo {
        running: true,
        cancel_sent: true,
        ..base.clone()
    };
    assert!(status_text(&cancelling, None, 40).ends_with("… 取消中"));
}

#[test]
fn status_indicator_kept_on_narrow_bar() {
    let base = StatusInfo {
        api_base: "http://long".into(),
        model: Some("model-name".into()),
        role: None,
        mode: None,
        running: true,
        cancel_sent: false,
        view_offset: 0,
        search_term: None,
        search_total: 0,
    };
    let s = status_text(&base, None, 16);
    assert!(s.ends_with("● 生成中"), "窄条也保留右缘指示");
    assert!(
        UnicodeWidthStr::width(s.as_str()) <= 16 + 16,
        "不爆炸性超出"
    );
}

#[test]
fn status_uses_serve_defaults_when_no_override() {
    let info = StatusInfo {
        api_base: "http://x".into(),
        model: Some("deepseek".into()),
        role: None,
        mode: None,
        running: false,
        cancel_sent: false,
        view_offset: 0,
        search_term: None,
        search_total: 0,
    };
    let s = status_text(&info, None, 200);
    assert!(s.contains("model deepseek"));
    assert!(!s.contains("mode "));
}

#[test]
fn status_shows_search_and_view_back() {
    let info = StatusInfo {
        api_base: "http://x".into(),
        model: None,
        role: None,
        mode: None,
        running: false,
        cancel_sent: false,
        view_offset: 12,
        search_term: Some("grep".into()),
        search_total: 2,
    };
    let s = status_text(&info, None, 200);
    assert!(s.contains("find 「grep」"));
    assert!(s.contains("↑12"));
}

fn row(id: &str, title: &str, conv: Option<&str>) -> SessionListItem {
    SessionListItem {
        id: id.to_string(),
        title: title.to_string(),
        server_conversation_id: conv.map(str::to_string),
    }
}

#[test]
fn sidebar_rows_marks_current_and_active() {
    let mut st = UiState::new();
    st.replace_sessions(vec![
        row("a", "Alpha", Some("c1")),
        row("b", "Beta", Some("c2")),
    ]);
    st.conversation_id = Some("c1".into());
    st.active_session_id = Some("b".into());
    let rows = sidebar_rows(&st, 20);
    assert_eq!(rows.len(), 3);
    assert!(rows[1].to_string().contains("> Alpha"));
    assert!(rows[2].to_string().contains("* Beta"));
}

#[test]
fn sidebar_rows_untitled_and_truncate() {
    let mut st = UiState::new();
    st.replace_sessions(vec![row("a", "这是一个很长的会话标题标题", None)]);
    let rows = sidebar_rows(&st, 10);
    assert!(rows[1].to_string().contains("(untitled)") || rows[1].to_string().contains("…"));
}

#[test]
fn cell_window_keeps_wide_char_boundary() {
    assert_eq!(cell_window("你好世界", 2, 4), "好世");
    assert_eq!(cell_window("你a好b", 3, 3), "好b");
}

#[test]
fn visible_window_scrolls_to_cursor_on_right() {
    let (start, c) = horizontal_window("abcdefgh", 8, 4);
    assert_eq!(cell_window("abcdefgh", start, 4), "efgh");
    assert_eq!(c, 4);
}

#[test]
fn visible_window_wide_char_cursor() {
    // 光标在末尾(14)，可视宽 6 → 滚到光标附近。
    let content = "crabmate> 你好";
    let (start, c) = horizontal_window(content, 14, 6);
    let shown = cell_window(content, start, 6);
    assert!(shown.contains('好'));
    assert_eq!(c, 6);
}

#[test]
fn composer_window_multiline_follows_cursor_line() {
    let mut st = UiState::new();
    for ch in "a\nb\nc\nd\ne".chars() {
        st.insert_char(ch);
    }
    let (shown, disp_row, _, _) = composer_window(&st, 40, 4);
    assert_eq!(shown.len(), 4);
    // 光标在末行 e（行 4），窗口顶移 2，末行可见
    assert_eq!(shown.last().map(String::as_str), Some("e"));
    assert_eq!(disp_row, 3);
}

#[test]
fn adjacent_message_bubbles_get_blank_separator() {
    let mut st = UiState::new();
    st.push_line(LineKind::User, "q");
    st.stream_delta(LineKind::Assistant, "a");
    let rows = body_rows(&st, 80);
    let separated = rows
        .windows(2)
        .any(|w| w[0].line.to_string().contains('q') && w[1].line.to_string().is_empty());
    assert!(separated, "用户块与助手块之间应有一个空行");
}
