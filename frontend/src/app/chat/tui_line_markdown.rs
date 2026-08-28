//! 终端流按**块** Markdown：已闭合块做安全 HTML（冻结，增量只 append）；
//! 活跃末块做**流式安全**行内增强（成对 `**` / `*` / `` ` `` / `~~` / `[text](url)` 才着色；半截标记保持字面量）；
//! 未闭合围栏仍纯文本。段落/表格/列表/引用在空行或块类型切换时才冻结，避免终态全文重渲抖动。

use crate::markdown::{plaintext_to_safe_html, stream_inline_safe_html, to_safe_html};

/// 活跃块是否为未闭合围栏缓冲（须 `textContent`，禁止行内 HTML）。
#[must_use]
pub fn open_block_is_fence_buffer(text: &str) -> bool {
    let first = text.split('\n').next().unwrap_or(text);
    parse_fence_line(first).is_some()
}

/// 活跃块写入 DOM / `to_inner_html`。
///
/// `markdown_render=false` 时全程纯文本转义（含活跃块，对齐 `CM_WEB_DISABLE_MARKDOWN`）。
#[must_use]
pub fn render_open_active_html(text: &str, markdown_render: bool) -> String {
    if !markdown_render || open_block_is_fence_buffer(text) {
        return plaintext_to_safe_html(text);
    }
    if !text.contains('\n') {
        return stream_inline_safe_html(text);
    }
    let mut out = String::with_capacity(text.len().saturating_mul(2));
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push_str("<br />");
        }
        out.push_str(&stream_inline_safe_html(line));
    }
    out
}

/// 活跃块 DOM class（围栏 / 关 MD → plain；否则 active）。
#[must_use]
pub fn open_active_block_class(text: &str, markdown_render: bool) -> &'static str {
    if !markdown_render || open_block_is_fence_buffer(text) {
        "chat-tui-line chat-tui-line--plain"
    } else {
        "chat-tui-line chat-tui-line--active"
    }
}

/// CommonMark 围栏：行首最多 3 空格 + 至少 3 个 `` ` `` 或 `~`。
fn parse_fence_line(line: &str) -> Option<(u8, usize, &str)> {
    let t = line.trim_start();
    let indent = line.len() - t.len();
    if indent > 3 {
        return None;
    }
    let bytes = t.as_bytes();
    if bytes.len() < 3 {
        return None;
    }
    let ch = bytes[0];
    if ch != b'`' && ch != b'~' {
        return None;
    }
    let mut run = 0usize;
    while run < bytes.len() && bytes[run] == ch {
        run += 1;
    }
    if run < 3 {
        return None;
    }
    let info = t[run..].trim();
    if ch == b'`' && info.contains('`') {
        return None;
    }
    Some((ch, run, info))
}

fn fence_marker_str(ch: u8, run: usize) -> String {
    (ch as char).to_string().repeat(run.max(3))
}

fn blank_line_html() -> String {
    "<div class=\"chat-tui-line chat-tui-line--blank\"><br /></div>".to_string()
}

fn wrap_closed_html(inner: &str) -> String {
    if inner.is_empty() {
        return blank_line_html();
    }
    format!("<div class=\"chat-tui-line chat-tui-line--block msg-md-prose\">{inner}</div>")
}

fn closed_md_html(src: &str, markdown_render: bool) -> String {
    let html = if markdown_render {
        to_safe_html(src)
    } else {
        plaintext_to_safe_html(src)
    };
    wrap_closed_html(&html)
}

fn fence_html(ch: u8, run: usize, lang: &str, body: &str, markdown_render: bool) -> String {
    let mark = fence_marker_str(ch, run);
    let mut fenced = String::with_capacity(body.len() + lang.len() + mark.len() * 2 + 8);
    fenced.push_str(&mark);
    fenced.push_str(lang);
    fenced.push('\n');
    fenced.push_str(body);
    if !body.is_empty() && !body.ends_with('\n') {
        fenced.push('\n');
    }
    fenced.push_str(&mark);
    let html = if markdown_render {
        to_safe_html(&fenced)
    } else {
        plaintext_to_safe_html(&fenced)
    };
    format!("<div class=\"chat-tui-line chat-tui-line--fence msg-md-prose\">{html}</div>")
}

fn open_fence_plain_text(ch: u8, run: usize, lang: &str, body: &str, open_tail: &str) -> String {
    let mark = fence_marker_str(ch, run);
    let mut plain = String::new();
    plain.push_str(&mark);
    plain.push_str(lang);
    plain.push('\n');
    plain.push_str(body);
    plain.push_str(open_tail);
    plain
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    Paragraph,
    Table,
    List,
    Blockquote,
}

fn is_table_line(line: &str) -> bool {
    // 启发式：行首 `|` 视为表行（「a | b」散文不会命中；`| note` 仍会进 Table）。
    line.trim_start().starts_with('|')
}

fn is_list_line(line: &str) -> bool {
    let t = line.trim_start();
    if t.starts_with("- ") || t.starts_with("* ") || t.starts_with("+ ") {
        return true;
    }
    let mut chars = t.chars().peekable();
    let mut saw_digit = false;
    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() {
            saw_digit = true;
            chars.next();
            continue;
        }
        break;
    }
    if !saw_digit {
        return false;
    }
    match chars.next() {
        Some('.') | Some(')') => chars.next() == Some(' '),
        _ => false,
    }
}

fn is_atx_heading_line(line: &str) -> bool {
    let t = line.trim_start();
    let n = t.chars().take_while(|c| *c == '#').count();
    (1..=6).contains(&n) && t.as_bytes().get(n) == Some(&b' ')
}

fn is_thematic_break_line(line: &str) -> bool {
    let t = line.trim();
    if t.len() < 3 {
        return false;
    }
    let marker = t.as_bytes()[0];
    // 95 = b'_'；避免 byte char `b'_'`（lizard 会把后续函数 nloc 并进本函数）。
    if marker != b'-' && marker != b'*' && marker != 95 {
        return false;
    }
    for byte in t.bytes() {
        if byte != marker && byte != b' ' {
            return false;
        }
    }
    true
}

fn is_block_continuation(line: &str) -> bool {
    // 用 "\t" 而非 '\t'：后者会让 lizard 误判后续函数 nloc（fn-nloc ratchet）。
    line.starts_with("    ") || line.starts_with("\t")
}

fn is_blockquote_line(line: &str) -> bool {
    line.trim_start().starts_with('>')
}

fn classify_block_line(line: &str) -> BlockKind {
    if is_blockquote_line(line) {
        BlockKind::Blockquote
    } else if is_table_line(line) {
        BlockKind::Table
    } else if is_list_line(line) {
        BlockKind::List
    } else {
        BlockKind::Paragraph
    }
}

fn push_pending_line(pending: &mut String, line: &str) {
    if !pending.is_empty() {
        pending.push('\n');
    }
    pending.push_str(line);
}

/// 思维链折叠块（`<details>`）渲染数据：`open` 默认展开，`summary_html` 携带 i18n 标签，`body_html` 为已转义思考正文。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThinkBlock {
    pub open: bool,
    pub summary_html: String,
    pub body_html: String,
}

impl ThinkBlock {
    /// 完整 `<details>` 元素 HTML（`to_inner_html` 用）。
    #[must_use]
    pub fn to_details_html(&self) -> String {
        let open_attr = if self.open { " open" } else { "" };
        format!(
            "<details class=\"chat-tui-think\"{open_attr}>{}\
             <div class=\"chat-tui-think-body msg-md-prose\">{}</div>\
             </details>",
            self.summary_html, self.body_html
        )
    }
}

/// 按块解析结果：思维链折叠块 + 闭合块 HTML 列表（冻结）+ 可选活跃块**源文本**（渲染见 [`render_open_active_html`]）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TuiBodyChunks {
    /// 思维链折叠块（`<details>`）；无思考时为 `None`。
    pub think: Option<ThinkBlock>,
    pub closed: Vec<String>,
    /// 活跃块源文本；围栏缓冲以 \`\`\` / `~~~`（可带最多 3 空格缩进）开头。DOM 用 [`render_open_active_html`] 写入。
    pub open_plain: Option<String>,
    /// 与解析时 `markdown_render` 一致；活跃块与 Incremental 渲染均读此标志（patch 不再另带一份）。
    pub markdown_render: bool,
}

impl Default for TuiBodyChunks {
    fn default() -> Self {
        Self {
            think: None,
            closed: Vec::new(),
            open_plain: None,
            markdown_render: true,
        }
    }
}

impl TuiBodyChunks {
    #[must_use]
    pub fn to_inner_html(&self) -> String {
        let mut out = String::new();
        if let Some(think) = &self.think {
            out.push_str(&think.to_details_html());
        }
        for chunk in &self.closed {
            out.push_str(chunk);
        }
        if let Some(plain) = &self.open_plain {
            out.push_str("<div class=\"");
            out.push_str(open_active_block_class(plain, self.markdown_render));
            out.push_str("\">");
            out.push_str(&render_open_active_html(plain, self.markdown_render));
            out.push_str("</div>");
        }
        out
    }
}

/// live body 增量：优先 append **冻结**闭合块 + 只改活跃块；工具行改 status/one-line；否则整 body 替换。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TuiBodyPatch {
    ReplaceAll {
        chunks: TuiBodyChunks,
    },
    Incremental {
        append_closed: Vec<String>,
        /// 活跃块源文本（非 HTML）；`None` 表示移除活跃块节点。
        open_plain: Option<String>,
    },
    /// 思维链流式：只更新 `.chat-tui-think-body` 的 innerHTML（思考正文逐帧变化时不整块替换）；
    /// 若同帧还有新闭合块/活跃块则一并应用（思维链冻结与终答开头的过渡帧）。
    ThinkBody {
        body_html: String,
        append_closed: Vec<String>,
        open_plain: Option<String>,
    },
    /// 工具折叠行：只改文案，不重写 HTML（高度与结构保持不变）。
    ToolRow {
        status: String,
        status_label: String,
        one_line: String,
        /// 若 DOM 已有 details，同步更新详情正文；`None` 表示无详情块。
        detail: Option<String>,
    },
}

enum FenceMode {
    Off,
    Open {
        ch: u8,
        run: usize,
        lang: String,
        body: String,
    },
}

struct BlockAbsorbState {
    closed: Vec<String>,
    pending: String,
    pending_kind: Option<BlockKind>,
    fence: FenceMode,
    markdown_render: bool,
}

impl BlockAbsorbState {
    fn new(markdown_render: bool) -> Self {
        Self {
            closed: Vec::new(),
            pending: String::new(),
            pending_kind: None,
            fence: FenceMode::Off,
            markdown_render,
        }
    }

    fn flush_pending(&mut self) {
        if self.pending.is_empty() {
            self.pending_kind = None;
            return;
        }
        self.closed
            .push(closed_md_html(&self.pending, self.markdown_render));
        self.pending.clear();
        self.pending_kind = None;
    }

    fn close_open_fence(&mut self) {
        if let FenceMode::Open {
            ch,
            run,
            lang,
            body,
        } = std::mem::replace(&mut self.fence, FenceMode::Off)
        {
            self.closed
                .push(fence_html(ch, run, &lang, &body, self.markdown_render));
        }
    }

    /// 消息结束：把 pending / 未闭合围栏全部冻进 closed。
    fn seal_for_finalize(&mut self) {
        self.flush_pending();
        self.close_open_fence();
    }

    fn open_fence_plain(&self, open_tail: &str) -> Option<String> {
        match &self.fence {
            FenceMode::Off => None,
            FenceMode::Open {
                ch,
                run,
                lang,
                body,
            } => Some(open_fence_plain_text(*ch, *run, lang, body, open_tail)),
        }
    }

    fn try_absorb_fence(&mut self, line: &str) -> bool {
        if matches!(&self.fence, FenceMode::Open { .. }) {
            let closing = match &self.fence {
                FenceMode::Open { ch, run, .. } => parse_fence_line(line)
                    .is_some_and(|(c, r, info)| c == *ch && r >= *run && info.is_empty()),
                FenceMode::Off => false,
            };
            if closing {
                self.close_open_fence();
            } else if let FenceMode::Open { body, .. } = &mut self.fence {
                body.push_str(line);
                body.push('\n');
            }
            return true;
        }
        let Some((ch, run, info)) = parse_fence_line(line) else {
            return false;
        };
        self.flush_pending();
        self.fence = FenceMode::Open {
            ch,
            run,
            lang: info.to_string(),
            body: String::new(),
        };
        true
    }

    fn absorb_complete_line(&mut self, line: &str) {
        if self.try_absorb_fence(line) {
            return;
        }
        if line.trim().is_empty() {
            self.flush_pending();
            self.closed.push(blank_line_html());
            return;
        }
        if is_atx_heading_line(line) || is_thematic_break_line(line) {
            self.flush_pending();
            self.closed.push(closed_md_html(line, self.markdown_render));
            return;
        }
        let kind = classify_block_line(line);
        match self.pending_kind {
            None => {
                push_pending_line(&mut self.pending, line);
                self.pending_kind = Some(kind);
            }
            Some(prev)
                if prev == kind
                    || (prev == BlockKind::List && is_block_continuation(line))
                    || (prev == BlockKind::Paragraph && is_block_continuation(line)) =>
            {
                push_pending_line(&mut self.pending, line);
            }
            Some(_) => {
                self.flush_pending();
                push_pending_line(&mut self.pending, line);
                self.pending_kind = Some(kind);
            }
        }
    }
}

/// 将正文解析为可挂载的 closed / open 块。
///
/// `markdown_render=false` 时闭合块与围栏均走纯文本转义（对齐 `CM_WEB_DISABLE_MARKDOWN`）。
#[must_use]
pub fn parse_tui_body_chunks_with(
    text: &str,
    finalize_open_block: bool,
    markdown_render: bool,
) -> TuiBodyChunks {
    if text.is_empty() {
        return TuiBodyChunks::default();
    }

    let ends_with_nl = text.ends_with('\n');
    let raw: Vec<&str> = text.split('\n').collect();
    let (complete, open_line): (&[&str], Option<&str>) = if ends_with_nl {
        let n = raw.len().saturating_sub(1);
        (&raw[..n], None)
    } else if raw.is_empty() {
        (&[], None)
    } else {
        let last = raw.len() - 1;
        (&raw[..last], Some(raw[last]))
    };

    let mut state = BlockAbsorbState::new(markdown_render);
    for line in complete {
        state.absorb_complete_line(line);
    }

    let open_plain = match open_line {
        Some(open) if finalize_open_block => {
            state.absorb_complete_line(open);
            state.seal_for_finalize();
            None
        }
        Some(open) => {
            if let Some(plain) = state.open_fence_plain(open) {
                Some(plain)
            } else {
                let mut open_buf = std::mem::take(&mut state.pending);
                let _ = state.pending_kind.take();
                push_pending_line(&mut open_buf, open);
                Some(open_buf)
            }
        }
        None => {
            if let Some(plain) = state.open_fence_plain("") {
                Some(plain)
            } else if !state.pending.is_empty() {
                // 完整行已到、尚无空行收束：留在 open，避免提前冻结导致后续同段续写时 ReplaceAll。
                Some(std::mem::take(&mut state.pending))
            } else {
                None
            }
        }
    };

    TuiBodyChunks {
        think: None,
        closed: state.closed,
        open_plain,
        markdown_render,
    }
}

/// 对比上一帧 closed 前缀：可增量则 append，否则整段替换。
///
/// Incremental / ThinkBody 不携带 `markdown_render`；应用侧从 [`TuiBodyChunks::markdown_render`]（`next`）读取。
/// 思维链：`open` 与 `summary_html` 稳定时，正文变化走定向 [`TuiBodyPatch::ThinkBody`]（不整块替换）；
/// `open` 翻转（回合结束收起）等结构变化仍整段替换。
#[must_use]
pub fn plan_tui_body_patch(prev: Option<&TuiBodyChunks>, next: &TuiBodyChunks) -> TuiBodyPatch {
    let Some(prev) = prev else {
        return TuiBodyPatch::ReplaceAll {
            chunks: next.clone(),
        };
    };
    let think_head_match = match (&prev.think, &next.think) {
        (Some(p), Some(n)) => p.open == n.open && p.summary_html == n.summary_html,
        (None, None) => true,
        _ => false,
    };
    if next.markdown_render == prev.markdown_render
        && think_head_match
        && next.closed.len() >= prev.closed.len()
        && next.closed[..prev.closed.len()] == prev.closed[..]
    {
        let append_closed = next.closed[prev.closed.len()..].to_vec();
        let open_plain = next.open_plain.clone();
        let think_body_changed =
            prev.think.as_ref().map(|t| &t.body_html) != next.think.as_ref().map(|t| &t.body_html);
        if think_body_changed && let Some(think) = &next.think {
            return TuiBodyPatch::ThinkBody {
                body_html: think.body_html.clone(),
                append_closed,
                open_plain,
            };
        }
        return TuiBodyPatch::Incremental {
            append_closed,
            open_plain,
        };
    }
    TuiBodyPatch::ReplaceAll {
        chunks: next.clone(),
    }
}

/// 将助手/用户正文转为可写入 `innerHTML` 的按块流式 HTML。
#[must_use]
#[cfg(test)]
pub fn render_tui_block_markdown(text: &str, finalize_open_block: bool) -> String {
    parse_tui_body_chunks_with(text, finalize_open_block, true).to_inner_html()
}

#[cfg(test)]
fn parse_tui_body_chunks(text: &str, finalize_open_block: bool) -> TuiBodyChunks {
    parse_tui_body_chunks_with(text, finalize_open_block, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_bold_line_renders_strong() {
        let h = render_tui_block_markdown("**你好**\n", false);
        assert!(h.contains("<strong>") || h.contains("<b>"), "got {h}");
        assert!(!h.contains("**你好**"), "got {h}");
    }

    #[test]
    fn closed_markdown_block_reuses_msg_md_prose() {
        let h = render_tui_block_markdown("# Title\n\n", true);
        assert!(
            h.contains("chat-tui-line--block") && h.contains("msg-md-prose"),
            "closed transcript blocks should share changelist prose, got {h}"
        );
        assert!(!h.contains("chat-tui-line--active"), "got {h}");
    }

    #[test]
    fn active_line_does_not_use_msg_md_prose() {
        let h = render_tui_block_markdown("**第一段", false);
        assert!(h.contains("chat-tui-line--active"), "got {h}");
        assert!(!h.contains("msg-md-prose"), "got {h}");
    }

    #[test]
    fn incomplete_bold_stays_plain_while_streaming() {
        let h = render_tui_block_markdown("**第一段", false);
        assert!(h.contains("**第一段"), "got {h}");
        assert!(!h.contains("<strong>"), "got {h}");
        assert!(
            h.contains("chat-tui-line--active"),
            "active line class, got {h}"
        );
    }

    #[test]
    fn complete_inline_bold_in_active_line_while_streaming() {
        let h = render_tui_block_markdown("见 **粗体** 与尾", false);
        assert!(h.contains("<strong>"), "got {h}");
        assert!(h.contains("粗体"), "got {h}");
        assert!(!h.contains("**粗体**"), "got {h}");
    }

    #[test]
    fn complete_inline_italic_and_link_in_active_line() {
        let h = render_tui_block_markdown("见 *斜体* 与 [site](https://example.com)", false);
        assert!(h.contains("<em>斜体</em>"), "got {h}");
        assert!(h.contains("href=\"https://example.com\""), "got {h}");
        assert!(h.contains(">site</a>"), "got {h}");
    }

    #[test]
    fn balanced_then_incomplete_bold_in_active_line() {
        let h = stream_inline_safe_html("**ok** and **no");
        assert!(h.contains("<strong>ok</strong>"), "got {h}");
        assert!(h.contains("**no"), "got {h}");
        assert_eq!(h.matches("<strong>").count(), 1, "got {h}");
    }

    #[test]
    fn active_inline_code_and_escape() {
        let h = stream_inline_safe_html("用 `a<b>` 与 x");
        assert!(h.contains("<code>a&lt;b&gt;</code>"), "got {h}");
    }

    #[test]
    fn active_bare_url_becomes_link() {
        let h = stream_inline_safe_html("见 https://example.com 与尾");
        assert!(h.contains("href=\"https://example.com\""), "got {h}");
        assert!(h.contains("target=\"_blank\""), "got {h}");
        assert!(!h.contains("https://example.com 与尾"), "got {h}");
    }

    #[test]
    fn finalize_renders_open_line_as_markdown() {
        let h = render_tui_block_markdown("**第一段，第二段**", true);
        assert!(h.contains("<strong>") || h.contains("<b>"), "got {h}");
    }

    #[test]
    fn open_fence_stays_plain_until_closed() {
        let h = render_tui_block_markdown("```rust\nlet x = 1;\n", false);
        assert!(h.contains("let x = 1;"), "got {h}");
        assert!(
            !h.contains("<code"),
            "open fence should stay plain, got {h}"
        );
        assert!(
            h.contains("chat-tui-line--plain"),
            "fence buffer class, got {h}"
        );
    }

    #[test]
    fn closed_fence_renders_code() {
        let h = render_tui_block_markdown("```rust\nlet x = 1;\n```\n", false);
        assert!(h.contains("<code") || h.contains("<pre"), "got {h}");
        assert!(h.contains("let x = 1;"), "got {h}");
    }

    #[test]
    fn closed_prefix_frozen_when_active_grows() {
        let a = parse_tui_body_chunks("done\n\n**a", false);
        let b = parse_tui_body_chunks("done\n\n**ab", false);
        assert_eq!(a.closed, b.closed, "closed chunks must stay identical");
        assert!(!a.closed.is_empty(), "blank should freeze prior paragraph");
        match plan_tui_body_patch(Some(&a), &b) {
            TuiBodyPatch::Incremental {
                append_closed,
                open_plain,
            } => {
                assert!(append_closed.is_empty());
                assert_eq!(open_plain.as_deref(), Some("**ab"));
            }
            other => panic!("expected Incremental, got {other:?}"),
        }
    }

    #[test]
    fn streaming_open_line_growth_is_incremental_text_only() {
        let a = parse_tui_body_chunks("**a", false);
        let b = parse_tui_body_chunks("**ab", false);
        match plan_tui_body_patch(Some(&a), &b) {
            TuiBodyPatch::Incremental {
                append_closed,
                open_plain,
            } => {
                assert!(append_closed.is_empty());
                assert_eq!(open_plain.as_deref(), Some("**ab"));
            }
            other => panic!("expected Incremental, got {other:?}"),
        }
    }

    #[test]
    fn prose_newline_stays_in_open_until_blank() {
        let a = parse_tui_body_chunks("hello", false);
        let b = parse_tui_body_chunks("hello\nworld", false);
        assert!(a.closed.is_empty());
        assert!(b.closed.is_empty(), "no blank yet, got {:?}", b.closed);
        assert_eq!(b.open_plain.as_deref(), Some("hello\nworld"));
        match plan_tui_body_patch(Some(&a), &b) {
            TuiBodyPatch::Incremental {
                append_closed,
                open_plain,
            } => {
                assert!(append_closed.is_empty());
                assert_eq!(open_plain.as_deref(), Some("hello\nworld"));
            }
            other => panic!("expected Incremental, got {other:?}"),
        }
    }

    #[test]
    fn blank_line_freezes_paragraph_block() {
        let a = parse_tui_body_chunks("hello\nworld", false);
        let b = parse_tui_body_chunks("hello\nworld\n\nnext", false);
        match plan_tui_body_patch(Some(&a), &b) {
            TuiBodyPatch::Incremental {
                append_closed,
                open_plain,
            } => {
                assert!(
                    append_closed.iter().any(|c| c.contains("hello")),
                    "got {append_closed:?}"
                );
                assert_eq!(open_plain.as_deref(), Some("next"));
            }
            other => panic!("expected Incremental, got {other:?}"),
        }
    }

    #[test]
    fn table_block_freezes_as_one_chunk() {
        let src = "|a|b|\n|---|---|\n|1|2|\n\n";
        let chunks = parse_tui_body_chunks(src, false);
        assert!(
            chunks
                .closed
                .iter()
                .any(|c| c.contains("<table") || c.contains("<th") || c.contains("<td")),
            "got {:?}",
            chunks.closed
        );
        assert!(chunks.open_plain.is_none());
    }

    #[test]
    fn markdown_off_escapes_instead_of_strong() {
        let chunks = parse_tui_body_chunks_with("**x**\n\n", true, false);
        let html = chunks.to_inner_html();
        assert!(!html.contains("<strong>"), "got {html}");
        assert!(
            html.contains("**x**") || html.contains("&#42;"),
            "got {html}"
        );
    }

    #[test]
    fn markdown_off_active_open_line_skips_inline_strong() {
        let h = render_open_active_html("见 **粗体**", false);
        assert!(!h.contains("<strong>"), "got {h}");
        assert!(h.contains("**粗体**") || h.contains("&#42;"), "got {h}");
    }

    #[test]
    fn markdown_off_active_line_skips_autolink() {
        let h = render_open_active_html("见 https://example.com", false);
        assert!(!h.contains("<a "), "got {h}");
        assert!(h.contains("https://example.com"), "got {h}");
    }

    #[test]
    fn prose_pipe_is_not_table_block() {
        let chunks = parse_tui_body_chunks("use a | b as or\n\n", false);
        assert!(
            chunks.closed.iter().all(|c| !c.contains("<table")),
            "got {:?}",
            chunks.closed
        );
    }

    #[test]
    fn heading_line_freezes_immediately() {
        let a = parse_tui_body_chunks("# Title\nmore", false);
        assert!(
            a.closed.iter().any(|c| c.contains("Title")),
            "heading should freeze, got {:?}",
            a.closed
        );
        assert_eq!(a.open_plain.as_deref(), Some("more"));
    }

    #[test]
    fn open_tilde_fence_stays_plain_until_closed() {
        let h = render_tui_block_markdown("~~~\nlet x = 1;\n", false);
        assert!(h.contains("let x = 1;"), "got {h}");
        assert!(
            !h.contains("<code"),
            "open tilde fence should stay plain, got {h}"
        );
        assert!(h.contains("chat-tui-line--plain"), "got {h}");
    }

    #[test]
    fn closed_tilde_fence_renders_code() {
        let h = render_tui_block_markdown("~~~\nlet x = 1;\n~~~\n", false);
        assert!(h.contains("<code") || h.contains("<pre"), "got {h}");
        assert!(h.contains("let x = 1;"), "got {h}");
    }

    #[test]
    fn blockquote_does_not_merge_with_following_paragraph() {
        let chunks = parse_tui_body_chunks("> quote\n\npara\n\n", true);
        assert!(
            chunks.closed.iter().any(|c| c.contains("<blockquote")),
            "got {:?}",
            chunks.closed
        );
        assert!(
            chunks
                .closed
                .iter()
                .any(|c| c.contains("para") && !c.contains("<blockquote")),
            "paragraph should be its own block, got {:?}",
            chunks.closed
        );
    }
}

#[cfg(test)]
#[path = "tui_line_markdown_tests.rs"]
mod think_patch_tests;
