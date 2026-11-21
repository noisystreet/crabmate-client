use super::{
    LayoutProjection, LayoutProjectionFallback, assign_page_scoped_row_ids, project_hydrated_rows,
};
use crate::session_model::StoredMessage;
use crabmate::cm_api_contract::chat::{ConversationLayoutMeta, ConversationLayoutSegment};

fn message(id: &str, role: &str, text: &str) -> StoredMessage {
    StoredMessage {
        id: id.into(),
        role: role.into(),
        text: text.into(),
        reasoning_text: String::new(),
        image_urls: vec![],
        state: None,
        is_tool: false,
        tool_call_id: None,
        tool_name: None,
        created_at: 0,
    }
}

fn tool_row(id: &str, tool_call_id: &str) -> StoredMessage {
    StoredMessage {
        is_tool: true,
        tool_call_id: Some(tool_call_id.into()),
        ..message(id, "tool", "read")
    }
}

fn timeline_row(id: &str, text: &str) -> StoredMessage {
    StoredMessage {
        ..message(id, "system", text)
    }
}

fn segment(
    turn: &str,
    kind: &str,
    segment_id: &str,
    anchor: Option<&str>,
    seq: u32,
) -> ConversationLayoutSegment {
    ConversationLayoutSegment {
        turn_id: Some(turn.into()),
        segment_id: segment_id.into(),
        segment_kind: kind.into(),
        before_tool_call_id: anchor.map(str::to_string),
        sequence: seq,
    }
}

fn meta(segments: Vec<ConversationLayoutSegment>) -> ConversationLayoutMeta {
    ConversationLayoutMeta {
        layout_schema_version: 2,
        projection_hash: Some("deadbeef".into()),
        segments,
    }
}

/// 服务端对同一条 assistant 消息（正文 + tool_calls）产「旁白 + 工具」两段。
fn one_tool_turn(turn: &str, tcid: &str, seq: u32) -> Vec<ConversationLayoutSegment> {
    vec![
        segment(
            turn,
            "assistant_commentary",
            &format!("seg-before-{tcid}"),
            Some(tcid),
            seq,
        ),
        segment(turn, "tool", tcid, None, seq + 1),
    ]
}

fn ids(rows: &[StoredMessage]) -> Vec<&str> {
    rows.iter().map(|row| row.id.as_str()).collect()
}

fn mapped(
    rows: &[StoredMessage],
    layout: &ConversationLayoutMeta,
    page_start: u32,
) -> Vec<StoredMessage> {
    match project_hydrated_rows(rows, Some(layout), page_start) {
        LayoutProjection::Mapped(rows) => rows,
        other => panic!("expected mapped, got {other:?}"),
    }
}

#[test]
fn no_layout_or_old_schema_falls_back() {
    let rows = vec![message("h_0", "assistant", "hi")];
    assert_eq!(
        project_hydrated_rows(&rows, None, 0),
        LayoutProjection::Fallback(LayoutProjectionFallback::NoLayout)
    );
    let mut old = meta(vec![]);
    old.layout_schema_version = 1;
    assert_eq!(
        project_hydrated_rows(&rows, Some(&old), 0),
        LayoutProjection::Fallback(LayoutProjectionFallback::LayoutSchemaTooOld)
    );
}

#[test]
fn commentary_and_tool_rows_get_stable_keys() {
    let rows = vec![
        message("u", "user", "go"),
        message("h_1", "assistant", "先读。"),
        tool_row("h_2", "tc1"),
    ];
    let layout = meta(one_tool_turn("u0", "tc1", 0));
    let out = mapped(&rows, &layout, 0);
    assert_eq!(
        ids(&out),
        vec!["turn-user-u0", "turn-commentary-tc1", "turn-tool-tc1"]
    );
}

#[test]
fn answer_row_gets_archived_key_not_the_canonical_one() {
    let rows = vec![
        message("u", "user", "go"),
        message("h_1", "assistant", "答"),
    ];
    let layout = meta(vec![segment(
        "u0",
        "assistant_answer",
        "assistant_answer-0",
        None,
        0,
    )]);
    let out = mapped(&rows, &layout, 0);
    assert_eq!(ids(&out), vec!["turn-user-u0", "turn-final-answer#prev1"]);
}

#[test]
fn answers_are_numbered_in_time_order() {
    let rows = vec![
        message("u1", "user", "q1"),
        message("h_1", "assistant", "a1"),
        message("u2", "user", "q2"),
        message("h_2", "assistant", "a2"),
    ];
    let layout = meta(vec![
        segment("u0", "assistant_answer", "assistant_answer-0", None, 0),
        segment("u1", "assistant_answer", "assistant_answer-1", None, 1),
    ]);
    let out = mapped(&rows, &layout, 0);
    assert_eq!(
        ids(&out),
        vec![
            "turn-user-u0",
            "turn-final-answer#prev1",
            "turn-user-u1",
            "turn-final-answer#prev2"
        ]
    );
}

#[test]
fn timeline_rows_are_turn_scoped_and_counted() {
    let rows = vec![
        timeline_row("h_lead", "启动"),
        message("u", "user", "go"),
        timeline_row("h_tl", "中间旁注"),
        message("h_a", "assistant", "答"),
    ];
    let layout = meta(vec![
        segment(
            "lead",
            "assistant_timeline",
            "assistant_timeline-0",
            None,
            0,
        ),
        segment("u0", "assistant_timeline", "assistant_timeline-1", None, 1),
        segment("u0", "assistant_answer", "assistant_answer-2", None, 2),
    ]);
    let out = mapped(&rows, &layout, 0);
    assert_eq!(
        ids(&out),
        vec![
            "turn-timeline-lead-0",
            "turn-user-u0",
            "turn-timeline-u0-0",
            "turn-final-answer#prev1"
        ]
    );
}

#[test]
fn reasoning_only_assistant_row_is_not_projectable() {
    let rows = vec![
        message("u", "user", "go"),
        StoredMessage {
            reasoning_text: "想一想".into(),
            ..message("h_think", "assistant", "")
        },
        message("h_a", "assistant", "答"),
    ];
    let layout = meta(vec![segment(
        "u0",
        "assistant_answer",
        "assistant_answer-0",
        None,
        0,
    )]);
    let out = mapped(&rows, &layout, 0);
    assert_eq!(
        ids(&out),
        vec!["turn-user-u0", "turn-row-u0-1", "turn-final-answer#prev1"]
    );
}

#[test]
fn kind_order_mismatch_falls_back() {
    let rows = vec![
        message("u", "user", "go"),
        tool_row("h_t", "tc1"),
        message("h_a", "assistant", "答"),
    ];
    let layout = meta(vec![
        segment(
            "u0",
            "assistant_commentary",
            "seg-before-tc1",
            Some("tc1"),
            0,
        ),
        segment("u0", "tool", "tc1", None, 1),
    ]);
    assert_eq!(
        project_hydrated_rows(&rows, Some(&layout), 0),
        LayoutProjection::Fallback(LayoutProjectionFallback::TurnShapeMismatch)
    );
}

#[test]
fn tool_id_mismatch_falls_back() {
    let rows = vec![
        message("u", "user", "go"),
        message("h_1", "assistant", "先读。"),
        tool_row("h_2", "tc9"),
    ];
    let layout = meta(one_tool_turn("u0", "tc1", 0));
    assert_eq!(
        project_hydrated_rows(&rows, Some(&layout), 0),
        LayoutProjection::Fallback(LayoutProjectionFallback::TurnShapeMismatch)
    );
}

#[test]
fn turn_count_mismatch_falls_back() {
    let rows = vec![
        message("u1", "user", "q1"),
        message("h_1", "assistant", "a1"),
        message("u2", "user", "q2"),
        message("h_2", "assistant", "a2"),
    ];
    let layout = meta(vec![segment(
        "u0",
        "assistant_answer",
        "assistant_answer-0",
        None,
        0,
    )]);
    assert_eq!(
        project_hydrated_rows(&rows, Some(&layout), 0),
        LayoutProjection::Fallback(LayoutProjectionFallback::TurnCountMismatch)
    );
}

#[test]
fn repeated_tool_call_id_across_turns_archives_earlier_turn() {
    let rows = vec![
        message("u1", "user", "q1"),
        message("h_1", "assistant", "先读。"),
        tool_row("h_2", "tc1"),
        message("u2", "user", "q2"),
        message("h_3", "assistant", "再读。"),
        tool_row("h_4", "tc1"),
    ];
    let mut segments = one_tool_turn("u0", "tc1", 0);
    segments.extend(one_tool_turn("u1", "tc1", 2));
    let layout = meta(segments);
    let out = mapped(&rows, &layout, 0);
    assert_eq!(
        ids(&out),
        vec![
            "turn-user-u0",
            "turn-commentary-tc1#prev1",
            "turn-tool-tc1#prev1",
            "turn-user-u1",
            "turn-commentary-tc1",
            "turn-tool-tc1",
        ]
    );
}

#[test]
fn windowed_page_maps_only_aligned_tail_turns() {
    // 页首是上一回合的尾巴（user 行不在本页）：不参与校验，但仍拿确定性 `turn-prev-*`。
    let rows = vec![
        message("h_old_1", "assistant", "旧旁白"),
        tool_row("h_old_2", "tc0"),
        message("u2", "user", "q2"),
        message("h_3", "assistant", "再读。"),
        tool_row("h_4", "tc1"),
    ];
    let mut segments = one_tool_turn("u0", "tc0", 0);
    segments.extend(one_tool_turn("u1", "tc1", 2));
    let layout = meta(segments);
    let out = mapped(&rows, &layout, 40);
    assert_eq!(
        ids(&out),
        vec![
            "turn-prev-40-0",
            "turn-prev-40-1",
            "turn-user-u1",
            "turn-commentary-tc1",
            "turn-tool-tc1",
        ]
    );
}

#[test]
fn mapping_is_idempotent() {
    let rows = vec![
        message("u", "user", "go"),
        message("h_1", "assistant", "先读。"),
        tool_row("h_2", "tc1"),
    ];
    let layout = meta(one_tool_turn("u0", "tc1", 0));
    let first = mapped(&rows, &layout, 0);
    let second = mapped(&first, &layout, 0);
    assert_eq!(ids(&first), ids(&second));
}

#[test]
fn batch_narration_segment_is_not_silently_accepted() {
    let rows = vec![
        message("u", "user", "go"),
        message("h_1", "assistant", "批说明"),
    ];
    let layout = meta(vec![segment(
        "u0",
        "assistant_batch_narration",
        "assistant_batch_narration-0",
        None,
        0,
    )]);
    assert_eq!(
        project_hydrated_rows(&rows, Some(&layout), 0),
        LayoutProjection::Fallback(LayoutProjectionFallback::TurnShapeMismatch)
    );
}

/// 回归锁（#545 review High）：映射成功后**整页**不得残留 `h_{毫秒}_{序号}`。
///
/// 之前只改可投影行，user 行与「只带 reasoning 的 assistant 行」仍是每次水合新生成的
/// 时间戳 id → 纯问答会话每次刷新都重新赋值并重写持久化，「刷新 N 次 id 序列不变」不成立。
#[test]
fn mapping_leaves_no_hydration_ids_behind() {
    let rows = vec![
        message("h_u", "user", "go"),
        message("h_c", "assistant", "先读。"),
        tool_row("h_t", "tc1"),
        StoredMessage {
            reasoning_text: "想一想".into(),
            ..message("h_think", "assistant", "")
        },
        message("h_a", "assistant", "答"),
    ];
    let mut segments = one_tool_turn("u0", "tc1", 0);
    segments.push(segment(
        "u0",
        "assistant_answer",
        "assistant_answer-2",
        None,
        2,
    ));
    let out = mapped(&rows, &meta(segments), 0);
    assert_eq!(
        ids(&out),
        vec![
            "turn-user-u0",
            "turn-commentary-tc1",
            "turn-tool-tc1",
            "turn-row-u0-3",
            "turn-final-answer#prev1",
        ]
    );
    assert!(
        out.iter().all(|row| !row.id.starts_with("h_")),
        "映射后不得残留水合时间戳 id: {:?}",
        ids(&out)
    );
}
/// 更早一页只做页内编号，且窗口下标进键名：两个不同的页不会撞键（`#547`）。
#[test]
fn page_scoped_ids_are_namespaced_per_window() {
    let make = || {
        vec![
            message("h_a", "assistant", "旧旁白"),
            tool_row("h_t", "tc0"),
        ]
    };
    let mut first = make();
    assign_page_scoped_row_ids(&mut first, 40);
    let mut second = make();
    assign_page_scoped_row_ids(&mut second, 80);

    assert_eq!(ids(&first), vec!["turn-prev-40-0", "turn-prev-40-1"]);
    assert_eq!(ids(&second), vec!["turn-prev-80-0", "turn-prev-80-1"]);

    // 同一窗口重复加载 → 逐字相同（prepend 的 id 去重因此幂等）。
    let mut again = make();
    assign_page_scoped_row_ids(&mut again, 40);
    assert_eq!(first, again);
}

/// 「末回合只有 `assistant(text)+tool_calls`、还没有 `role=tool` 结果」时本地少一条工具行：
/// 回落原因应能区分出来，便于排查「明明有 layout 却没升级」。
#[test]
fn missing_local_tool_row_gets_its_own_fallback_reason() {
    let rows = vec![
        message("h_u", "user", "go"),
        message("h_c", "assistant", "先读。"),
        message("h_a", "assistant", "答"),
    ];
    let mut segments = one_tool_turn("u0", "tc1", 0);
    segments.push(segment(
        "u0",
        "assistant_answer",
        "assistant_answer-2",
        None,
        2,
    ));
    assert_eq!(
        project_hydrated_rows(&rows, Some(&meta(segments)), 0),
        LayoutProjection::Fallback(LayoutProjectionFallback::MissingLocalToolRow)
    );
}
