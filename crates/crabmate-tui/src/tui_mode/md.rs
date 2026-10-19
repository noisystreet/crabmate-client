//! 助手正文的**行内 Markdown 轻渲染**辅助（ratatui span 层，无 transcript 状态）。
//!
//! 文本先经共享 [`crabmate_client_api::normalize_markdown_for_render`] 规范化（与
//! desktop/web 同一入口，修粘连围栏 / 标题与列表缺空格等），再由共享
//! [`crabmate_client_api::markdown_inline`] 提供 token；这里把 token 映射成样式并
//! 折成按显示宽度的物理行。搜索高亮叠加规则也在此统一（锚定整行反色、
//! 命中改黄前景且保留修饰位），保证与普通行的高亮观感一致。

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use unicode_width::UnicodeWidthChar;

use super::palette::Palette;
use crabmate_client_api::{InlineSpan, normalize_markdown_for_render, parse_inline_markdown};

/// 行内 markdown 片段 → 渲染样式（前景仅在需要区分时给出，其余随基色）。
/// 行内代码 / 链接跟主题走（`muted`→`accent`→`code_bg`），无色模式退到修饰位。
pub(crate) fn md_span_style(span: &InlineSpan, pal: &Palette) -> Style {
    match span {
        InlineSpan::Text(_) => Style::new(),
        InlineSpan::Bold(_) => Style::new().add_modifier(Modifier::BOLD),
        InlineSpan::Italic(_) => Style::new().add_modifier(Modifier::ITALIC),
        InlineSpan::InlineCode(_) => {
            if pal.is_mono() {
                Style::new().add_modifier(Modifier::REVERSED)
            } else {
                Style::new().fg(pal.fg).bg(pal.code_bg)
            }
        }
        InlineSpan::Strike(_) => Style::new().add_modifier(Modifier::CROSSED_OUT),
        InlineSpan::Link { .. } => {
            if pal.is_mono() {
                Style::new().add_modifier(Modifier::UNDERLINED)
            } else {
                Style::new()
                    .fg(pal.accent)
                    .add_modifier(Modifier::UNDERLINED)
            }
        }
    }
}

/// 片段文本（Link 取展示文本）。
fn span_text(span: &InlineSpan) -> &str {
    match span {
        InlineSpan::Text(t)
        | InlineSpan::Bold(t)
        | InlineSpan::Italic(t)
        | InlineSpan::InlineCode(t)
        | InlineSpan::Strike(t) => t,
        InlineSpan::Link { text, .. } => text,
    }
}

/// 按显示宽度断行（普通纯文本路径用；规则同 styled 版）。
/// `\n` 强制断行，控制字符丢弃，空输入产出单个空行。
pub(crate) fn wrap_physical(text: &str, width: usize) -> Vec<String> {
    let mut rows: Vec<String> = Vec::new();
    let mut row = String::new();
    let mut row_width = 0usize;
    for ch in text.chars() {
        if ch == '\n' {
            if !row.is_empty() {
                rows.push(std::mem::take(&mut row));
                row_width = 0;
            }
            continue;
        }
        if ch.is_control() {
            continue;
        }
        let cw = ch.width().unwrap_or(0);
        if !row.is_empty() && row_width + cw > width {
            rows.push(std::mem::take(&mut row));
            row_width = 0;
        }
        row.push(ch);
        row_width += cw;
    }
    if !row.is_empty() {
        rows.push(row);
    }
    if rows.is_empty() {
        rows.push(String::new());
    }
    rows
}

/// 单段（不含换行）行内文本 → 带 markdown 样式的字符流。
///
/// 图片（`![alt](url)`）先由 [`inline_image`] 切出去降级成 `[图] alt`：
/// 终端显示不了位图，而共享 parser 不认 `![`（会把 `!` 单独留成文本）。
pub(crate) fn inline_styled_chars(part: &str, pal: &Palette) -> Vec<(Style, char)> {
    inline_image::inline_styled_chars(part, pal)
}

/// 该行是否为代码围栏开关行（`` ``` `` / `~~~`，容忍行首缩进）。
fn is_fence_switch(line: &str) -> bool {
    let s = line.trim_start();
    s.starts_with("```") || s.starts_with("~~~")
}

/// 行首至多 3 个空格的缩进宽度（字节数）。
fn leading_indent(line: &str) -> usize {
    line.bytes().take(3).take_while(|&b| b == b' ').count()
}

/// `b` 起点是否为合法 UTF-8 边界并取首字符；调用方保证切片从 ASCII 边界开始。
fn first_char(b: &[u8]) -> Option<char> {
    std::str::from_utf8(b).ok().and_then(|s| s.chars().next())
}

/// 标记后紧贴的 CJK 等非 ASCII 字母正文（对应共享 normalize 的 `-规范`/`1.下一步` 规则）。
fn glued_cjk_body(b: &[u8]) -> bool {
    first_char(b).is_some_and(|c| c.is_alphabetic() && !c.is_ascii())
}

/// 行首 ATX 标题匹配：返回（`#` 个数, 内容起始字节偏移）。
/// 与 normalize 语义对齐：`#` 后可跟空格/tab，也允许紧贴正文（`###规范`/`#Title`）。
fn heading_match(rest: &[u8]) -> Option<(usize, usize)> {
    let mut h = 0usize;
    while h < rest.len() && h < 6 && rest[h] == b'#' {
        h += 1;
    }
    if h == 0 {
        return None;
    }
    match rest.get(h) {
        None => Some((h, h)),
        Some(b' ') | Some(b'\t') => Some((h, h + 1)),
        Some(b'#') => None,
        Some(_) => Some((h, h)),
    }
}

/// 引用 `>` 匹配：`>正文`（紧贴）/ `> 正文`（分隔）都可。
fn quote_match(rest: &[u8]) -> Option<(usize, usize)> {
    if rest.first() != Some(&b'>') {
        return None;
    }
    Some((
        1,
        match rest.get(1) {
            Some(b' ') | Some(b'\t') => 2,
            _ => 1,
        },
    ))
}

/// 无序列表 `-`/`+`（CJK 紧贴放行）；`*` 仅空格/tab 分隔（紧贴按斜体语义）。
fn unordered_match(rest: &[u8]) -> Option<(usize, usize)> {
    let c0 = *rest.first()?;
    if matches!(c0, b'-' | b'+') {
        return match rest.get(1) {
            Some(b' ') | Some(b'\t') => Some((1, 2)),
            Some(_) if glued_cjk_body(&rest[1..]) => Some((1, 1)),
            _ => None,
        };
    }
    if c0 == b'*' {
        return match rest.get(1) {
            Some(b' ') | Some(b'\t') => Some((1, 2)),
            _ => None,
        };
    }
    None
}

/// 有序列表 `1. `/`1) `（≤9 位；CJK 紧贴放行）。
fn ordered_match(rest: &[u8]) -> Option<(usize, usize)> {
    let mut d = 0usize;
    while d < rest.len() && d < 9 && rest[d].is_ascii_digit() {
        d += 1;
    }
    if d == 0 || !matches!(rest.get(d), Some(b'.') | Some(b')')) {
        return None;
    }
    match rest.get(d + 1) {
        Some(b' ') | Some(b'\t') => Some((d + 1, d + 2)),
        Some(_) if glued_cjk_body(&rest[d + 1..]) => Some((d + 1, d + 1)),
        _ => None,
    }
}

/// 行首列表/引用匹配：返回（标记字节数, 内容起始字节偏移）。
fn list_or_quote_match(rest: &[u8]) -> Option<(usize, usize)> {
    quote_match(rest)
        .or_else(|| unordered_match(rest))
        .or_else(|| ordered_match(rest))
}

/// 分隔线的最短可见长度（`width == 0` 的纯文本场景退化成这个）。
const RULE_MIN: usize = 3;

/// 水平分隔线：整行只有 `-` / `*` / `_`（≥3 个），允许中间夹空格。
fn is_rule(rest: &[u8]) -> bool {
    let mut marks = 0usize;
    for &b in rest {
        match b {
            b'-' | b'*' | b'_' => marks += 1,
            b' ' | b'\t' => {}
            _ => return false,
        }
    }
    marks >= 3
}

/// 任务列表：`- [ ] 文本` / `- [x] 文本` → 返回（是否勾选, 正文起点）。
fn task_match(rest: &[u8]) -> Option<(bool, usize)> {
    let (_, ca) = unordered_match(rest)?;
    let checked = match rest.get(ca..)? {
        [b'[', b' ', b']', ..] => false,
        [b'[', b'x' | b'X', b']', ..] => true,
        _ => return None,
    };
    let after = ca + 3;
    let start = if rest.get(after) == Some(&b' ') {
        after + 1
    } else {
        after
    };
    Some((checked, start))
}

/// 标题层级缩进（列）：H1 用 `▌` 竖条代替缩进，H4 及更深不再加宽。
fn heading_indent(level: usize) -> usize {
    match level {
        0 | 1 => 0,
        2 => 2,
        3 => 4,
        _ => 6,
    }
}

/// 分隔线：一条 `─` 铺到可用宽度。
fn rule_chars(width: usize, pal: &Palette) -> Vec<(Style, char)> {
    let st = if pal.is_mono() {
        Style::new()
    } else {
        Style::new().fg(pal.muted)
    };
    (0..width.max(RULE_MIN)).map(|_| (st, '─')).collect()
}

/// 标题：**标记不上屏**，改用层级缩进 + 强调；H1 额外加 `▌` 竖条。
///
/// 早前把 `###` 灰化留在行内，问题有二：白占 2～6 列；且 H1–H6 除字数外看起来
/// 完全一样，长文档里读不出骨架。
fn push_heading(out: &mut Vec<(Style, char)>, level: usize, body: &str, pal: &Palette) {
    let mut base = Style::new().add_modifier(Modifier::BOLD);
    if !pal.is_mono() && level <= 2 {
        base = base.fg(pal.accent);
    }
    if level == 1 {
        let bar = if pal.is_mono() {
            Style::new()
        } else {
            Style::new().fg(pal.accent)
        };
        out.push((bar, '▌'));
        out.push((Style::new(), ' '));
    } else {
        out.extend((0..heading_indent(level)).map(|_| (Style::new(), ' ')));
    }
    // 行内样式（粗体/代码底纹）叠加在标题样式之上，不是被覆盖。
    out.extend(
        inline_styled_chars(body, pal)
            .into_iter()
            .map(|(s, c)| (s.patch(base), c)),
    );
}

/// 任务列表：`☑` / `☐` 取代 `[x]` / `[ ]` —— 省一列，且方框本身就是列表语义。
fn push_task(out: &mut Vec<(Style, char)>, checked: bool, body: &str, pal: &Palette) {
    let st = match (pal.is_mono(), checked) {
        (true, _) => Style::new(),
        (false, true) => Style::new().fg(pal.accent),
        (false, false) => Style::new().fg(pal.muted),
    };
    out.push((st, if checked { '☑' } else { '☐' }));
    out.push((Style::new(), ' '));
    out.extend(inline_styled_chars(body, pal));
}

/// 无序列表符号：一级 `•`、二级 `◦`，让嵌套层级一眼可辨。
///
/// 只分两级是**故意的**：`leading_indent` 最多认 3 个空格 —— CommonMark 里
/// 4 空格缩进是代码块，不该被当成列表项，那种行原样呈现。
fn bullet_char(indent: usize) -> char {
    if indent >= 2 { '◦' } else { '•' }
}

/// 行首标记渲染。
///
/// 三类标记的处理**故意不同**：
/// - 无序列表 `-` / `*` / `+`：换成圆点 —— 短横线在终端里和分隔线、负号容易混
/// - 有序列表 `1.`：保留编号。数字本身携带顺序信息，换成圆点就丢了
/// - 引用 `>`：换成竖条 `▎`。连续几行的竖条自然连成一条线，范围一眼可见；
///   嵌套（`> > x`）就叠几条
fn push_marker(
    out: &mut Vec<(Style, char)>,
    marker_len: usize,
    content_at: usize,
    rest: &str,
    indent: usize,
    pal: &Palette,
) {
    let bullet_st = if pal.is_mono() {
        Style::new()
    } else {
        Style::new().fg(pal.muted)
    };
    match rest.as_bytes().first().copied() {
        Some(b'-') | Some(b'*') | Some(b'+') => {
            out.push((bullet_st, bullet_char(indent)));
            out.push((Style::new(), ' '));
        }
        Some(b'>') => {
            push_quote(out, rest, pal);
            return;
        }
        _ => {
            let marker: String = rest[..marker_len].to_string();
            out.extend(marker.chars().map(|c| (Style::new().fg(Color::Gray), c)));
            if content_at > marker_len {
                let sep = rest[marker_len..].chars().next().unwrap_or(' ');
                out.push((Style::new(), sep));
            }
        }
    }
    out.extend(inline_styled_chars(&rest[content_at..], pal));
}

/// 引用：每个 `>` 换成一个竖条，嵌套就叠几条。
///
/// `>` 本身留在屏幕上时，读者要自己数「这几行前面有几个尖括号」才能看出引用范围；
/// 换成竖条后，连续几行自然连成一条竖线，范围一眼可见。
fn push_quote(out: &mut Vec<(Style, char)>, rest: &str, pal: &Palette) {
    let bar = if pal.is_mono() {
        Style::new()
    } else {
        Style::new().fg(pal.accent)
    };
    let mut body = rest;
    while let Some((_, content_at)) = quote_match(body.as_bytes()) {
        // 竖条后固定补一个空格：`>正文`（无空格）与 `> 正文` 输出一致，
        // 也和 `• ` / `☐ ` 一样占 2 列。
        out.push((bar, '▎'));
        out.push((Style::new(), ' '));
        body = &body[content_at..];
    }
    out.extend(inline_styled_chars(body, pal));
}

/// 单行：行首标记（分隔线/标题/任务列表/列表/引用）渲染 + 其余部分行内 md。
///
/// 宽容识别只影响样式判定，不增删正文；只有**结构性标记**（分隔线、标题 `#`、
/// 任务列表的 `[ ]`）不上屏 —— 它们由更明确的视觉形式替代。
fn styled_part_chars(part: &str, pal: &Palette, width: usize) -> Vec<(Style, char)> {
    let indent = leading_indent(part);
    let rest = &part[indent..];
    let mut out: Vec<(Style, char)> = part[..indent].chars().map(|c| (Style::new(), c)).collect();
    if is_rule(rest.as_bytes()) {
        out.extend(rule_chars(width, pal));
        return out;
    }
    if let Some((level, ca)) = heading_match(rest.as_bytes()) {
        push_heading(&mut out, level, &rest[ca..], pal);
        return out;
    }
    if let Some((checked, ca)) = task_match(rest.as_bytes()) {
        push_task(&mut out, checked, &rest[ca..], pal);
        return out;
    }
    if let Some((m, ca)) = list_or_quote_match(rest.as_bytes()) {
        push_marker(&mut out, m, ca, rest, indent, pal);
        return out;
    }
    inline_styled_chars(part, pal)
}

/// 是否丢弃这一行：围栏外的连续空行只留一个，首部空行也不产生间距
/// （desktop 走 pulldown-cmark，多个空行只算一个段落分隔）。
///
/// 围栏切换行不必特判：以 ``` / ~~~ 开头的行不可能是空行。
fn skip_blank_gap(blank: bool, in_fence: bool, prev_blank: bool) -> bool {
    blank && !in_fence && prev_blank
}

/// 代码块内 tab 的展开宽度（与多数编辑器默认一致）。
const FENCE_TAB_WIDTH: usize = 4;

/// 开围栏行的语言标签：`` ```rust `` / `~~~rust` / `` ``` rust `` 都取 `rust`。
fn fence_lang(part: &str) -> &str {
    part.trim()
        .trim_start_matches('`')
        .trim_start_matches('~')
        .trim()
}

/// 代码块的整行样式：深色主题铺 `code_bg`，无色模式不铺底。
///
/// 无色模式下**不用** `REVERSED`：行内代码那样做是因为它嵌在正文里、不反色就看不出来；
/// 整块代码若整片反白，屏幕会像选中的一大段，反而更乱。
fn code_block_style(pal: &Palette) -> Style {
    if pal.is_mono() {
        Style::new()
    } else {
        Style::new().fg(pal.fg).bg(pal.code_bg)
    }
}

/// 把当前行补空格到 `width`（形成等宽矩形）；已超宽则不动，交给折行处理。
fn pad_to(out: &mut Vec<(Style, char)>, used: usize, width: usize, st: Style) {
    for _ in used..width {
        out.push((st, ' '));
    }
}

/// 开围栏 → 语言标签行（`accent`），整行铺 `code_bg` 并补到目标宽度。
///
/// 标签行也铺底（而不只是画个 `rust`），否则代码块顶端会缺一角，矩形不闭合。
/// **无语言时返回空**：不画标签行，` ``` ` 后直接就是代码，块上边界是第一行代码。
fn fence_open_chars(part: &str, pal: &Palette, width: usize) -> Vec<(Style, char)> {
    let lang = fence_lang(part);
    // 无语言：返回空（**不要**补满一行空格 —— 那在屏幕上就是一行空色块，
    // 读起来像代码块上方多了一个空行）。
    if lang.is_empty() {
        return Vec::new();
    }
    let code = code_block_style(pal);
    let mut out: Vec<(Style, char)> = Vec::new();
    let mut used = 0usize;
    let base = Style::new().fg(pal.accent);
    // 语言名坐在代码底上（无色模式下无底）。
    let st = if pal.is_mono() {
        base
    } else {
        base.bg(pal.code_bg)
    };
    for ch in lang.chars() {
        out.push((st, ch));
        used += ch.width().unwrap_or(0);
    }
    pad_to(&mut out, used, width, code);
    out
}

/// 代码行 → 内容，整行铺 `code_bg` 并补到 `width`（等宽矩形）。
///
/// 代码从最左列开始：留内边距会让代码与色块左边界之间多出一段看似「没对齐」的空隙。
/// 不画行号：行号在窄终端里会挤占代码宽度，而且流式输出时「行号 + 内容」两套
/// 编号列容易被读成排版噪声。块的边界靠底色矩形表达即可。
///
/// tab 按 [`FENCE_TAB_WIDTH`] 展开：`wrap_styled_chars` 把控制字符整个丢掉，
/// 不展开的话代码里按 tab 缩进的行会全部左对齐，缩进层级直接消失。
fn code_line_chars(part: &str, pal: &Palette, width: usize) -> Vec<(Style, char)> {
    let code = code_block_style(pal);
    let mut out: Vec<(Style, char)> = Vec::new();
    let mut used = 0usize;
    let mut col = 0usize;
    for ch in part.chars() {
        if ch == '\t' {
            let pad = FENCE_TAB_WIDTH - (col % FENCE_TAB_WIDTH);
            out.extend((0..pad).map(|_| (code, ' ')));
            col += pad;
            used += pad;
            continue;
        }
        if ch.is_control() {
            continue;
        }
        out.push((code, ch));
        let w = ch.width().unwrap_or(0);
        col += w;
        used += w;
    }
    // 至少留一格右侧间距，让代码不贴着色块右边缘。
    let target = width.max(used + 1);
    pad_to(&mut out, used, target, code);
    out
}

/// 追加一行（首行前不补换行）；空段落也要过这里，否则段落间距会消失。
fn push_row(out: &mut Vec<(Style, char)>, first: &mut bool, row: Vec<(Style, char)>) {
    if !*first {
        out.push((Style::new(), '\n'));
    }
    out.extend(row);
    *first = false;
}

/// 去掉末尾的换行（CommonMark：文档结尾的空行不算段落）。
fn trim_tail(out: &mut Vec<(Style, char)>) {
    while let Some(&(_, '\n')) = out.last() {
        out.pop();
    }
}

/// 若 `rows[i]` 起是表格，渲染并追加；返回消费掉的行数。
fn emit_table(
    rows: &[&str],
    i: usize,
    pal: &Palette,
    width: usize,
    in_fence: bool,
    out: &mut Vec<(Style, char)>,
    first: &mut bool,
) -> Option<usize> {
    // 围栏内的 `|` 是代码内容，不是表格。
    if in_fence {
        return None;
    }
    let (lines, consumed) = table::table_rows(rows, i, pal, width)?;
    for line in lines {
        push_row(out, first, line);
    }
    Some(consumed)
}

/// 整段助手文本 → 带样式的字符流（段间补 `\n` 作强制断行，标记不跨段解析）。
/// 先过共享 [`crabmate_client_api::normalize_markdown_for_render`]（与 desktop/web
/// 同一入口），再逐行渲染；处于 `` ``` `` / `~~~` 围栏内的行整行按纯文本，
/// 避免代码内容里的 `**`/反引号/`[t](u)` 等被误当成行内样式而吞字符。
///
/// 空行语义对齐 desktop（CommonMark）：围栏外的连续空行折叠成**一个**，
/// 首部与尾部空行不产生间距；围栏内的空行原样保留（desktop 的 `<pre>` 同样保留），
/// 含流式中途停在未闭合围栏里时的结尾空行。
/// 段落间距由此在折行层落成一行空白，与 `.msg-md-prose p { margin }` 对应。
///
/// `code_width` 是代码块要铺满的目标显示宽度（让整个块是个等宽矩形）；
/// 传 `0` 表示不铺满（纯文本场景与大多数用例只关心内容，不关心块宽）。
pub(crate) fn assistant_styled_text(
    text: &str,
    pal: &Palette,
    code_width: usize,
) -> Vec<(Style, char)> {
    let text = normalize_markdown_for_render(text);
    let mut out = Vec::new();
    let mut first = true;
    let mut in_fence = false;
    // 开头视为「上一行为空」，让首部空行不产生间距。
    let mut prev_blank = true;
    let rows: Vec<&str> = text.split('\n').collect();
    let mut i = 0usize;
    while i < rows.len() {
        let part = rows[i];
        if is_fence_switch(part) {
            let opening = !in_fence;
            in_fence = !in_fence;
            // 围栏标记行本身**不上屏**：开围栏折成一行语言标签，闭合行直接丢掉 ——
            // ` ``` ` 只是解析记号。无语言时不画标签行（空色块会读作「多一个空行」）。
            // 流式下不抖动：未闭合时标签行已经画出，闭合后只是不再新增行。
            if opening {
                let label = fence_open_chars(part, pal, code_width);
                if !label.is_empty() {
                    push_row(&mut out, &mut first, label);
                }
            }
            prev_blank = false;
            i += 1;
            continue;
        }
        if let Some(consumed) =
            emit_table(&rows, i, pal, code_width, in_fence, &mut out, &mut first)
        {
            prev_blank = false;
            i += consumed;
            continue;
        }
        let blank = part.trim().is_empty();
        if skip_blank_gap(blank, in_fence, prev_blank) {
            i += 1;
            continue;
        }
        let row = if in_fence {
            code_line_chars(part, pal, code_width)
        } else {
            styled_part_chars(part, pal, code_width)
        };
        push_row(&mut out, &mut first, row);
        prev_blank = blank;
        i += 1;
    }
    // 围栏未闭合时不动：流式输出随时可能续写，围栏内的空行属于代码内容，
    // 提前裁掉会让它在块闭合时「消失又出现」。
    if !in_fence {
        trim_tail(&mut out);
    }
    out
}

/// 按显示宽度把带样式字符流折成物理行（控制字符丢弃）。
///
/// `\n` 一律落行——包括当前行为空时：空行要保留成一行空白，段落间距才存在
/// （对齐 desktop 的 `.msg-md-prose p { margin }`）。普通纯文本走
/// [`wrap_physical`]，仍折叠空行。
pub(crate) fn wrap_styled_chars(styled: &[(Style, char)], width: usize) -> Vec<Vec<(Style, char)>> {
    let mut rows: Vec<Vec<(Style, char)>> = Vec::new();
    let mut row: Vec<(Style, char)> = Vec::new();
    let mut row_width = 0usize;
    for &(st, ch) in styled {
        if ch == '\n' {
            rows.push(std::mem::take(&mut row));
            row_width = 0;
            continue;
        }
        if ch.is_control() {
            continue;
        }
        let cw = ch.width().unwrap_or(0);
        if !row.is_empty() && row_width + cw > width {
            rows.push(std::mem::take(&mut row));
            row_width = 0;
        }
        row.push((st, ch));
        row_width += cw;
    }
    if !row.is_empty() {
        rows.push(row);
    }
    if rows.is_empty() {
        rows.push(Vec::new());
    }
    rows
}

/// 叠加行内样式到行基色上（md 未给前景/底色时随基色）。
///
/// **三个槽位都要搬**：`fg`、`bg`、modifier。曾经只搬 `fg` 与 modifier，于是
/// `InlineCode` 的 `bg(code_bg)` 在合并时被静默丢掉 —— 行内代码在屏幕上从来没有底色，
/// 而 `md_span_style` 的单测一直是绿的（它只验证「算出来了」，不验证「画上去了」）。
/// 现在由 `render_color_tests::inline_code_gets_code_background` 在缓冲区层面守住。
pub(crate) fn md_row_style(md: Style, base: Style) -> Style {
    let mut s = Style::new();
    if let Some(c) = md.fg.or(base.fg) {
        s = s.fg(c);
    }
    if let Some(c) = md.bg.or(base.bg) {
        s = s.bg(c);
    }
    s.add_modifier(md.add_modifier | base.add_modifier)
}

/// 搜索高亮叠加：锚定行整体反色（跟主题，无色模式退 `REVERSED`）；
/// 其余命中行改 `hit` 前景（保留修饰位）。
pub(crate) fn highlight_override(s: Style, matched: bool, anchor: bool, pal: &Palette) -> Style {
    if anchor {
        return if pal.is_mono() {
            Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else {
            Style::new()
                .fg(pal.bg)
                .bg(pal.hit)
                .add_modifier(Modifier::BOLD)
        };
    }
    if matched {
        return if pal.is_mono() {
            s.add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(pal.hit).add_modifier(s.add_modifier)
        };
    }
    s
}

fn same_style(a: &Style, b: &Style) -> bool {
    a.fg == b.fg && a.bg == b.bg && a.add_modifier == b.add_modifier
}

/// 一行带样式字符 → span 序列（连续同款式字符合并）。
pub(crate) fn styled_row_spans(
    row: &[(Style, char)],
    base: Style,
    matched: bool,
    anchor: bool,
    pal: &Palette,
) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut cur: Option<(String, Style)> = None;
    for &(md, ch) in row {
        let style = highlight_override(md_row_style(md, base), matched, anchor, pal);
        match &mut cur {
            Some((t, s)) if same_style(s, &style) => t.push(ch),
            _ => {
                if let Some((t, s)) = cur.take() {
                    spans.push(Span::styled(t, s));
                }
                cur = Some((ch.to_string(), style));
            }
        }
    }
    if let Some((t, s)) = cur.take() {
        spans.push(Span::styled(t, s));
    }
    spans
}

// 表格渲染拆到独立文件：`md.rs` 贴着 920 行门禁，且表格要跨行收集（见文件头注释）。
#[path = "md_table.rs"]
mod table;

// 行内图片降级（`![alt](url)` → `[图] alt`）单独成文件：见 md_image.rs 头注释。
#[path = "md_image.rs"]
mod inline_image;

// 测试拆到独立文件：避免 lizard 函数体合并跨度超 fn-nloc 门禁（同 render / settings_panel 惯例）。
#[cfg(test)]
#[path = "md_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "md_marker_tests.rs"]
mod tests_marker;
