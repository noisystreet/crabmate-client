//! 行内图片降级：`![alt](url)` → `[图] alt`。
//!
//! **为什么不给共享 parser 加 `InlineSpan::Image`**：终端永远显示不了位图，
//! `Image` 变体在 TUI 侧最终还是要降级成文字；而 web/desktop 走的是另一条
//! HTML 转换路径（`.msg-md-prose img` 有样式，说明 web 端出的是真 `<img>`）。
//! 降级逻辑放在降级的地方，不必动 `crabmate-client-api` 的契约。
//!
//! 但**完全不处理**是不行的：共享 parser 不认 `![`，`!` 会落成普通文本、
//! 后面的 `[alt](url)` 被当成普通链接，屏幕上就出现「孤零零的感叹号 + 下划线文字」，
//! 看着像漏渲染。

use super::*;

/// 行内片段：普通文本，或一张图（只保留 alt）。
enum InlinePiece<'a> {
    Text(&'a str),
    Image(&'a str),
}

/// 行内渲染入口：先把图片切出来，普通文本仍交给共享 parser。
///
/// alt 内部**不再扫图片**（`push_image` 走 `push_inline`），避免递归。
pub(super) fn inline_styled_chars(part: &str, pal: &Palette) -> Vec<(Style, char)> {
    let mut out = Vec::new();
    for piece in split_images(part) {
        match piece {
            InlinePiece::Text(t) => push_inline(&mut out, t, pal),
            InlinePiece::Image(alt) => push_image(&mut out, alt, pal),
        }
    }
    out
}

/// 普通文本片段：走共享 parser 得到 Bold/Italic/Code/Link 等 span。
fn push_inline(out: &mut Vec<(Style, char)>, text: &str, pal: &Palette) {
    if text.is_empty() {
        return;
    }
    for span in parse_inline_markdown(text) {
        let st = md_span_style(&span, pal);
        out.extend(span_text(&span).chars().map(|c| (st, c)));
    }
}

/// 图片降级：`[图]`（accent）+ 空格 + alt（alt 里的行内样式照常生效）。
fn push_image(out: &mut Vec<(Style, char)>, alt: &str, pal: &Palette) {
    let tag = if pal.is_mono() {
        Style::new()
    } else {
        Style::new().fg(pal.accent)
    };
    out.extend("[图]".chars().map(|c| (tag, c)));
    if alt.trim().is_empty() {
        return;
    }
    out.push((Style::new(), ' '));
    push_inline(out, alt, pal);
}

/// 把文本切成普通片段与图片片段。
///
/// 输入已是单行（调用方按 `\n` 切过），所以不必处理跨行；alt 里遇到 `]` 即结束，
/// **不支持 `\]` 转义**（真实场景里 alt 极少含方括号）。
fn split_images(text: &str) -> Vec<InlinePiece<'_>> {
    let mut out = Vec::new();
    let mut last = 0usize;
    let mut i = 0usize;
    while i + 1 < text.len() {
        if let Some((end, alt_start, alt_end)) = match_image(text, i) {
            if i > last {
                out.push(InlinePiece::Text(&text[last..i]));
            }
            out.push(InlinePiece::Image(&text[alt_start..alt_end]));
            i = end;
            last = end;
            continue;
        }
        i += 1;
    }
    if last < text.len() {
        out.push(InlinePiece::Text(&text[last..]));
    }
    if out.is_empty() {
        out.push(InlinePiece::Text(text));
    }
    out
}

/// 在 `start` 处若匹配 `![alt](url)`，返回（结束下标, alt 起点, alt 终点）。
///
/// URL 不参与渲染（终端里点不了），只校验它存在 —— 缺了 `)` 就不算图片，
/// 原样留给共享 parser，避免把残句吞掉。
fn match_image(text: &str, start: usize) -> Option<(usize, usize, usize)> {
    let bytes = text.as_bytes();
    if bytes.get(start) != Some(&b'!') || bytes.get(start + 1) != Some(&b'[') {
        return None;
    }
    let alt_start = start + 2;
    let mut close = alt_start;
    while bytes.get(close).is_some_and(|b| *b != b']') {
        close += 1;
    }
    if bytes.get(close) != Some(&b']') || bytes.get(close + 1) != Some(&b'(') {
        return None;
    }
    let mut end = close + 2;
    while bytes.get(end).is_some_and(|b| *b != b')') {
        end += 1;
    }
    if bytes.get(end) != Some(&b')') {
        return None;
    }
    Some((end + 1, alt_start, close))
}
