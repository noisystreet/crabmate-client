//! TOML 的行级词法（`editor_hl` 的特例之一，issue #510）。
//!
//! 为什么不走通用扫描器：TOML 的**键名**要单独上色（`=` 之前那段），表头 `[table]` 是整段
//! 结构而不是标识符，多行字符串又跨行 —— 这三件事在「通用 token 扫描」里没有对应位置，
//! 硬塞进去只会让通用扫描器长出一堆 `if lang == Toml`。
//!
//! 保守原则与通用扫描器一致：认不出的保持正文色。

use super::{HlSpan, LexState, SyntaxCat};

/// `true` / `false` 是 TOML 的唯一关键字。
const KEYWORDS: &[&str] = &["true", "false"];

pub(super) fn classify(chars: &[char], start: LexState) -> (Vec<HlSpan>, LexState) {
    // 多行字符串的续行：整行都在字符串里，只需找收尾。
    if let LexState::Triple(quote) = start {
        return resume_triple(chars, quote, Vec::new());
    }
    let mut out: Vec<HlSpan> = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        let Some((end, cat, next)) = lex_one(chars, i, out.is_empty()) else {
            i += 1;
            continue;
        };
        // 防御：任何子扫描器返回「没前进」都不许把主循环挂死。
        if end <= i {
            i += 1;
            continue;
        }
        if let Some(cat) = cat {
            out.push(span(i, end, cat));
        }
        i = end;
        if next != LexState::Normal {
            return (out, next);
        }
    }
    (out, LexState::Normal)
}

/// 尝试在 `i` 处识别一个 TOML 记号；`first` = 这一行还没有任何区间（表头必须在行首）。
fn lex_one(chars: &[char], i: usize, first: bool) -> Option<(usize, Option<SyntaxCat>, LexState)> {
    if let Some(found) = lex_comment(chars, i) {
        return Some(found);
    }
    if let Some(found) = lex_string(chars, i) {
        return Some(found);
    }
    if first && let Some(found) = lex_table_header(chars, i) {
        return Some(found);
    }
    if let Some(found) = lex_number(chars, i) {
        return Some(found);
    }
    lex_ident(chars, i)
}

fn lex_comment(chars: &[char], i: usize) -> Option<(usize, Option<SyntaxCat>, LexState)> {
    if chars.get(i) != Some(&'#') {
        return None;
    }
    Some((chars.len(), Some(SyntaxCat::Comment), LexState::Normal))
}

fn lex_string(chars: &[char], i: usize) -> Option<(usize, Option<SyntaxCat>, LexState)> {
    let quote = *chars.get(i)?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    if at_triple(chars, i) {
        let (end, closed) = scan_triple(chars, i + 3, quote);
        let next = if closed {
            LexState::Normal
        } else {
            LexState::Triple(quote)
        };
        return Some((end, Some(SyntaxCat::Str), next));
    }
    let (end, closed) = scan_quoted(chars, i + 1, quote);
    // 单行字符串没闭合：不上色（TOML 的单行字符串不跨行，画一半更像 bug）。
    Some((end, closed.then_some(SyntaxCat::Str), LexState::Normal))
}

fn lex_table_header(chars: &[char], i: usize) -> Option<(usize, Option<SyntaxCat>, LexState)> {
    if chars.get(i) != Some(&'[') || !is_blank_before(chars, i) {
        return None;
    }
    Some((
        scan_table_header(chars, i),
        Some(SyntaxCat::Keyword),
        LexState::Normal,
    ))
}

fn lex_number(chars: &[char], i: usize) -> Option<(usize, Option<SyntaxCat>, LexState)> {
    if !chars.get(i)?.is_ascii_digit() {
        return None;
    }
    Some((
        scan_number(chars, i),
        Some(SyntaxCat::Number),
        LexState::Normal,
    ))
}

/// 标识符：关键字（`true`/`false`）或键名（后面跟着 `=`）。
fn lex_ident(chars: &[char], i: usize) -> Option<(usize, Option<SyntaxCat>, LexState)> {
    if !is_ident_start(*chars.get(i)?) {
        return None;
    }
    let end = scan_ident(chars, i);
    let word: String = chars[i..end].iter().collect();
    if KEYWORDS.contains(&word.as_str()) {
        return Some((end, Some(SyntaxCat::Keyword), LexState::Normal));
    }
    // `a.b = 1` 里只有 `b` 命中（`a` 后面是 `.`）。
    if follows_equals(chars, end) {
        return Some((end, Some(SyntaxCat::Type), LexState::Normal));
    }
    Some((end, None, LexState::Normal))
}

fn resume_triple(chars: &[char], quote: char, mut out: Vec<HlSpan>) -> (Vec<HlSpan>, LexState) {
    let (end, closed) = scan_triple(chars, 0, quote);
    out.push(span(0, end, SyntaxCat::Str));
    let next = if closed {
        LexState::Normal
    } else {
        LexState::Triple(quote)
    };
    (out, next)
}

fn span(start: usize, end: usize, cat: SyntaxCat) -> HlSpan {
    HlSpan { start, end, cat }
}

/// `i` 处是否是 `"""` / `'''`。
fn at_triple(chars: &[char], i: usize) -> bool {
    let Some(&quote) = chars.get(i) else {
        return false;
    };
    chars.get(i + 1) == Some(&quote) && chars.get(i + 2) == Some(&quote)
}

/// 三引号收尾，返回 (结束位置, 是否闭合)。
fn scan_triple(chars: &[char], i: usize, quote: char) -> (usize, bool) {
    let mut j = i;
    while j < chars.len() {
        if at_triple(chars, j) && chars[j] == quote {
            return (j + 3, true);
        }
        j += 1;
    }
    (chars.len(), false)
}

/// 单行引号字符串（`"…"` / `'…'`，可含 `\"` 转义）。
fn scan_quoted(chars: &[char], i: usize, quote: char) -> (usize, bool) {
    let mut j = i;
    while j < chars.len() {
        match chars[j] {
            '\\' => j += 2,
            ch if ch == quote => return (j + 1, true),
            _ => j += 1,
        }
    }
    (chars.len(), false)
}

/// `[table]` / `[[array]]` 的结束位置（含收尾 `]`）；没闭合就到行尾。
fn scan_table_header(chars: &[char], i: usize) -> usize {
    let double = chars.get(i + 1) == Some(&'[');
    let mut j = i + if double { 2 } else { 1 };
    while j < chars.len() {
        if chars[j] == ']' {
            return if double && chars.get(j + 1) == Some(&']') {
                j + 2
            } else {
                j + 1
            };
        }
        j += 1;
    }
    chars.len()
}

/// 数字与日期时间：数字开头就一直吃到非 `[0-9A-Za-z_:.+-]`（`2024-01-01T00:00:00Z` 一并）。
fn scan_number(chars: &[char], i: usize) -> usize {
    let mut j = i;
    while j < chars.len() {
        let ch = chars[j];
        let keep = ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | ':' | '+' | '-');
        if !keep {
            break;
        }
        j += 1;
    }
    j
}

fn scan_ident(chars: &[char], i: usize) -> usize {
    let mut j = i;
    while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_' || chars[j] == '-') {
        j += 1;
    }
    j
}

/// `i` 之前只有空白（表头必须在行首缩进之后）。
fn is_blank_before(chars: &[char], i: usize) -> bool {
    chars[..i].iter().all(|c| c.is_whitespace())
}

/// `i` 之后跳过空白就是 `=`（键名的判定）。
fn follows_equals(chars: &[char], i: usize) -> bool {
    let mut j = i;
    while j < chars.len() && chars[j].is_whitespace() {
        j += 1;
    }
    chars.get(j) == Some(&'=')
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}
