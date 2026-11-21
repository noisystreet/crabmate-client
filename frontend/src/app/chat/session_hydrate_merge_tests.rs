use super::{drop_superseded_tool_call_stubs, hydration_content_guard_outcome};
use crate::app::chat::session_hydrate::{
    SessionHydrationMergeOutcome, apply_hydrated_tail_if_newer, hydration_response_meta_is_fresh,
    hydration_revision_after_response, merge_tail_page_into_session_messages,
    should_merge_hydrated_messages,
};
use crate::conversation_hydrate::ConversationMessagesResponse;
use crate::storage::{
    CURRENT_LAYOUT_SCHEMA_VERSION, ChatSession, LEGACY_LAYOUT_SCHEMA_VERSION, StoredMessage,
};

fn revision_session(messages: Vec<StoredMessage>, revision: Option<u64>) -> ChatSession {
    ChatSession {
        id: "sid".into(),
        layout_schema_version: LEGACY_LAYOUT_SCHEMA_VERSION,
        title: "t".into(),
        draft: String::new(),
        messages,
        updated_at: 0,
        pinned: false,
        starred: false,
        server_conversation_id: Some("cid".into()),
        server_revision: revision,
        workspace_root: None,
        history_total: None,
        history_window_start: Some(0),
        history_has_older: None,
    }
}

fn revision_response(revision: u64) -> ConversationMessagesResponse {
    ConversationMessagesResponse {
        conversation_id: "cid".into(),
        messages: vec![],
        revision,
        total_count: 1,
        window_start_index: 0,
        has_older: false,
        active_agent_role: None,
        active_session_mode: None,
        tiktoken_prompt_tokens: None,
        context_artifacts: vec![],
        layout: None,
    }
}

fn plain_message(id: &str, role: &str, text: &str) -> StoredMessage {
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

/// 同 revision 也合并（v2 追加是 no-op、legacy 快照替换幂等）；本地更新时才跳过。
#[test]
fn same_revision_still_reapplies_server_snapshot() {
    let local = plain_message("turn-commentary-tc_read", "assistant", "已关闭的本地旁注");
    let session = revision_session(vec![local], Some(7));
    assert!(should_merge_hydrated_messages(
        &session,
        &revision_response(7)
    ));
    assert!(should_merge_hydrated_messages(
        &session,
        &revision_response(8)
    ));
    assert!(!should_merge_hydrated_messages(
        &session,
        &revision_response(6)
    ));
    assert!(should_merge_hydrated_messages(
        &revision_session(vec![], Some(7)),
        &revision_response(7)
    ));
}

#[test]
fn hydration_meta_fresh_when_response_not_older() {
    assert!(hydration_response_meta_is_fresh(None, 1));
    assert!(hydration_response_meta_is_fresh(Some(7), 7));
    assert!(hydration_response_meta_is_fresh(Some(7), 8));
    assert!(!hydration_response_meta_is_fresh(Some(9), 7));
}

#[test]
fn stale_hydration_response_neither_merges_nor_downgrades_revision() {
    let local = plain_message("turn-final-answer", "assistant", "本地较新终答");
    let session = revision_session(vec![local], Some(9));
    let stale_response = revision_response(7);
    assert!(!should_merge_hydrated_messages(&session, &stale_response));
    assert_eq!(
        hydration_revision_after_response(session.server_revision, stale_response.revision),
        9
    );
}

#[test]
fn newer_same_turn_hydration_keeps_v2_projection_without_legacy_pool_merge() {
    let local = vec![
        plain_message("u-local", "user", "question"),
        plain_message("turn-commentary-tc_read", "assistant", "不可变 commentary"),
        plain_message("turn-final-answer", "assistant", "本地终答"),
    ];
    let mut session = revision_session(local.clone(), Some(7));
    session.layout_schema_version = CURRENT_LAYOUT_SCHEMA_VERSION;
    let hydrated = vec![
        plain_message("h-user", "user", "question"),
        plain_message("h-assistant", "assistant", "服务端 canonical 快照"),
    ];

    apply_hydrated_tail_if_newer(&mut session, hydrated, &revision_response(8));

    assert_eq!(
        session
            .messages
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>(),
        local.iter().map(|m| m.id.as_str()).collect::<Vec<_>>()
    );
    assert_eq!(session.layout_schema_version, CURRENT_LAYOUT_SCHEMA_VERSION);
}

#[test]
fn newer_hydration_appends_server_only_turn_after_v2_projection() {
    let local = vec![
        plain_message("u-local", "user", "question"),
        plain_message("turn-final-answer", "assistant", "本地终答"),
    ];
    let mut session = revision_session(local, Some(7));
    session.layout_schema_version = CURRENT_LAYOUT_SCHEMA_VERSION;
    let hydrated = vec![
        plain_message("h-user-1", "user", "question"),
        plain_message("h-answer-1", "assistant", "服务端旧终答"),
        plain_message("h-user-2", "user", "new question"),
        plain_message("h-answer-2", "assistant", "服务端新增终答"),
    ];

    apply_hydrated_tail_if_newer(&mut session, hydrated, &revision_response(8));

    assert_eq!(
        session
            .messages
            .iter()
            .map(|message| message.id.as_str())
            .collect::<Vec<_>>(),
        ["u-local", "turn-final-answer", "h-user-2", "h-answer-2"]
    );
    assert_eq!(session.layout_schema_version, CURRENT_LAYOUT_SCHEMA_VERSION);
}

#[test]
fn empty_v2_cache_replaces_with_server_snapshot() {
    let mut session = revision_session(vec![], Some(7));
    session.layout_schema_version = CURRENT_LAYOUT_SCHEMA_VERSION;
    let hydrated = vec![plain_message("h-assistant", "assistant", "服务端终答")];

    apply_hydrated_tail_if_newer(&mut session, hydrated, &revision_response(7));

    assert_eq!(session.messages.len(), 1);
    assert_eq!(session.messages[0].id, "h-assistant");
    assert_eq!(session.layout_schema_version, LEGACY_LAYOUT_SCHEMA_VERSION);
}

#[test]
fn v1_history_is_replaced_by_server_snapshot() {
    let mut session = revision_session(
        vec![plain_message("local-assistant", "assistant", "本地旧投影")],
        Some(1),
    );
    let hydrated = vec![plain_message("h-assistant", "assistant", "服务端旧会话")];

    apply_hydrated_tail_if_newer(&mut session, hydrated, &revision_response(2));

    assert_eq!(session.messages.len(), 1);
    assert_eq!(session.messages[0].id, "h-assistant");
    assert_eq!(session.layout_schema_version, LEGACY_LAYOUT_SCHEMA_VERSION);
}

#[test]
fn merge_tail_page_replaces_local_draft_with_server_snapshot() {
    let session = ChatSession {
        id: "sid".into(),
        layout_schema_version: LEGACY_LAYOUT_SCHEMA_VERSION,
        title: "t".into(),
        draft: String::new(),
        messages: vec![
            plain_message("u1", "user", "question"),
            StoredMessage {
                created_at: 2,
                ..plain_message("a-local", "assistant", "stream draft")
            },
        ],
        updated_at: 0,
        pinned: false,
        starred: false,
        server_conversation_id: Some("cid".into()),
        server_revision: None,
        workspace_root: None,
        history_total: None,
        history_window_start: Some(0),
        history_has_older: None,
    };
    let hydrated = vec![
        plain_message("u1", "user", "question"),
        StoredMessage {
            created_at: 2,
            ..plain_message("a-srv", "assistant", "final answer")
        },
    ];
    let resp = ConversationMessagesResponse {
        conversation_id: "cid".into(),
        messages: vec![],
        revision: 1,
        total_count: 2,
        window_start_index: 0,
        has_older: false,
        active_agent_role: None,
        active_session_mode: None,
        tiktoken_prompt_tokens: None,
        context_artifacts: vec![],
        layout: None,
    };
    let merged = merge_tail_page_into_session_messages(&session, hydrated, &resp);
    let ids: Vec<_> = merged.messages.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, vec!["u1", "a-srv"]);
}

#[test]
fn merge_tail_page_does_not_resurrect_user_the_server_omits() {
    let session = ChatSession {
        id: "sid".into(),
        layout_schema_version: LEGACY_LAYOUT_SCHEMA_VERSION,
        title: "t".into(),
        draft: String::new(),
        messages: vec![
            plain_message("u-local", "user", "你好"),
            StoredMessage {
                created_at: 2,
                ..plain_message("a-local", "assistant", "你好！")
            },
        ],
        updated_at: 0,
        pinned: false,
        starred: false,
        server_conversation_id: Some("cid".into()),
        server_revision: None,
        workspace_root: None,
        history_total: None,
        history_window_start: Some(0),
        history_has_older: None,
    };
    let hydrated = vec![StoredMessage {
        created_at: 2,
        ..plain_message("a-srv", "assistant", "你好！我是 CrabMate 的 AI 助手。")
    }];
    let resp = ConversationMessagesResponse {
        conversation_id: "cid".into(),
        messages: vec![],
        revision: 1,
        total_count: 1,
        window_start_index: 0,
        has_older: false,
        active_agent_role: None,
        active_session_mode: None,
        tiktoken_prompt_tokens: None,
        context_artifacts: vec![],
        layout: None,
    };
    let merged = merge_tail_page_into_session_messages(&session, hydrated.clone(), &resp);
    let roles: Vec<_> = merged.messages.iter().map(|m| m.role.as_str()).collect();
    assert_eq!(
        roles,
        vec!["assistant"],
        "服务端快照权威：本地 user 不再被补回"
    );
    // 真缺回合（尾页 user 变少）由水合前的内容守卫整页跳过，不靠合并层补行。
    assert_eq!(
        hydration_content_guard_outcome(&session, &hydrated, &resp),
        Some(SessionHydrationMergeOutcome::SkippedHydratedUserRegression)
    );
}

#[test]
fn empty_cache_keeps_legacy_ids_and_schema() {
    let mut session = revision_session(vec![], Some(0));
    let hydrated = vec![
        plain_message("u", "user", "go"),
        plain_message("h_a", "assistant", "先读。"),
        StoredMessage {
            is_tool: true,
            tool_call_id: Some("tc1".into()),
            ..plain_message("h_t", "tool", "read")
        },
    ];
    let resp = revision_response(4);
    apply_hydrated_tail_if_newer(&mut session, hydrated, &resp);
    assert_eq!(session.layout_schema_version, LEGACY_LAYOUT_SCHEMA_VERSION);
    assert_eq!(session.messages[1].id, "h_a");
    assert_eq!(session.messages[2].id, "h_t");
    assert!(!session.has_v2_finalized_rows());
}

/// 每回合 1 user + 9 assistant 的长会话块。
fn turn_block(turn: usize) -> Vec<StoredMessage> {
    let mut block = vec![plain_message(
        &format!("u{turn}"),
        "user",
        &format!("q{turn}"),
    )];
    for i in 0..9 {
        block.push(plain_message(
            &format!("a{turn}_{i}"),
            "assistant",
            "answer",
        ));
    }
    block
}

fn messages_from_turns(turns: std::ops::Range<usize>) -> Vec<StoredMessage> {
    turns.flat_map(turn_block).collect()
}

fn precheck_response(window_start_index: u32, total_count: u32) -> ConversationMessagesResponse {
    let mut resp = revision_response(8);
    resp.total_count = total_count;
    resp.window_start_index = window_start_index;
    resp
}

/// 分页长会话（本地全量 user > 尾页 user，但尾页覆盖范围内一致）不得触发回退守卫：
/// 否则水合被永久跳过，tiktoken 快照无法写回，状态栏上下文将一直显示 `— / cap`。
#[test]
fn paginated_long_session_hydration_passes_user_regression_guard() {
    // 10 回合 = 10 user / 100 条；尾页 60 条（window_start=40）覆盖 turn4..turn9 = 6 user。
    let session = {
        let mut s = revision_session(messages_from_turns(0..10), Some(7));
        s.history_window_start = Some(0);
        s
    };
    let hydrated = messages_from_turns(4..10);
    assert_eq!(
        hydration_content_guard_outcome(&session, &hydrated, &precheck_response(40, 100)),
        None
    );
}

/// 尾页覆盖范围内 user 数真实减少（服务端快照缺回合）仍须拦截。
#[test]
fn genuine_user_regression_within_tail_page_still_skipped() {
    let session = {
        let mut s = revision_session(messages_from_turns(0..10), Some(7));
        s.history_window_start = Some(0);
        s
    };
    // 尾页窗口应有 6 user（turn4..turn9），服务端只回了 3 user（turn4..turn6）。
    let hydrated = messages_from_turns(4..7);
    assert_eq!(
        hydration_content_guard_outcome(&session, &hydrated, &precheck_response(40, 100)),
        Some(SessionHydrationMergeOutcome::SkippedHydratedUserRegression)
    );
}

/// v2 projection 合并只追加服务端新回合，本地行不替换 → 不做回退比较。
#[test]
fn v2_projection_hydration_bypasses_user_regression_guard() {
    let mut session = revision_session(
        vec![
            plain_message("u1", "user", "q1"),
            plain_message("turn-final-answer", "assistant", "ans1"),
            plain_message("u2", "user", "q2"),
            plain_message("turn-final-answer-2", "assistant", "ans2"),
        ],
        Some(7),
    );
    session.layout_schema_version = CURRENT_LAYOUT_SCHEMA_VERSION;
    let hydrated = vec![
        plain_message("u3", "user", "q3"),
        plain_message("a3", "assistant", "ans3"),
    ];
    assert_eq!(
        hydration_content_guard_outcome(&session, &hydrated, &precheck_response(0, 6)),
        None
    );
}

/// 无窗口信息时整个本地列表都处于覆盖范围，维持全量比较的保守旧行为。
#[test]
fn no_window_meta_falls_back_to_full_local_user_compare() {
    let mut session = revision_session(messages_from_turns(0..10), Some(7));
    session.history_window_start = None;
    let hydrated = messages_from_turns(4..10);
    assert_eq!(
        hydration_content_guard_outcome(&session, &hydrated, &precheck_response(40, 100)),
        Some(SessionHydrationMergeOutcome::SkippedHydratedUserRegression)
    );
}

/// 水合快照里「调用短卡 + 结果卡」并存时，短卡必须并入结果卡（冷启动只显示一张工具卡）。
#[test]
fn hydrate_stub_is_dropped_when_result_card_shares_the_tool_call_id() {
    let rows = vec![
        plain_message("u", "user", "现在时间是什么"),
        StoredMessage {
            is_tool: true,
            tool_call_id: Some("tc_time".into()),
            tool_name: Some("get_current_time".into()),
            ..plain_message("stub", "assistant", "工具：get_current_time")
        },
        StoredMessage {
            text: "get_current_time".into(),
            reasoning_text: "当前时间：21:25:05".into(),
            tool_call_id: Some("tc_time".into()),
            tool_name: Some("get_current_time".into()),
            ..{
                let mut m = plain_message("result", "system", "");
                m.is_tool = true;
                m
            }
        },
        plain_message("a", "assistant", "当前时间是 21:25:05。"),
    ];
    let ids: Vec<String> = drop_superseded_tool_call_stubs(rows)
        .into_iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(ids, ["u", "result", "a"]);
}

#[test]
fn hydrate_stub_survives_when_no_result_card_matches() {
    let rows = vec![
        StoredMessage {
            is_tool: true,
            tool_call_id: Some("tc_a".into()),
            tool_name: Some("read_file".into()),
            ..plain_message("stub-a", "assistant", "工具：read_file")
        },
        StoredMessage {
            text: "read_file".into(),
            reasoning_text: "body".into(),
            tool_call_id: Some("tc_b".into()),
            tool_name: Some("read_file".into()),
            ..{
                let mut m = plain_message("result-b", "system", "");
                m.is_tool = true;
                m
            }
        },
    ];
    let ids: Vec<String> = drop_superseded_tool_call_stubs(rows)
        .into_iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(
        ids,
        ["stub-a", "result-b"],
        "id 不同即另一次调用：短卡不得被同名结果吞掉"
    );
}

/// 无 `tool_call_id` 的旧快照短卡：同名结果唯一 → 收编；多张同名结果 → 保留（无法判定是哪一次调用）。
#[test]
fn idless_hydrate_stub_is_dropped_only_when_the_same_name_result_is_unique() {
    let stub = StoredMessage {
        is_tool: true,
        tool_name: Some("read_file".into()),
        ..plain_message("stub", "assistant", "工具：read_file")
    };
    let mut result = plain_message("result", "system", "read_file");
    result.is_tool = true;
    result.tool_name = Some("read_file".into());

    let unique = drop_superseded_tool_call_stubs(vec![stub.clone(), result.clone()]);
    assert_eq!(
        unique.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["result"]
    );

    let mut second = result.clone();
    second.id = "result-2".into();
    let ambiguous = drop_superseded_tool_call_stubs(vec![stub, result, second]);
    assert_eq!(
        ambiguous.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["stub", "result", "result-2"],
        "同名结果不唯一时必须保守保留短卡"
    );
}

/// 每回合「旁白 + 工具」两段的 layout（`tcids[i]` 对应 `u{i}`）。
fn tool_layout(tcids: &[&str]) -> crabmate::cm_api_contract::chat::ConversationLayoutMeta {
    use crabmate::cm_api_contract::chat::{ConversationLayoutMeta, ConversationLayoutSegment};
    let mut segments = Vec::new();
    for (turn, tcid) in tcids.iter().enumerate() {
        let turn_id = format!("u{turn}");
        let seq = segments.len() as u32;
        segments.push(ConversationLayoutSegment {
            turn_id: Some(turn_id.clone()),
            segment_id: format!("seg-before-{tcid}"),
            segment_kind: "assistant_commentary".into(),
            before_tool_call_id: Some((*tcid).into()),
            sequence: seq,
        });
        segments.push(ConversationLayoutSegment {
            turn_id: Some(turn_id),
            segment_id: (*tcid).into(),
            segment_kind: "tool".into(),
            before_tool_call_id: None,
            sequence: seq + 1,
        });
    }
    ConversationLayoutMeta {
        layout_schema_version: 2,
        projection_hash: Some("deadbeef".into()),
        segments,
    }
}

fn tool_row(id: &str, tool_call_id: &str) -> StoredMessage {
    StoredMessage {
        is_tool: true,
        tool_call_id: Some(tool_call_id.into()),
        ..plain_message(id, "tool", "read")
    }
}

fn tool_turn(user_id: &str, text: &str, commentary: &str, tcid: &str) -> Vec<StoredMessage> {
    vec![
        plain_message(user_id, "user", text),
        plain_message(&format!("h_{tcid}_c"), "assistant", commentary),
        tool_row(&format!("h_{tcid}_t"), tcid),
    ]
}

/// 服务端已有 `layout` 的会话：历史行升级成 v2 行键并 stamp，刷新不再整段重建。
#[test]
fn hydration_layout_mapping_stamps_v2_and_stays_stable_across_refreshes() {
    let hydrated = tool_turn("h_u", "go", "先读。", "tc1");
    let mut resp = revision_response(4);
    resp.layout = Some(tool_layout(&["tc1"]));
    let mut session = revision_session(vec![], Some(0));

    apply_hydrated_tail_if_newer(&mut session, hydrated.clone(), &resp);

    assert_eq!(
        session
            .messages
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>(),
        ["h_u", "turn-commentary-tc1", "turn-tool-tc1"]
    );
    assert_eq!(session.layout_schema_version, CURRENT_LAYOUT_SCHEMA_VERSION);
    assert!(session.has_v2_finalized_rows());

    // 同一 revision 再刷新：映射幂等 → 内容不变 → 不重新赋值（也就不会触发持久化）。
    let first = session.messages.clone();
    apply_hydrated_tail_if_newer(&mut session, hydrated, &resp);
    assert_eq!(session.messages, first);
}

/// 映射成功后的追加路径：新回合也只追加，且新回合带 v2 行键。
#[test]
fn hydration_layout_mapping_appends_new_turn_with_v2_keys() {
    let mut resp = revision_response(4);
    resp.layout = Some(tool_layout(&["tc1"]));
    let mut session = revision_session(vec![], Some(0));
    apply_hydrated_tail_if_newer(
        &mut session,
        tool_turn("h_u1", "q1", "先读。", "tc1"),
        &resp,
    );
    let local = session.messages.clone();

    let mut next_resp = revision_response(5);
    next_resp.layout = Some(tool_layout(&["tc1", "tc2"]));
    let mut hydrated = tool_turn("h_u1", "q1", "先读。", "tc1");
    hydrated.extend(tool_turn("h_u2", "q2", "再读。", "tc2"));
    next_resp.window_start_index = 0;
    apply_hydrated_tail_if_newer(&mut session, hydrated, &next_resp);

    let ids: Vec<_> = session.messages.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(
        ids[..local.len()],
        local.iter().map(|m| m.id.as_str()).collect::<Vec<_>>()[..]
    );
    assert_eq!(
        &ids[local.len()..],
        ["h_u2", "turn-commentary-tc2", "turn-tool-tc2"]
    );
}

/// 段序列对不齐（服务端 layout 与本地快照不同步）时不得 stamp v2，保持 legacy。
#[test]
fn hydration_layout_mismatch_keeps_legacy_ids() {
    let hydrated = vec![
        plain_message("h_u", "user", "go"),
        plain_message("h_a", "assistant", "不是旁白"),
    ];
    let mut resp = revision_response(4);
    resp.layout = Some(tool_layout(&["tc1"]));
    let mut session = revision_session(vec![], Some(0));

    apply_hydrated_tail_if_newer(&mut session, hydrated, &resp);

    assert_eq!(
        session
            .messages
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>(),
        ["h_u", "h_a"]
    );
    assert_eq!(session.layout_schema_version, LEGACY_LAYOUT_SCHEMA_VERSION);
}
