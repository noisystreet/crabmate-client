//! 右栏底部的纵向状态栈（对齐 desktop 的 `.side-status-stack`）。
//!
//! desktop 早已把底栏迁进右栏底部、改成 `flex-direction: column` + 可折叠；TUI 这一轮
//! 才跟上。**动机不只是「像 desktop」**：右栏只有 [`super::render::SIDEBAR_WIDTH`] 列，
//! 横排在这么窄的地方必然一路截断（`serve http://… │ model … │ role …` 三项就放不下），
//! 纵向每段一行反而能把值显示全。
//!
//! 三形态（矩形由 `layout::panes` 负责，本模块只负责行）：
//! - 宽屏展开：各段一行 + **运行指示独占最后一行**（最多 [`MAX_STACK_ROWS`] 行，超出的按优先级丢弃）
//! - 宽屏折叠：只留运行指示那一行
//! - 窄屏：本模块不画，底栏走 `status::status_line` 的横版（两栏都隐藏时状态栈无处安置）

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use super::palette::Palette;
use super::render::{StatusInfo, truncate_display};
use super::status::{
    Segment, left_segments, reduced_motion, running_kind, running_spans_right_aligned,
};

/// 展开态最多占的行数（含运行态那一行）。
///
/// 8 行已是 24 行终端的三分之一，再高就把目录树挤成一条缝。超出的段按优先级丢弃 ——
/// 与横版 `fit_segments` 同一套优先级，只是把「列预算」换成「行预算」。
const MAX_STACK_ROWS: u16 = 8;

/// 状态栈占几行。
///
/// **必须与 [`stack_lines`] 同源。** 这里数出来的行数与那边实际产出的行数一旦不等，
/// 症状是状态栈底部空一行、或者最后一段被 `Paragraph` 裁掉 —— 与 B4 的
/// `settings_panel_content_width` 是同一类不变量（内容按宽度排版，宽度与矩形必须同源）。
pub(super) fn stack_rows(info: &StatusInfo, width: usize, collapsed: bool) -> u16 {
    if collapsed {
        return 1;
    }
    let rows = 1 + visible_segments(info, width).len();
    (rows as u16).min(MAX_STACK_ROWS)
}

/// 纵向上要显示的段（按**显示顺序**），已按优先级裁到 `MAX_STACK_ROWS - 1` 段。
///
/// 横版是「装不进列预算就整段丢弃」，纵版同理，只是预算是行数；两者共用
/// `Segment::priority`，避免出现「横条里 serve 优先、栈里 img 优先」这种自相矛盾。
fn visible_segments(info: &StatusInfo, width: usize) -> Vec<Segment> {
    let segs = left_segments(info, width);
    let keep = (MAX_STACK_ROWS as usize).saturating_sub(1).min(segs.len());
    if keep == segs.len() {
        return segs;
    }
    let mut by_priority: Vec<usize> = (0..segs.len()).collect();
    by_priority.sort_by_key(|&i| (segs[i].priority, i));
    let mut keep_flags = vec![false; segs.len()];
    for i in by_priority.into_iter().take(keep) {
        keep_flags[i] = true;
    }
    segs.into_iter()
        .zip(keep_flags)
        .filter_map(|(seg, keep)| keep.then_some(seg))
        .collect()
}

/// 状态栈的每一行：第 1 行运行态，其后每段一行。
pub(super) fn stack_lines(
    info: &StatusInfo,
    width: usize,
    pal: &Palette,
    collapsed: bool,
) -> Vec<Line<'static>> {
    stack_lines_with(info, width, pal, collapsed, reduced_motion())
}

/// [`stack_lines`] 的可注入版本（单测要能强制 reduced motion）。
fn stack_lines_with(
    info: &StatusInfo,
    width: usize,
    pal: &Palette,
    collapsed: bool,
    reduced_motion: bool,
) -> Vec<Line<'static>> {
    let kind = running_kind(info, reduced_motion);
    if collapsed {
        return vec![Line::from(running_spans_right_aligned(&kind, width, pal))];
    }
    let mut out: Vec<Line<'static>> = visible_segments(info, width)
        .iter()
        .map(|seg| segment_line(seg, width, pal))
        .collect();
    // 运行指示排**最后一行**并右对齐：desktop 的 `.side-status-stack` 里 chips 在前、
    // run indicator 在后，纵向 column 下它就贴着最底；贴右则与横版右缘同一取向。
    // 位置固定在最底的好处是：切回合只有这一行变，上面的段不跟着挪。
    out.push(Line::from(running_spans_right_aligned(&kind, width, pal)));
    out
}

/// 一段一行：`label` 淡色 + 值正文色；值超宽按**列**截断。
///
/// 不把超宽文本交给 `Paragraph` 去裁尾：那会在列边界直接切掉半个宽字符，
/// 而 `truncate_display` 会留 `…` 并保证结果 ≤ 预算。
fn segment_line(seg: &Segment, width: usize, pal: &Palette) -> Line<'static> {
    let value_style = Style::new().fg(pal.fg);
    if seg.label.is_empty() {
        return Line::from(vec![Span::styled(
            truncate_display(&seg.value, width),
            value_style,
        )]);
    }
    let label_w = UnicodeWidthStr::width(seg.label) + 1;
    Line::from(vec![
        Span::styled(seg.label, Style::new().fg(pal.muted)),
        Span::styled(" ", Style::new()),
        Span::styled(
            truncate_display(&seg.value, width.saturating_sub(label_w)),
            value_style,
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> StatusInfo {
        StatusInfo {
            api_base: "http://127.0.0.1:8080".into(),
            model: Some("gpt-x".into()),
            role: Some("coder".into()),
            mode: Some("plan".into()),
            running: false,
            cancel_sent: false,
            spinner: 0,
            view_offset: 0,
            search_term: None,
            search_total: 0,
        }
    }

    fn pal() -> Palette {
        Palette::default()
    }

    fn texts(lines: &[Line<'static>]) -> Vec<String> {
        lines.iter().map(|l| l.to_string()).collect()
    }

    #[test]
    fn collapsed_shows_only_the_running_row() {
        let i = info();
        let lines = stack_lines_with(&i, 24, &pal(), true, true);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].to_string().contains("空闲"), "{:?}", lines[0]);
        // 折叠态同样右对齐，否则一折叠运行指示就从右边缘跳到左边。
        assert!(lines[0].to_string().starts_with(' '), "{:?}", lines[0]);
        assert_eq!(stack_rows(&i, 24, true), 1);
    }

    /// 展开态：运行态一行 + 每段一行，且顺序与横版一致（serve 在最前）。
    #[test]
    fn expanded_lists_every_segment_one_per_line() {
        let i = info();
        let t = texts(&stack_lines_with(&i, 24, &pal(), false, true));
        assert_eq!(t.len(), 1 + 4, "serve/model/role/mode + 运行指示：{t:?}");
        assert!(t[0].starts_with("serve"), "{t:?}");
        assert!(t[1].starts_with("model"), "{t:?}");
        assert!(t[2].starts_with("role"), "{t:?}");
        assert!(t[3].starts_with("mode"), "{t:?}");
        // 运行指示贴**最底**（desktop 的 chips → run indicator 顺序）；切回合时
        // 只有这一行变，上面的段不跟着挪。
        assert!(t[4].contains("空闲"), "运行指示应在最下一行：{t:?}");
        // 右对齐：补前导空白后行宽恰好等于列宽（与横版右缘同一取向）。
        assert!(t[4].starts_with(' '), "运行指示应先补前导空格：{t:?}");
        assert_eq!(
            UnicodeWidthStr::width(t[4].as_str()),
            24,
            "运行指示应右对齐到列宽：{t:?}"
        );
        // 会话 id 不占行：desktop 的状态区也没有它，而一行放个截断 id 没有信息量。
        assert!(!t.join(" ").contains("conv"), "{t:?}");
    }

    /// 行数与渲染结果必须同源：不等的话底部空行、或末段被裁。
    #[test]
    fn stack_rows_matches_the_rendered_line_count() {
        for collapsed in [false, true] {
            let i = info();
            let rows = stack_rows(&i, 24, collapsed) as usize;
            let lines = stack_lines_with(&i, 24, &pal(), collapsed, true);
            assert_eq!(rows, lines.len(), "collapsed={collapsed}");
        }
    }

    /// 26 列侧栏里值必须截断而不是溢出（宽字符尤其容易差一列）。
    #[test]
    fn long_values_are_truncated_to_the_column() {
        let mut i = info();
        i.api_base = "https://very-long-host-name.example.com/crabmate/api".into();
        let lines = stack_lines_with(&i, 24, &pal(), false, true);
        for line in &lines {
            let text = line.to_string();
            assert!(
                UnicodeWidthStr::width(text.as_str()) <= 24,
                "行宽超过列宽：{text:?}"
            );
        }
    }

    /// 段数再多也不超过行预算（超出的按优先级丢弃，不撑破右栏）。
    #[test]
    fn row_budget_is_capped() {
        let mut i = info();
        i.search_term = Some("ab".into());
        i.search_total = 3;
        i.view_offset = 12;
        let rows = stack_rows(&i, 24, false);
        assert!(rows <= MAX_STACK_ROWS, "{rows}");
        let lines = stack_lines_with(&i, 24, &pal(), false, true);
        assert!(lines.len() <= MAX_STACK_ROWS as usize, "{}", lines.len());
    }
}
