//! markdown 行内字符流里的**链接**：URL 跟着字符流走，折行后换算成屏幕列区间。
//!
//! 单独成文件：`md.rs` 贴着 920 行文件门禁，而这块自成一套（字符容器 + 列换算），
//! 与「markdown 怎么解析」关系松散。
//!
//! 【为什么 URL 必须跟着字符流走】落到 `Style` 上的只有一个下划线修饰位：
//! `[文档](https://x)` 渲染完只剩「文档」两个字，地址在这一层之后再也取不回来 ——
//! 终端认不出超链接（屏幕上没有 URL 字面量），点开也不知道该开哪条。
//!
//! 【为什么列区间必须在折行层算】一个长 URL 标签会被折成两行，屏幕上因此分成两块；
//! 「哪一列属于链接」必须与「哪一列真的画出了这些字」出自同一次计算。放到鼠标侧
//! 重算一遍折行，就是「点到的列与看到的列错开」的唯一来源。

use std::ops::{Deref, DerefMut};

use ratatui::style::{Modifier, Style};
use unicode_width::UnicodeWidthChar;

use super::Palette;

/// 一条链接在字符流里的下标区间（左闭右开）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LinkSeg {
    pub start: usize,
    pub end: usize,
    pub url: String,
}

/// 行内字符流 + 其中链接的字符区间。
///
/// `Deref` 到字符向量是为了让既有的 `out.push` / `out.extend` 写法原样保留：链接只是
/// **额外**记一层，字符流本身不变。
#[derive(Debug, Default, Clone)]
pub(crate) struct PartChars {
    chars: Vec<(Style, char)>,
    pub(crate) links: Vec<LinkSeg>,
}

impl Deref for PartChars {
    type Target = Vec<(Style, char)>;

    fn deref(&self) -> &Self::Target {
        &self.chars
    }
}

impl DerefMut for PartChars {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.chars
    }
}

impl From<Vec<(Style, char)>> for PartChars {
    fn from(chars: Vec<(Style, char)>) -> Self {
        Self {
            chars,
            links: Vec::new(),
        }
    }
}

impl PartChars {
    /// 追加一段链接文字，并记下它在字符流里的区间。
    pub(super) fn push_link(&mut self, st: Style, text: &str, url: &str) {
        if text.is_empty() {
            return;
        }
        let start = self.chars.len();
        self.chars.extend(text.chars().map(|c| (st, c)));
        self.links.push(LinkSeg {
            start,
            end: self.chars.len(),
            url: url.to_string(),
        });
    }

    /// 拼接另一段字符流，连带把它的链接区间平移到当前坐标系。
    pub(super) fn absorb(&mut self, other: Self) {
        let base = self.chars.len();
        self.links.extend(other.links.into_iter().map(|l| LinkSeg {
            start: l.start + base,
            end: l.end + base,
            url: l.url,
        }));
        self.chars.extend(other.chars);
    }

    /// 叠加一层基样式（不改字符数，链接区间原样保留）。
    #[must_use]
    pub(super) fn patched(mut self, base: Style) -> Self {
        for cell in &mut self.chars {
            cell.0 = cell.0.patch(base);
        }
        self
    }
}

/// 按「正文字符下标 → 输出下标」把链接区间搬到输出坐标系。
///
/// `pos_map` 由 `wrap_body` 边写边记；区间端点都落在真实字符上（[`PartChars::push_link`]
/// 只收非空文本），所以查不到即视为「该段被裁掉了」，直接丢弃。
pub(crate) fn remap_links(links: &[LinkSeg], pos_map: &[usize]) -> Vec<LinkSeg> {
    links
        .iter()
        .filter_map(|l| {
            let start = *pos_map.get(l.start)?;
            let end = pos_map.get(l.end.checked_sub(1)?)? + 1;
            Some(LinkSeg {
                start,
                end,
                url: l.url.clone(),
            })
        })
        .collect()
}

/// 一条物理行：字符 + 该行上的链接列区间。
#[derive(Debug, Default, Clone)]
pub(crate) struct WrappedRow {
    pub chars: Vec<(Style, char)>,
    /// `(起始列, 结束列, url)`，左闭右开、列相对行首。一个链接跨行时每行各得一段 ——
    /// 屏幕上被切成几块，点击就得逐块命中。
    pub links: Vec<(u16, u16, String)>,
}

/// 折行，并把每条链接换算成屏幕列区间。
///
/// `\n` 与超宽折行的规则与 [`wrap_styled_chars`] 完全是同一套（后者就是它的包装），
/// 因为「哪一列属于链接」必须与「哪一列真的画出了这些字」出自同一次计算。
pub(crate) fn wrap_styled_rows(styled: &PartChars, width: usize) -> Vec<WrappedRow> {
    let mut rows: Vec<WrappedRow> = Vec::new();
    let mut row = WrappedRow::default();
    let mut row_width = 0usize;
    // 当前行上还没收口的链接：(url, 起始列)
    let mut open: Option<(String, u16)> = None;
    let mut seg = 0usize;
    for (idx, cell) in styled.iter().enumerate() {
        let (st, ch) = *cell;
        if ch == '\n' {
            close_row(&mut rows, &mut row, &mut open, row_width);
            row_width = 0;
            continue;
        }
        if ch.is_control() {
            continue;
        }
        let cw = ch.width().unwrap_or(0);
        if !row.chars.is_empty() && row_width + cw > width {
            close_row(&mut rows, &mut row, &mut open, row_width);
            row_width = 0;
        }
        track_link(
            &styled.links,
            &mut seg,
            idx,
            &mut open,
            &mut row.links,
            row_width,
        );
        row.chars.push((st, ch));
        row_width += cw;
    }
    // 末尾这一行：与旧语义一致，非空才落；整段为空时补一个空行。
    if !row.chars.is_empty() {
        close_open(&mut open, &mut row.links, row_width);
        rows.push(row);
    }
    if rows.is_empty() {
        rows.push(WrappedRow::default());
    }
    rows
}

/// 只要字符、不要列区间的版本：测试与纯文本场景用（生产路径走 [`wrap_styled_rows`]）。
///
/// 签名收切片而非 `&PartChars`：调用方手里往往只有字符流（没有链接可谈），
/// 让它们为此去包一层 `PartChars` 只会把噪声带进断言。
#[cfg(test)]
pub(crate) fn wrap_styled_chars(styled: &[(Style, char)], width: usize) -> Vec<Vec<(Style, char)>> {
    wrap_styled_rows(&PartChars::from(styled.to_vec()), width)
        .into_iter()
        .map(|r| r.chars)
        .collect()
}

/// 收口当前行（换到下一行 / 文本结束）：即便行内没有字符也落下（`\n` 就是空行）。
fn close_row(
    rows: &mut Vec<WrappedRow>,
    row: &mut WrappedRow,
    open: &mut Option<(String, u16)>,
    cols: usize,
) {
    close_open(open, &mut row.links, cols);
    rows.push(std::mem::take(row));
}

/// 把还开着的链接按当前列收口。
fn close_open(open: &mut Option<(String, u16)>, links: &mut Vec<(u16, u16, String)>, cols: usize) {
    if let Some((url, start)) = open.take() {
        links.push((start, cols as u16, url));
    }
}

/// 按字符下标推进链接游标，必要时在当前行开一段新区间。
fn track_link(
    links: &[LinkSeg],
    seg: &mut usize,
    idx: usize,
    open: &mut Option<(String, u16)>,
    out: &mut Vec<(u16, u16, String)>,
    col: usize,
) {
    while *seg < links.len() && links[*seg].end <= idx {
        *seg += 1;
    }
    let Some(link) = links.get(*seg).filter(|l| l.start <= idx && idx < l.end) else {
        // 这个字符不属于任何链接：上一段必须在当前列收口。少了这一句，热区会
        // 一路延伸到行尾 —— 点链接后面的正文也会打开浏览器。
        close_open(open, out, col);
        return;
    };
    // 两条链接紧贴（`[a](u1)[b](u2)`）时中间没有间隔字符：上一段必须在当前列收口，
    // 否则它们会被并成一段，点右边那条会打开左边那条的地址。
    if let Some((prev, start)) = open.as_ref()
        && *prev != link.url
    {
        out.push((*start, col as u16, prev.clone()));
        *open = None;
    }
    if open.is_none() {
        *open = Some((link.url.clone(), col as u16));
    }
}

/// 链接样式（`[文字](url)` 与裸 URL 共用，否则两者一眼就能看出差别）。
///
/// 无色模式只剩一条下划线：没有可靠的强调色可用。
pub(super) fn link_style(pal: &Palette) -> Style {
    if pal.is_mono() {
        Style::new().add_modifier(Modifier::UNDERLINED)
    } else {
        Style::new()
            .fg(pal.accent)
            .add_modifier(Modifier::UNDERLINED)
    }
}

/// 把纯文本里的裸 `http(s)://…` 按 `[url](url)` 的语义登记成链接。
///
/// 展示文本就是 URL 本身：屏幕上与不处理时**逐字一致**，区别只在多了链接样式与一个
/// 热区。为什么非做不可：共享 parser 只认 `[文字](url)`，而模型最常直接吐裸地址 ——
/// 那正是「终端自己画了下划线、点下去却毫无反应」的形态（本程序开了鼠标上报，点击被
/// 送进本进程，终端自己的链接处理器再也收不到，见 `links.rs` 头注释）。
///
/// **只该用在行内纯文本上**：行内代码（`` `https://x` ``）与代码块是人写给机器看的
/// 字面量，不该变成可点目标；表格单元格走的是同一条字符流，列宽压缩与「`…` 之后
/// 不再有热区」都在 `md_table` 里一次做完（见 `truncate_cell` 的 `clip_links`）。
pub(super) fn push_plain_with_urls(
    out: &mut PartChars,
    text_st: Style,
    link_st: Style,
    text: &str,
) {
    let mut cursor = 0usize;
    while let Some((start, end)) = find_url(&text[cursor..]) {
        let (start, end) = (cursor + start, cursor + end);
        out.extend(text[cursor..start].chars().map(|c| (text_st, c)));
        // 展示文本 = URL 本身，与手写 `[url](url)` 完全等价。
        out.push_link(link_st, &text[start..end], &text[start..end]);
        cursor = end;
    }
    out.extend(text[cursor..].chars().map(|c| (text_st, c)));
}

/// 第一段裸 URL 的字节区间。
fn find_url(text: &str) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut at = 0usize;
    while at < bytes.len() {
        if let Some(scheme) = scheme_len_at(bytes, at)
            && let Some(end) = url_end(text, at + scheme)
        {
            return Some((at, end));
        }
        at += 1;
    }
    None
}

/// `at` 处若是 scheme，返回它的长度（`https` 先判，否则 `http` 会抢先命中）。
///
/// 按字节比对是安全的：UTF-8 的多字节序列里不会出现 ASCII 字节，命中点必然落在
/// 字符边界上。
fn scheme_len_at(bytes: &[u8], at: usize) -> Option<usize> {
    const HTTPS: &[u8] = b"https://";
    const HTTP: &[u8] = b"http://";
    [HTTPS, HTTP]
        .into_iter()
        .find(|needle| matches_at(bytes, at, needle))
        .map(<[u8]>::len)
}

/// `bytes[at..]` 是否以 `needle` 开头（ASCII 大小写不敏感）。
fn matches_at(bytes: &[u8], at: usize, needle: &[u8]) -> bool {
    bytes.len() >= at + needle.len() && bytes[at..at + needle.len()].eq_ignore_ascii_case(needle)
}

/// 从 scheme 之后扫到地址结尾，并剥掉不该属于地址的尾随标点。
///
/// 返回 `None` = 只有 scheme 没有正文（`https://` 后面直接断开）：打开一个空地址
/// 没有意义，按普通文本处理。
fn url_end(text: &str, body_start: usize) -> Option<usize> {
    let mut end = body_start;
    // 括号按配平走：`…/Rust_(programming_language)` 的 `)` 属于地址，
    // 而 `(见 https://x.com)` 里的那个不属于。
    let mut depth = 0i32;
    for (off, ch) in text[body_start..].char_indices() {
        if is_url_stop(ch) {
            break;
        }
        match ch {
            '(' => depth += 1,
            ')' if depth == 0 => break,
            ')' => depth -= 1,
            _ => {}
        }
        end = body_start + off + ch.len_utf8();
    }
    let end = trim_trailing_punct(text, body_start, end);
    (end > body_start).then_some(end)
}

/// 地址的终止字符：空白、控制字符，以及会切断地址的引号/尖括号/全角标点。
///
/// 全角标点必须单列：`见 https://x.com，然后` 里那个逗号不属于地址，而
/// `char::is_ascii_punctuation` 不认识它。
fn is_url_stop(ch: char) -> bool {
    ch.is_whitespace()
        || ch.is_control()
        || matches!(ch, '<' | '>' | '"' | '\'' | '`' | '\\' | '|' | '^')
        || matches!(
            ch,
            '，' | '。'
                | '、'
                | '；'
                | '：'
                | '！'
                | '？'
                | '（'
                | '）'
                | '【'
                | '】'
                | '「'
                | '」'
                | '『'
                | '』'
                | '《'
                | '》'
                | '〈'
                | '〉'
                | '…'
                | '—'
                | '～'
                | '·'
        )
}

/// 剥掉地址尾部的标点：`见 https://x.com.` 里的句号是句子的，不是地址的。
fn trim_trailing_punct(text: &str, start: usize, mut end: usize) -> usize {
    while end > start {
        let Some(ch) = text[start..end].chars().next_back() else {
            break;
        };
        if !matches!(
            ch,
            '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '"' | ']' | '}' | '>'
        ) {
            break;
        }
        end -= ch.len_utf8();
    }
    end
}
