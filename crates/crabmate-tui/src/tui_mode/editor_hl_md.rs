//! Markdown 的行级词法（`editor_hl` 的特例之二，issue #510）。
//!
//! **它不是 token 级的**：Markdown 的语义单位是行与区间（标题行、围栏块、引用、列表标记、
//! 行内代码、链接目标），套通用 token 扫描器只会把 `#` 当注释、把 `*` 当运算符。
//!
//! 另外提醒一句使用场景：`.md` 默认走 `EditorMode::Markdown` 预览（已有语义渲染），
//! 这份词法只在 `/editor --src README.md` 那条路径上生效。

use super::{HlSpan, LexState, SyntaxCat};

pub(super) fn classify(chars: &[char], start: LexState) -> (Vec<HlSpan>, LexState) {
    // 围栏内：整行算注释色（它是「代码块」的视觉标记，不是注释语义），收尾行另算。
    if let LexState::MdFence(marker, len) = start {
        return inside_fence(chars, marker, len);
    }
    let mut out: Vec<HlSpan> = Vec::new();
    let indent = chars.iter().take_while(|c| c.is_whitespace()).count();
    let rest = &chars[indent..];
    if let Some((marker, len)) = fence_open(rest) {
        out.push(span(indent, chars.len(), SyntaxCat::Keyword));
        return (out, LexState::MdFence(marker, len));
    }
    if is_heading(rest) {
        out.push(span(indent, chars.len(), SyntaxCat::Keyword));
        return (out, LexState::Normal);
    }
    if rest.first() == Some(&'>') {
        out.push(span(indent, chars.len(), SyntaxCat::Comment));
        return (out, LexState::Normal);
    }
    if let Some(end) = list_marker_end(rest) {
        out.push(span(indent, indent + end, SyntaxCat::Keyword));
    }
    inline(chars, indent, &mut out);
    (out, LexState::Normal)
}

fn inside_fence(chars: &[char], marker: char, len: usize) -> (Vec<HlSpan>, LexState) {
    let mut out = Vec::new();
    let indent = chars.iter().take_while(|c| c.is_whitespace()).count();
    let rest = &chars[indent..];
    // 收尾围栏：长度 **≥ 开围栏** 的一段连续同类 marker，且其后只有空白。
    //
    // 两个条件缺一不可：少了「≥ 长度」，短收尾会被当内容；少了「其后只有空白」，
    // 围栏里贴一行 ```rust（讲 markdown 语法的文档很常见）会被当收尾，后面整块错位。
    let run = rest.iter().take_while(|c| **c == marker).count();
    let closing = run >= len && rest[run..].iter().all(|c| c.is_whitespace());
    if closing {
        // 收尾行画成关键字色：与开围栏行对称，读起来才知道「这一块到这儿结束」。
        out.push(span(indent, chars.len(), SyntaxCat::Keyword));
        return (out, LexState::Normal);
    }
    out.push(span(0, chars.len(), SyntaxCat::Comment));
    (out, LexState::MdFence(marker, len))
}

/// 行内代码与链接目标：`\`code\`` 与 `[text](url)` 的 `url`。
fn inline(chars: &[char], from: usize, out: &mut Vec<HlSpan>) {
    let mut i = from;
    while i < chars.len() {
        match chars[i] {
            '`' => {
                let end = chars[i + 1..]
                    .iter()
                    .position(|c| *c == '`')
                    .map_or(chars.len(), |p| i + 1 + p + 1);
                out.push(span(i, end, SyntaxCat::Str));
                i = end;
            }
            '(' if i > 0 && chars[i - 1] == ']' => {
                let end = chars[i..]
                    .iter()
                    .position(|c| *c == ')')
                    .map_or(chars.len(), |p| i + p + 1);
                out.push(span(i, end, SyntaxCat::Str));
                i = end;
            }
            _ => i += 1,
        }
    }
}

/// 开围栏：` ``` ` 或 `~~~`（≥3 个，允许尾随语言标签）。返回 (记号, 长度)。
fn fence_open(rest: &[char]) -> Option<(char, usize)> {
    let marker = *rest.first()?;
    if marker != '`' && marker != '~' {
        return None;
    }
    let len = rest.iter().take_while(|c| **c == marker).count();
    (len >= 3).then_some((marker, len))
}

/// `#` 起首且后跟空白或行尾（`#hashtag` 不是标题）。
fn is_heading(rest: &[char]) -> bool {
    let hashes = rest.iter().take_while(|c| **c == '#').count();
    if hashes == 0 || hashes > 6 {
        return false;
    }
    rest.get(hashes).is_none_or(|c| c.is_whitespace())
}

/// 列表标记：`- ` / `* ` / `+ ` / `1. ` / `1) `；返回标记结束位置（相对 `rest`）。
fn list_marker_end(rest: &[char]) -> Option<usize> {
    match rest.first()? {
        '-' | '*' | '+' => {
            let end = if rest.get(1).is_some_and(|c| c.is_whitespace()) {
                2
            } else {
                return None;
            };
            Some(end)
        }
        ch if ch.is_ascii_digit() => {
            let digits = rest.iter().take_while(|c| c.is_ascii_digit()).count();
            let punct = *rest.get(digits)?;
            let spaced = rest.get(digits + 1).is_some_and(|c| c.is_whitespace());
            ((punct == '.' || punct == ')') && spaced).then_some(digits + 2)
        }
        _ => None,
    }
}

fn span(start: usize, end: usize, cat: SyntaxCat) -> HlSpan {
    HlSpan { start, end, cat }
}
