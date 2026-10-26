//! 浮层布局与配色的 buffer 级断言（B4）。
//!
//! 这里只测「浮层画在哪、用什么颜色」：位置/尺寸走纯函数（`settings_panel_layout`），
//! 分区与底色走 `TestBackend` 的一帧。与 `render_color_tests.rs` 分工一致 ——
//! 那边管全屏条带与聊天区，这边管浮层。

use super::super::*; // render：draw / UiState / Palette / StatusInfo / build_body_rows / chat_body_width
use super::*; // overlay：settings_panel_layout / settings_panel_content_width / choice_line
use crate::tui_mode::approve::ApprovalChoice;
use crate::tui_mode::state::ApprovalOverlay;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

const W: u16 = 80;
const H: u16 = 24;

fn status_info() -> StatusInfo {
    StatusInfo {
        api_base: "http://probe".into(),
        model: None,
        role: None,
        mode: None,
        running: false,
        cancel_sent: false,
        spinner: 0,
        view_offset: 0,
        search_term: None,
        search_total: 0,
        context: "—".to_string(),
        permission: None,
    }
}

fn render_frame(st: &mut UiState, panel: Option<&PanelContent>) -> Buffer {
    let rows = build_body_rows(st, chat_body_width(false, W));
    let info = status_info();
    let mut terminal = Terminal::new(TestBackend::new(W, H)).expect("terminal");
    terminal
        .draw(|f| draw(f, st, &info, &rows, panel))
        .expect("draw");
    terminal.backend().buffer().clone()
}

/// 按行拼出可见文本。
///
/// 必须跳过**宽字符续格**：CJK 占 2 列，第二列的 `symbol` 是空格，直接按列拼接会把
/// 「命令审批」变成「命 令 审 批」，`contains` 就再也匹配不上。
fn row_text(buf: &Buffer, y: u16) -> String {
    use unicode_width::UnicodeWidthStr;

    let mut out = String::new();
    let mut skip = 0u16;
    for x in 0..buf.area.width {
        if skip > 0 {
            skip -= 1;
            continue;
        }
        let sym = buf[(x, y)].symbol();
        let width = UnicodeWidthStr::width(sym) as u16;
        out.push_str(sym);
        if width > 1 {
            skip = width - 1;
        }
    }
    out
}

fn row_with(buf: &Buffer, needle: &str) -> u16 {
    (0..buf.area.height)
        .find(|&y| row_text(buf, y).contains(needle))
        .unwrap_or_else(|| panic!("屏幕上看不到 {needle:?}"))
}

/// 设置页四区布局：全屏铺满（不居中、不留边距），顶栏 + 左导航 + 内容 + 贴底底栏。
#[test]
fn settings_page_is_full_screen_with_four_regions() {
    let screen = Rect::new(0, 0, 80, 24);
    let l = settings_panel_layout(screen);

    // 顶栏贴顶，只占标题一行（区域靠底色块区分，不画分隔线）。
    assert_eq!((l.header.x, l.header.y, l.header.width), (0, 0, 80));
    assert_eq!(l.header.height, 1, "顶栏只有标题行");

    // 底栏贴底，末行正好结束于屏幕底部。
    assert_eq!(l.footer.y + l.footer.height, 24, "底栏必须贴底");
    assert_eq!(l.footer.width, 80);

    // 左导航 + 内容区在中间横向铺满，且互不重叠。
    assert!(l.nav.width > 0, "80 列足够画导航轨");
    assert_eq!(l.nav.x, 0);
    assert_eq!(l.content.x, l.nav.width, "内容区紧贴导航轨右侧");
    assert_eq!(l.nav.width + l.content.width, 80, "两栏合起来铺满整宽");
    assert_eq!(l.nav.height, l.content.height);
    assert_eq!(l.nav.y + l.nav.height, l.footer.y, "主体不与底栏重叠");
}

/// 宽终端同样铺满（不再有 78 列上限）；窄终端放弃导航轨，极小终端只留内容。
#[test]
fn settings_page_adapts_to_terminal_width() {
    let wide = settings_panel_layout(Rect::new(0, 0, 200, 40));
    assert_eq!(wide.nav.width + wide.content.width, 200, "宽终端也铺满");
    assert!(wide.nav.width <= 200 / 3, "导航轨不超过 1/3 宽");

    let narrow = settings_panel_layout(Rect::new(0, 0, 30, 24));
    assert_eq!(narrow.nav.width, 0, "窄终端不画导航轨");
    assert_eq!(narrow.content.width, 30, "内容区独占整宽");

    // 极小终端：整屏给内容，不画顶栏/底栏。
    let tiny = Rect::new(0, 0, 16, 6);
    let l = settings_panel_layout(tiny);
    assert_eq!(l.header.height, 0);
    assert_eq!(l.footer.height, 0);
    assert_eq!(l.content, tiny);
}

/// 内容宽度必须与绘制内宽同源 —— 否则值列会错位、行尾被裁。
#[test]
fn settings_panel_content_width_matches_the_drawn_inner_width() {
    // 直接对照绘制侧用的同一个矩形函数 —— 这才是「同源」的真正含义；
    // 在测试里把内缩公式再抄一遍，反而会把公式漂移一起锁进期望值。
    for w in [3u16, 20, 30, 44, 80, 120, 200] {
        let screen = Rect::new(0, 0, w, 24);
        let layout = settings_panel_layout(screen);
        assert_eq!(
            settings_panel_content_width(screen) as u16,
            content_inner_rect(layout.content).width,
            "内容宽度必须等于绘制侧内容区宽度（w={w}）"
        );
    }
}

/// 设置是「页面」而非浮层：整屏铺页面底色，底下的对话不再可见（对齐 desktop）。
#[test]
fn settings_page_covers_the_conversation() {
    let mut st = UiState::new();
    st.set_palette(Palette::from_theme(Some("shadcn-dark")));
    st.sidebar_visible = false;
    // 用独特标记串而不是 "hi"：子串否定断言容易被别的文案里出现的两个字母误伤。
    st.push_line(LineKind::User, "UNIQUE-CONV-MARKER");
    let pal = st.palette;
    let content = PanelContent {
        lines: vec![Line::from("模型: demo")],
        ..Default::default()
    };
    let buf = render_frame(&mut st, Some(&content));

    assert!(
        (0..H).all(|y| !row_text(&buf, y).contains("UNIQUE-CONV-MARKER")),
        "设置页应盖住底下对话（对齐 desktop 全屏页）"
    );

    let l = settings_panel_layout(Rect::new(0, 0, W, H));
    // 顶栏 / 底栏是「条」，用卡片色与页面底色区分（不画分隔线）。
    assert_eq!(buf[(1, l.header.y)].bg, pal.overlay_bg, "顶栏条");
    assert_eq!(buf[(1, l.footer.y)].bg, pal.overlay_bg, "底栏条");
    let body_y = l.header.y + l.header.height;
    assert_eq!(buf[(W / 2, body_y)].bg, pal.bg, "主体应为页面底色");
}

/// 无色模式下 `bg` / `accent` 全是 `Reset`，选中项必须退回 `REVERSED`：
/// 否则导航轨里「当前在哪个分区」完全看不出来（见 `palette.rs` 的 mono 约定）。
#[test]
fn settings_nav_keeps_the_active_section_visible_in_colourless_mode() {
    let mut st = UiState::new();
    st.set_palette(Palette::mono());
    st.sidebar_visible = false;
    assert!(st.palette.mono, "前提：mono 调色板");
    let content = PanelContent {
        title: "模型".into(),
        nav: vec![("模型".into(), true), ("会话".into(), false)],
        ..Default::default()
    };
    let buf = render_frame(&mut st, Some(&content));
    let l = settings_panel_layout(Rect::new(0, 0, W, H));

    let active = &buf[(1, l.nav.y)];
    let idle = &buf[(1, l.nav.y + 1)];
    assert!(
        active.modifier.contains(Modifier::REVERSED),
        "无色模式下选中项应退回反色"
    );
    assert!(
        !idle.modifier.contains(Modifier::REVERSED),
        "未选中项不应反色"
    );
}

/// 审批浮层：纵向居中（不再贴底），边框用 `err` 表达「回合被阻塞」。
#[test]
fn approval_overlay_is_centered_and_uses_the_error_border() {
    let mut st = UiState::new();
    st.set_palette(Palette::from_theme(Some("shadcn-dark")));
    st.sidebar_visible = false;
    let (tx, _rx) = std::sync::mpsc::channel();
    st.approval = Some(ApprovalOverlay {
        command: "rm".into(),
        args: "-rf build".into(),
        allowlist_key: None,
        answer: tx,
        expanded: false,
        scroll: 0,
    });
    let pal = st.palette;
    let buf = render_frame(&mut st, None);

    let title_y = row_with(&buf, "命令审批");
    // 聊天主体 = 整屏 − 状态栏(1) − composer(内容行 + 上下边框)；
    // 浮层在主体内垂直居中。
    let body_h = H.saturating_sub(1).saturating_sub(composer_outer_rows(&st));
    let body = Rect::new(0, 0, W, body_h);
    let want_y = body.y + (body.height - OVERLAY_HEIGHT) / 2;
    assert_eq!(title_y, want_y, "审批浮层应垂直居中，而不是贴底");
    assert_ne!(
        title_y,
        body.y + body.height - OVERLAY_HEIGHT,
        "贴底位置（旧行为）"
    );

    // 边框色：找到标题行上的左上角。
    let x = (0..W)
        .find(|&x| buf[(x, title_y)].symbol() == "┌")
        .expect("浮层左上角");
    assert_eq!(buf[(x, title_y)].fg, pal.err, "阻塞态边框应为 err 色");
}

#[test]
fn approval_overlay_fills_its_whole_rect_with_the_card_background() {
    let mut st = UiState::new();
    st.set_palette(Palette::from_theme(Some("shadcn-dark")));
    st.sidebar_visible = false;
    let (tx, _rx) = std::sync::mpsc::channel();
    st.approval = Some(ApprovalOverlay {
        command: "rm".into(),
        args: "-rf build".into(),
        allowlist_key: None,
        answer: tx,
        expanded: false,
        scroll: 0,
    });
    let pal = st.palette;
    let buf = render_frame(&mut st, None);

    let body_h = H.saturating_sub(1).saturating_sub(composer_outer_rows(&st));
    let body = Rect::new(0, 0, W, body_h);
    let rect = centered(
        body,
        body.width.saturating_sub(4).min(OVERLAY_MAX_WIDTH),
        OVERLAY_HEIGHT,
    );

    let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
    for y in rect.y..rect.y + rect.height {
        for x in rect.x..rect.x + rect.width {
            *counts.entry(format!("{:?}", buf[(x, y)].bg)).or_default() += 1;
        }
    }
    let total = (rect.width as usize) * (rect.height as usize);
    let filled = counts
        .get(&format!("{:?}", pal.overlay_bg))
        .copied()
        .unwrap_or(0);
    // 少数格子是 CJK 续格（缓冲区里恒为 Reset），故不要求 100%。
    assert!(
        filled * 100 / total >= 90,
        "浮层矩形应铺满 overlay_bg，实际 {filled}/{total}：{counts:?}"
    );
    // 浮层底色必须与聊天区可区分，否则「填充」等于没填充。
    assert_ne!(pal.overlay_bg, pal.bg, "卡片色不能与聊天区底色相同");
    assert_ne!(pal.overlay_bg, pal.chrome_bg, "卡片色不应复用条带色");
}

/// 三个选项排一行，选中项是 `err` 色块、未选中项没有底色（span 级精确断言）。
#[test]
fn choice_line_paints_only_the_selected_option_as_a_block() {
    let pal = Palette::from_theme(Some("shadcn-dark"));
    let line = choice_line(ApprovalChoice::Once, &pal);
    // spans: [「 仅此一次 」, 间隙, 「 始终允许 」, 间隙, 「 拒绝 」]
    assert_eq!(line.spans.len(), 5, "三个选项 + 两个间隙");
    assert_eq!(line.spans[0].style.bg, Some(pal.err), "选中项应铺 err 底色");
    assert_eq!(line.spans[2].style.bg, None, "未选中项不该有底色");
    assert_eq!(line.spans[4].style.bg, None, "未选中项不该有底色");
    // 标签文字与顺序必须和方向键的遍历顺序一致。
    assert_eq!(line.spans[0].content.trim(), "仅此一次");
    assert_eq!(line.spans[2].content.trim(), "始终允许");
    assert_eq!(line.spans[4].content.trim(), "拒绝");
}

/// 选中的色块要真的落到缓冲区上（span 正确 ≠ 像素正确，这里补齐另一半）。
#[test]
fn the_selected_choice_reaches_the_buffer_as_a_filled_block() {
    let mut st = UiState::new();
    st.set_palette(Palette::from_theme(Some("shadcn-dark")));
    st.sidebar_visible = false;
    let (tx, _rx) = std::sync::mpsc::channel();
    st.approval = Some(ApprovalOverlay {
        command: "rm".into(),
        args: "-rf build".into(),
        allowlist_key: None,
        answer: tx,
        expanded: false,
        scroll: 0,
    });
    // 选中「拒绝」（最后一格），色块应画在这一项上。
    st.approval_choice = ApprovalChoice::Deny;
    let pal = st.palette;
    let buf = render_frame(&mut st, None);

    let y = row_with(&buf, "始终允许"); // 选项行
    let err_cells = (0..W).filter(|&x| buf[(x, y)].bg == pal.err).count();
    // 「 拒绝 」视觉上 6 列宽，但两个汉字各有 1 列续格（缓冲区里恒为 `Reset`），
    // 真正带底色的只有 4 格：左空格 + 两个汉字的首列 + 右空格。
    assert!(
        err_cells >= 4,
        "选中项应画成连续色块，实际只有 {err_cells} 格 err 底色"
    );

    // 换成选「仅此一次」后，色块位置随之改变（说明它跟着光标走，不是写死的）。
    st.approval_choice = ApprovalChoice::Once;
    let buf = render_frame(&mut st, None);
    let y2 = row_with(&buf, "始终允许");
    let first_err_x = (0..W).find(|&x| buf[(x, y2)].bg == pal.err);
    let last_err_x = (0..W).rev().find(|&x| buf[(x, y2)].bg == pal.err);
    if let (Some(first), Some(last)) = (first_err_x, last_err_x) {
        // 色块靠左（「仅此一次」在第一格），且比「拒绝」那一块更宽。
        assert!(first < W / 2, "「仅此一次」的色块应在左侧");
        assert!(last - first >= 5, "色块宽度应覆盖标签 + 两侧空格");
    } else {
        panic!("选中「仅此一次」后没有 err 底色块");
    }
}

/// 展开档要把**整条**命令画出来。
///
/// 收起档只有 1 行预览（多行命令的第二行根本不在屏幕上），`Ctrl+E` 展开后它必须出现——
/// 这正是 G8「长命令看不到全貌」的回归点：审批问的是「要不要放行这条命令」，
/// 看不到后面还有什么就等于让人闭眼签字。
#[test]
fn expanding_the_approval_overlay_reveals_the_rest_of_the_command() {
    let mut st = UiState::new();
    st.set_palette(Palette::from_theme(Some("shadcn-dark")));
    st.sidebar_visible = false;
    let (tx, _rx) = std::sync::mpsc::channel();
    st.approval = Some(ApprovalOverlay {
        command: "bash".into(),
        args: "-c 'echo first\necho second'".into(),
        allowlist_key: None,
        answer: tx,
        expanded: false,
        scroll: 0,
    });

    let screen_text = |buf: &Buffer| -> String {
        (0..H)
            .map(|y| row_text(buf, y))
            .collect::<Vec<_>>()
            .join("\n")
    };

    let compact = screen_text(&render_frame(&mut st, None));
    // 收起档把语义换行压成空格（`Line` 里夹 `\n` 会把提示行顶掉），整条命令只占一行。
    assert!(
        compact.contains("echo first"),
        "收起档应显示第一行：{compact}"
    );
    let command_rows = |text: &str| text.lines().filter(|l| l.contains("echo ")).count();
    assert_eq!(
        command_rows(&compact),
        1,
        "收起档只能给 1 行（这就是要修的问题）：{compact}"
    );

    if let Some(ap) = st.approval.as_mut() {
        ap.expanded = true;
    }
    let expanded = screen_text(&render_frame(&mut st, None));
    assert!(
        command_rows(&expanded) >= 2 && expanded.contains("echo second"),
        "展开后第二行必须独立成行可见：{expanded}"
    );
    assert!(
        expanded.contains("已展开") || expanded.contains("Ctrl+E"),
        "标题或提示行要给出收起入口：{expanded}"
    );
}
