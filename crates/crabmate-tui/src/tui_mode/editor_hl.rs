//! 源码视图的词法分类器（issue #510 的 P1：**纯逻辑，不碰渲染**）。
//!
//! 它回答一个问题：这一行的哪些字符属于注释 / 字符串 / 数字 / 关键字 / 类型 / 函数。
//! 颜色怎么画不在这里 —— 那是 `Palette::syntax_style` 与 `source_body` 的事。
//!
//! 为什么不用 syntect / tree-sitter：这是**只读浏览视图**，不是 IDE。手写词法零依赖、
//! 体积零增长、与 `Palette` 天然一致；代价是只认词法不认语义（见 issue 的非目标）。
//!
//! # 两个约定
//!
//! 1. **区间单位是 char，不是字节**。渲染层按显示宽度裁切，按字节切会把宽字符切坏。
//! 2. **认不出就不上色**。词法器返回的区间可以少，绝不能错位 —— 错位的颜色比没有颜色
//!    更难查（`r#"…"#` 认错会让后面整段都变字符串色）。
//!
//! # 跨行状态
//!
//! 块注释、Rust raw string、三引号字符串都是跨行的，所以接口是
//! `classify_line(line, start_state) -> (spans, end_state)`：调用方先扫一遍求出每行的
//! 起始态（`scan_states`），渲染时只对可见行着色。逐行独立着色在这里是错的。

// issue #510 拆成两步：P1 只交付纯逻辑，P2 把它接进 `source_body`（已完成）。

// 两个语言特例（TOML 的键名 / Markdown 的行级语义）各自一份文件：它们的扫描形状与通用
// token 扫描不同，混进本文件只会长出一堆 `if lang == …`。路径相对本文件所在目录。
#[path = "editor_hl_md.rs"]
mod editor_hl_md;
#[path = "editor_hl_toml.rs"]
mod editor_hl_toml;

/// 支持的语言（issue #510 决策 0：只这 6 种，其余回正文色）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Language {
    Rust,
    Cpp,
    Python,
    Json,
    Toml,
    Markdown,
}

/// 语法类别。没有类别就是正文色 —— 刻意不设 `Other`，让「不上色」是默认值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SyntaxCat {
    Comment,
    Str,
    Number,
    Keyword,
    Type,
    Function,
}

/// 一个高亮区间：`[start, end)`，单位是 **char**。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HlSpan {
    pub start: usize,
    pub end: usize,
    pub cat: SyntaxCat,
}

/// 行末词法态：只记「下一行从哪开始」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum LexState {
    #[default]
    Normal,
    /// `/* … */`；参数是嵌套深度（只有 Rust 会 > 1）。
    BlockComment(u32),
    /// Rust raw string `r#"…"#`：参数是 `#` 的个数（0 = `r"…"`）。
    RawString(u8),
    /// 三引号字符串（Python `"""` / `'''`，TOML 同）。
    Triple(char),
    /// Markdown 围栏：记号字符与长度（收尾要同长）。
    MdFence(char, usize),
}

/// 路径 → 语言；不认识的扩展名返回 `None`（回正文色，不猜）。
#[must_use]
pub(crate) fn language_for_path(rel: &str) -> Option<Language> {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    let ext = name
        .rsplit_once('.')
        .map_or("", |(_, tail)| tail)
        .to_ascii_lowercase();
    match ext.as_str() {
        "rs" => Some(Language::Rust),
        "cpp" | "cc" | "cxx" | "hpp" | "hxx" | "hh" | "c" | "h" => Some(Language::Cpp),
        "py" | "pyi" => Some(Language::Python),
        "json" | "jsonc" => Some(Language::Json),
        "toml" => Some(Language::Toml),
        "md" | "markdown" => Some(Language::Markdown),
        _ => None,
    }
}

/// 扫描全文，求**每行的起始词法态**（长度 = `lines.len() + 1`，末项是收尾态）。
///
/// 这是「两阶段」的第一阶段：只跑状态转移、不建 span，所以即使 2 万行也很便宜。
/// 第二阶段（渲染）拿着 `states[i]` 只处理视口内的行。
#[must_use]
pub(crate) fn scan_states(lines: &[String], lang: Language) -> Vec<LexState> {
    let mut states = Vec::with_capacity(lines.len() + 1);
    let mut state = LexState::Normal;
    for line in lines {
        states.push(state);
        state = classify_line(line, state, lang).1;
    }
    states.push(state);
    states
}

/// 分类一行，返回 `(区间, 行末态)`。
///
/// 缩进与空白永远不进区间；未闭合的跨行记号不产出区间（下一行继续，见 `LexState`）。
#[must_use]
pub(crate) fn classify_line(
    line: &str,
    start: LexState,
    lang: Language,
) -> (Vec<HlSpan>, LexState) {
    let chars: Vec<char> = line.chars().collect();
    match lang {
        Language::Markdown => editor_hl_md::classify(&chars, start),
        Language::Toml => editor_hl_toml::classify(&chars, start),
        Language::Rust => classify_code(&chars, start, &RUST),
        Language::Cpp => classify_code(&chars, start, &CPP),
        Language::Python => classify_code(&chars, start, &PYTHON),
        Language::Json => classify_code(&chars, start, &JSON),
    }
}

/// 一种语言的词法差异（注释记号、字符串引号、关键字表）。
///
/// 差异全部放表里，扫描逻辑只有一份 —— 六种语言各写一个扫描器的后果是同一个 bug 要修
/// 六遍，而它们真正的差别其实只在记号与词表。
struct Spec {
    line_comments: &'static [&'static str],
    block_comment: Option<(&'static str, &'static str)>,
    /// 块注释可嵌套（只有 Rust）。
    nested_block: bool,
    /// 普通字符串的引号（可含反斜杠转义）。
    quotes: &'static [char],
    /// 三引号引号（Python / TOML）。
    triple: &'static [char],
    /// 认 Rust 的 `r"…"` / `r#"…"#`。
    raw_string: bool,
    /// 行首 `#` = 整行预处理指令（C++）。
    hash_preprocessor: bool,
    keywords: &'static [&'static str],
    types: &'static [&'static str],
    /// `UpperCamel` 当类型名（Rust / C++ / Python）。
    upper_camel_type: bool,
    /// `name!` 当宏（Rust）。
    macro_bang: bool,
}

impl Spec {
    /// 是不是行注释记号；多个记号时**最长的那个胜**。
    ///
    /// 按最长匹配是实现出来的保证，不是注释里的口号：将来某语言同时有 `--` 与 `-`
    /// 时，短的先命中会把 `--` 拆成两个减号。
    fn line_comment_at(&self, chars: &[char], i: usize) -> Option<usize> {
        self.line_comments
            .iter()
            .filter(|m| matches_at(chars, i, m))
            .map(|m| m.chars().count())
            .max()
    }
}

const RUST: Spec = Spec {
    line_comments: &["//"],
    block_comment: Some(("/*", "*/")),
    nested_block: true,
    quotes: &['"'],
    triple: &[],
    raw_string: true,
    hash_preprocessor: false,
    keywords: &[
        "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
        "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move",
        "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait",
        "true", "type", "unsafe", "use", "where", "while", "yield",
    ],
    types: &[
        "bool", "char", "f32", "f64", "i8", "i16", "i32", "i64", "i128", "isize", "str", "u8",
        "u16", "u32", "u64", "u128", "usize",
    ],
    upper_camel_type: true,
    macro_bang: true,
};

const CPP: Spec = Spec {
    line_comments: &["//"],
    block_comment: Some(("/*", "*/")),
    nested_block: false,
    quotes: &['"', '\''],
    triple: &[],
    raw_string: false,
    hash_preprocessor: true,
    keywords: &[
        "alignas",
        "auto",
        "break",
        "case",
        "catch",
        "class",
        "const",
        "constexpr",
        "continue",
        "default",
        "delete",
        "do",
        "else",
        "enum",
        "explicit",
        "export",
        "extern",
        "false",
        "for",
        "friend",
        "goto",
        "if",
        "inline",
        "namespace",
        "new",
        "noexcept",
        "nullptr",
        "operator",
        "private",
        "protected",
        "public",
        "return",
        "sizeof",
        "static",
        "struct",
        "switch",
        "template",
        "this",
        "throw",
        "true",
        "try",
        "typedef",
        "typename",
        "union",
        "using",
        "virtual",
        "void",
        "volatile",
        "while",
    ],
    types: &[
        "bool", "char", "char16_t", "char32_t", "double", "float", "int", "long", "short",
        "signed", "size_t", "ssize_t", "string", "uint8_t", "uint16_t", "uint32_t", "uint64_t",
        "int8_t", "int16_t", "int32_t", "int64_t", "unsigned", "wchar_t",
    ],
    upper_camel_type: true,
    macro_bang: false,
};

const PYTHON: Spec = Spec {
    line_comments: &["#"],
    block_comment: None,
    nested_block: false,
    quotes: &['"', '\''],
    triple: &['"', '\''],
    raw_string: false,
    hash_preprocessor: false,
    keywords: &[
        "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del",
        "elif", "else", "except", "False", "finally", "for", "from", "global", "if", "import",
        "in", "is", "lambda", "None", "nonlocal", "not", "or", "pass", "raise", "return", "True",
        "try", "while", "with", "yield",
    ],
    types: &[
        "bool",
        "bytes",
        "dict",
        "float",
        "frozenset",
        "int",
        "list",
        "object",
        "set",
        "str",
        "tuple",
        "type",
    ],
    upper_camel_type: true,
    macro_bang: false,
};

const JSON: Spec = Spec {
    line_comments: &[],
    block_comment: None,
    nested_block: false,
    quotes: &['"'],
    triple: &[],
    raw_string: false,
    hash_preprocessor: false,
    keywords: &["true", "false", "null"],
    types: &[],
    upper_camel_type: false,
    macro_bang: false,
};

/// 通用扫描（Rust / C++ / Python / JSON）。
fn classify_code(chars: &[char], start: LexState, spec: &Spec) -> (Vec<HlSpan>, LexState) {
    let mut out: Vec<HlSpan> = Vec::new();
    let mut state = start;
    let mut i = 0usize;
    // 行首预处理指令（C++）：整行一个区间，扫完即可。
    if spec.hash_preprocessor
        && let Some(start) = first_non_space(chars).filter(|p| chars[*p] == '#')
    {
        out.push(HlSpan {
            start,
            end: chars.len(),
            cat: SyntaxCat::Keyword,
        });
        return (out, LexState::Normal);
    }
    while i < chars.len() {
        let Some((end, cat, next)) = lex_one(chars, i, state, spec) else {
            i += 1;
            continue;
        };
        // 防御：任何子扫描器返回「没前进」都不许把主循环挂死。
        if end <= i {
            i += 1;
            state = next;
            continue;
        }
        if let Some(cat) = cat {
            out.push(HlSpan { start: i, end, cat });
        }
        i = end;
        state = next;
    }
    (out, state)
}

/// 尝试在 `i` 处识别一个记号；识别不了返回 `None`（调用方前进一个字符）。
fn lex_one(
    chars: &[char],
    i: usize,
    state: LexState,
    spec: &Spec,
) -> Option<(usize, Option<SyntaxCat>, LexState)> {
    if let Some(found) = resume_multiline(chars, i, state, spec) {
        return Some(found);
    }
    if let Some(found) = lex_line_comment(chars, i, spec) {
        return Some(found);
    }
    if let Some(found) = lex_block_comment(chars, i, spec) {
        return Some(found);
    }
    if let Some(found) = lex_string(chars, i, spec) {
        return Some(found);
    }
    if let Some(found) = lex_raw_string(chars, i, spec) {
        return Some(found);
    }
    if let Some(found) = lex_number(chars, i) {
        return Some(found);
    }
    lex_ident(chars, i, spec)
}

/// 续上上一行留下的跨行态。
///
/// 这是它存在的唯一理由：块注释 / raw string / 三引号都不是「一行内闭合」的，逐行独立
/// 扫描会把 `/*` 之后的整段误判成代码（或反之）。
fn resume_multiline(
    chars: &[char],
    i: usize,
    state: LexState,
    spec: &Spec,
) -> Option<(usize, Option<SyntaxCat>, LexState)> {
    if i != 0 {
        return None;
    }
    match state {
        LexState::Normal | LexState::MdFence(..) => None,
        LexState::BlockComment(depth) => {
            let (_, close) = spec.block_comment?;
            // 以行首的剩余深度起扫：上一行开了两层、只闭了一层时，`depth` 就是 1，
            // 这一行的第一个 `*/` 正好把它闭掉。
            let (end, remaining) = scan_until(
                chars,
                0,
                depth,
                close,
                if spec.nested_block { "/*" } else { "" },
            );
            let next = match remaining {
                0 => LexState::Normal,
                depth => LexState::BlockComment(depth),
            };
            Some((end, Some(SyntaxCat::Comment), next))
        }
        LexState::RawString(hashes) => {
            let (end, closed) = scan_raw_end(chars, 0, hashes);
            let next = if closed {
                LexState::Normal
            } else {
                LexState::RawString(hashes)
            };
            Some((end, Some(SyntaxCat::Str), next))
        }
        LexState::Triple(quote) => {
            let (end, closed) = scan_triple_end(chars, 0, quote);
            let next = if closed {
                LexState::Normal
            } else {
                LexState::Triple(quote)
            };
            Some((end, Some(SyntaxCat::Str), next))
        }
    }
}

fn lex_line_comment(
    chars: &[char],
    i: usize,
    spec: &Spec,
) -> Option<(usize, Option<SyntaxCat>, LexState)> {
    // 行注释一直吃到行尾，所以结束位置就是行宽。
    spec.line_comment_at(chars, i)?;
    Some((chars.len(), Some(SyntaxCat::Comment), LexState::Normal))
}

fn lex_block_comment(
    chars: &[char],
    i: usize,
    spec: &Spec,
) -> Option<(usize, Option<SyntaxCat>, LexState)> {
    let (open, close) = spec.block_comment?;
    if !matches_at(chars, i, open) {
        return None;
    }
    let start = i + open.chars().count();
    let (end, remaining) = scan_until(
        chars,
        start,
        1,
        close,
        if spec.nested_block { open } else { "" },
    );
    let next = match remaining {
        0 => LexState::Normal,
        depth => LexState::BlockComment(depth),
    };
    Some((end, Some(SyntaxCat::Comment), next))
}

fn lex_string(
    chars: &[char],
    i: usize,
    spec: &Spec,
) -> Option<(usize, Option<SyntaxCat>, LexState)> {
    let quote = *chars.get(i)?;
    if !spec.quotes.contains(&quote) {
        return None;
    }
    // 三引号（Python / TOML）先判，否则 `"""` 会被当成空字符串 + 又一个字符串。
    if spec.triple.contains(&quote) && matches_at(chars, i, &quote.to_string().repeat(3)) {
        let (end, closed) = scan_triple_end(chars, i + 3, quote);
        let next = if closed {
            LexState::Normal
        } else {
            LexState::Triple(quote)
        };
        return Some((end, Some(SyntaxCat::Str), next));
    }
    let (end, closed) = scan_until_escaped(chars, i + 1, quote);
    // 未闭合的单引号字符串：不上色（下一行不会续），比把余下行染成字符串安全。
    if !closed {
        return Some((end, None, LexState::Normal));
    }
    Some((end, Some(SyntaxCat::Str), LexState::Normal))
}

fn lex_raw_string(
    chars: &[char],
    i: usize,
    spec: &Spec,
) -> Option<(usize, Option<SyntaxCat>, LexState)> {
    if !spec.raw_string {
        return None;
    }
    // `r"…"` / `r#"…"#` / `br#"…"#`
    let (start, hashes) = raw_string_open(chars, i)?;
    let (end, closed) = scan_raw_end(chars, start, hashes);
    let next = if closed {
        LexState::Normal
    } else {
        LexState::RawString(hashes)
    };
    Some((end, Some(SyntaxCat::Str), next))
}

fn lex_number(chars: &[char], i: usize) -> Option<(usize, Option<SyntaxCat>, LexState)> {
    number_start(chars, i)?;
    Some((
        scan_number_end(chars, i),
        Some(SyntaxCat::Number),
        LexState::Normal,
    ))
}

/// 数字起点：当前字符是数字，且前面不是标识符字符（`abc1` 整体是标识符）。
fn number_start(chars: &[char], i: usize) -> Option<()> {
    if !chars.get(i)?.is_ascii_digit() {
        return None;
    }
    if i > 0 && is_ident_char(chars[i - 1]) {
        return None;
    }
    Some(())
}

/// 数字结束：吃掉标识符字符、`1.0` 的小数点（点后必须还有数字）、C++ 的 `1'000` 分隔符。
fn scan_number_end(chars: &[char], i: usize) -> usize {
    let mut j = i;
    while j < chars.len() {
        let ch = chars[j];
        if ch == '.' && !chars.get(j + 1).is_some_and(|c| c.is_ascii_digit()) {
            break;
        }
        if !(is_ident_char(ch) || ch == '.' || ch == '\'') {
            break;
        }
        j += 1;
    }
    j
}

fn lex_ident(
    chars: &[char],
    i: usize,
    spec: &Spec,
) -> Option<(usize, Option<SyntaxCat>, LexState)> {
    if !is_ident_start(*chars.get(i)?) {
        return None;
    }
    let mut j = i;
    while j < chars.len() && is_ident_char(chars[j]) {
        j += 1;
    }
    let word: String = chars[i..j].iter().collect();
    let (end, cat) = ident_cat(chars, j, &word, spec)?;
    Some((end, cat, LexState::Normal))
}

/// 标识符 → (结束位置, 类别)；认不出返回 `None`（跳过但不建区间）。
///
/// 抽出来是为了让 `lex_ident` 的 CCN 留在门禁内：关键字 / 类型 / 首字母大写 / 调用 /
/// 宏五条规则各有分支。
fn ident_cat(
    chars: &[char],
    end: usize,
    word: &str,
    spec: &Spec,
) -> Option<(usize, Option<SyntaxCat>)> {
    if spec.keywords.contains(&word) {
        return Some((end, Some(SyntaxCat::Keyword)));
    }
    if spec.types.contains(&word) {
        return Some((end, Some(SyntaxCat::Type)));
    }
    if spec.upper_camel_type && is_upper_camel(word) {
        return Some((end, Some(SyntaxCat::Type)));
    }
    // `name!` 是宏（`!=` 不算）。
    if spec.macro_bang && chars.get(end) == Some(&'!') && chars.get(end + 1) != Some(&'=') {
        return Some((end + 1, Some(SyntaxCat::Keyword)));
    }
    if chars.get(end) == Some(&'(') {
        return Some((end, Some(SyntaxCat::Function)));
    }
    Some((end, None))
}

/// 从 `i` 起扫到 `close`，可选地同时跟踪 `nested_open` 做深度计数。
///
/// 返回 `(结束位置, 剩余深度)`：**0 = 已闭合**，`> 0` = 还开着这么多层。
/// 返回剩余深度而不是 `bool`，是因为 Rust 的块注释可嵌套：一行里连开两层却没闭合时，
/// 「下一行还欠几个 `*/`」是这个词法器唯一需要跨行记住的东西。写成 `bool` 的后果是
/// 第二行遇到第一个 `*/` 就当闭合，把剩下的注释当代码 —— 而嵌套注释最常见的用处正是
/// 「把一段含注释的代码整块注释掉」。
///
/// 结束位置**含**收尾记号（区间要把它一起染色）。`start_depth` 必须是 ≥ 1 的当前深度。
fn scan_until(
    chars: &[char],
    i: usize,
    start_depth: u32,
    close: &str,
    nested_open: &str,
) -> (usize, u32) {
    let mut j = i;
    let mut depth = start_depth.max(1);
    let track = !nested_open.is_empty();
    while j < chars.len() {
        if track && matches_at(chars, j, nested_open) {
            depth += 1;
            j += nested_open.chars().count();
            continue;
        }
        if matches_at(chars, j, close) {
            depth -= 1;
            j += close.chars().count();
            if depth == 0 {
                return (j, 0);
            }
            continue;
        }
        j += 1;
    }
    (chars.len(), depth)
}

/// 单行内按转义规则找引号（`\"` 不算闭合）。
fn scan_until_escaped(chars: &[char], i: usize, quote: char) -> (usize, bool) {
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

/// 三引号收尾（`"""` / `'''`），返回 (结束位置, 是否闭合)。
fn scan_triple_end(chars: &[char], i: usize, quote: char) -> (usize, bool) {
    let triple: Vec<char> = std::iter::repeat_n(quote, 3).collect();
    let mut j = i;
    while j < chars.len() {
        if chars[j..].starts_with(&triple) {
            return (j + 3, true);
        }
        j += 1;
    }
    (chars.len(), false)
}

/// Rust raw string 的开头：返回 (正文起点, `#` 个数)。
fn raw_string_open(chars: &[char], i: usize) -> Option<(usize, u8)> {
    let mut j = i;
    if chars.get(j) == Some(&'b') {
        j += 1;
    }
    if chars.get(j) != Some(&'r') {
        return None;
    }
    j += 1;
    let mut hashes = 0u8;
    while chars.get(j) == Some(&'#') {
        hashes += 1;
        j += 1;
    }
    if chars.get(j) != Some(&'"') {
        return None;
    }
    Some((j + 1, hashes))
}

/// raw string 的收尾：`"` + 同样个数的 `#`。
fn scan_raw_end(chars: &[char], i: usize, hashes: u8) -> (usize, bool) {
    let mut j = i;
    while j < chars.len() {
        if chars[j] == '"' {
            let mut k = j + 1;
            let mut seen = 0u8;
            while seen < hashes && chars.get(k) == Some(&'#') {
                seen += 1;
                k += 1;
            }
            if seen == hashes {
                return (k, true);
            }
        }
        j += 1;
    }
    (chars.len(), false)
}

fn matches_at(chars: &[char], i: usize, pat: &str) -> bool {
    pat.chars()
        .enumerate()
        .all(|(offset, p)| chars.get(i + offset) == Some(&p))
}

fn first_non_space(chars: &[char]) -> Option<usize> {
    chars.iter().position(|c| !c.is_whitespace())
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `UpperCamel`（首字母大写且含小写字母）：`String` / `HashMap` 是类型，`ABC` 常量不是。
fn is_upper_camel(word: &str) -> bool {
    let mut chars = word.chars();
    chars.next().is_some_and(char::is_uppercase) && word.chars().any(char::is_lowercase)
}

#[cfg(test)]
#[path = "editor_hl_tests.rs"]
mod tests;
