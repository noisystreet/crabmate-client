//! 状态行：左侧上下文摘要 + 右缘运行态指示。
//!
//! 从 `render` 拆出（`render.rs` 已到 920 行门禁上限）：这里是一组相对独立的纯函数，
//! 输入 `StatusInfo` + 宽度、输出单行文本，便于单测。
//! spinner 帧表也放在这里——它是状态行、折叠思考行与进行中工具行共用的进度字形来源，
//! 统一一处保证「换任何一处都不破坏列宽」。

use unicode_width::UnicodeWidthStr;

use super::render::{StatusInfo, truncate_display};

/// 生成中 spinner 帧（Braille 点阵，均 1 列宽，不破坏右对齐与截断计算）。
pub(super) const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// 状态行左侧内容：serve / model / role / mode / conv 与搜索、回看标记
/// （不含运行态——运行态在整行右缘右对齐，仿 Desktop 状态栏）。
fn status_left_text(info: &StatusInfo, conv: Option<&str>) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.push(format!("serve {}", info.api_base));
    if let Some(m) = info.model.as_deref() {
        parts.push(format!("model {m}"));
    }
    if let Some(r) = info.role.as_deref() {
        parts.push(format!("role {r}"));
    }
    if let Some(mode) = info.mode.as_deref() {
        parts.push(format!("mode {mode}"));
    }
    parts.push(match conv {
        Some(c) => format!("conv {c}"),
        None => "conv (new)".to_string(),
    });
    if let Some(term) = info.search_term.as_deref() {
        if info.search_total == 0 {
            parts.push(format!("find 「{term}」 无匹配"));
        } else {
            parts.push(format!("find 「{term}」 {}处", info.search_total));
        }
    }
    if info.view_offset > 0 {
        parts.push(format!("↑{}", info.view_offset));
    }
    parts.join(" | ")
}

/// 状态行右侧运行态指示（空闲 / 生成中 spinner / 取消中）。
/// 只有生成中才换帧：空闲与取消中保持静态，帧号相同则整行不变，
/// 不会让终端每帧重写这一行。
fn status_running_text(info: &StatusInfo) -> String {
    match (info.running, info.cancel_sent) {
        (true, true) => "✳ 取消中".to_string(),
        (true, false) => format!(
            "{} 生成中",
            SPINNER_FRAMES[info.spinner % SPINNER_FRAMES.len()]
        ),
        (false, _) => "○ 空闲".to_string(),
    }
}

/// 把 `right` 显示列宽感知地对齐到整行右缘（`left` 先截断，避免挤压右缘指示）。
fn pad_right(left: &str, right: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let right_w = UnicodeWidthStr::width(right);
    let cut = truncate_display(left, width.saturating_sub(right_w));
    let pad = width.saturating_sub(UnicodeWidthStr::width(cut.as_str()) + right_w);
    format!("{cut}{}{right}", " ".repeat(pad))
}

/// 状态行内容（单行）：左侧上下文 + 右对齐的空闲/生成中指示（仿 Desktop）。
pub(super) fn status_text(info: &StatusInfo, conv: Option<&str>, width: usize) -> String {
    pad_right(
        &status_left_text(info, conv),
        &status_running_text(info),
        width,
    )
}
