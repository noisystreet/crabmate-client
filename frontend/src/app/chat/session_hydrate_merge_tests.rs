use super::hydration_content_guard_outcome;
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
        layout: None,
        context_artifacts: vec![],
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

#[test]
fn same_revision_keeps_nonempty_local_projection() {
    let local = plain_message("turn-commentary-tc_read", "assistant", "已关闭的本地旁注");
    let session = revision_session(vec![local], Some(7));
    assert!(!should_merge_hydrated_messages(
        &session,
        &revision_response(7)
    ));
    assert!(!should_merge_hydrated_messages(
        &session,
        &revision_response(6)
    ));
    assert!(should_merge_hydrated_messages(
        &session,
        &revision_response(8)
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
fn empty_v2_cache_uses_legacy_adapter_for_canonical_hydration() {
    let mut session = revision_session(vec![], Some(7));
    session.layout_schema_version = CURRENT_LAYOUT_SCHEMA_VERSION;
    let hydrated = vec![plain_message("h-assistant", "assistant", "服务端终答")];

    apply_hydrated_tail_if_newer(&mut session, hydrated, &revision_response(7));

    assert_eq!(session.messages.len(), 1);
    assert_eq!(session.messages[0].id, "h-assistant");
    assert_eq!(session.layout_schema_version, LEGACY_LAYOUT_SCHEMA_VERSION);
}

#[test]
fn v1_history_still_uses_legacy_adapter() {
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
fn merge_tail_page_replays_server_answer_over_local_draft() {
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
        layout: None,
        context_artifacts: vec![],
    };
    let merged = merge_tail_page_into_session_messages(&session, hydrated, &resp);
    let ids: Vec<_> = merged.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, vec!["u1", "a-srv"]);
}

#[test]
fn merge_tail_page_keeps_user_when_server_omits_user() {
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
        layout: None,
        context_artifacts: vec![],
    };
    let merged = merge_tail_page_into_session_messages(&session, hydrated, &resp);
    let roles: Vec<_> = merged.iter().map(|m| m.role.as_str()).collect();
    assert_eq!(roles, vec!["user", "assistant"]);
    assert_eq!(merged[0].text, "你好");
    assert_eq!(merged[1].id, "a-srv");
}

#[test]
fn empty_cache_with_v2_layout_keeps_legacy_ids_and_schema() {
    use crate::conversation_hydrate_layout::{ConversationLayoutMeta, ConversationLayoutSegment};

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
    let mut resp = revision_response(4);
    resp.layout = Some(ConversationLayoutMeta {
        layout_schema_version: CURRENT_LAYOUT_SCHEMA_VERSION,
        projection_hash: Some("ab".into()),
        segments: vec![ConversationLayoutSegment {
            turn_id: Some("u0".into()),
            segment_id: "seg-before-tc1".into(),
            segment_kind: "assistant_commentary".into(),
            before_tool_call_id: Some("tc1".into()),
            sequence: 0,
        }],
    });
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
