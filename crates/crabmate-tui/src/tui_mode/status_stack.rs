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

use ratatui::style::{Color, Style};
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

/// 状态栈**一次**构建的产物：段行 + 运行指示行。
///
/// 存在的理由是「一帧只建一次」：`render::draw` 先要知道行数才排得出矩形，再要逐行内容
/// 才画得出来。分开调就是两遍 `environment_segments`（每段带 `String`）—— 而它每帧都跑。
/// 行数与内容从此出自同一份 plan，[`StackPlan::rows_len`] 与 [`StackPlan::lines`] 不可能
/// 再对不上（那是「底部空一行 / 最后一段被裁掉」的成因）。
pub(super) struct StackPlan {
    /// 段行（按显示顺序，已裁到 `MAX_STACK_ROWS - 1` 段）；折叠态为空。
    rows: Vec<SegmentRow>,
    /// 运行指示那一行（回看提示 `↑N` 已并进去）。
    running: Line<'static>,
    collapsed: bool,
    /// 排版的宽度，`row_line` 与它同一口径。
    width: usize,
    /// 建 plan 时的配色板（`Copy`）：`lines` 里给每段上色还要用。
    pal: Palette,
}

impl StackPlan {
    /// 状态栈占几行（排版用）。与 [`Self::lines`] 同源，不会出现「按 N 行排版、画 N±1 行」。
    pub(super) fn rows_len(&self) -> u16 {
        clampi_rows(self.rows.len(), self.collapsed)
    }

    /// 逐行内容（含最后那一行运行指示）。
    pub(super) fn lines(&self) -> Vec<Line<'static>> {
        if self.collapsed {
            return vec![self.running.clone()];
        }
        let mut out: Vec<Line<'static>> = self
            .rows
            .iter()
            .map(|row| row_line(row, self.width, &self.pal))
            .collect();
        // 运行指示排**最后一行**、缩进到第二列（与各段的值同列）：desktop 的
        // `.side-status-stack` 里 chips 在前、run indicator 在后，纵向 column 下它就贴着最底。
        // 位置固定在最底的好处是：切回合只有这一行变，上面的段不跟着挪。
        out.push(self.running.clone());
        out
    }
}

/// 建一次状态栈（生产入口：reduced motion 现查）。
pub(super) fn stack_plan(
    info: &StatusInfo,
    width: usize,
    pal: &Palette,
    collapsed: bool,
) -> StackPlan {
    stack_plan_with(info, width, pal, collapsed, reduced_motion())
}

/// [`stack_plan`] 的可注入版本（单测要能强制 reduced motion）。
fn stack_plan_with(
    info: &StatusInfo,
    width: usize,
    pal: &Palette,
    collapsed: bool,
    reduced_motion: bool,
) -> StackPlan {
    let kind = running_kind(info, reduced_motion);
    // 回看提示（`↑N`）并进运行指示那一行：不占独立行，折叠态也因此不会把它丢掉。
    // 宽度不够时 `running_row_with_view` 会先丢回看、保住右缘运行态。
    let running = Line::from(running_row_with_view(
        &kind,
        view_back_text(info).as_deref(),
        width,
        pal,
    ));
    // 折叠态一段都不建：`rows_len` 恒为 1，建了也没人读。
    let rows = if collapsed {
        Vec::new()
    } else {
        segment_rows(info, width)
    };
    StackPlan {
        rows,
        running,
        collapsed,
        width,
        pal: *pal,
    }
}

/// 状态栈占几行。
///
/// **必须与 [`stack_lines`] 同源。** 这里数出来的行数与那边实际产出的行数一旦不等，
/// 症状是状态栈底部空一行、或者最后一段被 `Paragraph` 裁掉 —— 与 B4 的
/// `settings_panel_content_width` 是同一类不变量（内容按宽度排版，宽度与矩形必须同源）。
///
/// 生产渲染路径走 [`stack_plan`]（行数与内容同一份）；这个便捷入口留给鼠标命中与测试。
pub(super) fn stack_rows(info: &StatusInfo, width: usize, collapsed: bool) -> u16 {
    // 折叠态一段都不建（与 `stack_plan_with` 同一取舍）。
    let rows = if collapsed {
        0
    } else {
        segment_rows(info, width).len()
    };
    clampi_rows(rows, collapsed)
}

/// 「段行数 → 状态栈行数」的**唯一**公式：段行 + 最后那一行运行指示，上限
/// [`MAX_STACK_ROWS`]，折叠态恒 1 行。
///
/// 抽出来是因为两个入口都要它（[`StackPlan::rows_len`] 与 [`stack_rows`]）：各写一份就会
/// 在某次改上限时只改一边，症状是底部空一行或末段被裁。
fn clampi_rows(segment_rows_len: usize, collapsed: bool) -> u16 {
    if collapsed {
        return 1;
    }
    ((1 + segment_rows_len) as u16).min(MAX_STACK_ROWS)
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

/// [`stack_plan`] 的可注入版本（单测要能强制 reduced motion）。
///
/// 内容与 [`stack_plan`] 完全同源（就是它的 `lines()`）：两条路各写一遍行构建，就会在
/// 「行数怎么数」之外再多一处会漂的地方。
///
/// 只给单测用：生产路径拿的是 `render::draw` 建好的 plan，不需要再建一次。
#[cfg(test)]
fn stack_lines_with(
    info: &StatusInfo,
    width: usize,
    pal: &Palette,
    collapsed: bool,
    reduced_motion: bool,
) -> Vec<Line<'static>> {
    stack_plan_with(info, width, pal, collapsed, reduced_motion).lines()
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
/// 右半也得留在自己那半里，否则「各占一半」在视觉上就成了「谁短谁靠中间」。做法是把
/// 左半按它自己的实际宽度补齐到那个固定列，再整段拼上右半。
fn pair_line(left: &Segment, right: &Segment, width: usize, pal: &Palette) -> Line<'static> {
    let [(left_start, left_end), (right_start, right_end)] = pair_cols(width);
    let left_budget = left_end.saturating_sub(left_start);
    let right_budget = right_end.saturating_sub(right_start);
    let mut spans = segment_spans(left, left_budget, pal);
    let pad = (right_start.saturating_sub(left_start)).saturating_sub(spans_width(&spans));
    if pad > 0 {
        spans.push(Span::raw(" ".repeat(pad)));
    }
    spans.extend(segment_spans(right, right_budget, pal));
    Line::from(spans)
}

/// 一段的 span（label 补齐到定宽列 + 空格 + 截断后的值），宽度以 `budget` 为上限。
///
/// 可切换的段（`Segment::actionable`：`view` / `perm` / `think`）把**值所在的右半区**压在
/// 浅一档的底色块（[`Palette::actionable_block`]）上：从值的起点一直到段右缘，`label` 与
/// 分隔空格保持裸露的条带色，只读段一格都不铺 —— 终端里既没有悬停态也没有指针形状，两类
/// 段长得一样时，用户只能靠盲试去发现哪几行点得动。
///
/// 块铺满右半区、但不碰 `label`：`label` 那一列在每行上都必须长得一样（它是「这个字段
/// 叫什么」，不随能否点击变），跟着染色会把两列的视觉权重拉平，读起来像一整条告警条。
/// 值之后的留白留在块上，是因为块要划出的是「值占的是右边这一片」，围着几个字的碎块反而
/// 让人以为那半区里还有别的东西。代价是块宽不再等于可点列区间（[`segment_cols_in_row`]）——
/// 命中仍是整段，块只负责说「这段点得动」，不承诺「只有这一块能点」。
fn segment_spans(seg: &Segment, budget: usize, pal: &Palette) -> Vec<Span<'static>> {
    debug_assert!(!seg.label.is_empty(), "状态栈的段必须有 label");
    // `Paragraph` 不补位：标签必须自己补齐到定宽列，值才会落在同一列。
    let mut label = String::from(seg.label);
    label.push_str(&" ".repeat(LABEL_W.saturating_sub(UnicodeWidthStr::width(seg.label))));
    let value_budget = budget.saturating_sub(LABEL_COL_W);
    let value = truncate_display(&seg.value, value_budget);
    let block = block_style(seg, pal);
    let mut spans = vec![
        Span::styled(label, Style::new().fg(pal.muted)),
        // 分隔空格不带底色：带了它块就从值前面伸出一格，看着像值的一部分。
        Span::raw(" "),
        Span::styled(value.clone(), block_fg(block, pal.fg)),
    ];
    // 块铺到段的右缘：值起点是整块唯一的左边界，块因此是一条完整的浅色带。
    // 只对真底色这么做 —— 无色终端的下划线留在值上：拉到段尾就成了横贯的一整条线，
    // 说的是「这行有一段能点」，而标记该回答的是「这个值能点」。
    if let Some(bg) = block.filter(|b| b.bg.is_some()) {
        let pad = value_budget.saturating_sub(UnicodeWidthStr::width(value.as_str()));
        if pad > 0 {
            spans.push(Span::styled(" ".repeat(pad), bg.fg(pal.fg)));
        }
    }
    spans
}

/// 该段的底色块样式（只读段为 `None`）。
fn block_style(seg: &Segment, pal: &Palette) -> Option<Style> {
    seg.actionable().then(|| pal.actionable_block())
}

/// 值 span 的样式：有块就叠上块样式，否则只有前景色。
fn block_fg(block: Option<Style>, fg: Color) -> Style {
    match block {
        Some(block) => block.fg(fg),
        None => Style::new().fg(fg),
    }
}

/// span 序列的显示宽（`pair_line` 补位用；宽字符按列算，不按字节）。
fn spans_width(spans: &[Span<'static>]) -> usize {
    spans
        .iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
        .sum()
}

#[cfg(test)]
#[path = "status_stack_tests.rs"]
mod tests;
