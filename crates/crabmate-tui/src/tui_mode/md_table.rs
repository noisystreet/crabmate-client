//! 极简表格渲染：**无竖线**，靠表头加粗 + 一条 `─` 分隔 + 列间两空格。
//!
//! 不用第三方表格库：`comfy-table` / `tabled` 交出的是一整块 `String`（或 ANSI），
//! 而聊天区管线是带链接区间的 `PartChars`（字符流 + 链接的字符区间）—— 要么丢掉它们的
//! 样式再自己解析，
//! 要么为了接进来写一个 ANSI 反解析器，两头都不划算。这里用已有的 `unicode-width`
//! 算宽度、用主题槽取色，直接产出字符流。
//!
//! 流式输出下**列宽只由「表头 + 已见数据行」决定**：新来一行更宽时整块会重排一次，
//! 这是无法避免的（终端要等看到全部内容才知道最终列宽）；一旦表格写完就稳定。
//! 不做事后回退重排，也不缓存上一帧。

use ratatui::style::{Modifier, Style};
use unicode_width::UnicodeWidthChar;

use super::{Palette, PartChars, inline_styled_chars};

/// 列间距（空格数）。
const TABLE_GAP: usize = 2;
/// 压缩时的单列下限：再窄就只剩省略号，没有信息量。
const TABLE_MIN_COL: usize = 3;

/// 一行渲染结果：字符流 + 单元格内链接的**字符区间**。
///
/// 曾经这里是裸 `Vec<(Style, char)>`，链接在 `render_row` 里被丢掉（理由写的是
/// 「按列截断，没有稳定列」）。但屏幕列本来就不是这一层算的：`push_row` 会
/// `absorb` 并平移链接区间，`wrap_styled_rows` 再统一换算成列。所以这一层只负责
/// 把区间带着走，并在截断时把落在可见字符之外的区间裁掉 —— 热区因此与看到的字对齐。
type StyledRow = PartChars;
/// 整张表的渲染结果：若干行 + 消费掉的源行数。
type TableBlock = (Vec<StyledRow>, usize);

/// 单元格对齐方式，来自分隔行里的冒号。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Align {
    Left,
    Center,
    Right,
}

/// 从 `rows[i]` 起若是一张表（表头行 + 分隔行），返回（渲染好的行, 消费的行数）。
///
/// 表格必须紧跟分隔行才算数 —— 单看 `a | b` 不足以判断（普通正文里也有竖线）。
pub(super) fn table_rows(
    rows: &[&str],
    i: usize,
    pal: &Palette,
    limit: usize,
) -> Option<TableBlock> {
    let header = *rows.get(i)?;
    let align_row = *rows.get(i + 1)?;
    if !header.contains('|') || !is_align_row(align_row) {
        return None;
    }
    let headers = split_cells(header);
    let aligns: Vec<Align> = split_cells(align_row)
        .iter()
        .map(|c| cell_align(c))
        .collect();
    // 列数对不上就是普通文本里恰好有个 `|`，别硬当表。
    if headers.is_empty() || aligns.len() != headers.len() {
        return None;
    }
    let (body, consumed) = collect_body(rows, i + 2);
    let widths = col_widths(&headers, &body, pal, limit);
    let mut out: Vec<StyledRow> = Vec::with_capacity(body.len() + 2);
    out.push(render_row(&headers, &widths, &aligns, pal, true));
    out.push(rule_row(&widths, pal));
    for cells in &body {
        out.push(render_row(cells, &widths, &aligns, pal, false));
    }
    Some((out, consumed))
}

/// 收集数据行，直到空行或不含 `|` 的行；返回（各行单元格, 总消费行数含表头与分隔行）。
fn collect_body<'a>(rows: &[&'a str], start: usize) -> (Vec<Vec<&'a str>>, usize) {
    let mut body: Vec<Vec<&str>> = Vec::new();
    let mut consumed = 2usize;
    for row in rows.iter().skip(start) {
        if row.trim().is_empty() || !row.contains('|') {
            break;
        }
        body.push(split_cells(row));
        consumed += 1;
    }
    (body, consumed)
}

/// 一行是否表格分隔行（`|---|:--:|`）。宽容处理：`-` 只要求 ≥1 个。
fn is_align_row(line: &str) -> bool {
    if !line.contains('|') {
        return false;
    }
    let cells = split_cells(line);
    !cells.is_empty() && cells.iter().all(|c| is_align_cell(c))
}

fn is_align_cell(cell: &str) -> bool {
    let t = cell.trim().trim_matches(':');
    !t.is_empty() && t.chars().all(|ch| ch == '-')
}

/// 拆单元格：允许首尾竖线缺失；**不支持 `\|` 转义**（需要时再补）。
fn split_cells(line: &str) -> Vec<&str> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    if t.trim().is_empty() {
        return Vec::new();
    }
    t.split('|').map(|c| c.trim()).collect()
}

fn cell_align(cell: &str) -> Align {
    let t = cell.trim();
    match (t.starts_with(':'), t.ends_with(':')) {
        (true, true) => Align::Center,
        (false, true) => Align::Right,
        _ => Align::Left,
    }
}

/// 单元格渲染后的宽度（按行内 markdown **去掉标记后**的显示宽度算，
/// 否则 `**粗**` 会多算 4 列）。
fn cell_width(cell: &str, pal: &Palette) -> usize {
    inline_styled_chars(cell, pal)
        .iter()
        .map(|(_, ch)| ch.width().unwrap_or(0))
        .sum()
}

/// 各列宽度：先按内容取最大值，再按可用宽度压缩。
fn col_widths(headers: &[&str], body: &[Vec<&str>], pal: &Palette, limit: usize) -> Vec<usize> {
    let n = headers.len();
    let mut w: Vec<usize> = headers.iter().map(|h| cell_width(h, pal)).collect();
    for row in body {
        for (c, cell) in row.iter().enumerate().take(n) {
            w[c] = w[c].max(cell_width(cell, pal));
        }
    }
    for x in &mut w {
        *x = (*x).max(1);
    }
    shrink_to_fit(w, limit)
}

/// 总宽（含列间空格）超预算时，反复把最宽的列减 1，直到放下或全部到下限。
fn shrink_to_fit(mut w: Vec<usize>, limit: usize) -> Vec<usize> {
    if limit == 0 {
        return w;
    }
    let gap = TABLE_GAP * w.len().saturating_sub(1);
    let mut total: usize = w.iter().sum::<usize>() + gap;
    while total > limit {
        let widest = w
            .iter()
            .enumerate()
            .filter(|(_, x)| **x > TABLE_MIN_COL)
            .max_by_key(|(_, x)| **x)
            .map(|(i, _)| i);
        let Some(idx) = widest else { break };
        w[idx] -= 1;
        total -= 1;
    }
    w
}

fn rule_style(pal: &Palette) -> Style {
    if pal.is_mono() {
        Style::new()
    } else {
        Style::new().fg(pal.muted)
    }
}

/// 一行单元格 → 字符流：逐列按宽度对齐填满，列间留 [`TABLE_GAP`] 个空格。
fn render_row(
    cells: &[&str],
    widths: &[usize],
    aligns: &[Align],
    pal: &Palette,
    header: bool,
) -> StyledRow {
    let mut out = PartChars::default();
    for (c, w) in widths.iter().enumerate() {
        if c > 0 {
            out.extend((0..TABLE_GAP).map(|_| (Style::new(), ' ')));
        }
        let cell = cells.get(c).copied().unwrap_or("");
        // 单元格里的链接**保留**：文字本来就带 accent + 下划线，丢掉标记就是
        // 「看起来是链接、点不动、也不说为什么」。列宽截断时的裁剪在 `truncate_cell`。
        let mut chars = inline_styled_chars(cell, pal);
        if header {
            for (style, _) in chars.iter_mut() {
                *style = style.add_modifier(Modifier::BOLD);
            }
        }
        let used: usize = chars.iter().map(|(_, ch)| ch.width().unwrap_or(0)).sum();
        out.absorb(pad_cell(chars, used, *w, aligns[c], pal));
    }
    out
}

/// 按对齐填到列宽；超宽则截断并以 `…` 收尾（不折行 —— 表格里折行会撑乱行高）。
fn pad_cell(cell: PartChars, used: usize, w: usize, align: Align, pal: &Palette) -> PartChars {
    if used > w {
        return truncate_cell(cell, w, pal);
    }
    let spare = w - used;
    let (left, right) = match align {
        Align::Left => (0, spare),
        Align::Right => (spare, 0),
        Align::Center => (spare / 2, spare - spare / 2),
    };
    let mut out = PartChars::default();
    out.extend((0..left).map(|_| (Style::new(), ' ')));
    // 左填充也是字符：`absorb` 会把单元格里的链接区间一起右移，
    // 否则右对齐 / 居中的单元格热区会整体偏左。
    out.absorb(cell);
    out.extend((0..right).map(|_| (Style::new(), ' ')));
    out
}

/// 超宽单元格截断到列宽并补 `…`。
///
/// **落在被截掉部分的链接区间一并裁掉**：热区若越过 `…`，点下去打开的就是
/// 屏幕上根本没画出来的那一截链接。
fn truncate_cell(mut cell: PartChars, w: usize, pal: &Palette) -> PartChars {
    let budget = w.saturating_sub(1);
    let mut used = 0usize;
    let mut keep = 0usize;
    for (_, ch) in cell.iter() {
        let cw = ch.width().unwrap_or(0);
        if used + cw > budget {
            break;
        }
        used += cw;
        keep += 1;
    }
    cell.truncate(keep);
    clip_links(&mut cell, keep);
    cell.push((rule_style(pal), '…'));
    cell
}

/// 把链接区间裁到前 `keep` 个字符之内：越界的整条丢弃，跨界的收口。
///
/// `keep == 0` 时所有区间都被丢弃 —— 那是「列宽连一个字都放不下」的退化情形（列宽下限
/// `TABLE_MIN_COL = 3` 让它在实践中不可达），留空热区比留一个指向 `…` 的更诚实。
fn clip_links(cell: &mut PartChars, keep: usize) {
    cell.links.retain_mut(|link| {
        if link.start >= keep {
            return false;
        }
        link.end = link.end.min(keep);
        true
    });
}

/// 表头下的分隔线：每列一条 `─`，列间空格。无竖线、无交叉点。
fn rule_row(widths: &[usize], pal: &Palette) -> PartChars {
    let st = rule_style(pal);
    let mut out = PartChars::default();
    for (c, w) in widths.iter().enumerate() {
        if c > 0 {
            out.extend((0..TABLE_GAP).map(|_| (st, ' ')));
        }
        out.extend((0..*w).map(|_| (st, '─')));
    }
    out
}
