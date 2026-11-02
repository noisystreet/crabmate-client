//! 只读编辑器视图：`/editor` 进、`/chat` 回（issue #418）。
//!
//! 三条设计约束（每一条都是为了不把另一个问题引进来）：
//!
//! 1. **占 body 区，不占整屏**：composer 留着，`/chat` 与 `/editor <另一个文件>` 才能直接敲，
//!    该 lane 的在途回合也照常可见 —— 切视图不打断回合，与「切 lane 只改 `active`」同一原则。
//! 2. **状态随 `UiState` 走 = 每条 lane 一份**：多 lane 各绑不同 workspace，视图内容必须与 lane
//!    绑定，否则切 lane 会看到别人工作区的文件。回包还带 `generation`，中途切工作区后旧内容
//!    必须丢弃（与目录缓存同一口径）。
//! 3. **只读**：不做保存 / 编辑。写回涉及变更集与冲突语义，那是 Desktop IDE 的范围，
//!    在终端里放一个半成品编辑器只会让人误以为能存。
//!
//! 单独成文件也是为了不挤 `render.rs`（920 行门禁）与 `mod.rs`。

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::LaneApp;
use super::palette::Palette;
use super::state::{Focus, LineKind};
use crate::serve::{FILE_VIEW_MAX_LINES, WorkspaceFile};

/// 一个制表位展开成几个空格。
///
/// 按 ANSI 惯例取 4：8 在窄的右栏旁边会让缩进吃掉半屏，而 4 与仓库里 Rust / TS 的实际缩进
/// 也对得上（不引入「看起来错位」这种最难查的观感问题）。
const TAB_WIDTH: usize = 4;

/// 滚轮一格滚几行。
///
/// 与聊天区 transcript 的滚轮步长取同一个数：终端里的滚轮事件是离散的，一格只走一行
/// 读起来像「滚了没动」，一整屏又太粗 —— 一次跳过去就找不回刚看到的那一行。
const WHEEL_LINES: usize = 3;

/// 视图自身的行数下限（标题 / 内容 / 提示）：再矮就没有内容可言，渲染层保证不低于它。
pub(super) const MIN_VIEW_ROWS: usize = 3;

/// 只读文件视图的状态（每条 lane 一份，随 [`super::state::UiState`] 走）。
#[derive(Debug, Default)]
pub(super) struct EditorState {
    /// 是否处于编辑器视图（`false` = 聊天）。
    pub open: bool,
    /// 当前文件相对路径（`None` = 还没打开过任何文件）。
    pub rel: Option<String>,
    /// 已按 `\n` 切好的行（`\r` 已去掉、tab 已展开）。
    pub lines: Vec<String>,
    /// 顶部行偏移（渲染前按视口收敛）。
    pub scroll: usize,
    /// 读取在途。
    pub loading: bool,
    /// 读取失败文案（成功时清空）。
    pub error: Option<String>,
    /// 是否因行数上限被截断（提示行要明说，不能静默）。
    pub truncated: bool,
    /// 发起读取时的 [`super::state::UiState::ws_generation`]。
    pub generation: u64,
}

impl EditorState {
    /// 进入视图并开始读某个文件。
    pub fn open_file(&mut self, rel: &str, generation: u64) {
        self.open = true;
        self.rel = Some(rel.to_string());
        self.lines.clear();
        self.scroll = 0;
        self.loading = true;
        self.error = None;
        self.truncated = false;
        self.generation = generation;
    }

    /// 内容回包（`generation` 不符时由调用方丢弃，这里只管装）。
    ///
    /// 行数按 [`FILE_VIEW_MAX_LINES`] 截断：整份文件留在内存里没有意义，而截断这件事必须在
    /// 提示行里说出来 —— 静默只显示前 N 行，看起来就像「文件就这么长」。
    pub fn accept(&mut self, text: &str) {
        self.accept_limited(text, FILE_VIEW_MAX_LINES);
    }

    /// [`Self::accept`] 的可测版本（上限可注入，测试不必造两万行）。
    fn accept_limited(&mut self, text: &str, max_lines: usize) {
        self.loading = false;
        self.error = None;
        let mut rows: Vec<String> = text
            .lines()
            .map(|l| expand_tabs(l.strip_suffix('\r').unwrap_or(l)))
            .collect();
        self.truncated = rows.len() > max_lines;
        rows.truncate(max_lines);
        self.lines = rows;
        self.scroll = 0;
    }

    /// 读取失败：视图留着（标题仍显示是哪个文件），内容区换成这一句。
    pub fn fail(&mut self, msg: String) {
        self.loading = false;
        self.lines.clear();
        self.scroll = 0;
        self.error = Some(msg);
    }

    /// 返回聊天视图。文件与滚动位置保留：再 `/editor` 进来还是同一处。
    pub fn close(&mut self) {
        self.open = false;
        self.loading = false;
    }

    /// 滚动 `delta` 行，夹在 `[0, 末行]`。
    ///
    /// 上界用「末行」而不是「末页」：允许把最后一行顶到屏幕最上方，看文件尾部时不必先猜
    /// 视口高度（`less` 的行为）。渲染层的 `visible` 会再做一次裁剪。
    pub fn scroll_by(&mut self, delta: isize) {
        let last = self.lines.len().saturating_sub(1);
        let next = self.scroll.saturating_add_signed(delta);
        self.scroll = next.min(last);
    }

    /// Home：回到文件顶部。
    pub fn scroll_to_top(&mut self) {
        self.scroll = 0;
    }

    /// End：跳到最后一行。
    pub fn scroll_to_bottom(&mut self) {
        self.scroll = self.lines.len().saturating_sub(1);
    }

    /// 内容区能显示几行（去掉标题与提示行）。
    fn visible_rows(height: usize) -> usize {
        height.saturating_sub(2).max(1)
    }

    /// 渲染前把 `scroll` 收敛到可达范围（文件变短、终端变矮之后）。
    fn clamped_scroll(&self, height: usize) -> usize {
        let rows = Self::visible_rows(height);
        let max_start = self.lines.len().saturating_sub(rows);
        self.scroll.min(max_start)
    }
}

/// 制表位展开（渲染期只剩等宽字符，列对齐才好算）。
fn expand_tabs(line: &str) -> String {
    if !line.contains('\t') {
        return line.to_string();
    }
    let mut out = String::with_capacity(line.len() + TAB_WIDTH);
    for ch in line.chars() {
        if ch == '\t' {
            out.push_str(&" ".repeat(TAB_WIDTH));
        } else {
            out.push(ch);
        }
    }
    out
}

/// 按显示宽度裁剪（宽字符不切半），超出补 `…`。
fn clip(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    let budget = width.saturating_sub(1);
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + w > budget {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push('…');
    out
}

/// 路径末段（标题里显示 workspace 用，与右栏标题同一口径）。
fn basename(path: &str) -> &str {
    path.trim_end_matches('/')
        .rsplit_once('/')
        .map_or(path, |(_, tail)| tail)
}

/// 生成视图的整段行（含标题与提示行）。
///
/// 标题里的 workspace 末段是**防串台的主要线索**：多 lane 各绑不同工作区，同一路径
/// （比如 `src/main.rs`）在两条 lane 里是完全不同的文件，标题不写就看不出自己看的是哪份。
pub(super) fn view_lines(
    ed: &EditorState,
    workspace: Option<&str>,
    width: usize,
    height: usize,
    pal: &Palette,
) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::with_capacity(height.max(MIN_VIEW_ROWS));
    let file = ed.rel.as_deref().unwrap_or("（未选文件）");
    let ws = workspace
        .map(basename)
        .filter(|s| !s.is_empty())
        .unwrap_or("-");
    out.push(Line::from(vec![
        Span::styled("◧ ", Style::default().fg(pal.info)),
        Span::styled(
            clip(file, width.saturating_sub(2)),
            Style::default().fg(pal.fg).add_modifier(Modifier::BOLD),
        ),
    ]));
    out.push(Line::from(Span::styled(
        clip(&format!("  工作区 {ws}"), width),
        Style::default().fg(pal.muted),
    )));

    let rows = EditorState::visible_rows(height);
    if let Some(err) = ed.error.as_deref() {
        out.push(Line::from(Span::styled(
            clip(&format!("  读取失败：{err}"), width),
            Style::default().fg(pal.err),
        )));
    } else if ed.loading {
        out.push(Line::from(Span::styled(
            "  正在读取…".to_string(),
            Style::default().fg(pal.muted),
        )));
    } else if ed.rel.is_none() {
        // 视图开了但还没选文件（`/editor` 无参首次进入）：这里必须说清「怎么进来」，
        // 不然一片空白会被读成「还是空文件」。两种进法分别对应命令与目录树。
        out.push(Line::from(Span::styled(
            clip(
                "  未选文件：/editor <工作区相对路径>，或在右栏目录树上双击文件行",
                width,
            ),
            Style::default().fg(pal.muted),
        )));
    } else if ed.lines.is_empty() {
        out.push(Line::from(Span::styled(
            "  （空文件）".to_string(),
            Style::default().fg(pal.muted),
        )));
    } else {
        let start = ed.clamped_scroll(height);
        let gutter = ed.lines.len().to_string().len().max(3);
        for (i, text) in ed.lines.iter().enumerate().skip(start).take(rows) {
            let no = format!("{:>gutter$} ", i + 1, gutter = gutter);
            out.push(Line::from(vec![
                Span::styled(no, Style::default().fg(pal.muted)),
                Span::styled("│ ", Style::default().fg(pal.border)),
                Span::styled(
                    clip(text, width.saturating_sub(gutter + 2)),
                    Style::default().fg(pal.fg),
                ),
            ]));
        }
    }

    // 提示行：告诉人「怎么回去」。这是第一版唯一的键位来源（`/help` 里也有一行）。
    let mut hint =
        String::from("PgUp/PgDn · 滚轮/Alt+↑↓ 滚动 · /chat 或 Esc 返回 · /editor <路径>");
    if ed.truncated {
        hint.push_str(" · 已截断");
    }
    out.push(Line::from(Span::styled(
        clip(&hint, width),
        Style::default().fg(pal.muted),
    )));
    out
}

impl LaneApp<'_> {
    /// `/editor [<相对路径>]`：进入视图；给了路径就直接打开。
    pub(super) fn editor_command(&mut self, arg: Option<String>) {
        match arg {
            Some(rel) => self.open_editor(&rel),
            None => match self.st.editor.rel.clone() {
                // 再进来时重读一次：文件可能已经被 agent 改过，显示旧内容是更坏的选择。
                Some(rel) => {
                    self.st.editor.open_file(&rel, self.st.ws_generation);
                    let _ = self.fetch_ws_file(rel);
                }
                // 还没打开过任何文件：**照样把视图打开**，占位文案由 `view_lines` 给。
                //
                // 以前这里只往聊天区塞一行用法提示、视图保持关闭，看起来就是「/editor
                // 没生效」；更要紧的是树上双击/Enter 打开文件只在视图开着时才生效，
                // 于是用户先 `/editor`、再双击文件行 -- 两步都没有反应。而那一行提示
                // 本身也看不见：聊天区正被视图占着的话，提示落在被遮住的那一栏。
                None => self.st.editor.open = true,
            },
        }
    }

    /// 打开某个文件（`/editor <rel>`、目录树 Enter）。
    ///
    /// 只接受工作区相对路径：`/editor /etc/passwd` 这类绝对路径交给 serve 判（它会拒绝），
    /// 客户端不自己做前缀检查 —— 那条规则在 Server，抄一份只会两边漂移。
    pub(super) fn open_editor(&mut self, rel: &str) {
        let rel = normalize_rel(rel);
        if rel.is_empty() {
            self.st
                .push_line(LineKind::System, "用法：/editor <工作区相对路径>");
            return;
        }
        self.st.editor.open_file(&rel, self.st.ws_generation);
        let _ = self.fetch_ws_file(rel);
    }

    /// 滚轮：一格滚 [`WHEEL_LINES`] 行。
    ///
    /// 视图开着时它占的正是聊天列，滚轮继续滚 transcript 只能表现为「滚了没反应」——
    /// 那部分内容被视图整片盖住。步长与聊天区滚轮取同一个数，上下界沿用
    /// [`EditorState::scroll_by`]（末行可顶到顶部，`less` 的行为）。
    pub(super) fn editor_wheel(&mut self, up: bool) {
        let step = WHEEL_LINES as isize;
        self.st.editor.scroll_by(if up { -step } else { step });
    }

    /// `/chat`：回到聊天视图（幂等）。
    pub(super) fn editor_close(&mut self) {
        if self.st.editor.open {
            self.st.editor.close();
            // 焦点交回输入框：切回来就是为了继续打字。
            self.st.focus = Focus::Input;
            self.st
                .push_line(LineKind::System, "已回到聊天视图（/editor 再次打开）");
        } else {
            self.st.push_line(LineKind::System, "已在聊天视图");
        }
    }

    /// 文件内容回包。
    ///
    /// 只认「当前视图正在看的那一份」：读取在途时又打开了另一个文件（`/editor b.rs`），
    /// a.rs 的迟到回包不能覆盖 b.rs —— 判据是 `rel` 与当前视图一致，而不是「最近发出的请求」。
    pub(super) fn on_workspace_file(&mut self, rel: &str, result: Result<WorkspaceFile, String>) {
        if self.st.editor.rel.as_deref() != Some(rel) {
            return;
        }
        match result {
            Ok(f) => self
                .st
                .editor
                .accept(f.content.as_deref().unwrap_or_default()),
            Err(e) => self.st.editor.fail(e),
        }
    }
}

/// 归一化用户敲进来的路径：去首尾空白、去 `./` 前缀与尾部 `/`。
///
/// 不做 `..` 折叠：越权判定在 serve，客户端折叠过之后反而可能与服务端解析出不同的文件。
fn normalize_rel(raw: &str) -> String {
    let trimmed = raw.trim();
    let trimmed = trimmed.strip_prefix("./").unwrap_or(trimmed);
    trimmed.trim_end_matches('/').to_string()
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::super::test_support::{test_app, test_client, test_prefs};
    use super::{EditorState, LaneApp, MIN_VIEW_ROWS, WHEEL_LINES, basename, clip, normalize_rel};
    use crate::serve::WorkspaceFile;

    /// 外壳层的最小夹具：编辑器状态与命令都在 `LaneApp` 上，只测 `EditorState` 会漏掉
    /// 「命令到底有没有接上」这一层。
    fn app(f: impl FnOnce(&mut LaneApp<'_>)) {
        let client = test_client();
        let mut prefs = test_prefs();
        let (tx, rx) = mpsc::channel();
        let (job_tx, _job_rx) = mpsc::channel();
        let mut app = test_app(&client, &mut prefs, rx, tx, job_tx);
        f(&mut app);
    }

    #[test]
    fn editor_command_opens_and_chat_returns() {
        app(|a| {
            a.editor_command(Some("src/lib.rs".to_string()));
            assert!(a.st.editor.open);
            assert_eq!(a.st.editor.rel.as_deref(), Some("src/lib.rs"));
            // 没有 tokio runtime（单测）时停在 loading：不是假成功，也不是空文件
            assert!(a.st.editor.loading);

            a.editor_close();
            assert!(!a.st.editor.open);
            assert!(matches!(a.st.focus, super::super::state::Focus::Input));

            // 幂等：已经在聊天时再 /chat 只是多说一句，不改状态
            a.editor_close();
            assert!(!a.st.editor.open);
        });
    }

    #[test]
    fn a_reply_for_a_file_we_already_left_is_dropped() {
        app(|a| {
            a.editor_command(Some("a.rs".to_string()));
            a.editor_command(Some("b.rs".to_string()));
            a.on_workspace_file(
                "a.rs",
                Ok(WorkspaceFile {
                    content: Some("OLD".to_string()),
                    error: None,
                }),
            );
            assert_eq!(a.st.editor.rel.as_deref(), Some("b.rs"));
            assert!(
                a.st.editor.lines.is_empty(),
                "a.rs 的迟到回包不能写进 b.rs 的视图"
            );
        });
    }

    #[test]
    fn opening_a_blank_path_does_not_switch_the_view() {
        app(|a| {
            a.editor_command(Some("   ".to_string()));
            assert!(!a.st.editor.open, "空路径只该给用法提示");
        });
    }

    /// 只输入 `/editor`（这条 lane 还没打开过文件）也要进视图。
    ///
    /// 只往聊天区塞一行用法提示的话，用户看到的是「命令没生效」；更糟的是树上
    /// 双击/Enter 打开文件只在视图开着时才生效，于是他接着点也没反应。
    #[test]
    fn a_bare_editor_command_opens_the_view_without_a_file() {
        app(|a| {
            a.editor_command(None);
            assert!(a.st.editor.open, "只输入 /editor 也要进视图");
            assert!(a.st.editor.rel.is_none(), "没有文件就不该编一个路径出来");
            let rows = super::view_lines(&a.st.editor, None, 60, 20, &a.st.palette);
            let text = rows
                .iter()
                .map(std::string::ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n");
            assert!(text.contains("未选文件"), "占位行要说清怎么选文件：{text}");
            assert!(!text.contains("空文件"), "没选文件不能报「空文件」：{text}");
        });
    }

    #[test]
    fn paths_are_normalized_before_the_request() {
        assert_eq!(normalize_rel("  ./src/lib.rs  "), "src/lib.rs");
        assert_eq!(normalize_rel("src/"), "src");
        // `..` 不做折叠：越权判定在 serve，客户端折叠后可能与服务端解析出不同的文件
        assert_eq!(normalize_rel("../x"), "../x");
    }

    #[test]
    fn tabs_expand_and_cr_is_dropped() {
        let mut ed = EditorState::default();
        ed.open_file("src/lib.rs", 7);
        ed.accept_limited("fn main() {\r\n\tlet x = 1;\r\n}\r\n", 100);
        assert_eq!(ed.lines[0], "fn main() {");
        // tab 展开成 4 空格，行尾 `\r` 不留
        assert_eq!(ed.lines[1], "    let x = 1;");
        assert_eq!(ed.lines[2], "}");
        assert!(!ed.loading && ed.error.is_none() && !ed.truncated);
    }

    #[test]
    fn over_limit_is_truncated_and_says_so() {
        let mut ed = EditorState::default();
        ed.accept_limited("a\nb\nc\nd\n", 2);
        assert_eq!(ed.lines, vec!["a".to_string(), "b".to_string()]);
        assert!(ed.truncated, "截断要能被提示行说出来");
    }

    #[test]
    fn scroll_clamps_within_file() {
        let mut ed = EditorState::default();
        ed.accept_limited("a\nb\nc\n", 100);
        ed.scroll_by(-5);
        assert_eq!(ed.scroll, 0, "不能滚到负偏移");
        ed.scroll_by(99);
        assert_eq!(ed.scroll, 2, "上界是末行");
        ed.scroll_to_top();
        assert_eq!(ed.scroll, 0);
        ed.scroll_to_bottom();
        assert_eq!(ed.scroll, 2);
    }

    #[test]
    fn failure_clears_content_but_keeps_the_file_name() {
        let mut ed = EditorState::default();
        ed.open_file("bin/blob", 1);
        ed.accept("旧内容");
        ed.fail("二进制文件不支持预览".to_string());
        assert!(ed.lines.is_empty());
        assert_eq!(ed.rel.as_deref(), Some("bin/blob"));
        assert!(ed.error.as_deref().unwrap_or("").contains("二进制"));
        assert!(!ed.loading);
    }

    #[test]
    fn close_keeps_the_cursor_for_the_next_visit() {
        let mut ed = EditorState::default();
        ed.accept_limited("a\nb\nc\n", 100);
        ed.scroll_by(2);
        ed.close();
        assert!(!ed.open);
        assert_eq!(ed.scroll, 2, "再 /editor 回来还应停在同一处");
    }

    #[test]
    fn clip_keeps_wide_chars_whole() {
        assert_eq!(clip("abc", 5), "abc");
        assert_eq!(clip("abcdef", 4), "abc…");
        // 全角按 2 列算：3 个全角 = 6 列，宽度 4 只能放 1 个 + 省略号
        assert_eq!(clip("笔记说明", 4), "笔…");
    }

    #[test]
    fn basename_matches_the_right_pane_title() {
        assert_eq!(basename("/home/u/proj"), "proj");
        assert_eq!(basename("/home/u/proj/"), "proj");
        assert_eq!(basename("proj"), "proj");
    }

    /// 渲染后的整段文本（标题、行号、提示行都在里面）。
    fn view_text(a: &LaneApp<'_>, width: usize, height: usize) -> String {
        super::view_lines(&a.st.editor, None, width, height, &a.st.palette)
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// 行号列画的是**绝对行号**，滚动之后跟着各自的内容走。
    ///
    /// 两种坏法都能被这条抓住：没画行号（整列消失），以及滚动后按可见序号重新编号
    /// （从 1 数起 —— 看文件尾部时会误以为自己在文件开头）。
    #[test]
    fn line_numbers_are_absolute_and_follow_the_scroll() {
        app(|a| {
            a.st.editor.open_file("src/lib.rs", 0);
            a.st.editor.accept_limited("l1\nl2\nl3\nl4\n", 100);
            // 视口 5 行 = 3 行内容：滚 3 行后被收敛到 1，画面是 l2 / l3 / l4
            a.editor_wheel(false);
            let text = view_text(a, 60, 5);
            assert!(text.contains("  4 │ l4"), "行号要跟着内容走：{text}");
            assert!(text.contains("  2 │ l2"), "{text}");
            assert!(!text.contains("  1 │ l1"), "滚出去的行不该还在：{text}");
        });
    }

    /// 滚轮一格 [`WHEEL_LINES`] 行，顶部与末尾都夹住。
    #[test]
    fn the_wheel_moves_three_lines_and_clamps_at_both_ends() {
        app(|a| {
            a.st.editor.open_file("a.rs", 0);
            let body: String = (1..=40).map(|i| format!("l{i}\n")).collect();
            a.st.editor.accept_limited(&body, 100);
            a.editor_wheel(false);
            assert_eq!(a.st.editor.scroll, WHEEL_LINES, "一格 = 3 行");
            a.editor_wheel(true);
            assert_eq!(a.st.editor.scroll, 0);
            a.editor_wheel(true);
            assert_eq!(a.st.editor.scroll, 0, "顶部再往上不越界");
            for _ in 0..40 {
                a.editor_wheel(false);
            }
            assert_eq!(a.st.editor.scroll, 39, "末尾夹在末行");
        });
    }

    #[test]
    fn view_rows_never_collapse_below_one() {
        // 高度只够标题 + 提示时，内容区仍是 1 行而不是 0（否则整屏像是空的）
        assert_eq!(EditorState::visible_rows(MIN_VIEW_ROWS), 1);
        assert_eq!(EditorState::visible_rows(0), 1);
        assert_eq!(EditorState::visible_rows(10), 8);
    }
}
