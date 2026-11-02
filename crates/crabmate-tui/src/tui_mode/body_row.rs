//! 一条可渲染的 transcript 物理行（含可点击链接区间）。
//!
//! 单独成文件：`render.rs` 贴着 920 行文件门禁，而这块是**数据结构 + 命中查询**，
//! 与「怎么画」关系松散。与 `prefix.rs` 同一手法（那里的头注释记了同样的理由）。

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::super::links::RowLink;
use super::super::palette::Palette;

/// 一条可渲染的 transcript 物理行。
#[derive(Clone)]
pub struct BodyRow {
    /// 所属逻辑行下标（供搜索锚点定位）；块间分隔行不属于任何逻辑行，为 `None`。
    /// 用 `Option` 而非哨兵下标，是为了让"分隔行不该被锚点命中"成为类型级事实——
    /// 锚点查找按 `Some(target)` 匹配，无需依赖“首匹配/末匹配”之类的取行顺序。
    pub log_index: Option<usize>,
    pub line: Line<'static>,
    /// 本行上可点击的链接：列区间**相对正文区左边界**（前缀宽度已计入），
    /// 与 `BodyRow::line` 出自同一次折行计算，因此点到的列就是看到的列。
    pub links: Vec<RowLink>,
}

impl BodyRow {
    /// 内容行：逻辑行 `idx` 的一个物理行。
    ///
    /// `links` 已经是屏幕列（调用方把首行前缀的宽度加进去了）。
    pub(crate) fn content(idx: usize, line: Line<'static>, links: Vec<RowLink>) -> Self {
        Self {
            log_index: Some(idx),
            line,
            links,
        }
    }

    /// 块间分隔行：极暗的满宽细线，不归属任何逻辑行（搜索锚点据此跳过）。
    /// `indent` 取上一条逻辑行的前缀宽度，让线与上方块内容左对齐；
    /// `width` 小于 `indent` 或极窄时 `rule` 退化为 0，此时等同空行，不会 panic。
    pub(crate) fn separator(width: usize, indent: usize, pal: &Palette) -> Self {
        let indent = indent.min(width);
        let rule = width.saturating_sub(indent);
        let mut text = String::with_capacity(width);
        text.push_str(&" ".repeat(indent));
        text.push_str(&"─".repeat(rule));
        // 无色模式没有可靠的“极暗”色，退回 DIM；`─` 本身已能表达分隔。
        let style = if pal.mono {
            Style::new().add_modifier(Modifier::DIM)
        } else {
            Style::new().fg(pal.border)
        };
        Self {
            log_index: None,
            line: Line::from(Span::styled(text, style)),
            links: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pal() -> Palette {
        Palette::resolve(None)
    }

    #[test]
    fn separator_carries_no_links() {
        // 分隔行不属于任何逻辑行，也就不该有任何热区：点它只能落进 `Gap`。
        let row = BodyRow::separator(20, 2, &pal());
        assert!(row.links.is_empty());
        assert_eq!(row.log_index, None);
    }
}
