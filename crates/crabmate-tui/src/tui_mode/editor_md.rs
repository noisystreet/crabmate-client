//! `/editor` 的 markdown 预览（`.md` / `.markdown` 自动进入，`/editor --md` 可强制）。
//!
//! **复用的是聊天区那三个纯函数**，不是整条渲染链路：
//! `md::assistant_styled`（共享规范化 + 行内 token）→ `md::wrap_styled_rows`（按显示宽度
//! 折行）→ `md::styled_row_spans`（带样式字符流 → span）。这三层不碰 transcript 状态，
//! 换一个文本源就能用。
//!
//! **不复用**的是 `render::line_rows` 与 `body_cache`：前者的入参是 `LogLine`、出参带
//! `log_index` 回指 transcript（这里渲染的是一个文件）；后者按「一条逻辑行一行、只有末行
//! 在流式变化」做逐行缓存，而文件是一次整份到手、宽度或主题一变就全部作废。所以缓存
//! 由本文件自己持有（见 [`MdRows`]）。
//!
//! 也因此 **md 模式不画行号**：围栏行不上屏、代码块插语言标签行、连续空行折叠、一行源码
//! 可能折成多物理行 —— 物理行与源码行不再一一对应，行号只会指错地方。要行号就回源码视图
//! （`/editor --src`）。

use ratatui::style::Style;
use ratatui::text::Line;

use super::editor_view::EditorState;
use super::md::{self, WrappedRow};
use super::palette::Palette;

/// md 预览最多解析多少**源码行**。
///
/// 渲染结果是逐字符的 [`WrappedRow`]（每个字符带一份 `Style`）：源码一万行折行后能到几万
/// 物理行、几十 MB，而 md 预览的用途是「看一眼这份文档长什么样」，不是逐行校对。超出的
/// 部分在提示行明说 —— 静默只渲染前 N 行，看起来就像文件就这么长。
pub(super) const MD_MAX_SOURCE_LINES: usize = 4_000;

/// md 预览的渲染缓存。
///
/// 键是 [`Self::width`] + [`Self::palette`] + [`Self::rev`] 三者：折行结果依赖宽度、
/// 颜色依赖主题（换主题要重算样式）、内容变了整段都得重来。缓存是必需的而不是优化 ——
/// 每帧把整份文档重新解析一遍，在长 README 上直接看得出来。
#[derive(Debug)]
pub(super) struct MdRows {
    /// 折行用的宽度（已保证 ≥ 1）。
    pub width: usize,
    /// 生成这些行时用的配色板。
    pub palette: Palette,
    /// 生成这些行时的内容版本（[`EditorState::text_rev`]）。
    pub rev: u64,
    /// 折行后的物理行。
    pub rows: Vec<WrappedRow>,
    /// 实际参与渲染的源码行数。
    pub source_lines: usize,
    /// 文件本身有多少源码行（截断前后都要能说出来）。
    pub total_lines: usize,
}

impl MdRows {
    /// 是否被 [`MD_MAX_SOURCE_LINES`] 截断。
    pub fn is_cut(&self) -> bool {
        self.source_lines < self.total_lines
    }
}

/// 保证缓存与当前（内容 / 宽度 / 主题）一致，不一致就重算。
fn ensure_rows(ed: &mut EditorState, width: usize, pal: &Palette) {
    // 宽度 0 会让 `wrap_styled_rows` 把每个字符单独成行（一帧就几万行），这里统一兜到 1。
    let width = width.max(1);
    let fresh = ed
        .md_cache
        .as_ref()
        .is_some_and(|c| c.width == width && c.palette == *pal && c.rev == ed.text_rev);
    if fresh {
        return;
    }
    let total_lines = ed.lines.len();
    let source_lines = total_lines.min(MD_MAX_SOURCE_LINES);
    // 只喂参与渲染的那一段：`assistant_styled` 是整段解析，多喂的行照样要花时间。
    let text = ed.lines[..source_lines].join("\n");
    let styled = md::assistant_styled(&text, pal, width);
    let rows = md::wrap_styled_rows(&styled, width);
    ed.md_cache = Some(MdRows {
        width,
        palette: *pal,
        rev: ed.text_rev,
        rows,
        source_lines,
        total_lines,
    });
}

/// 内容区的可见行（无行号栏，宽度全给正文）。
///
/// `rows` 是内容区能画几行；滚动位置由调用方（[`EditorState::md_scroll`]）持有，这里只做
/// 末页夹取 —— 上界按**物理行**算，与源码视图按源码行算不是一回事。
pub(super) fn md_body(
    ed: &mut EditorState,
    width: usize,
    rows: usize,
    pal: &Palette,
) -> Vec<Line<'static>> {
    ensure_rows(ed, width, pal);
    // 查找的命中按**物理行**算，物理行只有这一步之后才存在（见 `EditorState::sync_find`）。
    ed.sync_find();
    let Some(cache) = ed.md_cache.as_ref() else {
        return Vec::new();
    };
    let start = ed.md_scroll.min(cache.rows.len().saturating_sub(rows));
    let base = Style::new().fg(pal.fg);
    cache
        .rows
        .iter()
        .enumerate()
        .skip(start)
        .take(rows)
        .map(|(i, row)| {
            // 链接**这轮不接热区**：样式照常（链接色），但点击不响应。与 `plain_body_rows`
            // 里那条「已知差异」同一口径 —— 宁可先说清楚，也不做一个点了没反应的热区。
            let (matched, anchor) = (ed.find.is_hit(i), ed.find.is_anchor(i));
            Line::from(md::styled_row_spans(&row.chars, base, matched, anchor, pal))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::LaneApp;
    use super::super::editor_view::EditorMode;
    use super::super::test_support::{test_app, test_client, test_prefs};
    use super::MD_MAX_SOURCE_LINES;

    fn app(f: impl FnOnce(&mut LaneApp<'_>)) {
        let client = test_client();
        let mut prefs = test_prefs();
        let (tx, rx) = std::sync::mpsc::channel();
        let (job_tx, _job_rx) = std::sync::mpsc::channel();
        let mut app = test_app(&client, &mut prefs, rx, tx, job_tx);
        f(&mut app);
    }

    /// 视图的整段文本（标题 / 内容 / 提示都在里面）。
    fn view_text(a: &mut LaneApp<'_>, width: usize, height: usize) -> String {
        super::super::editor_view::view_lines(&mut a.st.editor, None, width, height, &a.st.palette)
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// 扩展名决定模式；大小写不敏感。
    #[test]
    fn the_extension_picks_the_preview_mode() {
        assert_eq!(EditorMode::for_path("README.md"), EditorMode::Markdown);
        assert_eq!(
            EditorMode::for_path("docs/a.markdown"),
            EditorMode::Markdown
        );
        assert_eq!(EditorMode::for_path("docs/README.MD"), EditorMode::Markdown);
        assert_eq!(EditorMode::for_path("src/lib.rs"), EditorMode::Source);
        // 没有扩展名的文件（Makefile / LICENSE）走源码视图
        assert_eq!(EditorMode::for_path("Makefile"), EditorMode::Source);
        assert_eq!(EditorMode::for_path("a/b.md/inner.rs"), EditorMode::Source);
    }

    /// `--src` / `--md` 覆盖扩展名判定，且路径照常归一化。
    #[test]
    fn explicit_switches_override_the_extension() {
        app(|a| {
            a.editor_command(Some("--src README.md".to_string()));
            assert_eq!(a.st.editor.mode, EditorMode::Source);

            a.editor_command(Some("--md src/lib.rs".to_string()));
            assert_eq!(a.st.editor.mode, EditorMode::Markdown);
            assert_eq!(a.st.editor.rel.as_deref(), Some("src/lib.rs"));
        });
    }

    /// md 预览不画行号栏，且连续空行折叠 —— 这两条正是「不能照搬源码视图」的地方。
    #[test]
    fn the_preview_drops_the_gutter_and_folds_blank_lines() {
        app(|a| {
            a.st.editor.open_file("README.md", 0);
            a.st.editor
                .accept_limited("# 标题\n\n\n\n正文 **粗** 与 `code`\n", 100);
            let text = view_text(a, 60, 20);
            assert!(!text.contains("│ "), "md 预览不该有行号栏：{text}");
            assert!(text.contains("标题"), "{text}");
            assert!(text.contains("粗"), "{text}");
            // 围栏标记之外的连续空行折成一个段落间隔，不能照抄源码的 3 个空行
            assert!(!text.contains("\n\n\n"), "连续空行应折叠：{text}");
        });
    }

    /// 滚动按**渲染后的物理行**计数：折行让物理行多于源码行。
    #[test]
    fn scrolling_counts_rendered_rows_not_source_lines() {
        app(|a| {
            a.st.editor.open_file("a.md", 0);
            // 每行 20 列宽的文字，在 10 列宽下折成 2 行 → 物理行数翻倍
            let body: String = (0..10)
                .map(|_| "0123456789012345678\n".to_string())
                .collect();
            a.st.editor.accept_limited(&body, 100);
            let _ = view_text(a, 10, 6);
            let rows = a.st.editor.md_rows_len();
            assert_eq!(a.st.editor.lines.len(), 10, "源码还是 10 行");
            assert!(rows > 10, "折行后物理行应多于源码行：{rows}");
            a.st.editor.scroll_to_bottom();
            assert_eq!(
                a.st.editor.md_scroll,
                rows - 1,
                "End 要停在最后一条物理行上"
            );
        });
    }

    /// 缓存键：同宽同主题命中，宽度变了要重算（否则折行位置停在旧宽度上）。
    #[test]
    fn the_cache_follows_width_and_content() {
        app(|a| {
            a.st.editor.open_file("a.md", 0);
            let body: String = (0..8).map(|i| format!("第{i}行文字很长很长\n")).collect();
            a.st.editor.accept_limited(&body, 100);
            let _ = view_text(a, 40, 10);
            let wide = a.st.editor.md_rows_len();
            let rev = a.st.editor.text_rev;

            // 同宽再来一帧：命中缓存（内容版本没变）
            let _ = view_text(a, 40, 10);
            assert_eq!(a.st.editor.text_rev, rev, "渲染不该改内容版本");
            assert_eq!(a.st.editor.md_rows_len(), wide);

            // 变窄：折行更多，行数必须跟着变
            let _ = view_text(a, 12, 10);
            assert!(
                a.st.editor.md_rows_len() > wide,
                "窄宽度下折行更多（{} vs {wide}）",
                a.st.editor.md_rows_len()
            );

            // 换内容：缓存作废，按新内容的行数重算
            a.st.editor.accept_limited("只有一行\n", 100);
            assert_ne!(a.st.editor.text_rev, rev, "装进新内容要推进版本");
            let _ = view_text(a, 40, 10);
            assert_eq!(a.st.editor.md_rows_len(), 1);
        });
    }

    /// 超上限只渲染前 [`MD_MAX_SOURCE_LINES`] 行，并且这件事要能被提示行说出来。
    #[test]
    fn an_over_long_file_is_cut_and_says_so() {
        let mut ed = super::EditorState::default();
        ed.open_file("big.md", 0);
        let body: String = (0..MD_MAX_SOURCE_LINES + 10)
            .map(|i| format!("行{i}\n"))
            .collect();
        ed.accept_limited(&body, MD_MAX_SOURCE_LINES + 10);
        let cache = ed.md_cache.as_ref();
        assert!(cache.is_none(), "还没渲染过就没有缓存");
        let pal = super::Palette::default();
        super::ensure_rows(&mut ed, 40, &pal);
        let cache = ed.md_cache.as_ref().expect("渲染后应有缓存");
        assert_eq!(cache.source_lines, MD_MAX_SOURCE_LINES);
        assert!(cache.is_cut(), "超过上限要标记为截断");
    }
}
