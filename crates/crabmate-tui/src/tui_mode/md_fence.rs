//! 代码围栏块提取：给 `/copy code [n]` 用（issue #428 第 1 条）。
//!
//! 扫的是**原始正文的规范化结果**，不是渲染后的行：渲染态带竖条、padding 与 `…` 截断，
//! 复制出去就是脏内容。规范化必须与渲染走同一入口（[`normalize_markdown_for_render`]），
//! 否则 `依赖：```rust` 这种粘连围栏会在屏幕上是一块代码、扫出来却是零块。
//!
//! 状态机与 [`super::assistant_styled`] 同规则：**任何**围栏开关行都翻转状态，不比较
//! ``` 与 `~~~` 的字符是否相同 —— 渲染层就是这么判的，两处各写一套的后果是「屏幕上看着
//! 是一块、复制到的是另一块」，且只在混合围栏的正文里出现。
//!
//! 未闭合围栏（流式还在写）按 EOF 收尾：这一块照样可复制 —— 用户按下命令时看到的就是
//! 这些内容，拒绝复制只会让人以为命令坏了。

use crabmate_client_api::normalize_markdown_for_render;

use super::{fence_lang, is_fence_switch};

/// 一块围栏代码：语言标签（可能为空）与块内原文。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FencedBlock {
    /// 开围栏行的语言标签（`` ```rust `` → `rust`）；无标签为空串。
    pub lang: String,
    /// 块内正文（已去掉首尾空行，见 [`trim_blank_edges`]）。
    pub code: String,
}

/// 按出现顺序提取所有围栏块（因此最后一块是正文里最靠后的那块）。
#[must_use]
pub(crate) fn fenced_blocks(markdown: &str) -> Vec<FencedBlock> {
    let normalized = normalize_markdown_for_render(markdown);
    let mut out: Vec<FencedBlock> = Vec::new();
    let mut open: Option<(String, Vec<&str>)> = None;
    for line in normalized.split('\n') {
        if is_fence_switch(line) {
            match open.take() {
                Some((lang, body)) => out.push(finish(lang, &body)),
                None => open = Some((fence_lang(line).to_string(), Vec::new())),
            }
            continue;
        }
        if let Some((_, body)) = open.as_mut() {
            body.push(line);
        }
    }
    // 未闭合：流式还在写，此刻看得见的内容就是可复制的内容。
    if let Some((lang, body)) = open {
        out.push(finish(lang, &body));
    }
    out
}

/// 倒数第 `n` 块（`n` 从 1 起 = 正文里最后一块）；越界返回 `None`。
///
/// 从末尾数而不是从头数：用户说的是「刚才那段代码」，而正文越往后越接近「刚才」。
#[must_use]
pub(crate) fn block_from_end(blocks: &[FencedBlock], n: usize) -> Option<&FencedBlock> {
    blocks.get(blocks.len().checked_sub(n)?)
}

fn finish(lang: String, body: &[&str]) -> FencedBlock {
    FencedBlock {
        lang,
        code: trim_blank_edges(body).join("\n"),
    }
}

/// 去掉块内正文首尾的空行。
///
/// 围栏紧贴正文书写时（`` ```rust `` 与代码之间有意图性空行，或闭合前留一行）会带上
/// 多余空行；粘贴出去多一行空行比少一行更难发现，所以两端都裁。块内部的空行原样保留 ——
/// 那是代码的一部分。
fn trim_blank_edges<'a>(body: &'a [&'a str]) -> &'a [&'a str] {
    let Some(start) = body.iter().position(|line| !line.trim().is_empty()) else {
        return &body[body.len()..];
    };
    let end = body
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .map_or(start, |i| i + 1);
    &body[start..end]
}

#[cfg(test)]
#[path = "md_fence_tests.rs"]
mod tests;
