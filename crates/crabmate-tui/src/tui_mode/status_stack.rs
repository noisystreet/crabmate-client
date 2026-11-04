//! 右栏底部的纵向状态栈（对齐 desktop 的 `.side-status-stack`）。
//!
//! desktop 早已把底栏迁进右栏底部、改成 `flex-direction: column` + 可折叠；TUI 这一轮
//! 才跟上。**动机不只是「像 desktop」**：右栏只有 [`super::pane_width::WORKSPACE_PANE_WIDTH`] 列，
//! 横排在这么窄的地方必然一路截断（`serve http://… │ model … │ role …` 三项就放不下），
//! 纵向每段一行反而能把值显示全。
//!
//! 三形态（矩形由 `layout::panes` 负责，本模块只负责行）：
//! - 宽屏展开：各段一行，**但 `mode` 与 `think` 同排一行（左右各半）**；
//!   **运行指示独占最后一行**（最多 [`MAX_STACK_ROWS`] 行，超出的按优先级丢弃）
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
    LABEL_COL_W, LABEL_W, MODE_LABEL, Segment, THINK_LABEL, environment_segments, reduced_motion,
    running_kind, running_row_with_view, view_back_text,
};

/// 展开态最多占的行数（含运行态那一行）。
///
/// **10 行是上限，当前全填充只用 9 行**：`think` 与 `mode` 同排之后省回了它原先独占的
/// 那一行。再往上加段前先想清楚：24 行终端里状态栈会占掉四成、11 行会让目录树在常见
/// 窗口高度下只显示 3 条。
///
/// 超出的段按优先级丢弃 —— 与横版 `fit_segments` 同一套优先级，只是把「列预算」
/// 换成「行预算」。
const MAX_STACK_ROWS: u16 = 10;

/// 状态栈占几行。
///
/// **必须与 [`stack_lines`] 同源。** 这里数出来的行数与那边实际产出的行数一旦不等，
/// 症状是状态栈底部空一行、或者最后一段被 `Paragraph` 裁掉 —— 与 B4 的
/// `settings_panel_content_width` 是同一类不变量（内容按宽度排版，宽度与矩形必须同源）。
pub(super) fn stack_rows(info: &StatusInfo, width: usize, collapsed: bool) -> u16 {
    if collapsed {
        return 1;
    }
    let rows = 1 + segment_rows(info, width).len();
    (rows as u16).min(MAX_STACK_ROWS)
}

/// 纵向上要显示的段（按**显示顺序**），已按优先级裁到 `MAX_STACK_ROWS - 1` 段。
///
/// 横版是「装不进列预算就整段丢弃」，纵版同理，只是预算是行数；两者共用
/// `Segment::priority`，避免出现「横条里 serve 优先、栈里 img 优先」这种自相矛盾。
///
/// **裁剪分支目前没有触发条件**：[`environment_segments`] 全填充是 9 段（`serve` /
/// `model` / `mode` / `think` / `role` / `view` / `ctx` / `find` / `perm`），正好等于阈值
/// `MAX_STACK_ROWS - 1 = 9`。逻辑留着，是因为它是「加段就按优先级让位」这条纪律的唯一
/// 实现：下次有人再 push 一段，第一件事就是这里丢最不重要的那段，而不是让状态栈长到
/// 11 行去挤目录树。
///
/// 段数与行数**不再相等**：`mode` / `think` 同排（[`segment_rows`]）之后，9 段只画 8 行 ——
/// 裁剪仍按段算，省下来的那行是留给目录树的。
///
/// **不为 `view` 段抬 `MAX_STACK_ROWS`**：10 行已经是 24 行终端能接受的上限（再多一行，
/// 目录树就只剩 3 条）。加段就该落进这 9 行预算，落不进就按优先级让位 —— `view` 的
/// priority 排在 `perm` 之后，是全填充里最先让位的那一段（安全相关的 `perm` 保得住）。
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

/// 纵向栈的一行：通常只有一段；`mode` 与 `think` 同排时是两段（左 `mode`、右 `think`）。
type SegmentRow = Vec<Segment>;

/// 同排两段之间的视觉间隔（列）：左半末尾让出一列，右半从下一列起。
const PAIR_GAP: usize = 1;

/// 同排两段的列区间（相对行首，左闭右开）：左右各占一半，左半末尾留 [`PAIR_GAP`] 列。
///
/// 渲染（[`pair_line`]）与鼠标命中（[`segment_cols_in_row`]）共用这一份，各算一遍就是
/// 「点偏一列」的来源。窄到 `width < 2` 时两段都退化成空区间，由调用方按空矩形处理。
fn pair_cols(width: usize) -> [(usize, usize); 2] {
    let half = width / 2;
    [(0, half.saturating_sub(PAIR_GAP)), (half, width)]
}

/// 一行显示哪几段（按**显示顺序**）；`mode` 与 `think` 合成一行，其余每段独占一行。
///
/// 合并放在**段级裁剪之后**：`think` 是可选段（`None` = 还没拉到 user-data），被行预算
/// 丢掉时这一行就只剩 `mode` —— 与合并前一样，不会留下一个空着的右半。
///
/// 只认这两个 label：别的段（`serve` / `model` / `ctx` …）的值都长，挤半行只会变成
/// 一片省略号，省下来的那一行不值得。
fn segment_rows(info: &StatusInfo, width: usize) -> Vec<SegmentRow> {
    let mut rows: Vec<SegmentRow> = Vec::new();
    for seg in visible_segments(info, width) {
        let pairs_with_previous = matches!(seg.label, MODE_LABEL | THINK_LABEL)
            && rows.last().is_some_and(|prev: &SegmentRow| {
                prev.len() == 1 && matches!(prev[0].label, MODE_LABEL | THINK_LABEL)
            });
        if pairs_with_previous && let Some(last) = rows.last_mut() {
            last.push(seg);
            continue;
        }
        rows.push(vec![seg]);
    }
    rows
}

/// 某一段在栈内的行序号（0-based；运行指示另占最后一行）。
///
/// `None` = 折叠态（只画运行行）或该段被行预算裁掉 —— 两种情况下都不该有可点区域。
/// 与 [`stack_lines_with`] 共用 [`segment_rows`]，不重算顺序（重算就会与画面错行）。
/// 同排的两段（`mode` / `think`）返回**同一个行号**，左右之分交给
/// [`segment_cols_in_row`]。
///
/// 按 `label` 取而不是各写一份：`perm`（开档位菜单）与 `think`（切思考开关）是同一件事
/// 的两个实例，抄第二份的那天就会漏掉一侧的裁剪分支，表现为「窄屏点了点不动」。
#[must_use]
pub(super) fn segment_row_index(
    info: &StatusInfo,
    width: usize,
    collapsed: bool,
    label: &str,
) -> Option<u16> {
    if collapsed {
        return None;
    }
    let idx = segment_rows(info, width)
        .iter()
        .position(|row| row.iter().any(|s| s.label == label))?;
    u16::try_from(idx).ok()
}

/// 某一段在它那一行里占的列区间（相对行首，左闭右开）。
///
/// 独占行 → 整行；同排行 → 左半 / 右半。鼠标命中靠它把「点了 `think` 的右半」与
/// 「点了 `mode` 的左半」分开 —— `mode` 在 TUI 里没有点击语义，右半因此是唯一的热区。
#[must_use]
pub(super) fn segment_cols_in_row(
    info: &StatusInfo,
    width: usize,
    collapsed: bool,
    label: &str,
) -> Option<(u16, u16)> {
    if collapsed {
        return None;
    }
    for row in segment_rows(info, width) {
        let Some(pos) = row.iter().position(|s| s.label == label) else {
            continue;
        };
        let (start, end) = if row.len() < 2 {
            (0, width)
        } else {
            pair_cols(width)[pos.min(1)]
        };
        return Some((u16::try_from(start).ok()?, u16::try_from(end).ok()?));
    }
    None
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
    let mut out: Vec<Line<'static>> = segment_rows(info, width)
        .iter()
        .map(|row| row_line(row, width, pal))
        .collect();
    // 运行指示排**最后一行**、缩进到第二列（与各段的值同列）：desktop 的
    // `.side-status-stack` 里 chips 在前、run indicator 在后，纵向 column 下它就贴着最底。
    // 位置固定在最底的好处是：切回合只有这一行变，上面的段不跟着挪。
    out.push(Line::from(running));
    out
}

/// 一行：单段整行；`mode` + `think` 同排时左右各半。
fn row_line(row: &[Segment], width: usize, pal: &Palette) -> Line<'static> {
    match row {
        [one] => segment_line(one, width, pal),
        [left, right, ..] => pair_line(left, right, width, pal),
        [] => Line::default(),
    }
}

/// 一段一行：`label` 淡色（补齐到定宽列）+ 值正文色；值超宽按**列**截断。
///
/// 不把超宽文本交给 `Paragraph` 去裁尾：那会在列边界直接切掉半个宽字符，
/// 而 `truncate_display` 会留 `…` 并保证结果 ≤ 预算。
///
/// 调用方只喂 [`environment_segments`] 的段，其 label 一律非空；横版的回看段
/// （label 为空）走 `status::status_line`，不经过这里。
fn segment_line(seg: &Segment, width: usize, pal: &Palette) -> Line<'static> {
    Line::from(segment_spans(seg, width, pal))
}

/// 同排两段：左半 `mode`、右半 `think`，各自在自己的半列里截断，中间留 [`PAIR_GAP`] 列。
///
/// 右半从 `pair_cols` 算出的固定列起，**不是**跟着左半实际宽度走 —— 左半值短的时候
/// 右半也得留在自己那半里，否则「各占一半」在视觉上就成了「谁短谁靠中间」。
fn pair_line(left: &Segment, right: &Segment, width: usize, pal: &Palette) -> Line<'static> {
    let [(left_start, left_end), (right_start, right_end)] = pair_cols(width);
    let mut spans = segment_spans(left, left_end.saturating_sub(left_start), pal);
    let pad = (right_start.saturating_sub(left_start)).saturating_sub(spans_width(&spans));
    if pad > 0 {
        spans.push(Span::raw(" ".repeat(pad)));
    }
    spans.extend(segment_spans(
        right,
        right_end.saturating_sub(right_start),
        pal,
    ));
    Line::from(spans)
}

/// 一段的 span（label 补齐到定宽列 + 空格 + 截断后的值），宽度以 `budget` 为上限。
fn segment_spans(seg: &Segment, budget: usize, pal: &Palette) -> Vec<Span<'static>> {
    debug_assert!(!seg.label.is_empty(), "状态栈的段必须有 label");
    // `Paragraph` 不补位：标签必须自己补齐到定宽列，值才会落在同一列。
    let mut label = String::from(seg.label);
    label.push_str(&" ".repeat(LABEL_W.saturating_sub(UnicodeWidthStr::width(seg.label))));
    vec![
        Span::styled(label, Style::new().fg(pal.muted)),
        Span::styled(" ", Style::new()),
        Span::styled(
            truncate_display(&seg.value, budget.saturating_sub(LABEL_COL_W)),
            Style::new().fg(pal.fg),
        ),
    ]
}

/// span 序列的显示宽（`pair_line` 补位用；宽字符按列算，不按字节）。
fn spans_width(spans: &[Span<'static>]) -> usize {
    spans
        .iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::super::status::{PERM_LABEL, VIEW_LABEL};
    use super::*;

    fn info() -> StatusInfo {
        StatusInfo {
            editor_open: false,
            api_base: "http://127.0.0.1:8080".into(),
            model: Some("gpt-x".into()),
            role: Some("coder".into()),
            mode: Some("plan".into()),
            thinking: None,
            running: false,
            cancel_sent: false,
            spinner: 0,
            view_offset: 0,
            search_term: None,
            search_total: 0,
            context: "—".to_string(),
            permission: None,
            permission_intent: None,
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

    /// 展开态：运行态一行 + 每段一行，且顺序与横版一致
    /// （`view` / `perm` 在前两行，见 `environment_segments`）。
    #[test]
    fn expanded_lists_every_segment_one_per_line() {
        let i = info();
        let t = texts(&stack_lines_with(&i, 24, &pal(), false, true));
        assert_eq!(
            t.len(),
            1 + 6,
            "view/serve/model/mode/role/ctx + 运行指示（fixture 没有 think，mode 独占该行）：{t:?}"
        );
        // 第 1 行是 `view`（`chat` 也算）：它是「主区现在是不是文件视图」的唯一指示，
        // 也是这一段的点击入口 —— 隐掉就等于「想点的时候找不到」。
        // 这个 fixture 没有 `perm`（未拉到 user-data → 整段不 push），所以第 2 行是 `serve`。
        assert!(t[0].starts_with("view"), "{t:?}");
        assert!(t[0].contains("chat"), "{t:?}");
        assert!(t[1].starts_with("serve"), "{t:?}");
        assert!(t[2].starts_with("model"), "{t:?}");
        // `mode` 在 `role` 之上：`think` 与它同排（左右各半），`role` 落到下一行。
        assert!(t[3].starts_with("mode"), "{t:?}");
        assert!(t[4].starts_with("role"), "{t:?}");
        // `ctx` 段恒在（无实测时值是占位 `—`）：段消失会让人误判「功能没生效」。
        assert!(t[5].starts_with("ctx"), "{t:?}");
        // 连接计数（`连接 [n/8] ▲k`）**不在这里**：它长在左栏 lane 区的标题行，
        // 与 `/lane attach`、`F4` 跳转同一个落点。状态栈再画一遍只是第二份读数。
        assert!(
            !t.iter().any(|l| l.starts_with("lanes")),
            "lanes 段已并入左栏标题：{t:?}"
        );
        // 运行指示贴**最底**（desktop 的 chips → run indicator 顺序）；切回合时
        // 只有这一行变，上面的段不跟着挪。
        assert!(t[6].contains("空闲"), "运行指示应在最下一行：{t:?}");
        // 与各段的值同列：第一列（label 列）留空，运行态从第二列起。
        assert!(
            t[6].starts_with(&" ".repeat(LABEL_COL_W)),
            "运行指示应缩进到第二列：{t:?}"
        );
        assert_eq!(
            UnicodeWidthStr::width(t[6].as_str()),
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
        // 第 1 行是 `view`（短值 `chat`），数值段从 `serve` 起：环境事实仍必须对齐同一列。
        assert!(t[1].starts_with("serve 127"), "{t:?}");
        assert!(t[3].starts_with("mode  plan"), "短标签要补齐：{t:?}");
        assert!(t[4].starts_with("role  coder"), "短标签要补齐：{t:?}");
        // 定宽列的真正判据：值起始列逐行相等（这几段全是 ASCII，字节偏移即列）。
        for (line, value) in [
            (&t[1], "127.0.0.1:8080"),
            (&t[2], "gpt-x"),
            (&t[3], "plan"),
            (&t[4], "coder"),
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

    /// 行预算哨兵：**展示**的段数不得超过 `MAX_STACK_ROWS - 1`，且行数必须与渲染同源。
    ///
    /// 这里把**所有可选段都填上**（`think` / `perm` / `find`），让 `environment_segments`
    /// 产出它现在**能产出的全部 9 段** —— 正好用满 `MAX_STACK_ROWS - 1 = 9` 的行预算。
    /// 于是这条同时压住两件事：段数没有越界（`lanes` 段撤掉之后，全填充不再触发裁剪），
    /// 且展示出来的行数没有撑破右栏。
    ///
    /// 只跑默认 fixture 的话可选段是 `None`，这条会宽松地通过，等真填满时才在别的测试里炸。
    #[test]
    fn stack_never_exceeds_the_row_budget() {
        let mut i = info();
        i.search_term = Some("ab".into());
        i.search_total = 3;
        i.view_offset = 12;
        i.permission = Some("read".into());
        i.thinking = Some("关".into());
        let segs = environment_segments(&i, 24);
        assert_eq!(
            segs.len(),
            MAX_STACK_ROWS as usize - 1,
            "fixture 应正好用满一行预算"
        );
        let kept = visible_segments(&i, 24);
        assert_eq!(kept.len(), segs.len(), "用满预算时不该丢段");
        let rows = stack_rows(&i, 24, false);
        assert!(rows <= MAX_STACK_ROWS, "{rows}");
        // 段数 9、行数 9（含运行行）：`mode` / `think` 同排省掉一行，因此行数恰好等于
        // 段数（合并前是 10）。这一条同时钉住「合并真的发生了」与「没有偷偷多出一行」。
        assert_eq!(rows, segs.len() as u16, "{rows}");
        let lines = stack_lines_with(&i, 24, &pal(), false, true);
        assert_eq!(rows as usize, lines.len(), "行数必须与渲染同源");
    }

    /// 越过预算时按优先级丢段：`serve` / `find` / `view` / `perm` 这些「正在变化」或
    /// 「我在哪」的段必须保住。
    ///
    /// `lanes` 段撤掉后，`environment_segments` 最多产出 9 段，正好等于行预算，没有 fixture
    /// 能越过它 —— 所以这里只钉住「不裁剪」这一侧，裁剪分支留给将来加段时再补用例
    /// （它按 `Segment::priority` 排序，逻辑本身在 [`visible_segments`] 里）。
    #[test]
    fn full_segments_fit_without_dropping_any() {
        let mut i = info();
        i.search_term = Some("ab".into());
        i.search_total = 3;
        i.permission = Some("read".into());
        i.thinking = Some("关".into());
        let kept_labels: Vec<&str> = visible_segments(&i, 24).iter().map(|s| s.label).collect();
        for label in ["serve", "find", VIEW_LABEL, PERM_LABEL] {
            assert!(
                kept_labels.contains(&label),
                "{label} 不该被丢：{kept_labels:?}"
            );
        }
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

    /// 前两行固定是 `view` / `perm`，且「先 view、后 perm」 —— 与 Desktop 右列状态栈的
    /// 前两颗 chip 同序（`StatusBarChipsLoaded` 里先 `StatusViewChip` 再 `StatusPermissionChip`）。
    ///
    /// 这条钉的是**次序**，不是「包含哪几段」：两端各按自己的直觉排，读起来就是同一个开关
    /// 在这里排第一、在那里排最后。
    #[test]
    fn view_and_perm_are_the_first_two_rows() {
        let mut i = info();
        i.permission = Some("read".into());
        let t = texts(&stack_lines_with(&i, 40, &pal(), false, true));
        assert!(t[0].starts_with(VIEW_LABEL), "{t:?}");
        assert!(t[0].contains("chat"), "{t:?}");
        assert!(t[1].starts_with(PERM_LABEL), "{t:?}");
        assert!(t[1].contains("read"), "{t:?}");
        // `serve` 让到第三行：次序是排出来的，不是「谁先 push 谁在前」的巧合。
        assert!(t[2].starts_with("serve"), "{t:?}");
    }

    /// `think` 与 `mode` **同一行**、左右各占一半，`role` 落到下一行。
    ///
    /// 这一行装的是「一回合内会动、但不值得各占一行」的两个开关：同排省下的那一行留给
    /// 目录树。各占一半是硬约束 —— 右半从行宽中线起，不跟着左半的实际长度漂。
    #[test]
    fn think_shares_the_mode_row_half_each() {
        let mut i = info();
        i.thinking = Some("开".into());
        let w = 40;
        let t = texts(&stack_lines_with(&i, w, &pal(), false, true));
        let mode_row = t
            .iter()
            .position(|l| {
                l.strip_prefix(MODE_LABEL)
                    .is_some_and(|rest| rest.starts_with(' '))
            })
            .unwrap_or_else(|| panic!("缺 {MODE_LABEL} 行：{t:?}"));
        let row = &t[mode_row];
        // 同一行里两段都在：`mode` 打头、`think` 从右半起。
        assert!(row.contains(THINK_LABEL), "row={row:?} t={t:?}");
        assert!(row.contains('开'), "think 的值要在同一行：{row:?}");
        assert_eq!(
            row.find(THINK_LABEL),
            Some(w / 2),
            "think 应从行宽中线起（各占一半）：{row:?}"
        );
        // 值也各自落在自己的半区里：`plan` 在第 6 列，`开` 在中线 + `LABEL_COL_W`。
        assert!(row.starts_with("mode  plan"), "{row:?}");
        assert_eq!(
            display_col_of(row, '开'),
            Some(w / 2 + LABEL_COL_W),
            "{row:?}"
        );
        // `role` 紧接在这一行之后（`mode` / `think` 同排，`role` 独占下一行）。
        assert!(
            t[mode_row + 1].starts_with("role"),
            "role 应在 mode/think 那一行之后：{t:?}"
        );
        // 段数 7、行数 6：合并真的省了一行，而不是把 think 藏掉了。
        assert_eq!(
            t.len(),
            1 + 6,
            "view/serve/model/[mode|think]/role/ctx + 运行指示：{t:?}"
        );
    }

    /// 没拉到 user-data 时整段不画：画成「关」会把「不知道」说成「关掉了」。
    #[test]
    fn think_segment_is_absent_when_unknown() {
        let i = info();
        let t = texts(&stack_lines_with(&i, 40, &pal(), false, true));
        assert!(
            !t.iter().any(|l| l.starts_with("think")),
            "未知不等于关：{t:?}"
        );
    }

    /// 纵向栈每行都必须 ≤ 栏内宽：超了 ratatui 会折行，把「运行指示」那行挤走，
    /// 这正是「状态栏用量溢出」的表现。
    #[test]
    fn every_stack_line_fits_the_pane_width() {
        let mut i = info();
        // 最宽的真实值：两位数百分比 + 四位数 k。
        i.context = "999.9k / 1000k (99.9%)".into();
        let w = crate::tui_mode::pane_width::WORKSPACE_PANE_WIDTH.saturating_sub(2) as usize;
        for line in stack_lines_with(&i, w, &pal(), false, true) {
            let lw = UnicodeWidthStr::width(line.to_string().as_str());
            assert!(lw <= w, "行宽 {lw} 超过栏内宽 {w}：{:?}", line.to_string());
        }
    }

    #[test]
    fn segment_row_index_is_the_line_that_shows_the_level() {
        let mut i = info();
        i.permission = Some("read".into());
        let idx = segment_row_index(&i, 24, false, super::super::status::PERM_LABEL)
            .expect("展开态有 perm 行");
        let lines = stack_lines_with(&i, 24, &pal(), false, true);
        // 行号必须指向真画出来的那一行（与 `stack_lines_with` 同源，否则点击会错行）。
        let row = &lines[idx as usize];
        assert!(row.to_string().contains("read"), "{row:?}");
        // 折叠态只剩运行行：没有 perm 行，也就没有可点区域。
        assert!(segment_row_index(&i, 24, true, super::super::status::PERM_LABEL).is_none());
    }

    /// `think` 段同样可点（切思考开关）：行号指向画着「开/关」的那一行。
    #[test]
    fn segment_row_index_finds_the_think_row() {
        let mut i = info();
        i.thinking = Some("关".into());
        let idx = segment_row_index(&i, 24, false, super::super::status::THINK_LABEL)
            .expect("展开态有 think 行");
        let lines = stack_lines_with(&i, 24, &pal(), false, true);
        assert!(lines[idx as usize].to_string().contains("关"), "{lines:?}");
        // 没有该段（未知 / 未拉到 user-data）时没有可点区域。
        let i = info();
        assert!(segment_row_index(&i, 24, false, super::super::status::THINK_LABEL).is_none());
    }

    /// 同排的两段共用一个行号，列区间却一左一右：鼠标命中靠它把「点了 `mode` 左半」与
    /// 「点了 `think` 右半」分开（`mode` 在 TUI 里没有点击语义）。
    #[test]
    fn pair_segments_share_the_row_but_not_the_columns() {
        let mut i = info();
        i.thinking = Some("关".into());
        let w = 40;
        let mode_row = segment_row_index(&i, w, false, MODE_LABEL).expect("展开态有 mode 行");
        let think_row = segment_row_index(&i, w, false, THINK_LABEL).expect("展开态有 think 行");
        assert_eq!(mode_row, think_row, "同排就该同一行");
        let mode_cols = segment_cols_in_row(&i, w, false, MODE_LABEL).expect("mode 列区间");
        let think_cols = segment_cols_in_row(&i, w, false, THINK_LABEL).expect("think 列区间");
        assert_eq!(mode_cols.0, 0, "左半从行首起：{mode_cols:?}");
        assert_eq!(think_cols.0, 20, "右半从行宽中线起：{think_cols:?}");
        assert_eq!(think_cols.1, 40, "右半到行尾：{think_cols:?}");
        assert!(
            mode_cols.1 <= think_cols.0,
            "两半不重叠：{mode_cols:?} {think_cols:?}"
        );
        // 独占行的段仍是整行，且不与 `mode` 抢行（`role` 在下一行）。
        assert_eq!(segment_cols_in_row(&i, w, false, "role"), Some((0, 40)));
        assert_ne!(segment_row_index(&i, w, false, "role"), Some(mode_row));
        // 折叠态只画运行行：没有可点区域。
        assert!(segment_cols_in_row(&i, w, true, THINK_LABEL).is_none());
    }
}
