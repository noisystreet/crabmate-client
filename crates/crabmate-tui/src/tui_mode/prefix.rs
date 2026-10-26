//! transcript 消息前缀：文本、显示列数、配色。
//!
//! 三者必须同处一处：前缀文本是**折行预算的一部分**（[`prefix_width`] 决定每行首物理行
//! 还能放多少正文），在别处改文本却漏掉宽度，行尾就会被区域右缘裁掉。抽成独立模块也让
//! `render.rs` 从 920 行文件门禁线附近退开。

use ratatui::style::{Modifier, Style};
use unicode_width::UnicodeWidthStr;

use super::super::palette::Palette;
use super::super::state::{LineKind, LogLine, ToolState};

/// 消息前缀（宽 2–5 列；宽度参与折行计算，故与样式分开）。
///
/// 不带方括号：`[助手] ` 光括号就白占 2 列，而中文字符本身已有足够的视觉重量；
/// 去掉后每行正文多出 2 列，窄终端上的折行点因此后移。
pub(crate) fn kind_prefix(kind: LineKind) -> &'static str {
    match kind {
        LineKind::User => "你 ",
        LineKind::Assistant => "助手 ",
        LineKind::Thinking => "思考 ",
        LineKind::Tool => "工具 ",
        LineKind::System => "! ",
    }
}

/// 进行中工具行的进度位占位（`⠿` 与 spinner 帧同为 1 列宽，换帧不改折行点）。
pub(crate) const TOOL_PROGRESS_MARK: &str = "⠿ ";

/// 进行中工具行的完整前缀：`工具 ` + 进度位（即 [`TOOL_PROGRESS_MARK`]）。
/// 结束行不加进度位——它的文本自带 `✓ / ✗`，不重复。
pub(crate) const TOOL_PREFIX_PROGRESS: &str = "工具 ⠿ ";

/// 逻辑行前缀：进行中的工具行多一个进度位。
/// 进度位是**静态**占位，动画由渲染层换字符，因此不进入 body memo。
pub(crate) fn prefix_for(log: &LogLine) -> &'static str {
    // 只有「调用中」才加进度位；结束行不论成败是否已知，文本自带标记。
    match (log.kind(), log.tool_state()) {
        (LineKind::Tool, ToolState::Running) => TOOL_PREFIX_PROGRESS,
        _ => kind_prefix(log.kind()),
    }
}

/// 逻辑行前缀占用的**显示列数**（折行预算必须按列算，不能按字符数）：
/// `助手 ` 只有 3 个字符却占 5 列，按字符数留白会让每行首物理行超出正文区
/// 1–2 列，末字被区域右缘裁掉——观感就是「行尾被切、没有折回来」。
pub(crate) fn prefix_width(prefix: &str) -> usize {
    UnicodeWidthStr::width(prefix)
}

/// 前缀样式：每个角色一档色 + BOLD，让块首标签在满屏正文里能被一眼定位。
///
/// 用色刻意避开正文已占的语义色：工具行正文按结果取 `ok` / `err`，前缀若也用它们，
/// 失败的工具行就会出现「绿『工具』+ 红内容」这种自相矛盾的组合——所以工具前缀取
/// `info`（Web 侧 `--info` 与 `--accent` 同样刻意分色相：青 vs 靛）。
///
/// 无色模式（`NO_COLOR` / `TERM=dumb`）所有 `Color` 都被抹成 `Reset`，颜色这一维等于
/// 消失，必须改由修饰位分组：User / System 用 BOLD（要抢眼）、Thinking 用 DIM（低优先）、
/// 其余不加修饰。否则五类前缀全长一样，比改动前更难分辨。
pub(crate) fn prefix_style(kind: LineKind, pal: &Palette) -> Style {
    if pal.mono {
        return match kind {
            LineKind::User | LineKind::System => Style::new().add_modifier(Modifier::BOLD),
            LineKind::Thinking => Style::new().add_modifier(Modifier::DIM),
            LineKind::Assistant | LineKind::Tool => Style::new(),
        };
    }
    let fg = match kind {
        LineKind::User => pal.accent,
        LineKind::Assistant => pal.fg,
        LineKind::Thinking => pal.muted,
        LineKind::Tool => pal.info,
        LineKind::System => pal.err,
    };
    Style::new().fg(fg).add_modifier(Modifier::BOLD)
}
