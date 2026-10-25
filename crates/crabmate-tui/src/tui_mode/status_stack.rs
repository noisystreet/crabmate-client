//! 右栏底部的纵向状态栈（对齐 desktop 的 `.side-status-stack`）。
//!
//! desktop 早已把底栏迁进右栏底部、改成 `flex-direction: column` + 可折叠；TUI 这一轮
//! 才跟上。**动机不只是「像 desktop」**：右栏只有 [`super::render::WORKSPACE_PANE_WIDTH`] 列，
//! 横排在这么窄的地方必然一路截断（`serve http://… │ model … │ role …` 三项就放不下），
//! 纵向每段一行反而能把值显示全。
//!
//! 三形态（矩形由 `layout::panes` 负责，本模块只负责行）：
//! - 宽屏展开：各段一行 + **运行指示独占最后一行**（最多 [`MAX_STACK_ROWS`] 行，超出的按优先级丢弃）
//! - 宽屏折叠：只留运行指示那一行
//! - 两种形态都按**两列**排：第一列 label、第二列值。运行指示那一行不例外 ——
//!   第一列放回看提示（`↑N`，可空），第二列放运行态。回看提示因此不需要独立行，
//!   折叠后也不会消失。
//! - 窄屏：本模块不画，底栏走 `status::status_line` 的横版（两栏都隐藏时状态栈无处安置）

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use super::palette::Palette;
use super::render::{StatusInfo, truncate_display};
use super::status::{
    LABEL_COL_W, LABEL_W, Segment, environment_segments, reduced_motion, running_kind,
    running_row_with_view, view_back_text,
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
///
/// **裁剪分支目前走不到**：[`environment_segments`] 最多 6 段（含 `ctx`），而阈值是
/// `MAX_STACK_ROWS - 1 = 7`，于是 `keep == segs.len()` 恒成立、提前返回。留着它是给
/// 「以后加段」预留 —— 真加到第 8 段时，`stack_never_exceeds_the_row_budget` 会先失败。
fn visible_segments(info: &StatusInfo, width: usize) -> Vec<Segment> {
    let segs = environment_segments(info, width);
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

/// 状态栈的每一行：各段一行，运行指示独占最后一行。
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
    // 回看提示（`↑N`）并进运行指示那一行：不占独立行，折叠态也因此不会把它丢掉。
    // 宽度不够时 `running_row_with_view` 会先丢回看、保住右缘运行态。
    let running = running_row_with_view(&kind, view_back_text(info).as_deref(), width, pal);
    if collapsed {
        return vec![Line::from(running)];
    }
    let mut out: Vec<Line<'static>> = visible_segments(info, width)
        .iter()
        .map(|seg| segment_line(seg, width, pal))
        .collect();
    // 运行指示排**最后一行**、缩进到第二列（与各段的值同列）：desktop 的
    // `.side-status-stack` 里 chips 在前、run indicator 在后，纵向 column 下它就贴着最底。
    // 位置固定在最底的好处是：切回合只有这一行变，上面的段不跟着挪。
    out.push(Line::from(running));
    out
}

/// 一段一行：`label` 淡色（补齐到定宽列）+ 值正文色；值超宽按**列**截断。
///
/// 不把超宽文本交给 `Paragraph` 去裁尾：那会在列边界直接切掉半个宽字符，
/// 而 `truncate_display` 会留 `…` 并保证结果 ≤ 预算。
///
/// 调用方只喂 [`environment_segments`] 的段，其 label 一律非空；横版的回看段
/// （label 为空）走 `status::status_line`，不经过这里。
fn segment_line(seg: &Segment, width: usize, pal: &Palette) -> Line<'static> {
    let value_style = Style::new().fg(pal.fg);
    debug_assert!(!seg.label.is_empty(), "状态栈的段必须有 label");
    // `Paragraph` 不补位：标签必须自己补齐到定宽列，值才会落在同一列。
    let mut label = String::from(seg.label);
    label.push_str(&" ".repeat(LABEL_W.saturating_sub(UnicodeWidthStr::width(seg.label))));
    Line::from(vec![
        Span::styled(label, Style::new().fg(pal.muted)),
        Span::styled(" ", Style::new()),
        Span::styled(
            truncate_display(&seg.value, width.saturating_sub(LABEL_COL_W)),
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
            context: "—".to_string(),
        }
    }

    fn pal() -> Palette {
        Palette::default()
    }

    fn texts(lines: &[Line<'static>]) -> Vec<String> {
        lines.iter().map(|l| l.to_string()).collect()
    }

    /// `ch` 在 `text` 里的**显示列**（0-based）。
    ///
    /// 不能用字节偏移：`↑` 是 3 字节但只占 1 列，按字节算会把每个断言都算偏。
    fn display_col_of(text: &str, ch: char) -> Option<usize> {
        let byte = text.find(ch)?;
        Some(UnicodeWidthStr::width(&text[..byte]))
    }

    #[test]
    fn collapsed_shows_only_the_running_row() {
        let i = info();
        let lines = stack_lines_with(&i, 24, &pal(), true, true);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].to_string().contains("空闲"), "{:?}", lines[0]);
        // 折叠态也缩进到第二列（与展开态、与各段的值同列）：一折叠不跳列。
        assert!(
            lines[0].to_string().starts_with(&" ".repeat(LABEL_COL_W)),
            "{:?}",
            lines[0]
        );
        assert_eq!(stack_rows(&i, 24, true), 1);
    }

    /// 展开态：运行态一行 + 每段一行，且顺序与横版一致（serve 在最前）。
    #[test]
    fn expanded_lists_every_segment_one_per_line() {
        let i = info();
        let t = texts(&stack_lines_with(&i, 24, &pal(), false, true));
        assert_eq!(
            t.len(),
            1 + 5,
            "serve/model/role/mode/ctx + 运行指示：{t:?}"
        );
        assert!(t[0].starts_with("serve"), "{t:?}");
        assert!(t[1].starts_with("model"), "{t:?}");
        assert!(t[2].starts_with("role"), "{t:?}");
        assert!(t[3].starts_with("mode"), "{t:?}");
        // `ctx` 段恒在（无实测时值是占位 `—`）：段消失会让人误判「功能没生效」。
        assert!(t[4].starts_with("ctx"), "{t:?}");
        // 运行指示贴**最底**（desktop 的 chips → run indicator 顺序）；切回合时
        // 只有这一行变，上面的段不跟着挪。
        assert!(t[5].contains("空闲"), "运行指示应在最下一行：{t:?}");
        // 与各段的值同列：第一列（label 列）留空，运行态从第二列起。
        assert!(
            t[5].starts_with(&" ".repeat(LABEL_COL_W)),
            "运行指示应缩进到第二列：{t:?}"
        );
        assert_eq!(
            UnicodeWidthStr::width(t[5].as_str()),
            LABEL_COL_W + UnicodeWidthStr::width("○ 空闲"),
            "第二列起排，不该再补到列宽：{t:?}"
        );
        // 会话 id 不占行：desktop 的状态区也没有它，而一行放个截断 id 没有信息量。
        assert!(!t.join(" ").contains("conv"), "{t:?}");
    }

    /// 所有值必须从同一列起：label 长短不同（`serve`/`model` 5 列、`role`/`mode`/`find`
    /// 4 列）也不能把值推成锯齿。补齐后 `role` / `mode` 与 `serve` 之间是两个空格。
    #[test]
    fn values_start_at_one_column() {
        let i = info();
        let t = texts(&stack_lines_with(&i, 24, &pal(), false, true));
        assert!(t[0].starts_with("serve 127"), "{t:?}");
        assert!(t[2].starts_with("role  coder"), "短标签要补齐：{t:?}");
        assert!(t[3].starts_with("mode  plan"), "短标签要补齐：{t:?}");
        // 定宽列的真正判据：值起始列逐行相等（这几段全是 ASCII，字节偏移即列）。
        for (line, value) in [
            (&t[0], "127.0.0.1:8080"),
            (&t[1], "gpt-x"),
            (&t[2], "coder"),
            (&t[3], "plan"),
        ] {
            let col = line
                .find(value)
                .unwrap_or_else(|| panic!("{line} 里找不到 {value}"));
            assert_eq!(col, LABEL_COL_W, "值起始列不一致：{line}");
        }
    }

    /// `LABEL_W` 必须容得下所有 label：以后加更长标签却忘了同步定宽，这条先炸。
    #[test]
    fn labels_fit_the_fixed_column() {
        let mut i = info();
        i.search_term = Some("x".into());
        i.search_total = 1;
        for seg in environment_segments(&i, 24) {
            assert!(
                UnicodeWidthStr::width(seg.label) <= LABEL_W,
                "label {:?} 超出定宽列 {LABEL_W}，需同步 LABEL_W",
                seg.label
            );
        }
    }

    /// 行数与渲染结果必须同源：不等的话底部空行、或末段被裁。
    /// 回看提示并进运行行后仍须同源 —— 它不该多加一行（`view_offset > 0` 也要覆盖）。
    #[test]
    fn stack_rows_matches_the_rendered_line_count() {
        for collapsed in [false, true] {
            for view_offset in [0, 12] {
                let mut i = info();
                i.view_offset = view_offset;
                let rows = stack_rows(&i, 24, collapsed) as usize;
                let lines = stack_lines_with(&i, 24, &pal(), collapsed, true);
                assert_eq!(
                    rows,
                    lines.len(),
                    "collapsed={collapsed} view={view_offset}"
                );
            }
        }
    }

    /// 回看提示与运行指示**同一行**：不多占行，且折叠态也看得见。
    ///
    /// 这正是「不独立成段」的理由：独立成段时折叠态整个消失（折叠只画运行那一行），
    /// 而折叠恰恰是终端矮时最常见的形态。
    #[test]
    fn view_back_rides_the_running_row() {
        let mut scrolled = info();
        scrolled.view_offset = 12;
        let expanded = texts(&stack_lines_with(&scrolled, 24, &pal(), false, true));
        let base = texts(&stack_lines_with(&info(), 24, &pal(), false, true));
        assert_eq!(
            expanded.len(),
            base.len(),
            "并进运行行后不该多占一行：{expanded:?}"
        );

        let last = expanded.last().expect("至少一行");
        assert!(last.starts_with("↑12"), "回看提示应占第一列：{last:?}");
        assert!(
            last.trim_end().ends_with("空闲"),
            "运行指示仍在同一行：{last:?}"
        );
        // 运行态落在第二列：`↑12` 之后补齐到 `LABEL_COL_W`，与各段的值同列。
        assert_eq!(
            display_col_of(last, '○'),
            Some(LABEL_COL_W),
            "运行态应在第二列：{last:?}"
        );

        // 折叠态只剩运行那一行——回看提示必须还在。
        let folded = texts(&stack_lines_with(&scrolled, 24, &pal(), true, true));
        assert_eq!(folded.len(), 1);
        assert!(
            folded[0].starts_with("↑12"),
            "折叠后回看不该消失：{:?}",
            folded[0]
        );
        assert!(folded[0].trim_end().ends_with("空闲"), "{:?}", folded[0]);
        assert_eq!(
            display_col_of(&folded[0], '○'),
            Some(LABEL_COL_W),
            "{:?}",
            folded[0]
        );
    }

    /// 列宽连「第二列」都放不下时：不缩进、丢回看，运行态贴左保住可见性。
    #[test]
    fn narrow_column_drops_the_indent_and_the_view_back() {
        let mut i = info();
        i.view_offset = 1234;
        // 8 列 < `LABEL_COL_W`(6) + `○ 空闲`(6)：第二列根本起不来。
        let folded = texts(&stack_lines_with(&i, 8, &pal(), true, true));
        assert!(
            !folded[0].contains('↑'),
            "放不下就整段丢回看：{:?}",
            folded[0]
        );
        assert!(
            folded[0].starts_with('○'),
            "连第二列都起不来时贴左、不缩进：{:?}",
            folded[0]
        );
        assert_eq!(
            UnicodeWidthStr::width(folded[0].as_str()),
            UnicodeWidthStr::width("○ 空闲")
        );
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

    /// 行预算哨兵：段数不得超过 `MAX_STACK_ROWS - 1`，且行数必须与渲染同源。
    ///
    /// 当前 `environment_segments` 最多 5 段，`visible_segments` 里的按优先级丢弃
    /// **走不到**（阈值 7）。所以这条断言不是「验证裁剪生效」，而是「一旦加段越过阈值
    /// 就先炸」——免得哪天悄悄撑破右栏，还以为是布局算错了。
    #[test]
    fn stack_never_exceeds_the_row_budget() {
        let mut i = info();
        i.search_term = Some("ab".into());
        i.search_total = 3;
        i.view_offset = 12;
        let segs = environment_segments(&i, 24);
        // 段数必须 ≤ `MAX_STACK_ROWS - 1`（留一行给运行指示），即 < `MAX_STACK_ROWS`。
        assert!(
            segs.len() < MAX_STACK_ROWS as usize,
            "段数 {} 越过行预算，需要重新审视 visible_segments 的裁剪路径",
            segs.len()
        );
        let rows = stack_rows(&i, 24, false);
        assert!(rows <= MAX_STACK_ROWS, "{rows}");
        let lines = stack_lines_with(&i, 24, &pal(), false, true);
        assert_eq!(rows as usize, lines.len(), "行数必须与渲染同源");
    }

    #[test]
    fn stack_shows_ctx_line_when_snapshot_present() {
        let mut i = info();
        i.context = "116.9k / 1000k (11.7%)".into();
        let t = texts(&stack_lines_with(&i, 40, &pal(), false, true));
        let ctx = t
            .iter()
            .find(|s| s.starts_with("ctx"))
            .unwrap_or_else(|| panic!("纵向状态栈里应有 ctx 行，实际：{t:?}"));
        assert!(ctx.contains("116.9k / 1000k (11.7%)"), "{ctx}");
    }

    #[test]
    fn stack_keeps_ctx_line_as_placeholder_without_snapshot() {
        // 段恒在：值占位 `—`，而不是整行消失（消失会让人以为功能没生效）。
        let i = info();
        let t = texts(&stack_lines_with(&i, 40, &pal(), false, true));
        let ctx = t
            .iter()
            .find(|s| s.starts_with("ctx"))
            .unwrap_or_else(|| panic!("纵向状态栈里应有 ctx 行，实际：{t:?}"));
        assert!(ctx.contains('—'), "{ctx}");
    }

    /// 纵向栈每行都必须 ≤ 栏内宽：超了 ratatui 会折行，把「运行指示」那行挤走，
    /// 这正是「状态栏用量溢出」的表现。
    #[test]
    fn every_stack_line_fits_the_pane_width() {
        let mut i = info();
        // 最宽的真实值：两位数百分比 + 四位数 k。
        i.context = "999.9k / 1000k (99.9%)".into();
        let w = crate::tui_mode::render::WORKSPACE_PANE_WIDTH.saturating_sub(2) as usize;
        for line in stack_lines_with(&i, w, &pal(), false, true) {
            let lw = UnicodeWidthStr::width(line.to_string().as_str());
            assert!(lw <= w, "行宽 {lw} 超过栏内宽 {w}：{:?}", line.to_string());
        }
    }
}
