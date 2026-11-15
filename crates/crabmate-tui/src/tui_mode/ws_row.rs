//! 工作区树行的**行首字形与宽度算术**：一段前缀占几列，名字还剩几列。
//!
//! 单拆一个模块的理由是「这两件事必须同源」：字形占几列，决定了名称的截断预算
//! （[`ws_row_text`]），也决定了提示行要不要换成完整路径（[`ws_row_clipped`]）。以前两者
//! 各算一份、并且都用 `UnicodeWidthStr::width(prefix)` 现测 —— 现测恒把 Ambiguous 字形算成
//! 1 列，于是终端把 `▸` 画成 2 列时，排版比画面窄一列：整行右移、行尾被裁，而提示行还会
//! 说「名称放得下」。现在宽度是**策略**（[`GlyphPolicy::columns`]），只有一处算术。
//!
//! 字形本身在 [`super::ws_file_kind::icon`]（类型 → 标记），本模块只管「它占多宽」。

use unicode_width::UnicodeWidthStr;

use super::render::truncate_display;
use super::workspace_tree::WsRow;
use super::ws_file_kind;
use super::ws_truncate::middle_ellipsis;

/// 树行首字形的排版策略：类别标记开关 + 单个字形在终端里占几列。
///
/// 两件事打包传，是因为它们**只有合起来才能算对宽度预算**：`ws_row_text`（截断名称）与
/// `ws_row_clipped`（提示行要不要换成全名）各算一份，就会出现「提示行说名称放得下、
/// 画面上名称已经被省略」。
///
/// `columns` 来自 [`ws_file_kind::icon_columns`]（`CM_TUI_ICON_WIDTH`）：终端把 `▸` / `·`
/// 这类 Ambiguous 字形画成 2 列时，预算得跟着按 2 算，否则排版比画面窄一列。
#[derive(Clone, Copy)]
pub(super) struct GlyphPolicy {
    pub(super) icons: bool,
    pub(super) columns: usize,
}

impl GlyphPolicy {
    /// 默认：标记开、每字形 1 列 —— 与加 `columns` 之前的行为逐字一致。
    #[cfg(test)]
    pub(super) const DEFAULT: Self = Self {
        icons: true,
        columns: 1,
    };
    /// 关掉标记（`CM_TUI_FILE_ICONS=off`）：只剩缩进与折叠箭头。
    #[cfg(test)]
    pub(super) const NO_ICONS: Self = Self {
        icons: false,
        columns: 1,
    };

    /// 进程级策略（环境变量读一次，由 `ws_file_kind` 缓存）。
    pub(super) fn from_env() -> Self {
        Self {
            icons: ws_file_kind::icons_enabled(),
            columns: ws_file_kind::icon_columns(),
        }
    }
}

/// 单棵树行文本 + 名称是否被省略（供提示行判断要不要补全名）。
///
/// 缩进 + 行首字形 + 名称；名称超宽时中段省略，见 [`middle_ellipsis`]：开头与扩展名都
/// 保留，避免同目录长名被压成一串 `…xx.rs`。
///
/// `policy` 是显式参数而不是内部读开关：渲染单测要能同时覆盖「开 / 关」与「1 列 / 2 列」
/// 几种宽度预算。
pub(super) fn ws_row_text(row: &WsRow, width: usize, policy: GlyphPolicy) -> (String, bool) {
    let pad = "  ".repeat(usize::from(row.indent));
    let prefix = format!("{pad}{}", row_glyph(row, policy.icons));
    // 宽度按**策略**算，不按 `UnicodeWidthStr::width(prefix)` 现测：后者恒把 Ambiguous
    // 字形算成 1 列（`unicode-width` 按 UAX #11），终端画 2 列时预算就与画面差一列。
    let prefix_w = row_prefix_cols(row, policy);
    if prefix_w >= width {
        return (truncate_display(&prefix, width), true);
    }
    let room = width - prefix_w;
    let clipped = UnicodeWidthStr::width(row.name.as_str()) > room;
    (
        format!("{prefix}{}", middle_ellipsis(&row.name, room)),
        clipped,
    )
}

/// 行首前缀占的显示列数：每层缩进 2 列 + 一个字形 + 字形之后那 1 列空格。
fn row_prefix_cols(row: &WsRow, policy: GlyphPolicy) -> usize {
    2 * usize::from(row.indent) + policy.columns + 1
}

/// 行首字形：目录用折叠箭头，文件用类别标记（`icons` 关闭时留两列空白）。
///
/// 字形本身恒为「一个字符 + 一个空格」；它在终端里究竟占 1 列还是 2 列由
/// [`GlyphPolicy::columns`] 决定，所有宽度算术都走 [`row_prefix_cols`]，不再现测字符串 ——
/// 现测只会得到 `unicode-width` 的 1 列答案，而那正是与终端打架的地方。
fn row_glyph(row: &WsRow, icons: bool) -> &'static str {
    if row.is_dir {
        return if row.expanded { "▾ " } else { "▸ " };
    }
    if !icons {
        return "  ";
    }
    // `file_kind` 只接受 `is_dir`，这里已是文件分支，故传 `false`。
    ws_file_kind::icon(ws_file_kind::file_kind(&row.name, false)).unwrap_or("  ")
}

/// 该行在给定内宽下是否画不全（名称被中段省略）。
///
/// 选中不再占用额外列，所以「名称本身是否被省略」就是唯一判据 —— 曾经这里还要为
/// 高亮行的 `▎` 多扣一列，于是「名称明明放得下」也会被判成画不全。
pub(super) fn ws_row_clipped(row: &WsRow, width: usize, policy: GlyphPolicy) -> bool {
    ws_row_text(row, width, policy).1
}
