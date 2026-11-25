//! `ws_sidebar.rs` 的单测（`#[path]` 子模块）：压住单文件行数门禁。

use super::super::pane_width::SIDEBAR_WIDTH;
use super::*;

fn ws_row(indent: u8, rel: &str, name: &str, is_dir: bool, expanded: bool) -> WsRow {
    WsRow {
        indent,
        rel: rel.to_string(),
        name: name.to_string(),
        is_dir,
        expanded,
        loading: false,
        placeholder: false,
    }
}

/// 缓存路径必须与「每帧全量重建」逐行一致 —— 这是这套缓存唯一重要的事。
///
/// 比较用 `Debug` 而不是 `to_string()`：后者只比文本，样式（选中底色 / 失焦 BOLD）全被
/// 忽略，「`>` / `*` 标记忘了更新」这类 bug 就会从测试里漏过去（与 `body_cache` 同一手法）。
#[test]
fn cached_sessions_match_full_rebuild() {
    use super::super::render::sidebar_rows_rebuild;
    use crate::serve::SessionListRow;

    fn sess_row(id: &str, title: &str) -> SessionListRow {
        SessionListRow {
            id: id.to_string(),
            title: title.to_string(),
            updated_at: 0,
            pinned: false,
            starred: false,
            server_conversation_id: Some(format!("c-{id}")),
            server_revision: None,
            workspace_root: None,
        }
    }
    fn dump(lines: &[Line<'static>]) -> Vec<String> {
        lines.iter().map(|l| format!("{l:?}")).collect()
    }

    let mut st = UiState::new();
    st.replace_sessions(vec![
        sess_row("a", "Alpha"),
        sess_row("b", ""),
        sess_row("c", "登录闪退"),
    ]);
    st.conversation_id = Some("c-a".into());
    st.active_session_id = Some("b".into());
    st.selected = 1;

    let visible = st.visible_indices();
    let want = dump(&sidebar_rows_rebuild(&st, 20, &visible));
    assert_eq!(dump(&sessions_content(&mut st, 20).0), want, "首帧建缓存");
    assert_eq!(dump(&sessions_content(&mut st, 20).0), want, "无变化的次帧");

    // 焦点换了：文本不变，但 BOLD 要跟着变（缓存不能把它焊死）。
    st.focus = Focus::Sidebar;
    let want_focus = dump(&sidebar_rows_rebuild(&st, 20, &visible));
    assert_eq!(
        dump(&sessions_content(&mut st, 20).0),
        want_focus,
        "焦点变化只改样式"
    );

    // 标题改了（同一行）：只有那一行该重建。
    st.replace_sessions(vec![
        sess_row("a", "Alpha 改过"),
        sess_row("b", ""),
        sess_row("c", "登录闪退"),
    ]);
    let visible = st.visible_indices();
    let want2 = dump(&sidebar_rows_rebuild(&st, 20, &visible));
    assert_eq!(dump(&sessions_content(&mut st, 20).0), want2, "改标题");

    // 宽度变了：整表重建（截断点全变）。
    let want3 = dump(&sidebar_rows_rebuild(&st, 10, &visible));
    assert_eq!(dump(&sessions_content(&mut st, 10).0), want3, "改宽度");
}

fn ready_ws_state() -> UiState {
    let mut st = UiState::new();
    // 显式落深色配色：`UiState::new()` 走 `Palette::resolve`（会读 `NO_COLOR` 等环境），
    // 测试不能依赖运行环境是不是无色终端。
    st.set_palette(Palette::from_theme(Some("shadcn-dark")));
    st.workspace_path = Some("/data/proj".to_string());
    st.ws_ready = true;
    st.ws_cursor = 2;
    st.ws_rows = vec![
        ws_row(0, "src", "src", true, false),
        ws_row(1, "src/lib.rs", "lib.rs", false, false),
        ws_row(0, "README.md", "README.md", false, false),
    ];
    st
}

#[test]
fn workspace_title_shows_basename_or_placeholder() {
    let mut st = ready_ws_state();
    let rows = workspace_sidebar_rows(&st, 60);
    assert!(rows[0].to_string().contains("proj"));
    assert!(!rows[0].to_string().contains("工作区:"), "标题不带前缀");
    st.workspace_path = None;
    let rows = workspace_sidebar_rows(&st, 60);
    assert!(rows[0].to_string().contains("未获取"));
}

#[test]
fn workspace_sidebar_rows_render_tree_with_glyph_and_indent() {
    let mut st = ready_ws_state();
    st.focus = Focus::Workspace;
    let rows = workspace_sidebar_rows(&st, 60);
    assert_eq!(rows.len(), 4, "标题 + 3 树行");
    assert!(rows[1].to_string().contains("▸ src"), "折叠目录带 ▸");
    assert!(rows[2].to_string().contains("lib.rs"), "缩进子行");
    assert!(!rows[2].to_string().contains("▸"));
    // 光标行（README.md，cursor=2 → 行 3）：只有选中底色，不画左缘竖条。
    assert_eq!(rows[3].spans.len(), 1, "选中行不再拆出竖条 span");
    assert!(!rows[3].spans[0].content.contains('▎'), "不画竖条");
    assert_eq!(
        rows[3].spans[0].style.bg,
        Some(st.palette.selection_bg),
        "选中行应为 selection_bg 底色"
    );
    assert_eq!(rows[1].spans.len(), 1, "未选中行同样只有一个 span");
}

#[test]
fn workspace_sidebar_rows_loading_error_empty() {
    let mut st = UiState::new();
    st.focus = Focus::Workspace;
    st.workspace_path = None;
    let rows = workspace_sidebar_rows(&st, 60);
    assert!(rows[1].to_string().contains("加载中"));
    st.ws_root_err = Some("连接断开".to_string());
    let rows = workspace_sidebar_rows(&st, 60);
    assert!(rows[1].to_string().contains("读取失败"));
    assert!(rows[1].to_string().contains("连接断开"));
    st.ws_ready = true;
    let rows = workspace_sidebar_rows(&st, 60);
    assert!(rows[1].to_string().contains("空目录"));
}

#[test]
fn workspace_row_highlight_requires_workspace_focus() {
    let st = ready_ws_state(); // focus 默认 Input
    let rows = workspace_sidebar_rows(&st, 60);
    assert_ne!(
        rows[3].spans[0].style.bg,
        Some(st.palette.selection_bg),
        "焦点不在本栏就不铺选中底色"
    );
}

#[test]
fn mono_palette_falls_back_to_reverse() {
    let mut st = ready_ws_state();
    st.set_palette(Palette::mono());
    st.focus = Focus::Workspace;
    let rows = workspace_sidebar_rows(&st, 60);
    assert!(
        rows[3].spans[0]
            .style
            .add_modifier
            .contains(Modifier::REVERSED),
        "无色模式下选中态必须退回反色（selection_bg 已被抹成 Reset）"
    );
}

#[test]
fn workspace_loading_row_is_dimmed() {
    let mut st = ready_ws_state();
    st.focus = Focus::Workspace;
    st.ws_cursor = 0; // 占位行不被反色，便于检查灰色前景
    st.ws_rows[1].loading = true;
    st.ws_rows[1].placeholder = true;
    st.ws_rows[1].name = "\u{22ef}".to_string();
    let rows = workspace_sidebar_rows(&st, 60);
    assert_eq!(
        rows[2].spans[0].style.fg,
        Some(st.palette.muted),
        "在途占位行应为淡色"
    );
}

#[test]
fn panes_content_workspace_always_rows_sessions_only_left() {
    let mut st = ready_ws_state();
    st.focus = Focus::Workspace;
    let (rows, cursor, hint) = workspace_content(&st, 60);
    assert_eq!(cursor, st.ws_cursor);
    assert_eq!(rows.len(), 4);
    assert!(hint.is_some());
    // 默认（Input）右栏同样渲染树行，只是不给按键提示
    st.focus = Focus::Input;
    let (rows, _, hint) = workspace_content(&st, 60);
    assert_eq!(rows.len(), 4, "工作区树默认可见");
    assert!(hint.is_none());
    st.focus = Focus::Sidebar;
    let (rows, _, hint) = sessions_content(&mut st, 60);
    assert!(rows[0].to_string().contains("会话"));
    assert!(hint.is_some());
}

/// 鼠标点出来的焦点不画底部按键提示（点击后那行只会挤掉一行内容）。
#[test]
fn pointer_focus_hides_pane_hints() {
    let mut st = ready_ws_state();
    st.focus = Focus::Workspace;
    st.focus_by_pointer = true;
    let (_, _, hint) = workspace_content(&st, 60);
    assert!(hint.is_none(), "鼠标点进工作区时不显示按键提示");

    st.focus = Focus::Sidebar;
    let (_, _, hint) = sessions_content(&mut st, 60);
    assert!(hint.is_none(), "鼠标点进会话栏时不显示按键提示");

    // 键盘接管后（`on_key` 清掉这一位）提示照旧回来。
    st.focus_by_pointer = false;
    let (_, _, hint) = sessions_content(&mut st, 60);
    assert!(hint.is_some(), "键盘带来的焦点要显示提示");
}

/// 删除确认态：提示行必须换成确认文案，不能让「下一键就删」悄悄发生。
#[test]
fn sessions_hint_switches_to_confirm_copy_while_deleting() {
    let mut st = ready_ws_state();
    st.focus = Focus::Sidebar;
    let (_, _, hint) = sessions_content(&mut st, 60);
    let text = hint.expect("键盘聚焦要给提示").to_string();
    assert_eq!(text, SIDEBAR_HINT, "平时是按键提示");
    assert!(text.contains("Del"), "平时要提示删除键");

    st.confirm_delete = Some("s1".to_string());
    let (_, _, hint) = sessions_content(&mut st, 60);
    let text = hint.expect("确认态也要给提示").to_string();
    assert_eq!(text, SIDEBAR_DELETE_HINT, "确认态换确认文案");
    assert!(!text.contains("Del删"), "确认态不再重复按键说明");
}

#[test]
fn sidebar_view_keeps_selected_visible() {
    let pal = Palette::from_theme(Some("shadcn-dark"));
    let mut rows: Vec<Line<'static>> = vec![Line::from("header")];
    rows.extend((0..10).map(|i| Line::from(format!("item{i}"))));
    let (s, e) = sidebar_window(rows.len(), 4, 7, 0);
    let v = sidebar_view(rows, s, e, 20, &pal);
    assert_eq!(v.len(), 4);
    assert!(v.last().unwrap().to_string().contains("item7"));
    assert!(v.first().unwrap().to_string().contains("header"));
}

#[test]
fn sidebar_view_reports_hidden_rows_in_title() {
    let pal = Palette::from_theme(Some("shadcn-dark"));
    let mut rows: Vec<Line<'static>> = vec![Line::from("header")];
    rows.extend((0..10).map(|i| Line::from(format!("item{i}"))));
    // 高 4 行 = 标题 + 3 条；选中第 7 条 → 上方藏 5 条、下方藏 2 条。
    let (s, e) = sidebar_window(rows.len(), 4, 7, 0);
    let v = sidebar_view(rows, s, e, 20, &pal);
    let title = v[0].to_string();
    assert!(title.contains("↑5"), "上方隐藏计数：{title}");
    assert!(title.contains("↓2"), "下方隐藏计数：{title}");
    assert_eq!(v.len(), 4);
}

#[test]
fn sidebar_view_fits_without_window() {
    let pal = Palette::from_theme(Some("shadcn-dark"));
    let mut rows: Vec<Line<'static>> = vec![Line::from("header")];
    rows.extend((0..3).map(|i| Line::from(format!("item{i}"))));
    let (s, e) = sidebar_window(rows.len(), 5, 1, 0);
    let v = sidebar_view(rows, s, e, 20, &pal);
    assert_eq!(v.len(), 4);
    assert!(!v[0].to_string().contains('↑'), "没有隐藏行时不该出现计数");
}

/// 带溢出计数时标题仍要保住多 span 样式（工作区名 BOLD / 计数器 muted）。
#[test]
fn overflow_hint_keeps_counter_style_and_drops_padding() {
    let pal = Palette::from_theme(Some("shadcn-dark"));
    // 模拟 `workspace_title`：[名称(BOLD)] [右对齐填充] [计数器(Gray)]
    let mut rows = vec![Line::from(vec![
        Span::styled(
            "proj".to_string(),
            Style::new().fg(Color::White).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" ".repeat(13), Style::new()),
        Span::styled("2/3".to_string(), Style::new().fg(Color::Gray)),
    ])];
    rows.extend((0..5).map(|i| Line::from(format!("item{i}"))));
    // 高 2 行 = 标题 + 1 条；选中第 0 条 → 下方藏 4 条。
    let (s, e) = sidebar_window(rows.len(), 2, 0, 0);
    let v = sidebar_view(rows, s, e, 20, &pal);
    let head = &v[0];
    let text = head.to_string();
    assert!(text.contains("2/3"), "计数器该留下：{text}");
    assert!(text.contains("↓4"), "应带溢出计数：{text}");
    assert!(
        UnicodeWidthStr::width(text.as_str()) <= 20,
        "标题不得超宽：{text}"
    );
    let counter = head
        .spans
        .iter()
        .find(|s| s.content.contains("2/3"))
        .expect("计数器 span");
    assert_eq!(
        counter.style.fg,
        Some(Color::Gray),
        "计数器样式不能被压平成标题样式"
    );
}

#[test]
fn pick_rows_loading_error_disabled() {
    let mut st = UiState::new();
    st.focus = Focus::Workspace;
    assert_eq!(workspace_pick_rows(&st, 60).len(), 1, "无 pick 态仅标题");
    st.ws_pick = Some(super::super::workspace_tree::WsPickState {
        loading: true,
        ..Default::default()
    });
    assert!(
        workspace_pick_rows(&st, 60)[1]
            .to_string()
            .contains("加载中")
    );
    st.ws_pick = Some(super::super::workspace_tree::WsPickState::default());
    assert!(
        workspace_pick_rows(&st, 60)[1]
            .to_string()
            .contains("项目池未启用")
    );
    st.ws_pick = Some(super::super::workspace_tree::WsPickState {
        enabled: true,
        error: Some("e".to_string()),
        ..Default::default()
    });
    assert!(workspace_pick_rows(&st, 60)[1].to_string().contains("失败"));
}

#[test]
fn pick_rows_highlight_cursor() {
    let mut st = UiState::new();
    st.focus = Focus::Workspace;
    st.ws_cursor = 1;
    st.ws_pick = Some(super::super::workspace_tree::WsPickState {
        enabled: true,
        projects: vec!["proj-a".to_string(), "proj-b".to_string()],
        ..Default::default()
    });
    let rows = workspace_pick_rows(&st, 60);
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[1].spans.len(), 1, "非光标行只有一个 span");
    assert_eq!(rows[2].spans.len(), 1, "光标行同样只有一个 span");
    assert!(!rows[2].spans[0].content.contains('▎'), "不画竖条");
    assert_eq!(
        rows[2].spans[0].style.bg,
        Some(st.palette.selection_bg),
        "光标行应为 selection_bg 底色"
    );
}

#[test]
fn workspace_content_unset_and_pick_hints() {
    let mut st = UiState::new();
    st.focus = Focus::Workspace;
    let (_, _, hint) = workspace_content(&st, 60);
    assert!(
        hint.unwrap().to_string().contains("未设置"),
        "路径为空给未设置提示"
    );
    st.workspace_path = Some("/data/proj".to_string());
    st.ws_ready = true;
    let (_, _, hint) = workspace_content(&st, 60);
    let hint = hint.unwrap().to_string();
    assert!(hint.contains("Enter展开"), "正常树给树提示");
    assert!(hint.contains('p'), "提示 p 切项目");
    assert!(hint.contains('w'), "提示 w 回会话栏");
    st.ws_pick = Some(super::super::workspace_tree::WsPickState::default());
    let (rows, _, hint) = workspace_content(&st, 60);
    assert!(rows[0].to_string().contains("项目池"));
    assert!(hint.unwrap().to_string().contains("Enter切换"));
}

#[test]
fn hints_fit_sidebar_width() {
    // 提示占满侧栏内宽（`SIDEBAR_WIDTH` - 1），超出会被 `render_pane` 直接裁掉尾按键。
    for hint in [
        WORKSPACE_HINT,
        WORKSPACE_FILE_HINT,
        WORKSPACE_FILE_OPEN_HINT,
        WORKSPACE_UNSET_HINT,
        WORKSPACE_PICK_HINT,
        SIDEBAR_HINT,
        SIDEBAR_DELETE_HINT,
    ] {
        assert!(
            UnicodeWidthStr::width(hint) < SIDEBAR_WIDTH as usize,
            "提示超宽会被裁切：{hint}"
        );
    }
}

#[test]
fn file_row_hint_says_open_while_the_editor_view_is_open() {
    // `ready_ws_state` 的光标停在 README.md（文件行）
    let mut st = ready_ws_state();
    st.focus = Focus::Workspace;
    assert_eq!(workspace_hint(&st), WORKSPACE_FILE_HINT);
    st.editor.open_file("README.md", 0);
    assert_eq!(
        workspace_hint(&st),
        WORKSPACE_FILE_OPEN_HINT,
        "编辑器视图下 Enter 是打开，提示不能还写「插路径」"
    );
    // 目录行仍是展开提示：视图开着也只是文件行的语义变了
    st.ws_cursor = 0;
    assert_eq!(workspace_hint(&st), WORKSPACE_HINT);
}

#[test]
fn workspace_title_right_aligns_counter() {
    let mut st = ready_ws_state();
    st.focus = Focus::Workspace;
    st.ws_cursor = 1;
    let title = workspace_title(&st, 24).to_string();
    assert!(title.starts_with("proj"), "左对齐 basename：{title}");
    assert!(title.ends_with("2/3"), "右对齐光标/总数：{title}");
    assert_eq!(UnicodeWidthStr::width(title.as_str()), 24, "补足内宽");
}

#[test]
fn deep_row_keeps_filename_tail() {
    let row = ws_row(2, "a/b/c/module_name.rs", "module_name.rs", false, false);
    let (text, clipped) = ws_row_text(&row, 14, GlyphPolicy::DEFAULT);
    assert!(UnicodeWidthStr::width(text.as_str()) <= 14);
    assert!(text.ends_with("name.rs"), "保留尾部文件名：{text}");
    assert!(text.contains('…'), "中段省略：{text}");
    assert!(clipped, "名称超宽要报告给提示行");
}

/// 中段省略的核心收益：窄栏下开头与扩展名同时在，同目录长名才区分得开。
#[test]
fn narrow_row_keeps_name_head_and_extension() {
    let row = ws_row(
        2,
        "src/api/crabmate_super_long_module_name.rs",
        "crabmate_super_long_module_name.rs",
        false,
        false,
    );
    let (text, clipped) = ws_row_text(&row, 20, GlyphPolicy::DEFAULT);
    assert!(UnicodeWidthStr::width(text.as_str()) <= 20, "{text}");
    assert!(clipped);
    assert!(!text.contains("crabmate_super_long"), "整体已超宽：{text}");
    assert!(text.contains("…"), "{text}");
    // 缩进 + glyph 之后名称只剩 14 列，头尾都要有内容。
    assert!(text.contains("crab"), "保留名称开头：{text}");
    assert!(text.ends_with(".rs"), "保留扩展名：{text}");
}

/// 光标行画不全时，提示行改显示完整相对路径（终端没有 hover，这是唯一的第二展示位）。
#[test]
fn clipped_cursor_row_puts_full_rel_on_hint_line() {
    let mut st = ready_ws_state();
    st.focus = Focus::Workspace;
    st.ws_cursor = 2;
    st.ws_rows[2].rel = "src/deep/nested/very_long_module_name.rs".to_string();
    st.ws_rows[2].name = "very_long_module_name.rs".to_string();
    st.ws_rows[2].indent = 3;
    let (_, _, hint) = workspace_content(&st, SIDEBAR_WIDTH as usize - 2);
    let hint = hint.expect("聚焦要提示").to_string();
    // 提示行比行内多出缩进 + 字形，所以放下的是「路径」而不是裸文件名；
    // 仍超过列宽时继续省略，但首尾都给出来（这里放得下 `src/deep/` 前缀）。
    assert!(hint.starts_with("src/deep/"), "提示行给出相对路径：{hint}");
    assert!(hint.ends_with("module_name.rs"), "{hint}");
    assert_ne!(hint, WORKSPACE_FILE_HINT, "被截断时不再让位给键位提示");
}

/// 名字放得下时提示行保持键位，别把提示位长期占掉。
#[test]
fn fitting_cursor_row_keeps_key_hint() {
    let mut st = ready_ws_state();
    st.focus = Focus::Workspace;
    st.ws_cursor = 2; // README.md，宽度充裕
    let (_, _, hint) = workspace_content(&st, SIDEBAR_WIDTH as usize - 2);
    let hint = hint.expect("聚焦要提示").to_string();
    assert_eq!(hint, WORKSPACE_FILE_HINT, "{hint}");
}

#[test]
fn file_row_hint_mentions_insert() {
    let mut st = ready_ws_state();
    st.focus = Focus::Workspace;
    st.ws_cursor = 2; // README.md
    let (_, _, hint) = workspace_content(&st, 60);
    assert!(hint.unwrap().to_string().contains("插路径"));
}
