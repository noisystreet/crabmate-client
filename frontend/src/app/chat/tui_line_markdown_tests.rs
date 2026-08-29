//! 思维链折叠块增量 patch 规划测试（自 `tui_line_markdown` 拆出，控制该文件行数）。
//!
//! 覆盖 [`crate::app::chat::tui_line_markdown::TuiBodyPatch::ThinkBody`] 与 `think` 字段的
//! `plan_tui_body_patch` 分派：正文增长走定向 patch、冻结后终答走 Incremental、open 翻转/出现走 ReplaceAll。

use super::*;

fn think(open: bool, body: &str) -> ThinkBlock {
    ThinkBlock {
        open,
        summary_html: "<summary class=\"chat-tui-think-summary\">思考</summary>".to_string(),
        body_html: body.to_string(),
    }
}

fn chunks(think: Option<ThinkBlock>, answer: &str) -> TuiBodyChunks {
    // finalize=false：模拟流式，末尾行留在 open_plain（与 transcript 增量路径一致）。
    let mut c = parse_tui_body_chunks(answer, false);
    c.think = think;
    c
}

#[test]
fn growing_think_body_uses_think_body_patch() {
    let a = chunks(Some(think(true, "第一")), "");
    let b = chunks(Some(think(true, "第一第二")), "");
    match plan_tui_body_patch(Some(&a), &b) {
        TuiBodyPatch::ThinkBody {
            body_html,
            append_closed,
            open_plain,
        } => {
            assert_eq!(body_html, "第一第二");
            assert!(append_closed.is_empty());
            assert_eq!(open_plain, None);
        }
        other => panic!("expected ThinkBody, got {other:?}"),
    }
}

#[test]
fn frozen_think_growing_answer_uses_incremental() {
    let a = chunks(Some(think(true, "推理完毕")), "答");
    let b = chunks(Some(think(true, "推理完毕")), "答案二");
    match plan_tui_body_patch(Some(&a), &b) {
        TuiBodyPatch::Incremental {
            append_closed,
            open_plain,
        } => {
            assert!(append_closed.is_empty());
            assert_eq!(open_plain.as_deref(), Some("答案二"));
        }
        other => panic!("expected Incremental, got {other:?}"),
    }
}

#[test]
fn think_open_flip_uses_replace_all() {
    let a = chunks(Some(think(true, "推理完毕")), "答");
    let b = chunks(Some(think(false, "推理完毕")), "答");
    assert!(matches!(
        plan_tui_body_patch(Some(&a), &b),
        TuiBodyPatch::ReplaceAll { .. }
    ));
}

#[test]
fn think_appearing_uses_replace_all() {
    let a = chunks(None, "答");
    let b = chunks(Some(think(true, "推理")), "答");
    assert!(matches!(
        plan_tui_body_patch(Some(&a), &b),
        TuiBodyPatch::ReplaceAll { .. }
    ));
}

#[test]
fn think_block_to_details_html_embeds_open_and_body() {
    let t = think(true, "推理正文");
    let html = t.to_details_html();
    assert!(
        html.contains("<details class=\"chat-tui-think\" open>"),
        "got {html}"
    );
    assert!(html.contains("推理正文"), "got {html}");
    assert!(!html.contains("<think>"), "got {html}");
}
