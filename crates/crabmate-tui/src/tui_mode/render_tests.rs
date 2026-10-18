use super::*;
// 状态行函数已拆到兄弟模块（`render.rs` 行数门禁）；这里显式导入保持用例不变。
use super::super::status::{SPINNER_FRAMES, status_text};
use crate::serve::SessionListRow;

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
    // 固定调色板：参数化断言不能随终端环境（NO_COLOR 等）漂移。
    st.set_palette(super::Palette::default());
    st.push_line(LineKind::Assistant, "hello world");
    st.push_line(LineKind::User, "bye");
    st.start_search("hello");
    let rows = body_rows(&st, 60);
    let pal = st.palette;
    // 只取内容行：块间分隔空行的 spans 为空，`all(...)` 恒真会悄悄架空断言。
    let filled: Vec<&BodyRow> = rows.iter().filter(|r| r.log_index.is_some()).collect();
    assert_eq!(filled.len(), 2, "两条日志行应各产出一条内容行");
    // 锚定行第一物理行以 accent 底反色突出
    assert!(
        filled[0]
            .line
            .spans
            .iter()
            .any(|s| s.style.bg == Some(pal.hit))
    );
    // 非命中行（User 行）不带 accent 前景
    assert!(
        filled[1]
            .line
            .spans
            .iter()
            .all(|s| s.style.fg != Some(pal.hit))
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

/// 分隔行判定：非空且只由 `─` 组成（允许左侧缩进空格）。
fn is_rule(s: &str) -> bool {
    let t = s.trim();
    !t.is_empty() && t.chars().all(|c| c == '─')
}

#[test]
fn thinking_and_answer_get_rule_separator() {
    let mut st = UiState::new();
    st.push_line(LineKind::User, "q");
    st.stream_delta(LineKind::Thinking, "想");
    st.stream_delta(LineKind::Assistant, "答");
    let rows = body_rows(&st, 80);
    let rules: Vec<String> = rows
        .iter()
        .filter(|r| r.log_index.is_none())
        .map(|r| r.line.to_string())
        .collect();
    assert_eq!(rules.len(), 2, "用户 / 思考 / 正文三块之间各一条分隔线");
    assert!(
        rules.iter().all(|s| is_rule(s)),
        "块间分隔应为满宽细线而非空行: {rules:?}"
    );
}

#[test]
fn separator_rule_spans_full_width_in_border_color() {
    let mut st = UiState::new();
    st.set_palette(super::Palette::default());
    st.push_line(LineKind::User, "q");
    st.stream_delta(LineKind::Assistant, "a");
    let rows = body_rows(&st, 40);
    let sep = rows
        .iter()
        .find(|r| r.log_index.is_none())
        .expect("应有块间分隔行");
    let text = sep.line.to_string();
    assert_eq!(text.chars().count(), 40, "分隔线应铺满正文宽度: {text:?}");
    assert_eq!(
        sep.line.spans[0].style.fg,
        Some(super::Palette::default().border),
        "分隔线应取 palette.border"
    );
}

#[test]
fn spinner_marks_share_static_mark_width() {
    // 折叠行预算按「标记的显示列」算；若将来换成不同宽度的字符，
    // 折叠行宽度会随帧抖动。这条锁住契约。
    let base = UnicodeWidthStr::width("▸ ");
    for mark in SPINNER_FRAMES {
        assert_eq!(
            UnicodeWidthStr::width(format!("{mark} ").as_str()),
            base,
            "{mark} 必须与 ▸ 同宽"
        );
    }
    // 进行中工具行的进度位占位也必须是这个宽度：换帧才不挤动折行点。
    assert_eq!(
        UnicodeWidthStr::width(TOOL_PROGRESS_MARK),
        base,
        "进度位占位必须与折叠标记同宽"
    );
}

#[test]
fn folded_thinking_never_exceeds_content_width() {
    // CJK 与 ASCII 都要覆盖：`truncate_display` 截断时可能比预算宽 1 列，
    // 只有在首字符占 1 列（ASCII）时才会暴露「预算被抬高」的边界。
    for text in ["内容很长很长的思考文字", "abcdefghijklmnop"] {
        for cw in 0..40usize {
            let out = fold_thinking(text, cw);
            let w = UnicodeWidthStr::width(out.as_str());
            assert!(w <= cw.max(1), "text={text:?} cw={cw} 超宽 {w}: {out:?}");
        }
    }
}

#[test]
fn with_progress_mark_replaces_mark_and_keeps_width() {
    let mut st = UiState::new();
    st.push_line(LineKind::Thinking, "想");
    let rows = body_rows(&st, 40);
    let plain = rows[0].line.to_string();
    let spinning = with_progress_mark(rows[0].line.clone(), 5).to_string();
    assert!(plain.contains("▸ "), "静态标记: {plain:?}");
    assert!(spinning.contains("⠴ "), "第 5 帧: {spinning:?}");
    assert!(!spinning.contains('▸'), "静态标记应被替换: {spinning:?}");
    assert_eq!(
        UnicodeWidthStr::width(plain.as_str()),
        UnicodeWidthStr::width(spinning.as_str()),
        "替换不得改变行宽"
    );
}

/// 渲染期替换只认内容 span 的 `▸ ` 前缀，正文里出现的 `▸` 不能被误伤。
#[test]
fn with_progress_mark_leaves_other_content_untouched() {
    let line = Line::from(vec![Span::raw("[思考] "), Span::raw("正文里有 ▸ 符号")]);
    assert_eq!(
        with_progress_mark(line, 3).to_string(),
        "[思考] 正文里有 ▸ 符号"
    );
}

/// 极窄宽度下 `fold_thinking` 只留 `▸`（无空格）：此时不替换，避免超宽。
#[test]
fn with_progress_mark_degrades_when_mark_is_single_char() {
    let line = Line::from(vec![Span::raw("[思考] "), Span::raw("▸")]);
    let expected = line.to_string();
    assert_eq!(
        with_progress_mark(line, 4).to_string(),
        expected,
        "只留标记时不应替换"
    );
}

/// 进行中的工具行：前缀带静态进度位，渲染期换成 spinner 帧后宽度不变。
#[test]
fn in_flight_tool_row_gets_progress_slot() {
    let mut st = UiState::new();
    st.push_line(LineKind::Tool, "exec — 运行中");
    let rows = body_rows(&st, 60);
    let plain = rows[0].line.to_string();
    assert!(
        plain.starts_with("[工具] ⠿ "),
        "进行中应有进度位: {plain:?}"
    );

    let spinning = with_progress_mark(rows[0].line.clone(), 2).to_string();
    assert!(spinning.starts_with("[工具] ⠹ "), "第 2 帧: {spinning:?}");
    assert!(!spinning.contains('⠿'), "占位应被帧替换: {spinning:?}");
    assert_eq!(
        UnicodeWidthStr::width(plain.as_str()),
        UnicodeWidthStr::width(spinning.as_str()),
        "换帧不得改变行宽"
    );
}

/// 结束的工具行不加进度位（文本自带 `✓/✗`），避免双重标记。
#[test]
fn finished_tool_row_has_no_progress_slot() {
    let mut st = UiState::new();
    st.push_line(LineKind::Tool, "exec ✓ — exit 0");
    st.lines.last_mut().expect("刚 push 的行").tool_ok = Some(true);
    let plain = body_rows(&st, 60)[0].line.to_string();
    assert!(plain.starts_with("[工具] "), "结束行无进度位: {plain:?}");
    assert!(!plain.contains('⠿'), "结束行不应有占位: {plain:?}");
}

#[test]
fn progress_mark_frame_requires_live_folded_thinking() {
    fn info(running: bool, cancel_sent: bool) -> StatusInfo {
        StatusInfo {
            api_base: "http://x".into(),
            model: None,
            role: None,
            mode: None,
            running,
            cancel_sent,
            spinner: 7,
            view_offset: 0,
            search_term: None,
            search_total: 0,
        }
    }
    let mut st = UiState::new();
    st.push_line(LineKind::Thinking, "想");
    assert_eq!(
        progress_mark_frame(&st, &info(true, false)),
        Some(7),
        "生成中 + 折叠 + 末行"
    );
    assert_eq!(
        progress_mark_frame(&st, &info(true, true)),
        None,
        "取消中不转"
    );
    assert_eq!(
        progress_mark_frame(&st, &info(false, false)),
        None,
        "非生成中不转"
    );

    st.set_thinking_visible(true);
    assert_eq!(
        progress_mark_frame(&st, &info(true, false)),
        None,
        "展开态不转"
    );

    let mut ended = UiState::new();
    ended.push_line(LineKind::Thinking, "想");
    ended.push_line(LineKind::Assistant, "答");
    assert_eq!(
        progress_mark_frame(&ended, &info(true, false)),
        None,
        "末行非 thinking 后停转"
    );

    // 进行中的工具行同样驱动进度动画；一旦拿到 tool_ok 就停。
    let mut tool = UiState::new();
    tool.push_line(LineKind::Tool, "exec — 运行中");
    assert_eq!(
        progress_mark_frame(&tool, &info(true, false)),
        Some(7),
        "进行中工具行 + 末行"
    );
    tool.lines.last_mut().expect("刚 push 的行").tool_ok = Some(true);
    assert_eq!(
        progress_mark_frame(&tool, &info(true, false)),
        None,
        "工具结束即停转"
    );
}

#[test]
fn tool_block_separated_from_text_but_compact_inside() {
    let mut st = UiState::new();
    st.stream_delta(LineKind::Assistant, "先看看");
    st.tool_start("c1", "read_file");
    st.tool_end("c1", "read_file", Some(true), Some("read file: a.rs"));
    st.tool_start("c2", "grep");
    st.stream_delta(LineKind::Assistant, "结论");
    let rows = body_rows(&st, 80);
    let text: Vec<String> = rows.iter().map(|r| r.line.to_string()).collect();
    assert_eq!(text.len(), 6, "4 条内容行 + 2 条分隔线: {text:?}");
    assert!(is_rule(&text[1]), "正文与工具块之间应有分隔线: {text:?}");
    assert!(text[2].contains("read file: a.rs"), "工具结束行: {text:?}");
    assert!(
        text[3].contains("grep"),
        "连续工具行之间不插分隔线: {text:?}"
    );
    assert!(is_rule(&text[4]), "工具块与正文块之间应有分隔线: {text:?}");
}

#[test]
fn separators_carry_no_log_index() {
    let mut st = UiState::new();
    st.push_line(LineKind::User, "q");
    st.stream_delta(LineKind::Thinking, "想");
    st.stream_delta(LineKind::Assistant, "答");
    let seq: Vec<Option<usize>> = body_rows(&st, 80).iter().map(|r| r.log_index).collect();
    // 内容行按逻辑行号递增；块间分隔行为 None，/find 锚点（按 Some(target) 查找）
    // 因而不会停在空行上。
    assert_eq!(seq, vec![Some(0), None, Some(1), None, Some(2)]);
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
        spinner: 0,
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
        spinner: 0,
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
        status_text(&generating, None, 40).ends_with("⠋ 生成中"),
        "生成中指示贴右缘"
    );
    let cancelling = StatusInfo {
        running: true,
        cancel_sent: true,
        ..base.clone()
    };
    assert!(status_text(&cancelling, None, 40).ends_with("✳ 取消中"));
}

#[test]
fn spinner_advances_per_frame_and_wraps() {
    let base = StatusInfo {
        api_base: "http://x".into(),
        model: None,
        role: None,
        mode: None,
        running: true,
        cancel_sent: false,
        spinner: 0,
        view_offset: 0,
        search_term: None,
        search_total: 0,
    };
    assert!(status_text(&base, None, 40).ends_with("⠋ 生成中"));
    let f1 = StatusInfo {
        spinner: 1,
        ..base.clone()
    };
    assert!(status_text(&f1, None, 40).ends_with("⠙ 生成中"), "第二帧");
    // 10 帧一圈：帧号 10 回到首帧，任意 usize 都不会越界。
    let wrap = StatusInfo {
        spinner: 10,
        ..base.clone()
    };
    assert!(status_text(&wrap, None, 40).ends_with("⠋ 生成中"), "回绕");
    // 取消中不显 spinner（静态 ✳），帧号变化也不影响。
    let cancelling = StatusInfo {
        cancel_sent: true,
        spinner: 7,
        ..base
    };
    assert!(status_text(&cancelling, None, 40).ends_with("✳ 取消中"));
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
        spinner: 0,
        view_offset: 0,
        search_term: None,
        search_total: 0,
    };
    let s = status_text(&base, None, 16);
    assert!(s.ends_with("⠋ 生成中"), "窄条也保留右缘指示");
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
        spinner: 0,
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
        spinner: 0,
        view_offset: 12,
        search_term: Some("grep".into()),
        search_total: 2,
    };
    let s = status_text(&info, None, 200);
    assert!(s.contains("find 「grep」"));
    assert!(s.contains("↑12"));
}

fn row(id: &str, title: &str, conv: Option<&str>) -> SessionListRow {
    SessionListRow {
        id: id.to_string(),
        title: title.to_string(),
        updated_at: 0,
        pinned: false,
        starred: false,
        server_conversation_id: conv.map(str::to_string),
        server_revision: None,
        workspace_root: None,
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
fn adjacent_message_bubbles_get_rule_separator() {
    let mut st = UiState::new();
    st.push_line(LineKind::User, "q");
    st.stream_delta(LineKind::Assistant, "a");
    let rows = body_rows(&st, 80);
    let separated = rows
        .windows(2)
        .any(|w| w[0].line.to_string().contains('q') && is_rule(&w[1].line.to_string()));
    assert!(separated, "用户块与助手块之间应有一条分隔线");
}

#[test]
fn tool_row_colors_by_result() {
    let mut st = UiState::new();
    // 固定调色板：参数化断言不能随终端环境（NO_COLOR 等）漂移。
    st.set_palette(super::Palette::default());
    st.tool_start("c1", "read_file");
    st.tool_end("c1", "read_file", Some(true), None);
    st.tool_start("c2", "grep");
    st.tool_end("c2", "grep", Some(false), None);
    let pal = st.palette;
    let rows = body_rows(&st, 80);
    let filled: Vec<&BodyRow> = rows.iter().filter(|r| r.log_index.is_some()).collect();
    assert_eq!(filled.len(), 2, "两行工具日志各产出一条内容行");
    assert!(
        filled[0]
            .line
            .spans
            .iter()
            .any(|s| s.style.fg == Some(pal.ok)),
        "成功行用 ok 色"
    );
    assert!(
        filled[1]
            .line
            .spans
            .iter()
            .any(|s| s.style.fg == Some(pal.err) && s.style.add_modifier.contains(Modifier::BOLD)),
        "失败行用 err 色 + BOLD（无色模式下也靠 BOLD 区分）"
    );
}

#[test]
fn collapsed_thinking_gets_fold_marker() {
    let mut st = UiState::new();
    st.stream_delta(LineKind::Thinking, "想了很久");
    let rows = body_rows(&st, 80);
    let text = rows[0].line.to_string();
    assert!(text.contains('▸'), "折叠态应有 ▸ 标记: {text:?}");
}

#[test]
fn prefix_is_dim_and_content_is_not() {
    let mut st = UiState::new();
    // 固定调色板：参数化断言不能随终端环境（NO_COLOR 等）漂移。
    st.set_palette(super::Palette::default());
    st.push_line(LineKind::User, "hi");
    let rows = body_rows(&st, 40);
    let spans = &rows[0].line.spans;
    assert!(
        spans[0].style.add_modifier.contains(Modifier::DIM),
        "前缀应降噪为 DIM"
    );
    assert!(
        !spans[1].style.add_modifier.contains(Modifier::DIM),
        "正文不应被 DIM"
    );
}

#[test]
fn composer_border_geometry() {
    // 边框左右各占 1 列。
    let normal = Rect::new(0, 0, 80, 5);
    assert!(use_composer_border(normal));
    assert_eq!(composer_content_width(normal), 78);
    // 20 列窄终端仍有可用内宽（不能算出 0，否则光标窗口会退化）。
    assert_eq!(composer_content_width(Rect::new(0, 0, 20, 4)), 18);
    // 放不下边框时退化：宽度走旧的左内缩 1 列。
    let tiny_w = Rect::new(0, 0, 3, 5);
    assert!(!use_composer_border(tiny_w), "3 列放不下边框");
    assert_eq!(composer_content_width(tiny_w), 2);
    let tiny_h = Rect::new(0, 0, 80, 2);
    assert!(!use_composer_border(tiny_h), "2 行放不下上下边框");
    assert_eq!(composer_content_width(tiny_h), 79);
}

#[test]
fn composer_outer_rows_adds_borders() {
    let st = UiState::new();
    // 单行输入：1 内容行 + 上下边框。
    assert_eq!(composer_rows(&st), 1);
    assert_eq!(composer_outer_rows(&st), 3);
}
