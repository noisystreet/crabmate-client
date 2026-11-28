//! `/detail` 数据侧表（issue #428 第 3 条）的用例：写入、截断、预算淘汰、行号平移。
//!
//! 只覆盖**数据**这一层（侧表怎么填、什么时候丢）。浮层的按键与折行在
//! `detail_tests.rs`，两者分开是因为这里不需要构造渲染环境。

use super::super::{DetailOverlay, LineKind, ToolDetail, UiState};
use super::*;

/// 结果带的 `detail` 全文要落到**工具那一行**上，且不参与行文本。
#[test]
fn tool_end_stores_the_full_output() {
    let mut st = UiState::new();
    st.tool_start("c1", "run_command", Some("ls"));
    st.tool_end("c1", "run_command", Some(true), Some("ls"), Some("a\nb\nc"));

    let row = 0;
    let detail = st.tool_details.get(&row).expect("详情应落在工具行上");
    assert_eq!(detail.text, "a\nb\nc", "换行要原样保留（摘要才是一行）");
    assert!(!detail.truncated);
    assert_eq!(st.lines[row].text(), "✓ ls", "行文本仍是摘要，不被全文改写");
}

/// 空 / 纯空白输出不建条目：那会让 `/detail` 出现「打开了但什么都没有」的条目。
#[test]
fn empty_output_builds_no_entry() {
    let mut st = UiState::new();
    st.tool_start("c1", "exec", None);
    st.tool_end("c1", "exec", Some(true), None, Some("  \n "));
    assert!(st.tool_details.is_empty());
    assert_eq!(st.detail_chars, 0);
}

/// 单条超过上限时截断并置 `truncated`（浮层尾行据此说明「不一定全部」）。
#[test]
fn single_detail_is_capped_and_flagged() {
    let mut st = UiState::new();
    let long = "字".repeat(DETAIL_MAX_CHARS + 10);
    st.tool_start("c1", "exec", None);
    st.tool_end("c1", "exec", Some(true), None, Some(&long));

    let detail = st.tool_details.get(&0).expect("详情");
    assert_eq!(
        detail.text.chars().count(),
        DETAIL_MAX_CHARS,
        "按字符截断（不是字节）"
    );
    assert!(detail.truncated);
}

/// 没有 start 可对的第二个结果（续流中段重发）：按既有语义另起一行，
/// 两条详情各占一行、预算各算一次 —— 不覆盖、也不双计。
#[test]
fn second_result_without_start_gets_its_own_row() {
    let mut st = UiState::new();
    st.tool_start("c1", "exec", None);
    st.tool_end("c1", "exec", Some(true), None, Some("first"));
    st.tool_end("c1", "exec", Some(true), None, Some("second"));

    assert_eq!(
        st.tool_details.get(&0).map(|d| d.text.as_str()),
        Some("first")
    );
    assert_eq!(
        st.tool_details.get(&1).map(|d| d.text.as_str()),
        Some("second")
    );
    assert_eq!(
        st.detail_chars,
        "first".chars().count() + "second".chars().count()
    );
}

/// 总量到顶时先丢**最旧**的（`/detail` 默认落点是最近一条，丢最近的会让命令失效）。
#[test]
fn budget_evicts_the_oldest_rows_first() {
    let mut st = UiState::new();
    for row in 0..3 {
        st.tool_details.insert(
            row,
            ToolDetail {
                text: "x".repeat(1000),
                truncated: false,
            },
        );
    }
    st.detail_chars = DETAIL_TOTAL_CHARS;

    st.evict_details(1);

    assert!(!st.tool_details.contains_key(&0), "最旧的一条先走");
    assert!(st.tool_details.contains_key(&2), "最近的留着");
    assert_eq!(st.tool_details.len(), 2);
    assert_eq!(st.detail_chars, DETAIL_TOTAL_CHARS - 1000);
}

/// `/older` 前插：侧表的键与开着的浮层都要跟着平移（漏掉就是打开隔壁那行的输出）。
#[test]
fn prepend_shifts_detail_rows_and_the_open_overlay() {
    let mut st = UiState::new();
    st.push_line(LineKind::Tool, "✓ exec");
    st.tool_details.insert(
        0,
        ToolDetail {
            text: "out".to_string(),
            truncated: false,
        },
    );
    st.detail = Some(DetailOverlay { line: 0, scroll: 3 });

    st.prepend_replayed(vec![(LineKind::User, "老问题".to_string(), None)]);

    assert!(st.tool_details.contains_key(&1), "侧表键要平移");
    assert_eq!(st.detail.map(|d| d.line), Some(1), "浮层指向的仍是那一行");
    assert_eq!(st.detail.map(|d| d.scroll), Some(3), "滚动位置不该被重置");
}

/// 切会话清空 transcript 时，侧表与浮层必须一起作废。
#[test]
fn reset_transcript_drops_details_and_the_overlay() {
    let mut st = UiState::new();
    st.tool_details.insert(
        0,
        ToolDetail {
            text: "out".to_string(),
            truncated: false,
        },
    );
    st.detail_chars = 3;
    st.detail = Some(DetailOverlay { line: 0, scroll: 0 });

    st.reset_transcript();

    assert!(st.tool_details.is_empty());
    assert_eq!(st.detail_chars, 0);
    assert!(st.detail.is_none());
}
