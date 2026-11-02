//! 系统行（`LineKind::System`）的前缀与配色断言。
//!
//! 单独成文件的理由与 [`super::color_tests`] 同源：`render_tests.rs` 已贴着
//! `fn-nloc-ratchet` 的 920 行/文件门禁，在这里再塞三支测试就会把它顶过去。

use super::*;

/// 系统行读作一个**角色**，而不是一个警号：前缀与 `助手 ` / `工具 ` 同宽。
///
/// 宽度断言不是凑数：`prefix_width` 是折行预算的唯一来源，前缀从 2 列变 5 列
/// 就是每行正文少 3 列，若只改文本不改这条约束，改动会悄悄漂移。
#[test]
fn system_prefix_is_a_role_label_like_the_others() {
    assert_eq!(kind_prefix(LineKind::System), "系统 ");
    assert_eq!(prefix_width(kind_prefix(LineKind::System)), 5);
    assert_eq!(
        prefix_width(kind_prefix(LineKind::System)),
        prefix_width(kind_prefix(LineKind::Assistant)),
        "系统行与助手行的正文起点必须对齐"
    );
}

/// 系统行的配色归中性：前缀取 `info`（与工具行同档）、正文取 `muted`。
///
/// 这一档里既有真失败，也有「已加载更早的 3 条」这类通报，`LineKind` 分不出两者，
/// 所以判色只能落在「这是过程通报」上，不能落在「这是错误」上。
#[test]
fn system_row_uses_the_neutral_palette() {
    let mut st = UiState::new();
    st.set_palette(super::Palette::default());
    st.push_line(LineKind::System, "载入历史失败：boom");
    let rows = body_rows(&st, 60);
    let spans = &rows[0].line.spans;
    assert_eq!(spans[0].content.as_ref(), "系统 ");
    let pal = &st.palette;
    assert_eq!(spans[0].style.fg, Some(pal.info), "前缀与工具行同档");
    assert_eq!(spans[1].style.fg, Some(pal.muted), "正文不再整行报警");
}

/// 无色模式（`NO_COLOR` / `TERM=dumb`）下 BOLD 是系统行与思考行唯一的分野。
#[test]
fn system_prefix_keeps_its_bold_in_mono() {
    let mut st = UiState::new();
    let pal = super::Palette {
        mono: true,
        ..Default::default()
    };
    st.set_palette(pal);
    st.push_line(LineKind::System, "x");
    let rows = body_rows(&st, 40);
    assert!(
        rows[0].line.spans[0]
            .style
            .add_modifier
            .contains(ratatui::style::Modifier::BOLD),
        "mono 下前缀靠 BOLD 与 `思考 `（DIM）分开"
    );
}
