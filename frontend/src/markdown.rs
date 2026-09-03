//! 聊天气泡内 Markdown → 安全 HTML（`ammonia` 白名单），供助手消息渲染。
//!
//! 解析前会做 [`normalize_markdown_for_render`]，降低模型常见围栏误写导致的整段被吃进代码块等问题。

mod autolink;
mod code_block;
mod sanitize;
mod stream_inline;
pub(crate) mod workspace_image;

use pulldown_cmark::{Options, Parser, html};

pub(crate) use stream_inline::stream_inline_safe_html;

/// 在按行拆分前做**跨行**轻量修补：模型常把小节标题、围栏开符与正文粘在同一行，
/// `pulldown_cmark` 会收成单段 `<p>` 或把后续正文吃进代码块，界面上像「一整段」且标点后缺空格感更强。
fn normalize_glued_markdown_blocks(md: &str) -> String {
    let mut s = md.to_string();
    // 全角冒号后紧跟围栏：强制换行，避免 `：**…**：```code` 整行误解析。
    s = s.replace("：```", "：\n```");
    s = s.replace("：~~~", "：\n~~~");
    // 句末 / 右括号后紧贴下一小节 `**标题**`：拆成独立段，便于分段与列表式阅读。
    s = s.replace("）**", "）\n\n**");
    s = s.replace("。**", "。\n\n**");
    s = s.replace("！**", "！\n\n**");
    s = s.replace("？**", "？\n\n**");
    // 句末紧贴引用：避免 `结束。> 引用` 被收进同一段落。
    s = s.replace("。>", "。\n\n>");
    s = s.replace("！>", "！\n\n>");
    s = s.replace("？>", "？\n\n>");
    s
}

/// 在 `pulldown_cmark` 解析前做轻量规范化（单行规则，不解析嵌套结构）。
///
/// 处理常见误写：
/// 1. **行内围栏**：正文后紧贴 `` ```lang `` / `~~~lang`（如 `依赖：```rust`），拆成上一行 + 独立围栏行。
/// 2. **信息串与注释粘连**：行首合法围栏后写成 `` ```rust// comment ``，拆成 `` ```rust `` 与 `// comment` 两行。
/// 3. **行尾悬空围栏**：行首不是合法围栏行，但行尾仅剩一段 `` ``` `` / `~~~` 且无其它正文，去掉尾部 fence，避免误开空代码块。
/// 4. **ATX 标题缺空格**：如 `###规范与安全`（`#` 与标题字之间无空格），在至多 6 个 `#` 后补一个空格，满足 CommonMark 标题语法。
/// 5. **列表标记缺空格**：如 `-规范` / `1.下一步`（标记与**非 ASCII** 正文之间无空格）；不改 `-rf` / `1.0` / `*em*`。
/// 6. **GFM 表头与分隔行粘连**：如 `| a | b ||---|---|`（模型常漏换行），拆成表头行 + 独立对齐行。
/// 7. **引用粘连**：句末 `。` / `！` / `？` 后紧贴 `>`（如 `结束。> 引用`），拆成独立引用块；不拆 `：>`（避免 `阈值：> 0`）。
///
/// 无法覆盖所有非法 Markdown；不改写嵌套列表缩进（避免误伤代码/散文）。极端正文若以围栏标记结尾仍可能被改写（极少见）。
pub fn normalize_markdown_for_render(md: &str) -> String {
    if md.is_empty() {
        return String::new();
    }
    let md = normalize_glued_markdown_blocks(md);
    md.split('\n')
        .map(normalize_one_input_line)
        .collect::<Vec<_>>()
        .join("\n")
}

/// 去掉 `\r`，对单行应用围栏规范化（可能输出多行，以 `\n` 连接），再对**每一输出行**补 ATX 标题空格。
fn normalize_one_input_line(line: &str) -> String {
    let line = line.strip_suffix('\r').unwrap_or(line);
    let n = normalize_line_recursive(line);
    n.lines()
        .map(fix_atx_heading_missing_space)
        .map(|line| fix_list_marker_missing_space(&line))
        .collect::<Vec<_>>()
        .join("\n")
}

/// CommonMark ATX 标题：`#`…`#`（1–6 个）后须有空格或行尾；模型常写成 `###标题`。
/// 定位「正文起始字节」；不满足（无井号 / 井号超长 / 行尾 / 后随空格·制表符·#）返回 `None`。
fn atx_heading_split_byte(line: &str) -> Option<usize> {
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let mut idx = 0usize;
    let mut indent = 0usize;
    while idx < chars.len() && indent < 3 && chars[idx].1 == ' ' {
        indent += 1;
        idx += 1;
    }
    let hash_start = idx;
    let mut hash_end = idx;
    while hash_end < chars.len() && chars[hash_end].1 == '#' {
        hash_end += 1;
    }
    let n = hash_end - hash_start;
    if n == 0 || n > 6 {
        return None;
    }
    if hash_end >= chars.len() {
        return None;
    }
    match chars[hash_end].1 {
        ' ' | '\t' | '#' => None,
        _ => Some(chars[hash_end].0),
    }
}

fn fix_atx_heading_missing_space(line: &str) -> String {
    match atx_heading_split_byte(line) {
        Some(split_byte) => insert_space_at(line, split_byte),
        None => line.to_string(),
    }
}

fn insert_space_at(line: &str, byte: usize) -> String {
    let mut out = String::with_capacity(line.len() + 1);
    out.push_str(&line[..byte]);
    out.push(' ');
    out.push_str(&line[byte..]);
    out
}

fn skip_atx_or_list_indent(chars: &[(usize, char)]) -> usize {
    let mut idx = 0usize;
    let mut indent = 0usize;
    while idx < chars.len() && indent < 3 && chars[idx].1 == ' ' {
        indent += 1;
        idx += 1;
    }
    idx
}

/// 标记后紧贴的中文（等非 ASCII 字母）才补空格；避开 `-rf` / `1.Next`。
fn glued_cjk_list_body(ch: char) -> bool {
    ch.is_alphabetic() && !ch.is_ascii()
}

/// 行首 `-规范` / `+项` 补空格；不碰 `*`（与强调冲突）、`--` 主题分割、ASCII 短选项。
fn fix_unordered_list_missing_space(line: &str) -> Option<String> {
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let idx = skip_atx_or_list_indent(&chars);
    let c = chars.get(idx)?.1;
    if c != '-' && c != '+' {
        return None;
    }
    let next = chars.get(idx + 1)?.1;
    if !glued_cjk_list_body(next) {
        return None;
    }
    Some(insert_space_at(line, chars[idx + 1].0))
}

fn skip_ascii_digits(chars: &[(usize, char)], start: usize, max_digits: usize) -> usize {
    let mut dig = start;
    let mut n_dig = 0usize;
    while dig < chars.len() && chars[dig].1.is_ascii_digit() && n_dig < max_digits {
        n_dig += 1;
        dig += 1;
    }
    dig
}

/// `1.下一步` 的正文起始字节；`1.0` / `1.Next` / 已有空格则 `None`。
fn ordered_list_body_byte(line: &str) -> Option<usize> {
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let idx = skip_atx_or_list_indent(&chars);
    let dig = skip_ascii_digits(&chars, idx, 9);
    if dig == idx || dig >= chars.len() {
        return None;
    }
    if chars[dig].1 != '.' && chars[dig].1 != ')' {
        return None;
    }
    let after = dig + 1;
    let nch = chars.get(after)?.1;
    if !glued_cjk_list_body(nch) {
        return None;
    }
    Some(chars[after].0)
}

/// `1.下一步` 补空格；`1.0` 视为版本号不改。
fn fix_ordered_list_missing_space(line: &str) -> Option<String> {
    Some(insert_space_at(line, ordered_list_body_byte(line)?))
}

fn fix_list_marker_missing_space(line: &str) -> String {
    if let Some(s) = fix_unordered_list_missing_space(line) {
        return s;
    }
    if let Some(s) = fix_ordered_list_missing_space(line) {
        return s;
    }
    line.to_string()
}

fn normalize_line_recursive(line: &str) -> String {
    if let Some((header, delim)) = split_merged_table_header_separator_line(line) {
        let h = normalize_line_recursive(&header);
        let d = normalize_line_recursive(&delim);
        return format!("{h}\n{d}");
    }

    if let Some((left, right)) = split_mid_line_fence_if_needed(line) {
        let left = left.trim_end();
        let right_trim = right.trim_start_matches(' ');
        if is_fence_only_line(right) {
            return normalize_line_recursive(left);
        }
        let right_norm = normalize_line_recursive(right_trim);
        if left.is_empty() {
            return right_norm;
        }
        if right_norm.is_empty() {
            return left.to_string();
        }
        return format!("{left}\n{right_norm}");
    }

    let after_sticky = split_sticky_fence_lang_comment(line);
    normalize_trailing_orphan_fence(&after_sticky)
}

/// 在表头行字节流中找首个「`||` 合并点」；仅在左段像表头、右段像分隔行时命中。
fn merged_table_split(body: &str) -> Option<(usize, usize)> {
    let b = body.as_bytes();
    let mut i = 0usize;
    while i + 1 < b.len() {
        if b[i] == b'|' && b[i + 1] == b'|' {
            let header_body = body.get(..i)?;
            let rest = body.get(i + 2..)?;
            if table_row_looks_like_header(header_body) && looks_like_table_delimiter_row(rest) {
                return Some((i, i + 2));
            }
        }
        i += 1;
    }
    None
}

/// `| 列1 | 列2 ||------|------|` → 表头 + 对齐行（`pulldown_cmark` 要求分隔行独占一行）。
fn split_merged_table_header_separator_line(line: &str) -> Option<(String, String)> {
    if fence_starts_line(line) {
        return None;
    }
    let sp = leading_space_width(line).min(3);
    let body = line.get(sp..)?;
    if !body.trim_start().starts_with('|') {
        return None;
    }
    let indent = line.get(..sp).unwrap_or("");
    let (header_end, rest_start) = merged_table_split(body)?;
    let header_body = body.get(..header_end)?;
    let rest = body.get(rest_start..)?;
    let header = format!("{indent}{}", header_body.trim_end());
    let delim_trim = rest.trim_start();
    let delim_body = if delim_trim.starts_with('|') {
        delim_trim.to_string()
    } else {
        format!("|{delim_trim}")
    };
    let delim = format!("{indent}{delim_body}");
    Some((header, delim))
}

fn table_row_looks_like_header(row: &str) -> bool {
    let t = row.trim();
    if t.is_empty() || !t.starts_with('|') {
        return false;
    }
    if t.matches('|').count() < 2 {
        return false;
    }
    !looks_like_table_delimiter_row(t)
}

/// GFM 分隔行：由 `|` 分开的单元格，每格仅 `-` / `:` / 空白，且至少含三个 `-`。
fn looks_like_table_delimiter_row(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() || !s.contains('-') {
        return false;
    }
    let parts: Vec<&str> = s
        .split('|')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if parts.len() < 2 {
        return false;
    }
    parts.iter().copied().all(is_delimiter_cell)
}

fn is_delimiter_cell(cell: &str) -> bool {
    if cell.is_empty() {
        return false;
    }
    let dash_count = cell.chars().filter(|&c| c == '-').count();
    if dash_count < 3 {
        return false;
    }
    cell.chars()
        .all(|c| c == '-' || c == ':' || c.is_whitespace())
}

/// 行内第一个围栏开符段：`` ``` `` 或 `~~~`，`(字节起点, 字节长度)`，长度 ≥3。
fn first_fence_run(line: &str) -> Option<(usize, usize)> {
    let b = line.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        let ch = b[i];
        if ch == b'`' || ch == b'~' {
            let start = i;
            let mut j = i;
            while j < b.len() && b[j] == ch {
                j += 1;
            }
            if j - start >= 3 {
                return Some((start, j - start));
            }
            i = j;
        } else {
            i += 1;
        }
    }
    None
}

fn leading_space_width(line: &str) -> usize {
    line.chars().take_while(|&c| c == ' ').count()
}

/// CommonMark：围栏行前可有至多 3 个空格；其后为 `` ` `` 或 `~` 跑。
fn fence_starts_line(line: &str) -> bool {
    let sp = leading_space_width(line).min(3);
    match first_fence_run(line) {
        Some((ix, _)) => ix == sp,
        None => false,
    }
}

/// 若首个围栏不在合法行首位置，拆成 `(prefix, 从围栏起的后缀)`。
fn split_mid_line_fence_if_needed(line: &str) -> Option<(&str, &str)> {
    let sp = leading_space_width(line).min(3);
    let (ix, _) = first_fence_run(line)?;
    if ix == sp {
        return None;
    }
    Some((&line[..ix], &line[ix..]))
}

/// 后缀在去掉行首空格后仅为 ≥3 个 `` ` `` 或 `~`（可选尾随空白）。
fn is_fence_only_line(line: &str) -> bool {
    let s = line.trim_start_matches(' ');
    let bytes = s.as_bytes();
    if bytes.len() < 3 {
        return false;
    }
    let ch = bytes[0];
    if ch != b'`' && ch != b'~' {
        return false;
    }
    bytes.iter().all(|b| *b == ch)
}

fn is_fence_info_token(s: &str) -> bool {
    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '+' || c == '.')
}

/// 围栏 info 与 `//` 粘连时返回 `//` 在 info 后缀中的字节下标。
fn sticky_comment_slash_in_fence_info(after_ticks: &str) -> Option<usize> {
    if after_ticks.trim().is_empty() {
        return None;
    }
    let slash = after_ticks.find("//")?;
    let before_slash = &after_ticks[..slash];
    // info 段：语言 id，不含空白；允许空 info 后紧跟 `//`
    if before_slash.contains(char::is_whitespace) {
        return None;
    }
    if !is_fence_info_token(before_slash) {
        return None;
    }
    Some(slash)
}

/// 行首合法围栏且 info 与 `//` 粘在同一行时拆开（如 `` ```rust// x ``）。
fn split_sticky_fence_lang_comment(line: &str) -> String {
    if !fence_starts_line(line) {
        return line.to_string();
    }
    let sp = leading_space_width(line).min(3);
    let Some((ix, run_len)) = first_fence_run(line) else {
        return line.to_string();
    };
    if ix != sp {
        return line.to_string();
    }
    let Some(slash) = sticky_comment_slash_in_fence_info(&line[ix + run_len..]) else {
        return line.to_string();
    };
    let fence_head = &line[..ix + run_len + slash];
    let tail = &line[ix + run_len + slash..];
    format!("{fence_head}\n{tail}")
}

/// 非围栏行且行尾为「若干空格 + 纯围栏开符」时去掉尾部（避免行尾误写导致下一行被吃进代码块）。
fn normalize_trailing_orphan_fence(line: &str) -> String {
    if fence_starts_line(line) {
        return line.to_string();
    }
    let trimmed_end = line.trim_end_matches([' ', '\t']);
    let Some((body, _tail)) = split_trailing_fence_run(trimmed_end) else {
        return line.to_string();
    };
    if body.is_empty() {
        return line.to_string();
    }
    // 保留原行尾换行外的尾随空白策略：用 trim 后的 body + 原行尾在 trimmed_end 之后的空白
    let suffix_ws = &line[trimmed_end.len()..];
    format!("{body}{suffix_ws}")
}

/// `text + (spaces) + ```+` 或 `~~~+` → `(text, fence)`；`text` 不得以空白结尾。
fn split_trailing_fence_run(s: &str) -> Option<(&str, &str)> {
    let bytes = s.as_bytes();
    let ch = *bytes.last()?;
    if ch != b'`' && ch != b'~' {
        return None;
    }
    let mut i = bytes.len();
    while i > 0 && bytes[i - 1] == ch {
        i -= 1;
    }
    let tick_start = i;
    if bytes.len() - tick_start < 3 {
        return None;
    }
    let mut j = tick_start;
    while j > 0 && (bytes[j - 1] == b' ' || bytes[j - 1] == b'\t') {
        j -= 1;
    }
    if j == 0 {
        return None;
    }
    Some((&s[..j], &s[j..]))
}

/// 将 Markdown 转为经净化的 HTML 片段（不含外层 `<html>`）。
/// 段落内单换行按硬换行输出 `<br />`，避免在 `white-space: normal` 下被收成空格。
pub fn to_safe_html(md: &str) -> String {
    if md.trim().is_empty() {
        return String::new();
    }
    let md = normalize_markdown_for_render(md);
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_GFM);
    let parser = Parser::new_ext(&md, opts);
    let events = autolink::rewrite_events(parser);
    let mut body = String::new();
    html::push_html(&mut body, events.into_iter());
    let cleaned = sanitize::clean_chat_html(&body);
    code_block::decorate_fenced_code_blocks(&cleaned)
}

/// 调试：不做 Markdown 解析，将纯文本转义为可安全写入 `innerHTML` 的片段（换行 → `<br />`）。
pub fn plaintext_to_safe_html(text: &str) -> String {
    if text.trim().is_empty() {
        return String::new();
    }
    let mut out = String::with_capacity(text.len().saturating_mul(2));
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("<br />"),
            '\r' => {}
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{normalize_markdown_for_render, plaintext_to_safe_html, to_safe_html};

    #[test]
    fn multi_level_headings_produce_h_tags() {
        let h = to_safe_html("# Title\n\n## Sub\n\n### H3");
        assert!(!h.contains("<h1"), "message h1 must be demoted, got {h:?}");
        assert!(!h.contains("<h2"), "message h2 must be demoted, got {h:?}");
        assert!(h.contains("<h3"), "got {h:?}");
        assert!(h.contains("<h4"), "got {h:?}");
        assert!(
            h.contains("Title") && h.contains("Sub") && h.contains("H3"),
            "got {h:?}"
        );
    }

    #[test]
    fn empty_or_whitespace_yields_empty() {
        assert!(to_safe_html("").is_empty());
        assert!(to_safe_html("   \n\t  ").is_empty());
    }

    #[test]
    fn table_parsed_and_kept() {
        let h = to_safe_html("|h1|h2|\n|---|---|\n|a|b|");
        assert!(h.contains("<table"));
        assert!(h.to_lowercase().contains("h1"));
    }

    /// 模型/导出常把表头与 GFM 分隔行写在同一行：`||` 后应为独立一行。
    #[test]
    fn normalize_splits_table_header_glued_to_delimiter_row() {
        let raw = "| 类别 | 能力 ||------|------|";
        let n = normalize_markdown_for_render(raw);
        assert!(
            n.contains("能力") && n.contains('\n') && n.contains("------"),
            "expected two lines, got {n:?}"
        );
        let h = to_safe_html(raw);
        assert!(
            h.contains("<table"),
            "glued header+delimiter should become a table, got {h:?}"
        );
    }

    #[test]
    fn chat_links_get_target_blank() {
        let h = to_safe_html("see [site](https://example.com/path)");
        assert!(h.contains("target=\"_blank\""));
        assert!(h.contains("rel=\"noopener noreferrer\""));
        assert!(h.contains("https://example.com/path"));
    }

    #[test]
    fn bare_https_url_becomes_link() {
        let h = to_safe_html("见 https://example.com/path 与尾");
        assert!(h.contains("href=\"https://example.com/path\""), "got {h:?}");
        assert!(h.contains("target=\"_blank\""), "got {h:?}");
        assert!(h.contains(">https://example.com/path<"), "got {h:?}");
    }

    #[test]
    fn bare_url_inside_code_stays_plain() {
        let h = to_safe_html("`https://example.com`");
        assert!(h.contains("<code>"), "got {h:?}");
        assert!(
            !h.contains("<a "),
            "inline code must not autolink, got {h:?}"
        );
    }

    #[test]
    fn bare_url_inside_fence_stays_plain() {
        let h = to_safe_html("```\nhttps://example.com\n```");
        assert!(h.contains("<pre") || h.contains("<code"), "got {h:?}");
        assert!(!h.contains("<a "), "fence must not autolink, got {h:?}");
    }

    #[test]
    fn javascript_scheme_not_autolinked() {
        let h = to_safe_html("go javascript:alert(1) now");
        assert!(!h.contains("<a "), "got {h:?}");
        assert!(h.contains("javascript:alert(1)"), "got {h:?}");
    }

    #[test]
    fn markdown_link_is_not_double_wrapped() {
        let h = to_safe_html("[site](https://example.com/path)");
        assert_eq!(h.matches("<a ").count(), 1, "got {h:?}");
        assert!(h.contains("href=\"https://example.com/path\""), "got {h:?}");
    }

    #[test]
    fn cjk_path_stays_inside_href() {
        let h = to_safe_html("见 https://example.com/文档 尾");
        assert!(h.contains("<a "), "got {h:?}");
        assert!(
            h.contains("https://example.com/文档") || h.contains("%E6%96%87%E6%A1%A3"),
            "CJK path missing from link, got {h:?}"
        );
    }

    #[test]
    fn markdown_off_does_not_autolink() {
        let h = plaintext_to_safe_html("见 https://example.com");
        assert!(!h.contains("<a "), "got {h:?}");
        assert!(h.contains("https://example.com"), "got {h:?}");
    }

    #[test]
    fn script_tag_stripped_by_ammonia() {
        let h = to_safe_html("Hello<script>alert(1)</script>");
        assert!(!h.to_lowercase().contains("<script"));
        assert!(h.contains("Hello"));
    }

    #[test]
    fn fenced_code_keeps_language_class() {
        let h = to_safe_html("```rust\nlet x = 1;\n```");
        assert!(h.contains("<pre") || h.contains("<code"), "got {h:?}");
        assert!(h.contains("language-rust"), "got {h:?}");
        let csharp = to_safe_html("```c#\nConsole.WriteLine(1);\n```");
        assert!(csharp.contains("language-c#"), "got {csharp:?}");
    }

    #[test]
    fn task_list_unchecked_keeps_disabled_checkbox() {
        let h = to_safe_html("- [ ] todo");
        let lower = h.to_lowercase();
        assert!(lower.contains("<input"), "got {h:?}");
        assert!(lower.contains("type=\"checkbox\""), "got {h:?}");
        assert!(lower.contains("disabled"), "got {h:?}");
        assert!(!lower.contains("checked"), "got {h:?}");
        assert!(h.contains("todo"), "got {h:?}");
    }

    #[test]
    fn task_list_checked_keeps_checked_disabled_checkbox() {
        let h = to_safe_html("- [x] done");
        let lower = h.to_lowercase();
        assert!(lower.contains("<input"), "got {h:?}");
        assert!(lower.contains("type=\"checkbox\""), "got {h:?}");
        assert!(lower.contains("disabled"), "got {h:?}");
        assert!(lower.contains("checked"), "got {h:?}");
        assert!(h.contains("done"), "got {h:?}");
    }

    #[test]
    fn loose_task_list_keeps_disabled_checkboxes_inside_paragraphs() {
        let h = to_safe_html("- [ ] a\n\n- [ ] b");
        let lower = h.to_lowercase();
        assert_eq!(lower.matches("<input").count(), 2, "got {h:?}");
        assert!(
            lower.contains("<p>") && lower.contains("<input"),
            "got {h:?}"
        );
        assert!(lower.contains("type=\"checkbox\""), "got {h:?}");
        assert!(lower.contains("disabled"), "got {h:?}");
        assert!(h.contains('a') && h.contains('b'), "got {h:?}");
    }

    #[test]
    fn normalize_splits_inline_fence_opener() {
        let raw = "`llm` 依赖：```rust\ncode()\n```\n\n|a|b|\n|---|---|\n|1|2|";
        let n = normalize_markdown_for_render(raw);
        assert!(
            n.contains("依赖：\n```rust"),
            "expected fence on its own line, got {n:?}"
        );
        let h = to_safe_html(raw);
        assert!(
            h.contains("<table"),
            "table after code fence should parse, got {h:?}"
        );
    }

    #[test]
    fn normalize_sticky_lang_slash_slash() {
        let line = "```rust// comment here";
        let n = normalize_markdown_for_render(line);
        assert_eq!(n, "```rust\n// comment here");
    }

    #[test]
    fn normalize_strips_trailing_fence_on_heading_like_line() {
        let line = "#### 仍然存在的双向依赖```";
        let n = normalize_markdown_for_render(line);
        assert_eq!(n, "#### 仍然存在的双向依赖");
    }

    #[test]
    fn normalize_preserves_valid_fence_line() {
        let line = "```rust\nlet x = 1;\n```";
        assert_eq!(normalize_markdown_for_render(line), line);
    }

    #[test]
    fn normalize_preserves_valid_tilde_fence_line() {
        let line = "~~~\nlet x = 1;\n~~~";
        assert_eq!(normalize_markdown_for_render(line), line);
    }

    #[test]
    fn normalize_splits_inline_tilde_fence_opener() {
        let raw = "依赖：~~~rust\ncode()\n~~~";
        let n = normalize_markdown_for_render(raw);
        assert!(
            n.contains("依赖：\n~~~rust"),
            "expected tilde fence on its own line, got {n:?}"
        );
    }

    #[test]
    fn normalize_strips_trailing_tilde_fence_on_heading_like_line() {
        let line = "#### 仍然存在的双向依赖~~~";
        let n = normalize_markdown_for_render(line);
        assert_eq!(n, "#### 仍然存在的双向依赖");
    }

    #[test]
    fn normalize_sticky_tilde_lang_slash_slash() {
        let line = "~~~rust// comment here";
        let n = normalize_markdown_for_render(line);
        assert_eq!(n, "~~~rust\n// comment here");
    }

    #[test]
    fn normalize_splits_glued_blockquote_after_fullwidth_period() {
        let raw = "结束。> 引用";
        let n = normalize_markdown_for_render(raw);
        assert!(
            n.contains("结束。\n\n> 引用"),
            "expected quote on its own block, got {n:?}"
        );
        let h = to_safe_html(raw);
        assert!(h.contains("<blockquote"), "got {h:?}");
    }

    #[test]
    fn normalize_inserts_space_after_list_markers() {
        assert_eq!(normalize_markdown_for_render("-规范"), "- 规范");
        assert_eq!(normalize_markdown_for_render("1.下一步"), "1. 下一步");
        assert_eq!(normalize_markdown_for_render("1.0"), "1.0");
        let h = to_safe_html("-规范\n");
        assert!(h.contains("<li") && h.contains("规范"), "got {h:?}");
    }

    #[test]
    fn normalize_inserts_space_after_atx_hashes() {
        let raw = "###规范与安全\n\n正文。";
        assert_eq!(
            normalize_markdown_for_render(raw),
            "### 规范与安全\n\n正文。"
        );
        let h = to_safe_html(raw);
        assert!(
            h.contains("<h3") && h.contains("规范与安全"),
            "expected h3 heading, got {h:?}"
        );
    }

    #[test]
    fn single_newline_in_paragraph_emits_line_break() {
        let h = to_safe_html("不调用任何工具\n用 JSON 回复");
        let lower = h.to_lowercase();
        assert!(
            lower.contains("<br") || lower.contains("br>"),
            "expected hard line break in HTML, got {h:?}"
        );
    }

    #[test]
    fn plaintext_escapes_and_line_breaks() {
        let h = plaintext_to_safe_html("a <b>\nc");
        assert!(h.contains("&lt;"));
        assert!(h.to_lowercase().contains("<br"));
        assert!(!h.contains("<b>"));
    }

    /// 模型把「运行结果」与围栏粘在一行时，先插换行再解析，避免整段吃进代码块或单 `<p>`。
    #[test]
    fn normalize_inserts_newline_before_fence_after_fullwidth_colon() {
        let raw = "小结：**文件**：`x`（C++17）**编译**：`y`**运行结果**：```out```";
        let n = normalize_markdown_for_render(raw);
        assert!(
            n.contains("运行结果**：\n```") || n.contains("运行结果**：\n\n```"),
            "expected newline before fence, got {n:?}"
        );
        let h = to_safe_html(raw);
        assert!(
            h.matches("<p").count() >= 2 || h.contains("<pre"),
            "expected multiple blocks or a code block, got {h:?}"
        );
    }

    #[test]
    fn fenced_code_gets_toolbar() {
        let h = to_safe_html("```rust\nlet x = 1;\n```");
        assert!(h.contains("md-code-block"), "got {h:?}");
        assert!(h.contains("data-md-copy-code"), "got {h:?}");
        assert!(h.contains(">rust</span>"), "got {h:?}");
    }

    #[test]
    fn gfm_alert_note_keeps_class() {
        let h = to_safe_html("> [!NOTE]\n> hello");
        assert!(h.contains("markdown-alert-note"), "got {h:?}");
        assert!(h.contains("hello"), "got {h:?}");
    }

    #[test]
    fn javascript_href_is_stripped() {
        let h = to_safe_html("[x](javascript:alert(1))");
        let lower = h.to_lowercase();
        assert!(
            !lower.contains("href=\"javascript:") && !lower.contains("href='javascript:"),
            "javascript href must not survive, got {h:?}"
        );
        assert!(
            !lower.contains("<a"),
            "stripped javascript link must not leave a clickable <a>, got {h:?}"
        );
        assert!(h.contains('x'), "got {h:?}");
    }

    #[test]
    fn data_image_src_is_stripped() {
        let h = to_safe_html("![x](data:text/html;base64,PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0Pg==)");
        let lower = h.to_lowercase();
        assert!(!lower.contains("data:"), "got {h:?}");
    }

    #[test]
    fn https_image_gets_referrerpolicy() {
        let h = to_safe_html("![x](https://example.com/p.png)");
        assert!(h.contains("<img"), "got {h:?}");
        assert!(h.contains("referrerpolicy=\"no-referrer\""), "got {h:?}");
        assert!(h.contains("https://example.com/p.png"), "got {h:?}");
    }

    #[test]
    fn relative_workspace_png_rewrites_to_raw_api() {
        let h = to_safe_html("![x](plots/a.png)");
        assert!(h.contains("<img"), "got {h:?}");
        assert!(h.contains("/workspace/file/raw?path="), "got {h:?}");
        assert!(
            !h.contains("plots/a.png\""),
            "raw dest should be query-encoded, got {h:?}"
        );
    }

    #[test]
    fn relative_svg_image_is_stripped() {
        let h = to_safe_html("![x](plots/a.svg)");
        let lower = h.to_lowercase();
        assert!(!lower.contains("<img"), "got {h:?}");
    }

    #[test]
    fn heading_attribute_syntax_does_not_emit_id() {
        let h = to_safe_html("# Title {#main}");
        let lower = h.to_lowercase();
        assert!(!lower.contains("id=\"main\""), "got {h:?}");
        assert!(!lower.contains("id='main'"), "got {h:?}");
        assert!(h.contains("Title"), "got {h:?}");
    }

    #[test]
    fn atx_h1_is_demoted_to_h3() {
        let h = to_safe_html("# Only");
        assert!(h.contains("<h3"), "got {h:?}");
        assert!(!h.contains("<h1"), "got {h:?}");
        assert!(h.contains("Only"), "got {h:?}");
    }

    #[test]
    fn normalize_skips_ascii_flags_and_colon_greater_than() {
        assert_eq!(normalize_markdown_for_render("-rf"), "-rf");
        assert_eq!(normalize_markdown_for_render("-n"), "-n");
        assert_eq!(normalize_markdown_for_render("1.Next"), "1.Next");
        assert_eq!(
            normalize_markdown_for_render("```\n-e\n```"),
            "```\n-e\n```"
        );
        assert_eq!(normalize_markdown_for_render("阈值：> 0.5"), "阈值：> 0.5");
    }
}

/// WASM 下由 `wasm-bindgen-test` 跑通「Markdown → 净化 HTML」链路（与 CSR 目标一致）。
#[cfg(all(test, target_arch = "wasm32"))]
mod wasm_bindgen_tests {
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::to_safe_html;

    #[wasm_bindgen_test]
    fn wasm_markdown_bold_and_sanitized() {
        let h = to_safe_html("**x**");
        assert!(
            h.contains("<strong>") || h.contains("<b>"),
            "expected bold tag, got {h:?}"
        );
    }

    #[wasm_bindgen_test]
    fn wasm_markdown_table() {
        let h = to_safe_html("|c|\n|-|\n|v|");
        assert!(h.contains("<table"), "expected table, got {h:?}");
    }
}
