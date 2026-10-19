//! 颜色级渲染回归：把「配错了但看起来只是有点怪」的毛病钉在 CI 里。
//!
//! 与 `render_tests.rs` 的分工：那边测结构（字符数、span 数、行数、宽度不变式），
//! 这边只测**真正落到缓冲区的颜色**（`TestBackend`，不需要 TTY）。
//!
//! 两条编写原则：
//! 1. **断言语义槽位，不断言 RGB** —— 主题取值以后要调，测试不该因此变红；
//!    但「条带该用 `chrome_bg` 却用了 `bg`」必须变红。
//! 2. **不写死坐标** —— 按特征（含某段文本的行、`┌` 所在格）定位，
//!    这样 composer 行数、hint 占位等布局调整不会误伤本文件的断言。

use super::*;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::Color;

const W: u16 = 80;
const H: u16 = 12;

fn status_info() -> StatusInfo {
    StatusInfo {
        api_base: "http://probe".into(),
        model: Some("m".into()),
        role: None,
        mode: None,
        running: false,
        cancel_sent: false,
        spinner: 0,
        view_offset: 0,
        search_term: None,
        search_total: 0,
    }
}

/// 渲染一屏并取回缓冲区（无侧栏：聊天列即整屏，断言不必再算侧栏边界）。
fn render_screen(st: &UiState) -> Buffer {
    let rows = build_body_rows(st, chat_body_width(false, W));
    let info = status_info();
    let mut terminal = Terminal::new(TestBackend::new(W, H)).expect("terminal");
    terminal
        .draw(|f| draw(f, st, &info, &rows, None))
        .expect("draw");
    terminal.backend().buffer().clone()
}

fn text_row(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
}

/// 含 `needle` 的行号；找不到直接失败并打印整屏，便于定位。
fn row_with(buf: &Buffer, needle: &str) -> u16 {
    (0..buf.area.height)
        .find(|&y| text_row(buf, y).contains(needle))
        .unwrap_or_else(|| {
            let dump: Vec<String> = (0..buf.area.height).map(|y| text_row(buf, y)).collect();
            panic!("屏幕上看不到 {needle:?}；整屏：\n{}", dump.join("\n"));
        })
}

/// 铺了指定底色、且格子里有可见字符的单元格数量。
fn painted_cells(buf: &Buffer, bg: Color) -> usize {
    (0..buf.area.height)
        .flat_map(|y| (0..buf.area.width).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let cell = &buf[(x, y)];
            cell.bg == bg && !cell.symbol().trim().is_empty()
        })
        .count()
}

/// 某行里底色为 `want` 的格子数。
///
/// 逐格断言行不通：双宽字符（CJK）的第二列在缓冲区里是**续格**，`symbol` 为空格、
/// `fg`/`bg` 都是 `Reset`（ratatui 不给续格上样式，真实终端也只用前一个字符的右半
/// 来显示）。所以这里数「有多少格是目标底色」，再用「一格都不许是另一个槽位的底色」
/// 反向锁住槽位写反的回归。
fn row_bg_count(buf: &Buffer, y: u16, want: Color) -> usize {
    (0..buf.area.width)
        .filter(|&x| buf[(x, y)].bg == want)
        .count()
}

/// 六种主题（默认 + 5 套 Desktop 主题）的条带 / 聊天区底色都必须按槽位取色。
///
/// 这是 A1 主题化最核心的不变式：顶栏、状态栏走 `chrome_bg`（"框"），
/// 正文区走 `bg`（"纸"）。两者一旦互相写反，深色主题下几乎看不出来，
/// 浅色主题下会直接糊成一片 —— 正是没有颜色断言时最容易漏掉的回归。
#[test]
fn every_theme_paints_bands_and_chat_area_from_palette_slots() {
    for theme in [
        None,
        Some("crabmate-light"),
        Some("shadcn-light"),
        Some("shadcn-dark"),
        Some("material-dark"),
        Some("high-contrast-dark"),
    ] {
        let pal = Palette::from_theme(theme);
        let mut st = UiState::new();
        st.set_palette(pal);
        st.sidebar_visible = false;
        st.push_line(LineKind::User, "hi");
        let buf = render_screen(&st);

        // 顶栏 / 状态栏 = chrome_bg（"框"），聊天区 = bg（"纸"）。两处都做反向断言：
        // 只要涂到了另一个槽位的颜色，就说明取色写反了，不管涂了多少格。
        let half = W as usize / 2;
        for (y, what) in [(0, "顶栏"), (H - 1, "状态栏")] {
            assert_eq!(
                row_bg_count(&buf, y, pal.bg),
                0,
                "{what}不该用聊天区底色（theme={theme:?}）"
            );
            assert!(
                row_bg_count(&buf, y, pal.chrome_bg) >= half,
                "{what}应以 chrome_bg 为底（theme={theme:?}，实测 {} 格）",
                row_bg_count(&buf, y, pal.chrome_bg)
            );
        }
        // 顶栏下方第一行即聊天区（不含 composer）：底色必须是 bg，不是 chrome_bg。
        assert_eq!(
            row_bg_count(&buf, 1, pal.chrome_bg),
            0,
            "聊天区不该用条带底色（theme={theme:?}）"
        );
        assert!(
            row_bg_count(&buf, 1, pal.bg) >= half,
            "聊天区应以 bg 为底（theme={theme:?}，实测 {} 格）",
            row_bg_count(&buf, 1, pal.bg)
        );
    }
}

/// composer 边框颜色随焦点走：聚焦输入框 `accent`，焦点在侧栏/工作区 `muted`。
///
/// 这是 A5 的唯一视觉反馈 —— 边框不是 `accent` 时用户看不出「现在打字会进哪里」，
/// 而边框又只在有焦点时才被注意，改错了很难被发现。
#[test]
fn composer_border_follows_focus() {
    let pal = Palette::default();
    let mut st = UiState::new();
    st.set_palette(pal);
    st.sidebar_visible = false;
    st.focus = Focus::Input;

    let focused = render_screen(&st);
    let corner = row_with(&focused, "┌");
    let x = (0..W)
        .find(|&x| focused[(x, corner)].symbol() == "┌")
        .expect("composer 左上角");
    assert_eq!(
        focused[(x, corner)].fg,
        pal.accent,
        "聚焦输入框时 composer 边框应为 accent"
    );

    st.focus = Focus::Sidebar;
    let blurred = render_screen(&st);
    let corner = row_with(&blurred, "┌");
    let x = (0..W)
        .find(|&x| blurred[(x, corner)].symbol() == "┌")
        .expect("composer 左上角");
    assert_eq!(
        blurred[(x, corner)].fg,
        pal.muted,
        "焦点不在输入框时 composer 边框应退到 muted"
    );
}

/// `/find` 的命中必须真的画上底色：锚定行用 `hit`，没有搜索时一个格子都不该有。
///
/// 之前的颜色相关断言只到 `Span::style`（`render_tests::search_marks_and_anchors_rows`），
/// 那只能证明「行模型里带着颜色」，证明不了它穿过 `Paragraph` 之后还留在缓冲区里。
#[test]
fn search_hit_reaches_the_buffer_and_vanishes_without_search() {
    let pal = Palette::default();
    let mut st = UiState::new();
    st.set_palette(pal);
    st.sidebar_visible = false;
    st.push_line(LineKind::Assistant, "hello world");
    st.push_line(LineKind::Assistant, "nothing to see");

    let quiet = render_screen(&st);
    assert_eq!(
        painted_cells(&quiet, pal.hit),
        0,
        "没有搜索时不该出现 hit 底色"
    );

    st.start_search("hello");
    let hit = render_screen(&st);
    assert!(
        painted_cells(&hit, pal.hit) >= "hello".len(),
        "命中行的 hello 应至少有 5 个格子铺 hit 底色，实际 {}",
        painted_cells(&hit, pal.hit)
    );
}

/// 行内代码得有自己的底：否则 `code_bg` 这一档主题色等于白定义。
#[test]
fn inline_code_gets_code_background() {
    let pal = Palette::default();
    let mut st = UiState::new();
    st.set_palette(pal);
    st.sidebar_visible = false;
    st.push_line(LineKind::Assistant, "run `cargo test` now");
    let buf = render_screen(&st);
    let y = row_with(&buf, "cargo");
    assert!(
        (0..W).any(|x| buf[(x, y)].bg == pal.code_bg),
        "行内代码应铺 code_bg；该行：{:?}",
        text_row(&buf, y)
    );
}

/// 无色模式（`NO_COLOR` / `TERM=dumb`）下整屏不得有任何颜色 —— 语义只能靠修饰位。
///
/// 这条比“看起来没颜色”更严格：任何一处漏改成 `pal.fg` 之外的具名色都会被抓到。
#[test]
fn mono_mode_paints_no_color_at_all() {
    let mut st = UiState::new();
    st.set_palette(Palette::mono());
    st.sidebar_visible = false;
    st.push_line(LineKind::User, "hello");
    st.push_line(LineKind::Assistant, "world `code`");
    st.start_search("world");
    let buf = render_screen(&st);
    for y in 0..H {
        for x in 0..W {
            let cell = &buf[(x, y)];
            assert_eq!(
                cell.fg,
                Color::Reset,
                "无色模式不该有前景色（{x},{y} = {:?}）",
                cell.symbol()
            );
            assert_eq!(
                cell.bg,
                Color::Reset,
                "无色模式不该有背景色（{x},{y} = {:?}）",
                cell.symbol()
            );
        }
    }
}

/// 代码块：围栏记号不上屏、语言标签替代它、代码行整片 `code_bg`。
///
/// 这三条必须在缓冲区层面验证 —— `md_tests` 只能证明字符流里有正确的
/// `(Style, char)`，证明不了它最后有没有被画成像素（行内代码底色曾经就是这样丢的）。
#[test]
fn code_block_renders_label_and_background_without_fence_marks() {
    let pal = Palette::default();
    let mut st = UiState::new();
    st.set_palette(pal);
    st.sidebar_visible = false;
    st.push_line(LineKind::Assistant, "```rust\nlet x = 1;\n```");
    let buf = render_screen(&st);

    // 语言标签行存在、代码行存在
    let label_y = row_with(&buf, "rust");
    let code_y = row_with(&buf, "let x = 1;");
    assert_ne!(label_y, code_y, "标签与代码不该在同一行");

    // 围栏记号本身不能在屏上任何位置出现
    for y in 0..H {
        assert!(
            !text_row(&buf, y).contains("```"),
            "第 {y} 行仍显示代码围栏记号：{:?}",
            text_row(&buf, y)
        );
    }

    // 代码行整片 code_bg：铺成等宽矩形，应接近整行宽度（≥ 60 格），
    // 而不是「内容有多宽铺多宽」。
    let bg_cells = (0..W)
        .filter(|&x| buf[(x, code_y)].bg == pal.code_bg)
        .count();
    assert!(
        bg_cells >= 50,
        "代码行应铺开成矩形（≥50 格，内容仅 13 格），实际只有 {bg_cells} 格；该行：{:?}",
        text_row(&buf, code_y)
    );
    // 标签行同样参与矩形（右上角不能缺），只是最左几列是更深的 gutter 色带。
    let label_code_cells = (0..W)
        .filter(|&x| buf[(x, label_y)].bg == pal.code_bg)
        .count();
    assert!(
        label_code_cells >= 50,
        "标签行也应铺到同一宽度，实际 {label_code_cells} 格；该行：{:?}",
        text_row(&buf, label_y)
    );
    // 整行同一底色：左端不能有更深的 gutter 色带（那条接缝看起来就是边框线）
    assert!(
        (0..W).all(|x| buf[(x, code_y)].bg != pal.chrome_bg),
        "代码行左端不该再有 gutter 色带：{:?}",
        text_row(&buf, code_y)
    );
}

/// 表格在缓冲区层面：整屏不得出现 `|`，且有一行 `─` 分隔线。
///
/// 字符流测试只证明 span 里有正确内容，证明不了「竖线真的没被画上去」。
#[test]
fn table_renders_without_pipes_on_screen() {
    let pal = Palette::default();
    let mut st = UiState::new();
    st.set_palette(pal);
    st.sidebar_visible = false;
    st.push_line(
        LineKind::Assistant,
        "| 模型 | 上下文 |\n| --- | --- |\n| gpt-4o | 128k |",
    );
    let buf = render_screen(&st);
    let mut saw_rule = false;
    for y in 0..H {
        let t = text_row(&buf, y);
        assert!(!t.contains('|'), "第 {y} 行仍有竖线：{t:?}");
        if t.contains('─') {
            saw_rule = true;
        }
    }
    assert!(
        saw_rule,
        "应有一行 `─` 分隔线；整屏：{:?}",
        (0..6).map(|y| text_row(&buf, y)).collect::<Vec<_>>()
    );
}

/// 结构性标记在缓冲区层面：`#`、`[ ]`、`[x]` 不上屏，`▌` / `─` / `☑` 上屏。
#[test]
fn structural_markers_do_not_reach_the_screen() {
    let pal = Palette::default();
    let mut st = UiState::new();
    st.set_palette(pal);
    st.sidebar_visible = false;
    st.push_line(
        LineKind::Assistant,
        "# 一级\n\n---\n\n- [x] 完成\n- [ ] 待办",
    );
    let buf = render_screen(&st);
    let rows: Vec<String> = (0..H).map(|y| text_row(&buf, y)).collect();
    let joined = rows.join("\n");
    assert!(!joined.contains('#'), "标题标记不该上屏：{joined}");
    assert!(
        !joined.contains("[x]") && !joined.contains("[ ]"),
        "任务标记不该上屏：{joined}"
    );
    assert!(joined.contains('▌'), "H1 应有竖条：{joined}");
    assert!(
        joined.contains('☑') && joined.contains('☐'),
        "应有勾选框：{joined}"
    );
    assert!(
        rows.iter().any(|r| r.matches('─').count() > 20),
        "应有横向分隔线：{joined}"
    );
}
