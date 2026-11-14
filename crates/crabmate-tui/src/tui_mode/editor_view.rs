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
use super::md;
use super::palette::Palette;
use super::state::{Focus, LineKind};
use crate::serve::{FILE_VIEW_MAX_LINES, WorkspaceFile};

// 文件内查找（`Ctrl+F`）单独成文件，理由与 `editor_md` 相同：本文件与 `md.rs` 都贴着
// `fn-nloc` 的 920 行/文件门禁。挂在这里（而不是 `mod.rs`）也是这个原因 —— `mod.rs`
// 只剩几行余量。
#[path = "editor_find.rs"]
pub(super) mod editor_find;

// 词法分类器（issue #510 的 P1）：纯逻辑，挂在这里而不是 `mod.rs`——后者只剩 0 行余量。
#[path = "editor_hl.rs"]
pub(super) mod editor_hl;

use self::editor_find::FindState;
use self::editor_hl::{HlSpan, LexState};

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

/// 视图的渲染模式。
///
/// 由扩展名决定（`.md` / `.markdown` 走预览），`/editor --md|--src` 可覆盖。两种模式的
/// **行不是一回事**：源码模式一行 = 文件一行、带行号、不折行；md 预览一行 = 折行后的物理
/// 行、无行号。行号与语义渲染无法兼得（理由见 `editor_md`），所以这里是显式的二选一，
/// 而不是一个 `is_markdown` 布尔旗标散在滚动逻辑里。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum EditorMode {
    /// 源码视图：带行号，超宽行裁切（`…`），滚动按源码行。
    #[default]
    Source,
    /// markdown 预览：按语义重排 + 折行，滚动按物理行。
    Markdown,
}

impl EditorMode {
    /// 按扩展名判定（大小写不敏感）；`md` 与 `markdown` 两种写法都收。
    pub(super) fn for_path(rel: &str) -> Self {
        let ext = rel.rsplit_once('.').map_or("", |(_, tail)| tail);
        if ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown") {
            Self::Markdown
        } else {
            Self::Source
        }
    }

    /// 标题里显示的模式名（两个模式都要写出来：不写就看不出为什么没有行号）。
    fn label(self) -> &'static str {
        match self {
            Self::Source => "源码",
            Self::Markdown => "md 预览",
        }
    }
}

/// 源码视图的语法高亮状态（缓存，见 [`EditorState::hl`]）。
///
/// 存「语言 + 每行**起始**词法态」而不是每行的 span：span 只在可见行上算，而起始态必须
/// 从头算（块注释、raw string、围栏都可能从任意前面的行跨过来）。这一份因此与视口无关，
/// 滚动时不必重算。
#[derive(Debug)]
pub(super) struct HlRows {
    /// 算这份缓存时的 [`EditorState::text_rev`]，与当前值不等即作废。
    rev: u64,
    /// 判定出的语言；`None` = 扩展名不认识，整份不上色。
    lang: Option<editor_hl::Language>,
    /// 每行起始词法态，长度 = `lines.len() + 1`（末项是收尾态，当前没人读，留着是为了
    /// 与 [`editor_hl::scan_states`] 的返回形状一致）。
    states: Vec<LexState>,
}

/// 只读文件视图的状态（每条 lane 一份，随 [`super::state::UiState`] 走）。
#[derive(Debug, Default)]
pub(super) struct EditorState {
    /// 是否处于编辑器视图（`false` = 聊天）。
    pub open: bool,
    /// 当前文件相对路径（`None` = 还没打开过任何文件）。
    pub rel: Option<String>,
    /// 已按 `\n` 切好的行（`\r` 已去掉、tab 已展开）。
    pub lines: Vec<String>,
    /// 源码视图的顶部行偏移（源码行；渲染前按视口收敛）。
    pub scroll: usize,
    /// 当前渲染模式（扩展名判定，`/editor --md|--src` 覆盖）。
    pub mode: EditorMode,
    /// md 预览的顶部行偏移（**物理行**）。
    ///
    /// 与 [`Self::scroll`] 分开存：两者的「一行」不是一回事，共用一个字段会在切模式时
    /// 把位置扔到不相干的地方（源码第 800 行 ≠ 第 800 条折行）。
    pub md_scroll: usize,
    /// 内容版本：每次装内容 / 失败都 +1，md 缓存据此判断要不要重算。
    pub text_rev: u64,
    /// md 预览缓存（`None` = 还没渲染过，或内容刚变）。
    pub md_cache: Option<super::editor_md::MdRows>,
    /// 源码视图的语法高亮状态（`None` = 还没算过，或内容刚变）。
    ///
    /// 与 [`Self::md_cache`] 分开：两者的键相同（`text_rev`），但 md 只在预览模式下建，
    /// 源码模式永远用不到它。共用一份字段会让「切模式」看起来像是在作废另一份缓存。
    pub hl: Option<HlRows>,
    /// 读取在途。
    pub loading: bool,
    /// 读取失败文案（成功时清空）。
    pub error: Option<String>,
    /// 是否因行数上限被截断（提示行要明说，不能静默）。
    pub truncated: bool,
    /// 发起读取时的 [`super::state::UiState::ws_generation`]。
    pub generation: u64,
    /// 文件内查找（`Ctrl+F`）。与聊天区 `/find` 是两份状态，见 [`editor_find`] 模块头。
    pub find: FindState,
    /// 本 lane 的名字，`None` = 单 lane（或还没算出来）。
    ///
    /// 只给第二行标题用：多 lane 时「这是哪条连接」是唯一线索（同一个相对路径在两条 lane 里
    /// 是两个不同的文件，各绑各的 workspace）。放 `EditorState` 而不是 `UiState` —— 读它的
    /// 只有这一处渲染，`view_lines` 因此不必多一个参数；由外壳每帧按 lane 快照刷新，口径唯一
    /// 在 [`super::lane_list::current_label`]。
    pub lane_label: Option<String>,
}

impl EditorState {
    /// 进入视图并开始读某个文件（模式按扩展名判定）。
    pub fn open_file(&mut self, rel: &str, generation: u64) {
        self.open_file_with_mode(rel, generation, None);
    }

    /// [`Self::open_file`] + 显式模式（`/editor --md|--src`）；`None` = 按扩展名自动。
    pub fn open_file_with_mode(&mut self, rel: &str, generation: u64, mode: Option<EditorMode>) {
        self.open = true;
        self.rel = Some(rel.to_string());
        self.lines.clear();
        self.scroll = 0;
        self.md_scroll = 0;
        self.mode = mode.unwrap_or_else(|| EditorMode::for_path(rel));
        self.loading = true;
        self.error = None;
        self.truncated = false;
        self.generation = generation;
        self.invalidate_caches();
    }

    /// 内容变了：两份渲染缓存都必须作废（否则会继续画上一份文件的 md 排版与语法色），
    /// 版本号推进用作缓存键。
    fn invalidate_caches(&mut self) {
        self.text_rev = self.text_rev.wrapping_add(1);
        self.md_cache = None;
        self.hl = None;
    }

    /// 内容回包（`generation` 不符时由调用方丢弃，这里只管装）。
    ///
    /// 行数按 [`FILE_VIEW_MAX_LINES`] 截断：整份文件留在内存里没有意义，而截断这件事必须在
    /// 提示行里说出来 —— 静默只显示前 N 行，看起来就像「文件就这么长」。
    pub fn accept(&mut self, text: &str) {
        self.accept_limited(text, FILE_VIEW_MAX_LINES);
    }

    /// [`Self::accept`] 的可测版本（上限可注入，测试不必造两万行）。
    pub(super) fn accept_limited(&mut self, text: &str, max_lines: usize) {
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
        self.md_scroll = 0;
        self.invalidate_caches();
    }

    /// 读取失败：视图留着（标题仍显示是哪个文件），内容区换成这一句。
    pub fn fail(&mut self, msg: String) {
        self.loading = false;
        self.lines.clear();
        self.scroll = 0;
        self.md_scroll = 0;
        self.error = Some(msg);
        self.invalidate_caches();
    }

    /// 返回聊天视图。文件与滚动位置保留：再 `/editor` 进来还是同一处。
    ///
    /// 查找**输入态**顺手关掉（查询词与高亮留着）：带着输入态离开视图，再进来敲的第一下
    /// 会悄悄进查找词而不是 composer —— 那时屏幕上没有任何东西提示这件事。
    pub fn close(&mut self) {
        self.open = false;
        self.loading = false;
        self.find.close();
    }

    /// 滚动 `delta` 行，夹在 `[0, 末行]`。
    ///
    /// 上界用「末行」而不是「末页」：允许把最后一行顶到屏幕最上方，看文件尾部时不必先猜
    /// 视口高度（`less` 的行为）。渲染层的 `visible` 会再做一次裁剪。
    ///
    /// 「一行」在两种模式下含义不同（源码行 / 折行后的物理行），由 [`Self::line_count`]
    /// 与 [`Self::scroll_slot`] 统一分派 —— 按键侧不必知道当前是哪种模式。
    pub fn scroll_by(&mut self, delta: isize) {
        let last = self.line_count().saturating_sub(1);
        let next = self.scroll_slot().saturating_add_signed(delta);
        *self.scroll_slot_mut() = next.min(last);
    }

    /// Home：回到顶部。
    pub fn scroll_to_top(&mut self) {
        *self.scroll_slot_mut() = 0;
    }

    /// End：跳到最后一行。
    pub fn scroll_to_bottom(&mut self) {
        let last = self.line_count().saturating_sub(1);
        *self.scroll_slot_mut() = last;
    }

    /// 当前模式下「一共多少行」：源码行 / md 物理行。
    ///
    /// md 的行数只有渲染过才知道（折行依赖宽度），所以缓存未建时算 0 —— 那只会发生在
    /// 「内容刚变、下一帧还没画」的一瞬，此时任何偏移都不该沿用旧值。
    fn line_count(&self) -> usize {
        match self.mode {
            EditorMode::Source => self.lines.len(),
            EditorMode::Markdown => self.md_rows_len(),
        }
    }

    /// md 预览有多少条物理行（还没渲染过就是 0）。
    pub fn md_rows_len(&self) -> usize {
        self.md_cache.as_ref().map_or(0, |c| c.rows.len())
    }

    /// 当前模式的偏移字段（源码行 / 物理行）。
    fn scroll_slot(&self) -> &usize {
        match self.mode {
            EditorMode::Source => &self.scroll,
            EditorMode::Markdown => &self.md_scroll,
        }
    }

    /// [`Self::scroll_slot`] 的可写版本。
    fn scroll_slot_mut(&mut self) -> &mut usize {
        match self.mode {
            EditorMode::Source => &mut self.scroll,
            EditorMode::Markdown => &mut self.md_scroll,
        }
    }

    /// 内容区能显示几行（去掉标题与提示行）。
    fn visible_rows(height: usize) -> usize {
        height.saturating_sub(2).max(1)
    }

    /// 内容区此刻有没有正文（读取中 / 读取失败 / 未选文件 / 空文件）。
    ///
    /// 与 [`content_lines`] 的 if 链一一对应；单独成方法只为让那条贴着 CCN 门禁的 if 链
    /// 不必再多一个条件。
    fn has_no_body(&self) -> bool {
        self.loading || self.error.is_some() || self.rel.is_none() || self.lines.is_empty()
    }

    /// 让查找的命中集合跟上当前（查询词 / 内容 / 模式 / 行数 / 折行宽度）。
    ///
    /// 两条调用路径都必须调：渲染前（画高亮要正确的行号）与按键跳转前（`Enter` 直接读
    /// `hits`）。md 预览的命中按**物理行**算，而物理行只有渲染过才知道 —— 缓存还没建时
    /// 直接返回、下一帧渲染时补上（按错的物理行跳比不跳更糟）。
    pub fn sync_find(&mut self) {
        match self.mode {
            EditorMode::Source => {
                let lines = &self.lines;
                let key = editor_find::FindKey {
                    rev: self.text_rev,
                    mode: self.mode,
                    rows: lines.len(),
                    width: 0,
                };
                self.find.sync(key, |i, needle| {
                    editor_find::line_matches(&lines[i], needle)
                });
            }
            EditorMode::Markdown => {
                let Some(cache) = self.md_cache.as_ref() else {
                    return;
                };
                // 宽度由缓存给出（`ensure_rows` 已把 0 兜到 1）：预览的行是折行折出来的，
                // 宽度变了而物理行数恰好不变时，只有它能说明命中集合已经过期。
                let key = editor_find::FindKey {
                    rev: self.text_rev,
                    mode: self.mode,
                    rows: cache.rows.len(),
                    width: cache.width,
                };
                // 每行的可见字符拼进**同一个** buffer 再判：几千条物理行逐行新建 `String`
                // 会在按键路径上分配几千次。
                let mut row_text = String::new();
                self.find.sync(key, |i, needle| {
                    let Some(row) = cache.rows.get(i) else {
                        return false;
                    };
                    row_text.clear();
                    row_text.extend(row.chars.iter().map(|(_, ch)| *ch));
                    editor_find::line_matches(&row_text, needle)
                });
            }
        }
    }

    /// 把某一行顶到视口中间附近（查找跳转用），两端夹在可达范围内。
    ///
    /// 居中而不是「顶到最上面」：找一处命中通常要看它**上下**的上下文，命中行贴着屏幕
    /// 顶边时上一行永远看不见。
    pub fn reveal_row(&mut self, row: usize, visible: usize) {
        let last = self.line_count().saturating_sub(1);
        let offset = visible.saturating_sub(1) / 2;
        *self.scroll_slot_mut() = row.saturating_sub(offset).min(last);
    }

    /// 渲染前把当前偏移收敛到可达范围（文件变短、终端变矮之后）。
    fn clamped_scroll(&self, height: usize) -> usize {
        let rows = Self::visible_rows(height);
        let max_start = self.line_count().saturating_sub(rows);
        (*self.scroll_slot()).min(max_start)
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
/// 标题还必须写出**当前模式**：md 预览没有行号栏，不说明的话那看起来就是「行号坏了」。
///
/// 取 `&mut` 是因为 md 预览要维护自己的渲染缓存（宽度 / 主题 / 内容版本作键）；源码视图
/// 走不到那条路径，但这个签名让调用点只有一种形态。
pub(super) fn view_lines(
    ed: &mut EditorState,
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
        clip(&title_line(ed.mode, ws, ed.lane_label.as_deref()), width),
        Style::default().fg(pal.muted),
    )));

    out.extend(content_lines(ed, width, height, pal));
    // 提示行：告诉人「怎么回去」，以及**怎么切模式**（否则「没有行号」只能靠猜）。
    out.push(hint_line(ed, width, pal));
    out
}

/// 第二行标题：`工作区 <末段> · <模式>`，多 lane 时再补 `lane <名字>`。
///
/// lane 名只在**多 lane** 时出现：单 lane 时它恒等于「当前这条」，写上去只占宽度。多 lane
/// 时它是「这是哪条连接」的唯一线索 —— 同一个相对路径在两条 lane 里是两个不同的文件
/// （各绑各的 workspace），缺了它就看不出自己看的是哪一份。
fn title_line(mode: EditorMode, ws: &str, lane: Option<&str>) -> String {
    let mut title = format!("  工作区 {ws} · {}", mode.label());
    if let Some(lane) = lane {
        title.push_str(&format!(" · lane {lane}"));
    }
    title
}

/// 内容区（标题与提示行之外）：读取态 / 未选文件 / 空文件 / 正文。
///
/// 从 [`view_lines`] 拆出来只为 CCN 门禁（整条 if 链 + 两种模式的渲染挤在一个函数里
/// 已经 11），顺序不变：占位文案必须先于正文判断，否则空文件会被读成「未选文件」。
fn content_lines(
    ed: &mut EditorState,
    width: usize,
    height: usize,
    pal: &Palette,
) -> Vec<Line<'static>> {
    let rows = EditorState::visible_rows(height);
    // 没有正文可搜时先把命中集合作废：提示行读它，残留会让「查找 foo 第 1 / 3 处」
    // 一直挂在读取中 / 失败的画面上（`hits` 空着说「无匹配」同样是错话，见 `showing`）。
    let no_body = ed.has_no_body();
    ed.find.set_body_absent(no_body);
    let mut out: Vec<Line<'static>> = Vec::new();
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
        match ed.mode {
            EditorMode::Source => {
                // 命中集合先跟上内容：高亮是按行号画的，行号错了就画在别的行上。
                ed.sync_find();
                source_body(ed, width, height, rows, pal, &mut out);
            }
            // md 预览：没有行号栏，整列宽度都归正文（折行在渲染层做，见 `editor_md`）。
            EditorMode::Markdown => out.extend(super::editor_md::md_body(ed, width, rows, pal)),
        }
    }
    out
}

/// 源码正文：绝对行号 + `│` + 按显示宽裁切、按语法类别分词的一行。
///
/// 行号是**绝对**的（滚动后跟着各自内容走）：按可见序号重编号会让人误以为自己在文件开头。
///
/// 取 `&mut` 只为维护 [`EditorState::hl`]：每帧重扫全文不现实，见 [`ensure_hl`]。
fn source_body(
    ed: &mut EditorState,
    width: usize,
    height: usize,
    rows: usize,
    pal: &Palette,
    out: &mut Vec<Line<'static>>,
) {
    ensure_hl(ed);
    let start = ed.clamped_scroll(height);
    let gutter = ed.lines.len().to_string().len().max(3);
    // 正文可用宽度：行号栏（`{:>gutter$}` 加一个分隔空格）之后就是 `│ `。
    let budget = width.saturating_sub(gutter + 2);
    // 语言与每行起始态都出自同一份缓存；`None` = 这个扩展名不上色。
    let lang = ed.hl.as_ref().and_then(|h| h.lang);
    for (i, text) in ed.lines.iter().enumerate().skip(start).take(rows) {
        let no = format!("{:>gutter$} ", i + 1, gutter = gutter);
        // 高亮走聊天区同一套判据（`md::highlight_override`）：命中行改 `hit` 前景、
        // 锚定行整行反色。行号一起染，命中行才在左栏也看得出来。
        let (matched, anchor) = (ed.find.is_hit(i), ed.find.is_anchor(i));
        let spans = match (lang, ed.hl.as_ref().and_then(|h| h.states.get(i))) {
            (Some(l), Some(state)) => editor_hl::classify_line(text, *state, l).0,
            _ => Vec::new(),
        };
        let mut line = Vec::with_capacity(2 + spans.len().max(1));
        line.push(Span::styled(
            no,
            md::highlight_override(Style::default().fg(pal.muted), matched, anchor, pal),
        ));
        line.push(Span::styled("│ ", Style::default().fg(pal.border)));
        line.extend(source_line_spans(
            text, &spans, budget, matched, anchor, pal,
        ));
        out.push(Line::from(line));
    }
}

/// 让语法高亮状态跟上当前内容（内容或语言变了就重扫）。
///
/// 这是本模块唯一的性能陷阱：`source_body` 每帧只画几十行，但**起始态**必须从头扫出来
/// （块注释 / raw string / 围栏可能从任意前面的行跨过来），而 [`FILE_VIEW_MAX_LINES`] 是
/// 2 万行量级、`classify_line` 每行还要 `Vec<char>` 分配一次。逐帧重扫会直接掉帧，所以按
/// [`EditorState::text_rev`] 缓存 —— 与 `md_cache` 同一口径，`invalidate_caches` 负责推进。
///
/// 语言一起存进缓存：两次 `open_file` 之间可能没有新内容（`text_rev` 相同），但 `rel` 变了
/// 就是另一个文件，不能沿用旧语言。
fn ensure_hl(ed: &mut EditorState) {
    let lang = ed.rel.as_deref().and_then(editor_hl::language_for_path);
    let fresh = ed
        .hl
        .as_ref()
        .is_some_and(|h| h.rev == ed.text_rev && h.lang == lang);
    if fresh {
        return;
    }
    let states = match lang {
        Some(l) => editor_hl::scan_states(&ed.lines, l),
        None => Vec::new(),
    };
    ed.hl = Some(HlRows {
        rev: ed.text_rev,
        lang,
        states,
    });
}

/// 把一行正文切成「按语法类别分段 + 按显示宽度裁切」的 span 序列。
///
/// 两件事必须**在同一次遍历**里做完：[`HlSpan`] 的区间单位是 **char**，而裁剪预算按
/// **显示宽度**算（宽字符占 2 列、`…` 占 1 列）。先 `clip()` 再按 char 切会因尾部追加了
/// `…` 而整体错位，先切段再各自裁又会算出互相矛盾的边界。
///
/// 连续同款的段会合并：一行切成几百个单字符 span 会让 ratatui 每帧多做几百次比对，也让
/// 「这一行几个 span」这类断言失去意义。
fn source_line_spans(
    text: &str,
    spans: &[HlSpan],
    width: usize,
    matched: bool,
    anchor: bool,
    pal: &Palette,
) -> Vec<Span<'static>> {
    let total = text.width();
    let clipped = total > width;
    // 裁掉时给 `…` 留一列：与 [`clip`] 同一口径，避免超宽行比正文区多出一列。
    let budget = if clipped {
        width.saturating_sub(1)
    } else {
        total
    };
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut cur: Option<(String, Style)> = None;
    let mut used = 0usize;
    let mut it = spans.iter().peekable();
    for (idx, ch) in text.chars().enumerate() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + w > budget {
            break;
        }
        // span 已按起点升序（词法器逐段推进产出），游标单向走到底即可。
        while it.peek().is_some_and(|s| s.end <= idx) {
            it.next();
        }
        let base = match it.peek().filter(|s| idx >= s.start) {
            Some(s) => pal.syntax_style(s.cat),
            // 没有类别就是正文色 —— 词法器的「认不出就不上色」在这里落地。
            None => Style::default().fg(pal.fg),
        };
        // 叠加顺序固定：语法色在前、查找命中在后。命中必须压过语法色。
        push_merged(
            &mut out,
            &mut cur,
            ch,
            md::highlight_override(base, matched, anchor, pal),
        );
        used += w;
    }
    if clipped {
        let style = md::highlight_override(Style::default().fg(pal.fg), matched, anchor, pal);
        push_merged(&mut out, &mut cur, '…', style);
    }
    if let Some((text, style)) = cur.take() {
        out.push(Span::styled(text, style));
    }
    out
}

/// 往「正在累积的段」上追加一个字符；款式变了就把上一段收进 `out`。
fn push_merged(
    out: &mut Vec<Span<'static>>,
    cur: &mut Option<(String, Style)>,
    ch: char,
    style: Style,
) {
    match cur {
        Some((text, s)) if *s == style => text.push(ch),
        _ => {
            if let Some((text, s)) = cur.take() {
                out.push(Span::styled(text, s));
            }
            *cur = Some((ch.to_string(), style));
        }
    }
}

/// 提示行：查找态 / 滚动键位 + 返回方式 + 切模式 + 两类截断说明。
fn hint_line(ed: &EditorState, width: usize, pal: &Palette) -> Line<'static> {
    // 查找输入态：整行让给它。此刻每个键都归查找，底下再写「Esc 返回聊天」只会让人误以为
    // Esc 是退视图（那是输入框为空时的事；查找态里 Esc 只关查找）。
    if let Some(prompt) = ed.find.prompt() {
        return Line::from(Span::styled(
            clip(&prompt, width),
            Style::default().fg(pal.hit),
        ));
    }
    // 命中读数排在**最前**：它是查找唯一的反馈（视图占着主区，写进 transcript 也看不见），
    // 而窄终端只会从尾部裁。
    let mut hint = ed.find.inline_note().unwrap_or_default();
    hint.push_str(match ed.mode {
        EditorMode::Source => {
            "PgUp/PgDn · 滚轮/Alt+↑↓ 滚动 · Ctrl+F 查找 · Ctrl+Y 复制路径 · /chat 或 Esc 返回 · /editor --md 预览"
        }
        EditorMode::Markdown => {
            "md 预览 · PgUp/PgDn · 滚轮/Alt+↑↓ 滚动 · Ctrl+F 查找 · Ctrl+Y 复制路径 · /chat 或 Esc 返回 · /editor --src 看源码"
        }
    });
    if ed.truncated {
        hint.push_str(" · 已截断");
    }
    // 预览自己的上限（[`super::editor_md::MD_MAX_SOURCE_LINES`]）：截断了就得说出来，
    // 否则「文档就这么长」和「只解析了前面一段」在屏幕上长得一样。
    if ed.mode == EditorMode::Markdown
        && let Some(cut) = ed.md_cache.as_ref().filter(|c| c.is_cut())
    {
        hint.push_str(&format!(" · 仅前 {} 行", cut.source_lines));
    }
    Line::from(Span::styled(
        clip(&hint, width),
        Style::default().fg(pal.muted),
    ))
}

impl LaneApp<'_> {
    /// `/editor [--md|--src] [<相对路径>]`：进入视图；给了路径就直接打开。
    ///
    /// 模式开关只认**第一个** token（见 [`parse_editor_arg`]），所以文件名里的 `--md`
    /// 不会被当成开关。
    pub(super) fn editor_command(&mut self, arg: Option<String>) {
        let (mode, target) = parse_editor_arg(arg.as_deref());
        // 「没给参数」与「给了参数但没路径」是两件事：
        // - `/editor`：沿用当前文件（或先把视图打开）；
        // - `/editor --md`：同上，但换模式；
        // - `/editor    `：**只有空白**，仍按用法错误处理（与改造前一致，别让它悄悄变成
        //   「打开视图」—— 输入法误触更容易发生在这条上）。
        let Some(target) = target else {
            return self.editor_reopen_current(mode);
        };
        if target.trim().is_empty() {
            if mode.is_none() {
                self.editor_usage();
                return;
            }
            return self.editor_reopen_current(mode);
        }
        self.open_editor_with_mode(&target, mode);
    }

    /// 无路径的 `/editor`：重读当前文件，或（还没打开过文件时）只把视图打开。
    ///
    /// 重读而不是沿用旧内容：文件可能已经被 agent 改过，显示旧内容是更坏的选择。
    fn editor_reopen_current(&mut self, mode: Option<EditorMode>) {
        match self.st.editor.rel.clone() {
            Some(rel) => {
                self.st
                    .editor
                    .open_file_with_mode(&rel, self.st.ws_generation, mode);
                let _ = self.fetch_ws_file(rel);
            }
            // 还没打开过任何文件：**照样把视图打开**，占位文案由 `view_lines` 给。
            //
            // 以前这里只往聊天区塞一行用法提示、视图保持关闭，看起来就是「/editor
            // 没生效」；更要紧的是树上双击/Enter 打开文件只在视图开着时才生效，
            // 于是用户先 `/editor`、再双击文件行 -- 两步都没有反应。而那一行提示
            // 本身也看不见：聊天区正被视图占着的话，提示落在被遮住的那一栏。
            None => {
                self.st.editor.open = true;
                // 开关先记下：真打开文件时由 `open_file` 按扩展名重判，
                // 所以这里只管「当前没有文件可切」时的显示。
                if let Some(m) = mode {
                    self.st.editor.mode = m;
                }
            }
        }
    }

    /// 用法提示（路径为空时）。
    fn editor_usage(&mut self) {
        self.st.push_line(
            LineKind::System,
            "用法：/editor [--md|--src] <工作区相对路径>",
        );
    }

    /// 打开某个文件（`/editor <rel>`、目录树 Enter）。
    ///
    /// 只接受工作区相对路径：`/editor /etc/passwd` 这类绝对路径交给 serve 判（它会拒绝），
    /// 客户端不自己做前缀检查 —— 那条规则在 Server，抄一份只会两边漂移。
    pub(super) fn open_editor(&mut self, rel: &str) {
        self.open_editor_with_mode(rel, None);
    }

    /// [`Self::open_editor`] + 显式模式（`None` = 按扩展名判定）。
    pub(super) fn open_editor_with_mode(&mut self, rel: &str, mode: Option<EditorMode>) {
        let rel = normalize_rel(rel);
        if rel.is_empty() {
            self.editor_usage();
            return;
        }
        // `None` 走不带覆盖的入口：模式按扩展名判定，那是唯一一处默认口径。
        if let Some(m) = mode {
            self.st
                .editor
                .open_file_with_mode(&rel, self.st.ws_generation, Some(m));
        } else {
            self.st.editor.open_file(&rel, self.st.ws_generation);
        }
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

    /// `Ctrl+T` / 点状态栏 `view` 段：聊天 ↔ 编辑器视图。
    ///
    /// 两个方向都复用现成入口，**不另写一份「打开」逻辑**：`/editor` 的语义（重读当前
    /// 文件、还没打开过文件时开一个占位视图、切视图不打断在跑的回合）就是唯一定义，
    /// 抄第二份的那天起两个入口就会给出不同结果。
    ///
    /// 关闭方向走 [`Self::editor_close`]，连带那行系统提示与「焦点交回输入框」——
    /// 按 Ctrl+T 回来本来就是接着打字。
    pub(super) fn toggle_view(&mut self) {
        if self.st.editor.open {
            self.editor_close();
        } else {
            self.editor_reopen_current(None);
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

/// 拆 `/editor` 的参数：`--md` / `--src` 是模式开关，剩下的是路径。
///
/// 只把**第一个** token 当开关：路径本身可能带空格，把开关判定限制在首位，文件名里的
/// `--md` 就不会被误当成开关。开关后面没有路径时返回空串，由调用方理解为「切当前文件」。
fn parse_editor_arg(raw: Option<&str>) -> (Option<EditorMode>, Option<String>) {
    let Some(raw) = raw else {
        return (None, None);
    };
    let mode = match raw.split_whitespace().next() {
        Some("--md") => Some(EditorMode::Markdown),
        Some("--src" | "--source") => Some(EditorMode::Source),
        _ => None,
    };
    match mode {
        // `split_whitespace` 会把多个空格压成一个，路径按单空格拼回（终端里手敲的路径
        // 本来也不会用连续空格），后面的 `normalize_rel` 再统一去空白。
        Some(_) => {
            let rest = raw.split_whitespace().skip(1).collect::<Vec<_>>().join(" ");
            (mode, Some(rest))
        }
        None => (None, Some(raw.to_string())),
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

// 单测单独成文件：本文件要同时守住 920 行/文件与「查找别挤进来」两条门禁，而这一块测试
// 只是读状态、与渲染无关（`#[path]` 只换文件，模块路径仍是 `editor_view::tests`）。
#[cfg(test)]
#[path = "editor_view_tests.rs"]
mod tests;
