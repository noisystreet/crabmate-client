//! 写盘工具卡的 unified diff：行模型 + 详情文本切段（纯逻辑，可单测）。
//!
//! 数据来源是 **Server 已在工具正文首行写好的 `crabmate_tool_output` JSON**
//! （`preview: workspace_write_diff` + `files[].unified_diff`），由
//! [`crate::card`] 渲染成 ```` ```diff ```` 围栏拼进详情；本模块只负责把那段文本
//! 还原成「行 → 种类」，供前端着色与折叠。**不解析 `apply_patch` 的 `*** Begin Patch`**
//! 私有语法（未知行统一降级为 [`DiffLineKind::Meta`]，不猜）。

/// 详情里单个 diff 行的种类（只决定着色，行原文一字不改）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    /// 文件头 / `\ No newline` / 服务端体积截断提示等无增删语义的行。
    Meta,
    /// `@@ … @@` hunk 头。
    Hunk,
    /// 上下文行（前导一个空格）。
    Context,
    /// 新增行（前导 `+`，不含 `+++`）。
    Add,
    /// 删除行（前导 `-`，不含 `---`）。
    Del,
}

/// 一行 diff：保留前缀字符与空白，复制出去仍是合法 diff 文本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub text: String,
}

/// 默认折叠阈值：超过该行数只先渲染前 N 行，其余收进「展开」。
pub const DIFF_COLLAPSE_MAX_LINES: usize = 40;

/// 详情文本切段结果：普通文本按原顺序保留，diff 围栏内容变成行模型。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetailSegment {
    Text(String),
    Diff(Vec<DiffLine>),
}

/// 围栏开始行（crate 自己生成的是 ```` ```diff ````）。
const DIFF_FENCE_OPEN: &str = "```diff";
/// 任意围栏结束行。
const FENCE_CLOSE: &str = "```";

fn classify_line(line: &str) -> DiffLineKind {
    if line.starts_with("@@") {
        return DiffLineKind::Hunk;
    }
    // `+++` / `---` 是文件头，须先于 `+` / `-` 判定。
    if line.starts_with("+++") || line.starts_with("---") {
        return DiffLineKind::Meta;
    }
    if line.starts_with('+') {
        return DiffLineKind::Add;
    }
    if line.starts_with('-') {
        return DiffLineKind::Del;
    }
    if line.starts_with(' ') {
        return DiffLineKind::Context;
    }
    DiffLineKind::Meta
}

/// 把 unified diff 文本解析成行模型；**不含任何增删/hunk 行**时返回 `None`
/// （调用方据此把该段降级为原文，避免把普通代码块当 diff 着色）。
#[must_use]
pub fn parse_unified_diff(text: &str) -> Option<Vec<DiffLine>> {
    let lines: Vec<DiffLine> = text
        .lines()
        .map(|line| DiffLine {
            kind: classify_line(line),
            text: line.to_string(),
        })
        .collect();
    let has_change = lines.iter().any(|line| {
        matches!(
            line.kind,
            DiffLineKind::Add | DiffLineKind::Del | DiffLineKind::Hunk
        )
    });
    has_change.then_some(lines)
}

/// 折叠口径：可见前多少行 + 还剩多少行未展示（`max <= 1` 时按 1 处理，至少留一行）。
#[must_use]
pub fn collapse_diff_lines(lines: &[DiffLine], max: usize) -> (&[DiffLine], usize) {
    let max = max.max(1);
    if lines.len() <= max {
        return (lines, 0);
    }
    (&lines[..max], lines.len() - max)
}

fn flush_text(out: &mut Vec<DetailSegment>, text: &mut String) {
    if !text.is_empty() {
        out.push(DetailSegment::Text(std::mem::take(text)));
    }
}

/// diff 缓冲落段：能解析成行模型才出 [`DetailSegment::Diff`]，否则原样降级为文本。
fn flush_diff(out: &mut Vec<DetailSegment>, buf: &mut String) {
    match parse_unified_diff(buf) {
        Some(lines) => out.push(DetailSegment::Diff(lines)),
        None => out.push(DetailSegment::Text(std::mem::take(buf))),
    }
    buf.clear();
}

fn is_fence_open(line: &str) -> bool {
    let t = line.trim_end();
    t.trim_start().starts_with(DIFF_FENCE_OPEN)
}

fn is_fence_close(line: &str) -> bool {
    line.trim_end().trim_start() == FENCE_CLOSE
}

/// 把详情文本按 ```` ```diff ```` 围栏切成「文本 / diff」交替段。
///
/// 详情文本是持久化载体（SSE 与水合共用同一个 `detail`），因此切段而非换存储形态：
/// 存储、导出、TUI 仍看到围栏原文，只有 Web 工具卡把它渲染成着色行。
#[must_use]
pub fn split_detail_segments(detail: &str) -> Vec<DetailSegment> {
    let mut out: Vec<DetailSegment> = Vec::new();
    let mut text = String::new();
    let mut diff: Option<String> = None;
    for raw in detail.split_inclusive('\n') {
        let line = raw.trim_end_matches(['\n', '\r']);
        match diff.as_mut() {
            None => {
                if is_fence_open(line) {
                    flush_text(&mut out, &mut text);
                    diff = Some(String::new());
                } else {
                    text.push_str(raw);
                }
            }
            Some(buf) => {
                if is_fence_close(line) {
                    flush_diff(&mut out, buf);
                    diff = None;
                } else {
                    buf.push_str(raw);
                }
            }
        }
    }
    // 未闭合围栏：不猜，按原文留在文本段（宁可不着色，也不吞内容）。
    if let Some(mut buf) = diff {
        text.push_str(DIFF_FENCE_OPEN);
        text.push('\n');
        text.push_str(&buf);
        buf.clear();
    }
    flush_text(&mut out, &mut text);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "--- a/a.rs\n+++ b/a.rs\n@@ -1,2 +1,3 @@\n fn keep() {}\n-let old = 1;\n+let new = 2;\n+\n\\ No newline at end of file\n";

    fn kinds(lines: &[DiffLine]) -> Vec<DiffLineKind> {
        lines.iter().map(|l| l.kind).collect()
    }

    #[test]
    fn parses_add_del_context_and_headers() {
        let lines = parse_unified_diff(SAMPLE).expect("diff");
        assert_eq!(
            kinds(&lines),
            vec![
                DiffLineKind::Meta,
                DiffLineKind::Meta,
                DiffLineKind::Hunk,
                DiffLineKind::Context,
                DiffLineKind::Del,
                DiffLineKind::Add,
                DiffLineKind::Add,
                DiffLineKind::Meta,
            ]
        );
        assert_eq!(lines[5].text, "+let new = 2;");
    }

    #[test]
    fn empty_or_plain_text_is_not_a_diff() {
        assert!(parse_unified_diff("").is_none());
        assert!(parse_unified_diff("fn main() {}\n").is_none());
        assert!(parse_unified_diff("+only headerless plus is still a change").is_some());
    }

    #[test]
    fn collapse_keeps_first_n_and_reports_hidden() {
        let lines = parse_unified_diff(SAMPLE).expect("diff");
        let (visible, hidden) = collapse_diff_lines(&lines, 3);
        assert_eq!(visible.len(), 3);
        assert_eq!(hidden, 5);
        let (all, none) = collapse_diff_lines(&lines, 99);
        assert_eq!(all.len(), 8);
        assert_eq!(none, 0);
    }

    #[test]
    fn splits_fenced_diff_out_of_detail_text() {
        let detail = "变更预览（unified diff）\n\n`a.rs`\n```diff\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-x\n+y\n```\n\n路径：a.rs";
        let segs = split_detail_segments(detail);
        assert_eq!(segs.len(), 3, "{segs:?}");
        assert!(matches!(&segs[0], DetailSegment::Text(t) if t.contains("变更预览")));
        assert!(matches!(&segs[1], DetailSegment::Diff(lines) if lines.len() == 5));
        assert!(matches!(&segs[2], DetailSegment::Text(t) if t.contains("路径：a.rs")));
    }

    #[test]
    fn detail_without_fence_stays_one_text_segment() {
        let segs = split_detail_segments("$ cargo test\n\nexit 0");
        assert_eq!(segs.len(), 1);
        assert!(matches!(&segs[0], DetailSegment::Text(t) if t.contains("cargo test")));
    }

    #[test]
    fn fenced_non_diff_text_degrades_to_text() {
        let detail = "```diff\nhello\nworld\n```\n";
        let segs = split_detail_segments(detail);
        assert_eq!(segs.len(), 1);
        match &segs[0] {
            DetailSegment::Text(t) => assert!(t.contains("hello")),
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn unclosed_fence_keeps_raw_text() {
        let segs = split_detail_segments("```diff\n-x\n+y\n");
        assert_eq!(segs.len(), 1);
        match &segs[0] {
            DetailSegment::Text(t) => assert!(t.contains("-x") && t.contains("+y")),
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn server_volume_notice_lines_are_meta() {
        let text = "--- a/big.rs\n+++ b/big.rs\n@@ -1 +1 @@\n-a\n+b\n…\n（该文件 diff 已截断）";
        let lines = parse_unified_diff(text).expect("diff");
        assert_eq!(lines.last().map(|l| l.kind), Some(DiffLineKind::Meta));
        assert_eq!(lines[lines.len() - 2].kind, DiffLineKind::Meta);
    }

    #[test]
    fn crlf_detail_keeps_lines_clean() {
        let detail = "```diff\r\n--- a/x\r\n+++ b/x\r\n-a\r\n+b\r\n```\r\n";
        let segs = split_detail_segments(detail);
        assert!(matches!(&segs[0], DetailSegment::Diff(lines) if lines.len() == 4));
    }
}
