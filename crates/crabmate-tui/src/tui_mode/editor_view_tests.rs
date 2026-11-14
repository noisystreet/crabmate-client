use std::sync::mpsc;

use ratatui::style::Color;
use unicode_width::UnicodeWidthStr;

use super::super::palette::Palette;
use super::super::test_support::{test_app, test_client, test_prefs};
use super::{EditorState, LaneApp, MIN_VIEW_ROWS, WHEEL_LINES, basename, clip, normalize_rel};
use crate::serve::WorkspaceFile;

/// 外壳层的最小夹具：编辑器状态与命令都在 `LaneApp` 上，只测 `EditorState` 会漏掉
/// 「命令到底有没有接上」这一层。
fn app(f: impl FnOnce(&mut LaneApp<'_>)) {
    let client = test_client();
    let mut prefs = test_prefs();
    let (tx, rx) = mpsc::channel();
    let (job_tx, _job_rx) = mpsc::channel();
    let mut app = test_app(&client, &mut prefs, rx, tx, job_tx);
    f(&mut app);
}

#[test]
fn editor_command_opens_and_chat_returns() {
    app(|a| {
        a.editor_command(Some("src/lib.rs".to_string()));
        assert!(a.st.editor.open);
        assert_eq!(a.st.editor.rel.as_deref(), Some("src/lib.rs"));
        // 没有 tokio runtime（单测）时停在 loading：不是假成功，也不是空文件
        assert!(a.st.editor.loading);

        a.editor_close();
        assert!(!a.st.editor.open);
        assert!(matches!(a.st.focus, super::super::state::Focus::Input));

        // 幂等：已经在聊天时再 /chat 只是多说一句，不改状态
        a.editor_close();
        assert!(!a.st.editor.open);
    });
}

/// `Ctrl+T` / 点状态栏：一次开、一次关，且**复用** `/editor` 与 `/chat` 的语义。
///
/// 特意分两段断言：打开方向要重读当前文件（不沿用旧内容），关闭方向要把焦点
/// 交回输入框（切回来就是为了接着打字）。
#[test]
fn toggle_view_flips_both_ways() {
    app(|a| {
        a.toggle_view();
        assert!(a.st.editor.open, "关 → 开");
        assert!(a.st.editor.rel.is_none(), "没有文件就开占位视图，不编路径");

        a.toggle_view();
        assert!(!a.st.editor.open, "开 → 关");
        assert!(matches!(a.st.focus, super::super::state::Focus::Input));
    });
}

/// 已经有文件时再切回来，必须重读那一份：文件可能已被 agent 改过，
/// 沿用旧内容比多一次 GET 更坏。
#[test]
fn toggle_view_rereads_the_open_file() {
    app(|a| {
        a.editor_command(Some("src/lib.rs".to_string()));
        a.toggle_view();
        assert!(!a.st.editor.open);
        a.toggle_view();
        assert!(a.st.editor.open);
        assert_eq!(a.st.editor.rel.as_deref(), Some("src/lib.rs"));
        assert!(
            a.st.editor.loading,
            "重读在途（单测没有 runtime，停在 loading）"
        );
    });
}

#[test]
fn a_reply_for_a_file_we_already_left_is_dropped() {
    app(|a| {
        a.editor_command(Some("a.rs".to_string()));
        a.editor_command(Some("b.rs".to_string()));
        a.on_workspace_file(
            "a.rs",
            Ok(WorkspaceFile {
                content: Some("OLD".to_string()),
                error: None,
            }),
        );
        assert_eq!(a.st.editor.rel.as_deref(), Some("b.rs"));
        assert!(
            a.st.editor.lines.is_empty(),
            "a.rs 的迟到回包不能写进 b.rs 的视图"
        );
    });
}

#[test]
fn opening_a_blank_path_does_not_switch_the_view() {
    app(|a| {
        a.editor_command(Some("   ".to_string()));
        assert!(!a.st.editor.open, "空路径只该给用法提示");
    });
}

/// 只输入 `/editor`（这条 lane 还没打开过文件）也要进视图。
///
/// 只往聊天区塞一行用法提示的话，用户看到的是「命令没生效」；更糟的是树上
/// 双击/Enter 打开文件只在视图开着时才生效，于是他接着点也没反应。
#[test]
fn a_bare_editor_command_opens_the_view_without_a_file() {
    app(|a| {
        a.editor_command(None);
        assert!(a.st.editor.open, "只输入 /editor 也要进视图");
        assert!(a.st.editor.rel.is_none(), "没有文件就不该编一个路径出来");
        let rows = super::view_lines(&mut a.st.editor, None, 60, 20, &a.st.palette);
        let text = rows
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("未选文件"), "占位行要说清怎么选文件：{text}");
        assert!(!text.contains("空文件"), "没选文件不能报「空文件」：{text}");
    });
}

#[test]
fn paths_are_normalized_before_the_request() {
    assert_eq!(normalize_rel("  ./src/lib.rs  "), "src/lib.rs");
    assert_eq!(normalize_rel("src/"), "src");
    // `..` 不做折叠：越权判定在 serve，客户端折叠后可能与服务端解析出不同的文件
    assert_eq!(normalize_rel("../x"), "../x");
}

#[test]
fn tabs_expand_and_cr_is_dropped() {
    let mut ed = EditorState::default();
    ed.open_file("src/lib.rs", 7);
    ed.accept_limited("fn main() {\r\n\tlet x = 1;\r\n}\r\n", 100);
    assert_eq!(ed.lines[0], "fn main() {");
    // tab 展开成 4 空格，行尾 `\r` 不留
    assert_eq!(ed.lines[1], "    let x = 1;");
    assert_eq!(ed.lines[2], "}");
    assert!(!ed.loading && ed.error.is_none() && !ed.truncated);
}

#[test]
fn over_limit_is_truncated_and_says_so() {
    let mut ed = EditorState::default();
    ed.accept_limited("a\nb\nc\nd\n", 2);
    assert_eq!(ed.lines, vec!["a".to_string(), "b".to_string()]);
    assert!(ed.truncated, "截断要能被提示行说出来");
}

#[test]
fn scroll_clamps_within_file() {
    let mut ed = EditorState::default();
    ed.accept_limited("a\nb\nc\n", 100);
    ed.scroll_by(-5);
    assert_eq!(ed.scroll, 0, "不能滚到负偏移");
    ed.scroll_by(99);
    assert_eq!(ed.scroll, 2, "上界是末行");
    ed.scroll_to_top();
    assert_eq!(ed.scroll, 0);
    ed.scroll_to_bottom();
    assert_eq!(ed.scroll, 2);
}

#[test]
fn failure_clears_content_but_keeps_the_file_name() {
    let mut ed = EditorState::default();
    ed.open_file("bin/blob", 1);
    ed.accept("旧内容");
    ed.fail("二进制文件不支持预览".to_string());
    assert!(ed.lines.is_empty());
    assert_eq!(ed.rel.as_deref(), Some("bin/blob"));
    assert!(ed.error.as_deref().unwrap_or("").contains("二进制"));
    assert!(!ed.loading);
}

#[test]
fn close_keeps_the_cursor_for_the_next_visit() {
    let mut ed = EditorState::default();
    ed.accept_limited("a\nb\nc\n", 100);
    ed.scroll_by(2);
    ed.find.begin();
    ed.find.push('b');
    ed.close();
    assert!(!ed.open);
    assert!(
        !ed.find.open,
        "带着查找输入态离开视图，回来第一下敲字会悄悄进查找词"
    );
    assert_eq!(ed.find.query, "b", "查询词与高亮留着");
    assert_eq!(ed.scroll, 2, "再 /editor 回来还应停在同一处");
}

#[test]
fn clip_keeps_wide_chars_whole() {
    assert_eq!(clip("abc", 5), "abc");
    assert_eq!(clip("abcdef", 4), "abc…");
    // 全角按 2 列算：3 个全角 = 6 列，宽度 4 只能放 1 个 + 省略号
    assert_eq!(clip("笔记说明", 4), "笔…");
}

#[test]
fn basename_matches_the_right_pane_title() {
    assert_eq!(basename("/home/u/proj"), "proj");
    assert_eq!(basename("/home/u/proj/"), "proj");
    assert_eq!(basename("proj"), "proj");
}

/// 渲染后的整段文本（标题、行号、提示行都在里面）。
fn view_text(a: &mut LaneApp<'_>, width: usize, height: usize) -> String {
    super::view_lines(&mut a.st.editor, None, width, height, &a.st.palette)
        .iter()
        .map(std::string::ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

/// 行号列画的是**绝对行号**，滚动之后跟着各自的内容走。
///
/// 两种坏法都能被这条抓住：没画行号（整列消失），以及滚动后按可见序号重新编号
/// （从 1 数起 —— 看文件尾部时会误以为自己在文件开头）。
#[test]
fn line_numbers_are_absolute_and_follow_the_scroll() {
    app(|a| {
        a.st.editor.open_file("src/lib.rs", 0);
        a.st.editor.accept_limited("l1\nl2\nl3\nl4\n", 100);
        // 视口 5 行 = 3 行内容：滚 3 行后被收敛到 1，画面是 l2 / l3 / l4
        a.editor_wheel(false);
        let text = view_text(a, 60, 5);
        assert!(text.contains("  4 │ l4"), "行号要跟着内容走：{text}");
        assert!(text.contains("  2 │ l2"), "{text}");
        assert!(!text.contains("  1 │ l1"), "滚出去的行不该还在：{text}");
    });
}

/// 滚轮一格 [`WHEEL_LINES`] 行，顶部与末尾都夹住。
#[test]
fn the_wheel_moves_three_lines_and_clamps_at_both_ends() {
    app(|a| {
        a.st.editor.open_file("a.rs", 0);
        let body: String = (1..=40).map(|i| format!("l{i}\n")).collect();
        a.st.editor.accept_limited(&body, 100);
        a.editor_wheel(false);
        assert_eq!(a.st.editor.scroll, WHEEL_LINES, "一格 = 3 行");
        a.editor_wheel(true);
        assert_eq!(a.st.editor.scroll, 0);
        a.editor_wheel(true);
        assert_eq!(a.st.editor.scroll, 0, "顶部再往上不越界");
        for _ in 0..40 {
            a.editor_wheel(false);
        }
        assert_eq!(a.st.editor.scroll, 39, "末尾夹在末行");
    });
}

#[test]
fn view_rows_never_collapse_below_one() {
    // 高度只够标题 + 提示时，内容区仍是 1 行而不是 0（否则整屏像是空的）
    assert_eq!(EditorState::visible_rows(MIN_VIEW_ROWS), 1);
    assert_eq!(EditorState::visible_rows(0), 1);
    assert_eq!(EditorState::visible_rows(10), 8);
}

/// 多 lane 时第二行标题补 `lane <名字>`；单 lane 不写（恒等于「当前这条」，只占宽度）。
#[test]
fn the_title_names_the_lane_only_when_there_are_several() {
    app(|a| {
        a.st.editor.open_file("src/lib.rs", 0);
        a.st.editor.accept_limited("fn main() {}\n", 100);
        let render = |a: &mut LaneApp<'_>| {
            super::view_lines(&mut a.st.editor, Some("/ws/proj"), 80, 10, &a.st.palette)
                .iter()
                .map(std::string::ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        };

        // `lane_label` 由外壳每帧按 lane 快照写入（单 lane 是 `None`）。
        let one = render(a);
        assert!(one.contains("工作区 proj · 源码"), "{one}");
        assert!(!one.contains("lane "), "单 lane 不该占这段宽度：{one}");

        a.st.editor.lane_label = Some("staging".to_string());
        let two = render(a);
        assert!(two.contains("工作区 proj · 源码 · lane staging"), "{two}");
    });
}

/// 渲染这一帧就得把命中集合算出来：高亮按行号画，渲染前不同步就会画到别的行上。
#[test]
fn rendering_syncs_the_find_hits() {
    app(|a| {
        a.st.editor.open_file("src/lib.rs", 0);
        a.st.editor.accept_limited("alpha\nbeta\nALPHA\n", 100);
        for ch in "alp".chars() {
            a.st.editor.find.push(ch);
        }
        let text = view_text(a, 60, 10);
        assert_eq!(
            a.st.editor.find.hits,
            vec![0, 2],
            "大小写不敏感的命中：{text}"
        );
        assert!(
            text.contains("查找 alp 第 1 / 2 处"),
            "「第 n / N 处」要写在提示行最前面：{text}"
        );
    });
}

/// 查找输入态：整行让给查找（含「无匹配」），且说清只搜本文件。
#[test]
fn the_find_prompt_replaces_the_hint_line() {
    app(|a| {
        a.st.editor.open_file("a.rs", 0);
        a.st.editor.accept_limited("hello\n", 100);
        a.st.editor.find.begin();
        a.st.editor.find.push('z');
        let text = view_text(a, 80, 10);
        assert!(text.contains("查找：z▏ 无匹配"), "{text}");
        assert!(text.contains("只搜本文件"), "要说清与 /find 的分工：{text}");
        assert!(!text.contains("PgUp/PgDn"), "输入态整行让给查找：{text}");
    });
}

/// md 预览的命中按**物理行**算：一行源码折成几行后，命中落在折出来的那一行上。
#[test]
fn preview_hits_follow_the_physical_rows() {
    app(|a| {
        a.st.editor.open_file("README.md", 0);
        let body = format!("{}\n目标行\n", "0123456789".repeat(3));
        a.st.editor.accept_limited(&body, 100);
        a.st.editor.find.push('目');
        // 渲染建出物理行缓存，`sync_find` 才有东西可数。
        let _ = view_text(a, 10, 20);
        let hits = a.st.editor.find.hits.clone();
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert!(
            hits[0] > 0,
            "折行把目标行推到后面的物理行（源码行 1 → 物理行 {hits:?}）"
        );
    });
}

/// 命中 / 锚定要真的画到 span 上：只把命中集合算对而不染行，屏幕上与「没找到」一模一样。
///
/// 锚定行用 `hit` 作**底色**（整行反色），其余命中行改 `hit` 前景，未命中行保持正文色 ——
/// 与聊天区 `/find` 同一套判据（`md::highlight_override`）。
#[test]
fn find_highlights_reach_the_spans() {
    app(|a| {
        a.st.editor.open_file("src/lib.rs", 0);
        a.st.editor.accept_limited("alpha\nbeta\nalpha\n", 100);
        for ch in "alpha".chars() {
            a.st.editor.find.push(ch);
        }
        let rows = super::view_lines(&mut a.st.editor, None, 60, 10, &a.st.palette);
        let hit = a.st.palette.hit;
        let fg = a.st.palette.fg;

        // 标题两行 + 三条源码行 + 提示行；每行 span = [行号, "│ ", 正文]。
        assert_eq!(rows.len(), 6, "{rows:?}");
        assert_eq!(
            rows[2].spans[2].style.bg,
            Some(hit),
            "锚定行整行反色：{:?}",
            rows[2].spans[2]
        );
        assert_eq!(
            rows[4].spans[2].style.fg,
            Some(hit),
            "其余命中行改 hit 前景"
        );
        assert_eq!(rows[3].spans[2].style.fg, Some(fg), "未命中行保持正文色");
    });
}

// ---- 语法着色接线（issue #510 的 P2） ----
//
// 这一组测的都是「纯词法器对了，但接进渲染路径时接错」才出现的故障：吞字、跨行状态
// 没用上、查找色被语法色盖掉、缓存画到下一份文件上。词法器自己的单测在 `editor_hl_tests`。

/// 关键字与注释各自成段、颜色取自 `syn_*`，且分段不吞字不重排。
///
/// 「不吞字」不是形式主义：分段按 char 区间切，`HlSpan` 错位一个字符的后果是屏幕上
/// 少一个字 —— 比「没有颜色」难查得多。
#[test]
fn source_view_colours_keywords_and_comments() {
    app(|a| {
        a.st.editor.open_file("src/lib.rs", 0);
        a.st.editor.accept_limited("let x = 1; // note\n", 100);
        let rows = super::view_lines(&mut a.st.editor, None, 60, 10, &a.st.palette);
        let pal = a.st.palette;
        // span 0 = 行号、1 = 「│ 」、其余是正文分段。
        let body = &rows[2].spans[2..];
        let text: String = body.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "let x = 1; // note", "分段不能吞字：{body:?}");
        assert_eq!(body[0].content.as_ref(), "let");
        assert_eq!(body[0].style.fg, Some(pal.syn_keyword), "let 是关键字");
        assert_eq!(body.last().expect("末段").content.as_ref(), "// note");
        assert_eq!(
            body.last().unwrap().style.fg,
            Some(pal.syn_comment),
            "注释走 syn_comment"
        );
        assert!(
            body.iter().any(|s| s.style.fg == Some(pal.syn_number)),
            "数字单独一段：{body:?}"
        );
    });
}

/// 跨行块注释的第二行**从行首**就是注释色：起始态来自 `scan_states`，不是从 `Normal` 重扫。
#[test]
fn source_view_block_comment_continues_on_the_next_line() {
    app(|a| {
        a.st.editor.open_file("src/lib.rs", 0);
        a.st.editor
            .accept_limited("let a = 1;\n/* open\nstill */ let b = 2;\n", 100);
        let rows = super::view_lines(&mut a.st.editor, None, 60, 12, &a.st.palette);
        let pal = a.st.palette;
        // 两条标题占 `rows[0..2]`，源码行 i → `rows[2 + i]`；跨行注释的第二行是 i = 2。
        let body = &rows[4].spans[2..];
        assert_eq!(body[0].content.as_ref(), "still */");
        assert_eq!(
            body[0].style.fg,
            Some(pal.syn_comment),
            "第二行行首仍是注释"
        );
        assert!(
            body.iter().any(|s| s.style.fg == Some(pal.syn_keyword)),
            "闭合后同一行的 let 恢复成关键字：{body:?}"
        );
    });
}

/// 超宽行按**显示宽度**裁：宽字符不切半，末尾补 `…`，总宽不超出正文预算。
#[test]
fn source_view_clips_wide_characters_by_display_width() {
    app(|a| {
        a.st.editor.open_file("src/lib.rs", 0);
        a.st.editor
            .accept_limited("// 汉字汉字汉字汉字汉字汉字\n", 100);
        // 宽 20、行号栏 3 → 正文预算 15（裁切时给 `…` 留一列）。
        let rows = super::view_lines(&mut a.st.editor, None, 20, 10, &a.st.palette);
        let body = &rows[2].spans[2..];
        let text: String = body.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.ends_with('…'), "超宽行要补省略号：{text:?}");
        assert!(text.width() <= 15, "{text:?} 宽 {}", text.width());
        assert!(text.starts_with("// 汉字"), "宽字符不得被切半：{text:?}");
    });
}

/// 内容变了，语法缓存必须重建 —— 否则会把上一份文件的关键字色画在新内容上。
#[test]
fn syntax_cache_rebuilds_when_the_content_changes() {
    app(|a| {
        a.st.editor.open_file("src/lib.rs", 0);
        a.st.editor.accept_limited("let a = 1;\n", 100);
        let _ = super::view_lines(&mut a.st.editor, None, 60, 10, &a.st.palette);
        let first = a.st.editor.hl.as_ref().expect("首次渲染建缓存").rev;
        assert_eq!(first, a.st.editor.text_rev, "缓存键跟进内容版本");

        a.st.editor.accept_limited("// 只有注释\n", 100);
        let _ = super::view_lines(&mut a.st.editor, None, 60, 10, &a.st.palette);
        let second = a.st.editor.hl.as_ref().expect("重建").rev;
        assert_ne!(second, first, "内容变了要重建");
        assert_eq!(second, a.st.editor.text_rev);
    });
}

/// 不认识的扩展名整份不上色（「认不出就不上色」），且仍然只占一段。
#[test]
fn unknown_extension_stays_plain() {
    app(|a| {
        a.st.editor.open_file("notes.xyz", 0);
        a.st.editor.accept_limited("let x = 1; // note\n", 100);
        let rows = super::view_lines(&mut a.st.editor, None, 60, 10, &a.st.palette);
        let pal = a.st.palette;
        let body = &rows[2].spans[2..];
        assert_eq!(body.len(), 1, "不上色时整行一段：{body:?}");
        assert_eq!(body[0].style.fg, Some(pal.fg));
    });
}

/// 查找命中压过语法色：命中的整行同款，语法色不得冒出来。
///
/// 优先级是刻意的：正在找的东西 > 装饰。写反了的话，「搜到关键字那一行」看起来会像
/// 没搜到 —— 而它恰好是搜索最常命中的行。
#[test]
fn find_highlight_wins_over_syntax_colour() {
    app(|a| {
        a.st.editor.open_file("src/lib.rs", 0);
        a.st.editor.accept_limited("let alpha = 1;\n", 100);
        for ch in "alpha".chars() {
            a.st.editor.find.push(ch);
        }
        let rows = super::view_lines(&mut a.st.editor, None, 60, 10, &a.st.palette);
        let pal = a.st.palette;
        let body = &rows[2].spans[2..];
        let first = body[0].style;
        assert!(
            body.iter().all(|s| s.style == first),
            "命中行应整行同款：{body:?}"
        );
        assert!(
            body.iter()
                .all(|s| s.style.fg != Some(pal.syn_keyword) && s.style.fg != Some(pal.syn_number)),
            "语法色必须被查找命中压过：{body:?}"
        );
    });
}

/// 无色模式：六个语法槽全部 `Reset`。
///
/// 看着琐碎，但漏一个的后果只在 `NO_COLOR` 终端里可见 —— 那正是最难被发现的一类回归。
/// `MONO` 是手写 8 次字段，编译器补不出这一条。
#[test]
fn mono_palette_wipes_every_syntax_colour() {
    let pal = Palette::mono();
    for (name, color) in [
        ("syn_comment", pal.syn_comment),
        ("syn_str", pal.syn_str),
        ("syn_number", pal.syn_number),
        ("syn_keyword", pal.syn_keyword),
        ("syn_type", pal.syn_type),
        ("syn_fn", pal.syn_fn),
    ] {
        assert_eq!(color, Color::Reset, "{name} 在无色模式下必须 Reset");
    }
}
