//! 浮层布局与配色的 buffer 级断言（B4）。
//!
//! 这里只测「浮层画在哪、用什么颜色」：位置/尺寸走纯函数（`settings_panel_rect`），
//! 居中与边框色走 `TestBackend` 的一帧。与 `render_color_tests.rs` 分工一致 ——
//! 那边管全屏条带与聊天区，这边管浮层。

use super::super::*; // render：draw / UiState / Palette / StatusInfo / build_body_rows / chat_body_width
use super::*; // overlay：settings_panel_rect / settings_panel_content_width / choice_line
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
    }
}

fn render_frame(st: &UiState, panel: Option<&PanelContent>) -> Buffer {
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

/// 设置面板弹层：居中、宽度有上限、高度随内容，极小终端退回整屏。
#[test]
fn settings_panel_dialog_is_centered_and_bounded() {
    let screen = Rect::new(0, 0, 80, 24);
    let r = settings_panel_rect(screen, 10);
    assert_eq!(r.width, 76, "左右各留 2 列");
    assert_eq!(r.height, 12, "内容 10 行 + 上下边框");
    assert_eq!((r.x, r.y), (2, 6), "水平、垂直都居中");

    // 宽终端受 78 列上限约束，但仍在行内居中。
    let wide = settings_panel_rect(Rect::new(0, 0, 200, 40), 10);
    assert_eq!(wide.width, 78);
    assert_eq!(wide.x, (200 - 78) / 2);

    // 内容比可用高度多时按可用区裁剪（上下各留 1 行），不能顶到边。
    let tall = settings_panel_rect(Rect::new(0, 0, 80, 24), 100);
    assert_eq!(tall.height, 22);
    assert_eq!(tall.y, 1);

    // 极小终端：退回整屏（与拆出前一致），且绝不越界。
    let tiny = Rect::new(0, 0, 7, 20);
    assert_eq!(settings_panel_rect(tiny, 4), tiny);
    let short = Rect::new(0, 0, 80, 3);
    assert_eq!(settings_panel_rect(short, 4), short);
}

/// 内容宽度必须与绘制内宽同源 —— 否则居中之后值列会错位、行尾被裁。
#[test]
fn settings_panel_content_width_matches_the_drawn_inner_width() {
    for w in [20u16, 40, 80, 120, 200] {
        let screen = Rect::new(0, 0, w, 24);
        let rect = settings_panel_rect(screen, 8);
        assert_eq!(
            settings_panel_content_width(screen) as u16 + 2,
            rect.width,
            "内容宽度 + 左右边框 应等于弹层宽度（w={w}）"
        );
    }
}

/// 打开设置面板后，弹层四周仍能看到底下的对话（不再是整屏覆盖）。
#[test]
fn settings_panel_leaves_the_conversation_visible_around_it() {
    let mut st = UiState::new();
    st.set_palette(Palette::default());
    st.sidebar_visible = false;
    st.push_line(LineKind::User, "hi");
    let pal = st.palette;
    let content = PanelContent {
        lines: vec![Line::from("模型: demo")],
        cursor: None,
    };
    let buf = render_frame(&st, Some(&content));
    let rect = settings_panel_rect(Rect::new(0, 0, W, H), content.lines.len());

    // 弹层内部铺浮层卡片色。
    assert_eq!(
        buf[(rect.x + 1, rect.y + 1)].bg,
        pal.overlay_bg,
        "弹层内应铺 overlay_bg"
    );
    // 弹层左边距（x=0，在 rect 之外）保留聊天区底色 —— 证明不是整屏覆盖。
    assert_eq!(buf[(0, rect.y + 1)].bg, pal.bg, "弹层外侧应仍是聊天区底色");
    assert!(rect.x >= 2, "居中后左右应留出边距");
}

/// 审批浮层：纵向居中（不再贴底），边框用 `err` 表达「回合被阻塞」。
#[test]
fn approval_overlay_is_centered_and_uses_the_error_border() {
    let mut st = UiState::new();
    st.set_palette(Palette::default());
    st.sidebar_visible = false;
    let (tx, _rx) = std::sync::mpsc::channel();
    st.approval = Some(ApprovalOverlay {
        command: "rm".into(),
        args: "-rf build".into(),
        allowlist_key: None,
        answer: tx,
    });
    let pal = st.palette;
    let buf = render_frame(&st, None);

    let title_y = row_with(&buf, "命令审批");
    // 聊天主体 = 整屏 − 顶栏(1) − 状态栏(1) − composer(内容行 + 上下边框)；
    // 浮层在主体内垂直居中。
    let body_h = H.saturating_sub(2).saturating_sub(composer_outer_rows(&st));
    let body = Rect::new(0, 1, W, body_h);
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
    st.set_palette(Palette::default());
    st.sidebar_visible = false;
    let (tx, _rx) = std::sync::mpsc::channel();
    st.approval = Some(ApprovalOverlay {
        command: "rm".into(),
        args: "-rf build".into(),
        allowlist_key: None,
        answer: tx,
    });
    let pal = st.palette;
    let buf = render_frame(&st, None);

    let body_h = H.saturating_sub(2).saturating_sub(composer_outer_rows(&st));
    let body = Rect::new(0, 1, W, body_h);
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
    let pal = Palette::default();
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
    st.set_palette(Palette::default());
    st.sidebar_visible = false;
    let (tx, _rx) = std::sync::mpsc::channel();
    st.approval = Some(ApprovalOverlay {
        command: "rm".into(),
        args: "-rf build".into(),
        allowlist_key: None,
        answer: tx,
    });
    // 选中「拒绝」（最后一格），色块应画在这一项上。
    st.approval_choice = ApprovalChoice::Deny;
    let pal = st.palette;
    let buf = render_frame(&st, None);

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
    let buf = render_frame(&st, None);
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
