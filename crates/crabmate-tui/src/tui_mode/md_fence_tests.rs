//! [`super::fenced_blocks`] 的边界：粘连围栏、无语言、未闭合、多块索引、空块与空行。

use super::{FencedBlock, block_from_end, fenced_blocks};

fn block(lang: &str, code: &str) -> FencedBlock {
    FencedBlock {
        lang: lang.to_string(),
        code: code.to_string(),
    }
}

#[test]
fn single_block_keeps_language_and_body() {
    let md = "说明\n\n```rust\nfn main() {}\n```\n";
    assert_eq!(fenced_blocks(md), vec![block("rust", "fn main() {}")]);
}

#[test]
fn language_tag_tolerates_spacing_and_tildes() {
    assert_eq!(fenced_blocks("``` rust\nx\n```"), vec![block("rust", "x")]);
    assert_eq!(fenced_blocks("~~~sh\nls\n~~~"), vec![block("sh", "ls")]);
    // 无语言标签：块照样在，只是标签为空（不编一个 `text` 上去）。
    assert_eq!(fenced_blocks("```\nx\n```"), vec![block("", "x")]);
}

/// 粘连围栏（`依赖：```rust`）在渲染侧由共享 normalize 拆成独立围栏行；复制侧必须
/// 走同一个入口，否则屏幕上是一块代码、`/copy code` 却说「没有代码块」。
#[test]
fn glued_fence_is_found_because_normalize_runs_first() {
    let md = "依赖：```rust\nlet x = 1;\n```";
    assert_eq!(fenced_blocks(md), vec![block("rust", "let x = 1;")]);
}

/// 流式中途停在未闭合围栏里：此刻看得见的内容就是可复制的内容。
#[test]
fn unclosed_fence_is_copied_up_to_eof() {
    let md = "```rust\nlet a = 1;\nlet b = 2;";
    assert_eq!(
        fenced_blocks(md),
        vec![block("rust", "let a = 1;\nlet b = 2;")]
    );
}

#[test]
fn no_fence_yields_no_blocks() {
    assert!(fenced_blocks("只有正文，`行内代码` 不算围栏").is_empty());
    assert!(fenced_blocks("").is_empty());
}

/// 块内的空行属于代码内容，必须原样保留；块首尾的空行才裁掉。
#[test]
fn inner_blank_lines_survive_while_edges_are_trimmed() {
    let md = "```\n\nfn a() {}\n\nfn b() {}\n\n```";
    assert_eq!(fenced_blocks(md), vec![block("", "fn a() {}\n\nfn b() {}")]);
}

/// 只有空行的块：内容为空串（由调用方报「第 n 块是空的」），不 panic。
#[test]
fn blank_only_block_has_empty_body() {
    assert_eq!(fenced_blocks("```\n\n```"), vec![block("", "")]);
}

#[test]
fn blocks_keep_document_order_and_index_from_the_end() {
    let md = "```a\n1\n```\n文字\n```b\n2\n```\n```c\n3\n```";
    let blocks = fenced_blocks(md);
    assert_eq!(
        blocks,
        vec![block("a", "1"), block("b", "2"), block("c", "3")]
    );
    // 1 = 最后一块（「刚才那段」），3 = 最靠前那块。
    assert_eq!(block_from_end(&blocks, 1), Some(&block("c", "3")));
    assert_eq!(block_from_end(&blocks, 3), Some(&block("a", "1")));
    // 0 与越界都是 None：调用方据此报「第 n 块不存在」而不是回落到最后一块。
    assert_eq!(block_from_end(&blocks, 0), None);
    assert_eq!(block_from_end(&blocks, 4), None);
    assert_eq!(block_from_end(&[], 1), None);
}
