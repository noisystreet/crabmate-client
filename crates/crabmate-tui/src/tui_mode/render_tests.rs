use super::*;
// 状态行自身（分段取舍 / 右缘运行态）的断言已归 `status.rs::tests`；这里只留渲染层用得着的
// 帧表——折叠思考行与进行中工具行仍共用它做「换字形不改宽」的校验。
use super::super::status::SPINNER_FRAMES;
use crate::serve::SessionListRow;

#[test]
fn truncate_reserves_the_ellipsis_column() {
    // 截断结果必须 ≤ 预算：旧实现「放不下就无条件补 …」会多出第 `width+1` 列，
    // 让调用方按 width 算的预算（状态栏分段、侧栏选中行竖条）悄悄失准。
    assert_eq!(truncate_display("abcdef", 4), "abc…");
    assert_eq!(truncate_display("你好", 3), "你…");
    assert_eq!(truncate_display("", 4), "");
    assert_eq!(truncate_display("abc", 3), "abc", "装得下时不该改写");
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
    // 聊天列内左右各留 1 列空隙（与侧栏 / composer 的对称留白一致），
    // 故正文预算恒为「列宽 − 2」，不能只减左侧那 1 列。
    assert_eq!(chat_body_width(false, 100), 98);
    assert_eq!(
        chat_body_width(true, 100),
        100 - SIDEBAR_WIDTH as usize * 2 - 2
    );
}

/// 正文绘制区必须与 `chat_body_width` 同步收窄：两处都按「列宽 − 1」算会贴到列右缘，
/// 而只改一侧（预算比绘制区宽 1 列）则末列字符被区域右缘直接丢掉——后者由
/// `rendered_chat_area_keeps_all_characters` 按字符计数兜住，本用例补的是前者的观感缺口。
#[test]
fn rendered_chat_area_keeps_right_gutter() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    const TERMINAL_W: u16 = 80;
    const TERMINAL_H: u16 = 12;
    let mut st = UiState::new();
    st.set_palette(super::Palette::default());
    st.sidebar_visible = false;
    st.push_line(LineKind::Assistant, &"中".repeat(38));
    st.push_line(LineKind::Tool, &"工具输出abcd ".repeat(12));
    st.push_line(LineKind::User, &"中文内容abc ".repeat(20));
    let rows = build_body_rows(&st, chat_body_width(false, TERMINAL_W));
    let info = StatusInfo {
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
    let mut terminal = Terminal::new(TestBackend::new(TERMINAL_W, TERMINAL_H)).expect("terminal");
    terminal
        .draw(|f| draw(f, &st, &info, &rows, None))
        .expect("draw");
    let buffer = terminal.backend().buffer();
    // 无侧栏时聊天列即整屏：正文区为 [1, 79)，故 x=0 与 x=79 两列必须是留白。
    // transcript 的行范围按布局推导（composer / 状态栏各占几行），不写死行号：
    // composer 多于一行输入时正文会被挤短，写死的 y 会落到 composer 边框行上。
    let body_h = TERMINAL_H - 1; // 状态栏
    let composer_h = composer_outer_rows(&st)
        .min(body_h.saturating_sub(1))
        .max(1);
    let transcript_h = body_h - composer_h;
    assert!(transcript_h >= 1, "终端太矮，本用例前提不成立");
    let last_x = TERMINAL_W - 1;
    for y in 0..transcript_h {
        assert_eq!(buffer[(0, y)].symbol(), " ", "左侧留白被占用 (y={y})");
        assert_eq!(
            buffer[(last_x, y)].symbol(),
            " ",
            "右侧留白被占用：正文写到聊天列最右一列 (y={y})"
        );
    }
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
    // CJK 与 ASCII 都要覆盖：折叠行是「标记 + 正文 + …」拼出来的，
    // 正文那段由 `truncate_display` 截到预算内，超宽只可能来自拼接本身。
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
    st.tool_start("tc-1", "exec", None);
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
    // 走与 `tool_end` 同一条路（原位回填），而不是直接改字段：
    // `LogLine` 的字段是私有的，正是为了不让「改了内容却忘了换行版本」悄悄发生。
    st.tool_start("call-1", "exec", None);
    st.tool_end("call-1", "exec", Some(true), None);
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

    // 进行中的工具行同样驱动进度动画；一旦结束就停（成败未知也算结束）。
    let mut tool = UiState::new();
    tool.tool_start("tc-1", "exec", None);
    assert_eq!(
        progress_mark_frame(&tool, &info(true, false)),
        Some(7),
        "进行中工具行 + 末行"
    );
    tool.tool_end("tc-1", "exec", Some(true), None);
    assert_eq!(
        progress_mark_frame(&tool, &info(true, false)),
        None,
        "工具结束即停转"
    );
}

/// 修掉的 bug：结束但服务端没给成败的行（旧实现 `tool_ok = None`）被当成「进行中」，
/// 前缀带 `⠿` 且末行一直转 spinner。现在终态是显式的，`Finished(None)` 不再冒充它。
#[test]
fn finished_tool_row_without_verdict_is_not_running() {
    let mut st = UiState::new();
    st.tool_end("tc-1", "exec", None, Some("exit 0"));
    let plain = body_rows(&st, 60)[0].line.to_string();
    assert!(
        plain.starts_with("[工具] ? exec"),
        "成败未知也要算结束: {plain:?}"
    );
    assert!(!plain.contains('⠿'), "结束行不应有进度位: {plain:?}");

    let info = StatusInfo {
        api_base: "http://x".into(),
        model: None,
        role: None,
        mode: None,
        running: true,
        cancel_sent: false,
        spinner: 7,
        view_offset: 0,
        search_term: None,
        search_total: 0,
    };
    assert_eq!(
        progress_mark_frame(&st, &info),
        None,
        "结束的行（成败未知）不该继续转 spinner"
    );
}

#[test]
fn tool_block_separated_from_text_but_compact_inside() {
    let mut st = UiState::new();
    st.stream_delta(LineKind::Assistant, "先看看");
    st.tool_start("c1", "read_file", None);
    st.tool_end("c1", "read_file", Some(true), Some("read file: a.rs"));
    st.tool_start("c2", "grep", None);
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

/// 左会话栏选中态：竖条 + 选中底色；竖条颜色随焦点变，非聚焦时补 BOLD。
#[test]
fn sidebar_selected_row_gets_bar_and_selection_bg() {
    let mut st = UiState::new();
    st.set_palette(Palette::default());
    st.replace_sessions(vec![row("a", "Alpha", Some("c1")), row("b", "Beta", None)]);
    st.selected = 1;
    let width = 20;
    let rows = sidebar_rows(&st, width);
    // 焦点在输入框：竖条退到 muted，但仍在；底色照旧。
    assert_eq!(rows[2].spans[0].content, "▎", "选中行左缘应有竖条");
    assert_eq!(rows[2].spans[0].style.fg, Some(st.palette.muted));
    assert_eq!(rows[2].spans[1].style.bg, Some(st.palette.selection_bg));
    assert!(
        rows[2].spans[1].style.add_modifier.contains(Modifier::BOLD),
        "非聚焦选中行补 BOLD，避免底色变淡后看不出当前会话"
    );
    assert_eq!(rows[1].spans.len(), 1, "未选中行不带竖条");
    // 焦点移到左栏：竖条改 accent。
    st.focus = Focus::Sidebar;
    let rows = sidebar_rows(&st, width);
    assert_eq!(rows[2].spans[0].style.fg, Some(st.palette.accent));
    // 竖条占 1 列，故竖条 + 文本不得超过列宽预算。
    let used = 1 + UnicodeWidthStr::width(rows[2].spans[1].content.as_ref());
    assert!(used <= width, "选中行超宽：{used} > {width}");
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
    st.tool_start("c1", "read_file", None);
    st.tool_end("c1", "read_file", Some(true), None);
    st.tool_start("c2", "grep", None);
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

#[test]
fn body_rows_never_exceed_width() {
    let mut st = UiState::new();
    st.set_palette(super::Palette::default());
    for kind in [
        LineKind::User,
        LineKind::Assistant,
        LineKind::Thinking,
        LineKind::Tool,
        LineKind::System,
    ] {
        st.push_line(kind, &"中文内容abc ".repeat(30));
    }
    for width in [24usize, 40, 80, 120] {
        for r in body_rows(&st, width) {
            let text = r.line.to_string();
            let w = UnicodeWidthStr::width(text.as_str());
            assert!(w <= width, "width={width} 实际行宽={w} 超宽: {text:?}");
        }
    }
}

/// 端到端回归：预折行用的宽度必须与真实渲染区宽度一致，否则行尾字符会被
/// ratatui 按区域右缘直接丢掉——观感就是「超出宽度、没有折回来」。
/// 正文取 38 个汉字（76 列）：按正确宽度正好铺满首物理行并折 2 字到次行，
/// 而按「前缀字符数」多算 2 列时，首行会摆到 81 列、末字被裁掉且不再出现。
#[test]
fn rendered_chat_area_keeps_all_characters() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    const CHARS: usize = 38;
    let text = "中".repeat(CHARS);
    let mut st = UiState::new();
    st.set_palette(super::Palette::default());
    st.sidebar_visible = false;
    st.push_line(LineKind::Assistant, &text);
    let rows = build_body_rows(&st, chat_body_width(false, 80));
    let info = StatusInfo {
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
    let mut terminal = Terminal::new(TestBackend::new(80, 12)).expect("terminal");
    terminal
        .draw(|f| draw(f, &st, &info, &rows, None))
        .expect("draw");
    let buffer = terminal.backend().buffer();
    let mut flat = String::new();
    for y in 0..12u16 {
        for x in 0..80u16 {
            flat.push_str(buffer[(x, y)].symbol());
        }
    }
    let shown = flat.matches('中').count();
    assert_eq!(shown, CHARS, "行尾字符被裁掉了（屏幕只剩 {shown} 个）");
}

/// 段落间距必须活到 render 层：`BodyRow` 里要真有一个空行。
///
/// `md_tests` 的断言只证明 `wrap_styled_chars` 会产出空 row。若渲染层某天
/// 过滤掉空 `Line`（比如按 `spans.is_empty()` 跳过），界面上的空行仍会静默
/// 消失——这正是「终答没有空行」那个 bug 的成因，故在 BodyRow 这一层再钉一次。
#[test]
fn assistant_paragraph_gap_survives_to_body_rows() {
    let mut st = UiState::new();
    st.push_line(LineKind::Assistant, "甲\n\n乙");
    let rows = body_rows(&st, 60);
    assert_eq!(rows.len(), 3, "甲 / 空行 / 乙 三行");
    assert_eq!(rows[1].line.to_string(), "", "中间那行必须是空行");
    assert!(
        rows[2].line.to_string().contains('乙'),
        "末行应是第二段，got {:?}",
        rows[2].line.to_string()
    );
}

/// 连续空行折叠到 render 层仍然只有一个空行（不放大成多行空白）。
#[test]
fn body_rows_collapse_consecutive_blanks_to_one() {
    let mut st = UiState::new();
    st.push_line(LineKind::Assistant, "甲\n\n\n\n乙");
    let rows = body_rows(&st, 60);
    assert_eq!(rows.len(), 3, "多余空行不该变成多行空白");
}

/// 回放的历史工具行不该再画「进行中」的进度位：历史里没有「正在跑」这回事，
/// 否则打开旧会话会看到一排转圈的工具行。
#[test]
fn replayed_tool_row_has_no_progress_slot() {
    let mut st = UiState::new();
    st.tool_start("tc-1", "exec", None);
    st.push_replayed_line(LineKind::Tool, "history", None);
    assert_eq!(prefix_for(&st.lines[0]), TOOL_PREFIX_PROGRESS);
    assert_eq!(prefix_for(&st.lines[1]), kind_prefix(LineKind::Tool));
}

/// 把「环境变量取到什么值」直接喂给解析函数，避免单测碰真实 env。
fn composer_hint_from(v: Option<&str>) -> bool {
    composer_hint_enabled_with(&|_| v.map(str::to_string))
}

/// 占位提示默认关：只有显式开启的取值才画。
#[test]
fn composer_hint_is_off_unless_explicitly_enabled() {
    assert!(!composer_hint_from(None), "未设变量应保持关闭");
    for v in ["0", "off", "false", "no", "disable", "", "随便"] {
        assert!(!composer_hint_from(Some(v)), "{v:?} 不应开启");
    }
    for v in ["1", "on", " TRUE ", "yes", "ENABLE"] {
        assert!(composer_hint_from(Some(v)), "{v:?} 应开启");
    }
}

/// 空 composer 默认不画占位提示。
///
/// 只覆盖「默认」这一半：开启分支要读真实环境变量，而 `std::env::set_var` 在
/// edition 2024 是 unsafe、还会污染同进程其它用例，不值得为它开这个口子。
#[test]
fn empty_composer_has_no_placeholder_by_default() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    if composer_hint_enabled() {
        return; // 本机设了 CM_TUI_COMPOSER_HINT，本用例前提不成立。
    }
    let mut st = UiState::new();
    st.set_palette(Palette::default());
    st.sidebar_visible = false;
    let rows = build_body_rows(&st, chat_body_width(false, 80));
    let info = StatusInfo {
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
    let mut terminal = Terminal::new(TestBackend::new(80, 12)).expect("terminal");
    terminal
        .draw(|f| draw(f, &st, &info, &rows, None))
        .expect("draw");
    let screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    // 空会话正文的 BODY_EMPTY_HINT 也以「输入消息」开头，故匹配带分隔符的完整前缀。
    assert!(
        !screen.contains("输入消息 ·"),
        "默认不应画 composer 占位提示"
    );
}
