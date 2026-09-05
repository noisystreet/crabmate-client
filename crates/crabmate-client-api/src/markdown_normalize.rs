//! Markdown 渲染前文本规范化（纯逻辑，无 IO / DOM）。
//!
//! 由 WASM 聊天渲染（`frontend`）与全屏 TUI 等消费方共用，修正模型常见的
//! CJK 粘连围栏/标题缺空格/列表缺空格/表头粘连等误写；解析与渲染各自实现。

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

#[cfg(test)]
mod tests {
    use super::normalize_markdown_for_render;

    #[test]
    fn normalize_sticky_lang_slash_slash() {
        let line = "```rust// comment here";
        assert_eq!(
            normalize_markdown_for_render(line),
            "```rust\n// comment here"
        );
    }

    #[test]
    fn normalize_strips_trailing_fence_on_heading_like_line() {
        let line = "#### 仍然存在的双向依赖```";
        assert_eq!(
            normalize_markdown_for_render(line),
            "#### 仍然存在的双向依赖"
        );
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
        assert_eq!(
            normalize_markdown_for_render(line),
            "#### 仍然存在的双向依赖"
        );
    }

    #[test]
    fn normalize_sticky_tilde_lang_slash_slash() {
        let line = "~~~rust// comment here";
        assert_eq!(
            normalize_markdown_for_render(line),
            "~~~rust\n// comment here"
        );
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
