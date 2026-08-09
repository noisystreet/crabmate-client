//! v1 会话尾部水合 adapter：无稳定 v2 projection key 时由 [`merge_session_tail`] 合并。
//!
//! ## 算法（维护时只改本模块）
//!
//! 1. **服务端快照净化**：去掉展示层隐藏的注入类 `user`。
//! 2. **本地 plain user 保留**：服务端缺失或误含注入 user 时，按 local 补回真实 user。
//! 3. **按 local 顺序回放**：已落盘行取服务端内容；timeline / 子目标 / 流式工具等 local 独占行原位保留；
//!    canonical assistant / tool 由 id 或池匹配；`loading` 占位跳过；`placed_ids` 保证幂等。

use std::collections::{HashMap, HashSet, VecDeque};

use crate::storage::StoredMessage;

fn messages_contain_loading(messages: &[StoredMessage]) -> bool {
    messages
        .iter()
        .any(|m| m.state.as_ref().is_some_and(|s| s.is_loading()))
}

fn is_plain_user_bubble(m: &StoredMessage) -> bool {
    m.role == "user"
        && !m.is_tool
        && !crabmate_display_rules::user_message_should_hide_for_chat_display(m.text.as_str())
}

fn local_plain_user_bubbles_preserved(
    server_msgs: &[StoredMessage],
    local_msgs: &[StoredMessage],
) -> Vec<StoredMessage> {
    local_msgs
        .iter()
        .filter(|m| {
            if !is_plain_user_bubble(m) {
                return false;
            }
            let t = m.text.trim();
            if t.is_empty() {
                return false;
            }
            !server_msgs
                .iter()
                .any(|s| s.role == "user" && s.text.trim() == t)
        })
        .cloned()
        .collect()
}

fn inject_preserved_plain_users(
    mut server: Vec<StoredMessage>,
    local_tail: &[StoredMessage],
) -> Vec<StoredMessage> {
    let preserved = local_plain_user_bubbles_preserved(&server, local_tail);
    if preserved.is_empty() {
        return server;
    }
    server.retain(|m| {
        !(m.role == "user"
            && crabmate_display_rules::user_message_should_hide_for_chat_display(m.text.as_str()))
    });
    if let Some(pos) = server.iter().position(|m| m.role == "user") {
        for (i, u) in preserved.iter().enumerate() {
            server.insert(pos + i, u.clone());
        }
    } else {
        let mut prefixed = preserved;
        prefixed.append(&mut server);
        server = prefixed;
    }
    server
}

fn is_canonical_assistant_message(m: &StoredMessage) -> bool {
    m.role == "assistant"
        && !m.is_tool
        && !m
            .state
            .as_ref()
            .is_some_and(|s| s.is_local_timeline_snapshot_row())
}

fn is_local_only_row_to_replay(
    m: &StoredMessage,
    server_msgs: &[StoredMessage],
    local_msgs: &[StoredMessage],
) -> bool {
    let preserve_streaming_tools = messages_contain_loading(local_msgs);
    let server_msg_ids: HashSet<_> = server_msgs.iter().map(|x| x.id.as_str()).collect();
    if m.is_tool && !server_msg_ids.contains(m.id.as_str()) {
        return preserve_streaming_tools;
    }
    if let Some(ref state) = m.state {
        if state.is_local_timeline_snapshot_row() && !server_msg_ids.contains(m.id.as_str()) {
            return crate::timeline_scan::should_preserve_local_timeline_on_hydrate(m, server_msgs);
        }
    }
    false
}

fn pop_next_unplaced(
    queue: &mut VecDeque<StoredMessage>,
    placed_ids: &HashSet<String>,
) -> Option<StoredMessage> {
    while let Some(h) = queue.pop_front() {
        if !placed_ids.contains(&h.id) {
            return Some(h);
        }
    }
    None
}

fn push_once(out: &mut Vec<StoredMessage>, placed_ids: &mut HashSet<String>, msg: StoredMessage) {
    if placed_ids.insert(msg.id.clone()) {
        out.push(msg);
    }
}

fn matching_server_user(
    server: &[StoredMessage],
    placed_ids: &HashSet<String>,
    local: &StoredMessage,
) -> Option<StoredMessage> {
    server
        .iter()
        .find(|h| {
            h.role == "user" && !placed_ids.contains(&h.id) && h.text.trim() == local.text.trim()
        })
        .cloned()
}

fn hydrated_or_matching_user(
    hydrated_by_id: &HashMap<String, StoredMessage>,
    server: &[StoredMessage],
    placed_ids: &HashSet<String>,
    local: &StoredMessage,
) -> StoredMessage {
    hydrated_by_id
        .get(&local.id)
        .cloned()
        .or_else(|| matching_server_user(server, placed_ids, local))
        .unwrap_or_else(|| local.clone())
}

fn push_next_from_pool(
    out: &mut Vec<StoredMessage>,
    placed_ids: &mut HashSet<String>,
    pool: &mut VecDeque<StoredMessage>,
) {
    if let Some(h) = pop_next_unplaced(pool, placed_ids) {
        push_once(out, placed_ids, h);
    }
}

/// 水合/流式调用短卡：无结果明细（空 reasoning），文案多为「工具：name」/「Tool: name」。
fn is_hydrate_tool_call_stub(m: &StoredMessage) -> bool {
    if !m.is_tool || !m.reasoning_text.trim().is_empty() {
        return false;
    }
    let t = m.text.trim();
    t.starts_with("工具：")
        || t.starts_with("Tool:")
        || t == "工具调用"
        || t.eq_ignore_ascii_case("tool call")
        || t.eq_ignore_ascii_case("tool calls")
}

fn trimmed_nonempty(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|x| !x.is_empty())
}

/// 短卡是否已被非短卡结果代表。
/// - 双边有 `tool_call_id`：仅 id 相等才算（避免同名另一调用的 orphan stub 被误丢）
/// - 短卡无 id（旧快照）：才允许用同名结果收编
fn tool_stub_superseded_by_placed_result(out: &[StoredMessage], stub: &StoredMessage) -> bool {
    let stub_id = trimmed_nonempty(stub.tool_call_id.as_deref());
    out.iter().any(|m| {
        if !m.is_tool || is_hydrate_tool_call_stub(m) {
            return false;
        }
        let placed_id = trimmed_nonempty(m.tool_call_id.as_deref());
        match (stub_id, placed_id) {
            (Some(a), Some(b)) => a == b,
            (Some(_), None) => false,
            (None, _) => match (
                trimmed_nonempty(stub.tool_name.as_deref()),
                trimmed_nonempty(m.tool_name.as_deref()),
            ) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            },
        }
    })
}

fn tool_pool_match_score(local: &StoredMessage, candidate: &StoredMessage) -> i32 {
    let mut score = 0;
    if let (Some(a), Some(b)) = (
        local
            .tool_call_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty()),
        candidate
            .tool_call_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty()),
    ) {
        if a == b {
            score += 100;
        }
    }
    if let (Some(a), Some(b)) = (
        local
            .tool_name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty()),
        candidate
            .tool_name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty()),
    ) {
        if a == b {
            score += 10;
        }
    }
    if candidate.tool_call_id.is_some() || !candidate.reasoning_text.trim().is_empty() {
        score += 5;
    }
    if is_hydrate_tool_call_stub(candidate) {
        score -= 50;
    }
    score
}

fn push_best_tool_from_pool(
    out: &mut Vec<StoredMessage>,
    placed_ids: &mut HashSet<String>,
    pool: &mut VecDeque<StoredMessage>,
    local: &StoredMessage,
) {
    let mut best_idx: Option<usize> = None;
    let mut best_score = i32::MIN;
    for (idx, candidate) in pool.iter().enumerate() {
        if placed_ids.contains(&candidate.id) {
            continue;
        }
        let score = tool_pool_match_score(local, candidate);
        if best_idx.is_none() || score > best_score {
            best_score = score;
            best_idx = Some(idx);
        }
    }
    if let Some(idx) = best_idx {
        if let Some(h) = pool.remove(idx) {
            push_once(out, placed_ids, h);
            return;
        }
    }
    push_next_from_pool(out, placed_ids, pool);
}

struct PlaceLocalRowCtx<'a> {
    server: &'a [StoredMessage],
    local_tail: &'a [StoredMessage],
    hydrated_by_id: &'a HashMap<String, StoredMessage>,
    out: &'a mut Vec<StoredMessage>,
    placed_ids: &'a mut HashSet<String>,
    assistant_pool: &'a mut VecDeque<StoredMessage>,
    tool_pool: &'a mut VecDeque<StoredMessage>,
}

fn place_one_local_row(local: &StoredMessage, ctx: &mut PlaceLocalRowCtx<'_>) {
    if is_local_only_row_to_replay(local, ctx.server, ctx.local_tail) {
        push_once(ctx.out, ctx.placed_ids, local.clone());
        return;
    }
    if local.role == "user" && !local.is_tool {
        let h = hydrated_or_matching_user(ctx.hydrated_by_id, ctx.server, ctx.placed_ids, local);
        push_once(ctx.out, ctx.placed_ids, h);
        return;
    }
    if let Some(h) = ctx.hydrated_by_id.get(&local.id) {
        push_once(ctx.out, ctx.placed_ids, h.clone());
        return;
    }
    if local.state.as_ref().is_some_and(|s| s.is_loading()) {
        return;
    }
    if local.role == "assistant" && !local.is_tool {
        push_next_from_pool(ctx.out, ctx.placed_ids, ctx.assistant_pool);
        return;
    }
    if local.is_tool {
        push_best_tool_from_pool(ctx.out, ctx.placed_ids, ctx.tool_pool, local);
    }
}

fn append_unplaced_server_rows(
    server: Vec<StoredMessage>,
    out: &mut Vec<StoredMessage>,
    placed_ids: &mut HashSet<String>,
) {
    for h in server {
        if placed_ids.contains(&h.id) {
            continue;
        }
        // 丢弃已被结果卡代表的调用短卡，避免 append 成「工具→助手→工具」夹心。
        if is_hydrate_tool_call_stub(&h) && tool_stub_superseded_by_placed_result(out, &h) {
            placed_ids.insert(h.id.clone());
            continue;
        }
        push_once(out, placed_ids, h);
    }
}

fn replay_local_order_against_server(
    server: Vec<StoredMessage>,
    local_tail: &[StoredMessage],
) -> Vec<StoredMessage> {
    let hydrated_by_id: HashMap<_, _> = server.iter().map(|m| (m.id.clone(), m.clone())).collect();
    let mut assistant_pool: VecDeque<_> = server
        .iter()
        .filter(|m| is_canonical_assistant_message(m))
        .cloned()
        .collect();
    let mut tool_pool: VecDeque<_> = server.iter().filter(|m| m.is_tool).cloned().collect();

    let mut out = Vec::with_capacity(local_tail.len().max(server.len()));
    let mut placed_ids = HashSet::new();

    for local in local_tail {
        place_one_local_row(
            local,
            &mut PlaceLocalRowCtx {
                server: &server,
                local_tail,
                hydrated_by_id: &hydrated_by_id,
                out: &mut out,
                placed_ids: &mut placed_ids,
                assistant_pool: &mut assistant_pool,
                tool_pool: &mut tool_pool,
            },
        );
    }

    append_unplaced_server_rows(server, &mut out, &mut placed_ids);
    out
}

/// 将服务端尾部快照与 v1 `local_tail` 合并为单一消息序列。
#[must_use]
pub(crate) fn merge_session_tail(
    server_hydrated: Vec<StoredMessage>,
    local_tail: &[StoredMessage],
) -> Vec<StoredMessage> {
    let server = inject_preserved_plain_users(server_hydrated, local_tail);
    replay_local_order_against_server(server, local_tail)
}

#[cfg(test)]
mod golden {
    use super::*;
    use crate::storage::{StoredMessage, StoredMessageState};
    use crate::timeline_scan::timeline_state_tool;

    fn user_msg(id: &str, text: &str) -> StoredMessage {
        StoredMessage {
            id: id.into(),
            role: "user".into(),
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

    fn assistant_msg(id: &str, text: &str) -> StoredMessage {
        StoredMessage {
            id: id.into(),
            role: "assistant".into(),
            text: text.into(),
            reasoning_text: String::new(),
            image_urls: vec![],
            state: None,
            is_tool: false,
            tool_call_id: None,
            tool_name: None,
            created_at: 2,
        }
    }

    fn tool_msg(id: &str) -> StoredMessage {
        StoredMessage {
            id: id.into(),
            role: "system".into(),
            text: "list_tree".into(),
            reasoning_text: String::new(),
            image_urls: vec![],
            state: Some(timeline_state_tool(id, true)),
            is_tool: true,
            tool_call_id: None,
            tool_name: Some("list_tree".into()),
            created_at: 1,
        }
    }

    fn roles(msgs: &[StoredMessage]) -> Vec<&str> {
        msgs.iter().map(|m| m.role.as_str()).collect()
    }

    #[test]
    fn golden_restores_plain_user_over_injection() {
        const REAL: &str = "用户真实诉求";
        let reject = format!(
            "{} 请仅输出 JSON",
            crabmate_display_rules::STAGED_PLANNER_TOOL_CALL_REJECT_PREFIX
        );
        let server = vec![user_msg("srv-inj", &reject), assistant_msg("a1", "ok")];
        let local = vec![user_msg("local-u", REAL)];
        let merged = merge_session_tail(server, &local);
        assert!(
            merged
                .iter()
                .any(|m| m.role == "user" && m.text.contains(REAL))
        );
        assert!(!merged.iter().any(|m| {
            crabmate_display_rules::is_planner_tool_call_reject_injected_user_content(
                m.text.as_str(),
            )
        }));
    }

    #[test]
    fn golden_user_before_assistants_when_server_omits_user() {
        let local = vec![user_msg("local-u", "你好")];
        let server = vec![assistant_msg("a1", "你好！")];
        let merged = merge_session_tail(server, &local);
        assert_eq!(roles(&merged), vec!["user", "assistant"]);
        assert_eq!(merged[0].text, "你好");
        assert_eq!(merged[1].id, "a1");
    }

    #[test]
    fn golden_greeting_turn_merges_server_answer() {
        let local = vec![user_msg("u1", "你好"), assistant_msg("a-local", "你好！")];
        let server = vec![user_msg("u1", "你好"), assistant_msg("a-srv", "你好！")];
        let merged = merge_session_tail(server, &local);
        assert_eq!(roles(&merged), vec!["user", "assistant"]);
        assert_eq!(merged[0].text, "你好");
        assert_eq!(merged[1].id, "a-srv");
    }

    #[test]
    fn golden_server_omits_user_keeps_local_user_before_answer() {
        let local = vec![
            user_msg("u-local", "你好"),
            assistant_msg("a-local", "你好！"),
        ];
        let server = vec![assistant_msg("a-srv", "你好！我是 CrabMate 的 AI 助手。")];
        let merged = merge_session_tail(server, &local);
        assert_eq!(roles(&merged), vec!["user", "assistant"]);
        assert_eq!(merged[0].text, "你好");
        assert_eq!(merged[1].id, "a-srv");
    }

    #[test]
    fn golden_replays_server_answer_over_local_draft() {
        let local = vec![
            user_msg("u1", "question"),
            assistant_msg("a-local", "stream draft"),
        ];
        let server = vec![
            user_msg("u1", "question"),
            assistant_msg("a-srv", "final answer"),
        ];
        let merged = merge_session_tail(server, &local);
        let ids: Vec<_> = merged.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["u1", "a-srv"]);
    }

    #[test]
    fn golden_two_turn_skills_idempotent() {
        let local = vec![
            user_msg("u1", "你好"),
            assistant_msg("a1", "你好！"),
            user_msg("u2", "你有哪些技能"),
            assistant_msg("a2-local", ""),
        ];
        let server = vec![
            user_msg("u1", "你好"),
            assistant_msg("a1", "你好！"),
            user_msg("u2", "你有哪些技能"),
            assistant_msg("a2-srv", "技能列表…"),
        ];
        let once = merge_session_tail(server.clone(), &local);
        let twice = merge_session_tail(once.clone(), &local);
        assert_eq!(
            once.len(),
            4,
            "{:?}",
            once.iter().map(|m| &m.id).collect::<Vec<_>>()
        );
        assert_eq!(once.len(), twice.len());
        assert_eq!(roles(&once), vec!["user", "assistant", "user", "assistant"]);
        assert_eq!(once[3].id, "a2-srv");
    }

    #[test]
    fn golden_skips_loading_and_replays_server_answer() {
        let local = vec![
            user_msg("u1", "question"),
            StoredMessage {
                id: "a-local".into(),
                role: "assistant".into(),
                text: String::new(),
                reasoning_text: String::new(),
                image_urls: vec![],
                state: Some(StoredMessageState::Loading),
                is_tool: false,
                tool_call_id: None,
                tool_name: None,
                created_at: 2,
            },
        ];
        let server = vec![
            user_msg("u1", "question"),
            assistant_msg("a-srv", "final answer"),
        ];
        let merged = merge_session_tail(server, &local);
        let ids: Vec<_> = merged.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["u1", "a-srv"]);
    }

    #[test]
    fn golden_tool_pool_by_local_order() {
        let local = vec![
            user_msg("u1", "question"),
            tool_msg("sse-tool"),
            assistant_msg("a-local", "draft"),
        ];
        let server = vec![
            user_msg("u1", "question"),
            tool_msg("h_0_0"),
            assistant_msg("a-srv", "final answer"),
        ];
        let merged = merge_session_tail(server, &local);
        let ids: Vec<_> = merged.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["u1", "h_0_0", "a-srv"]);
    }

    /// 对齐 `chat_export_20260808_212552.md`：服务端水合出 call 短卡 + result 卡，
    /// 本地流式只剩 1 条工具；FIFO tool_pool 会把结果挤到终答后。
    /// 期望：同一次调用只保留 1 条工具，且工具在终答之前。
    #[test]
    fn golden_dedupes_hydrate_call_and_result_cards() {
        let local = vec![
            user_msg("u1", "现在时间是什么"),
            StoredMessage {
                id: "sse-tool".into(),
                role: "system".into(),
                text: "get_current_time".into(),
                reasoning_text: "当前时间：2026-08-08 21:25:05".into(),
                image_urls: vec![],
                state: Some(timeline_state_tool("sse-tool", true)),
                is_tool: true,
                tool_call_id: Some("tc_time".into()),
                tool_name: Some("get_current_time".into()),
                created_at: 1,
            },
            assistant_msg("a-local", "当前时间是 **2026-08-08 21:25:05**。"),
        ];
        let server = vec![
            user_msg("u1", "现在时间是什么"),
            StoredMessage {
                id: "h_call".into(),
                role: "system".into(),
                text: "工具：get_current_time".into(),
                reasoning_text: String::new(),
                image_urls: vec![],
                state: Some(timeline_state_tool("h_call", true)),
                is_tool: true,
                tool_call_id: None,
                tool_name: Some("get_current_time".into()),
                created_at: 1,
            },
            StoredMessage {
                id: "h_result".into(),
                role: "system".into(),
                text: "get_current_time".into(),
                reasoning_text: "当前时间：2026-08-08 21:25:05".into(),
                image_urls: vec![],
                state: Some(timeline_state_tool("h_result", true)),
                is_tool: true,
                tool_call_id: Some("tc_time".into()),
                tool_name: Some("get_current_time".into()),
                created_at: 2,
            },
            assistant_msg("a-srv", "当前时间是 **2026-08-08 21:25:05**。"),
        ];
        let merged = merge_session_tail(server, &local);
        let tools: Vec<_> = merged.iter().filter(|m| m.is_tool).collect();
        assert_eq!(
            tools.len(),
            1,
            "ids={:?}",
            merged.iter().map(|m| m.id.as_str()).collect::<Vec<_>>()
        );
        let tool_idx = merged.iter().position(|m| m.is_tool).unwrap();
        let answer_idx = merged
            .iter()
            .position(|m| m.role == "assistant" && !m.is_tool)
            .unwrap();
        assert!(
            tool_idx < answer_idx,
            "tool must precede final answer; ids={:?}",
            merged.iter().map(|m| m.id.as_str()).collect::<Vec<_>>()
        );
        // 应保留结果卡（有 tool_call_id / detail），而非仅「工具：name」短卡。
        assert!(
            tools[0].tool_call_id.as_deref() == Some("tc_time")
                || !tools[0].reasoning_text.is_empty(),
            "kept stub-only card: text={:?}",
            tools[0].text
        );
    }

    /// 同名另一调用的 orphan stub（有独立 tool_call_id）不得被已放置结果误丢。
    #[test]
    fn golden_keeps_orphan_stub_with_distinct_call_id() {
        let local = vec![
            user_msg("u1", "q"),
            StoredMessage {
                id: "sse-1".into(),
                role: "system".into(),
                text: "list_tree".into(),
                reasoning_text: ".".into(),
                image_urls: vec![],
                state: Some(timeline_state_tool("sse-1", true)),
                is_tool: true,
                tool_call_id: Some("tc_1".into()),
                tool_name: Some("list_tree".into()),
                created_at: 1,
            },
            assistant_msg("a-local", "done"),
        ];
        let server = vec![
            user_msg("u1", "q"),
            StoredMessage {
                id: "h_result".into(),
                role: "system".into(),
                text: "list_tree".into(),
                reasoning_text: ".".into(),
                image_urls: vec![],
                state: Some(timeline_state_tool("h_result", true)),
                is_tool: true,
                tool_call_id: Some("tc_1".into()),
                tool_name: Some("list_tree".into()),
                created_at: 1,
            },
            StoredMessage {
                id: "h_orphan".into(),
                role: "system".into(),
                text: "工具：list_tree".into(),
                reasoning_text: String::new(),
                image_urls: vec![],
                state: Some(timeline_state_tool("h_orphan", true)),
                is_tool: true,
                tool_call_id: Some("tc_2".into()),
                tool_name: Some("list_tree".into()),
                created_at: 2,
            },
            assistant_msg("a-srv", "done"),
        ];
        let merged = merge_session_tail(server, &local);
        let ids: Vec<_> = merged.iter().map(|m| m.id.as_str()).collect();
        assert!(
            ids.contains(&"h_orphan"),
            "orphan stub with distinct call id must remain; ids={ids:?}"
        );
        assert!(ids.contains(&"h_result"), "ids={ids:?}");
    }

    /// 英文短卡指纹也应被识别并丢弃（当结果已放置）。
    #[test]
    fn golden_dedupes_english_tool_call_stub() {
        let local = vec![
            user_msg("u1", "q"),
            StoredMessage {
                id: "sse-tool".into(),
                role: "system".into(),
                text: "get_current_time".into(),
                reasoning_text: "now".into(),
                image_urls: vec![],
                state: Some(timeline_state_tool("sse-tool", true)),
                is_tool: true,
                tool_call_id: Some("tc_en".into()),
                tool_name: Some("get_current_time".into()),
                created_at: 1,
            },
            assistant_msg("a-local", "ok"),
        ];
        let server = vec![
            user_msg("u1", "q"),
            StoredMessage {
                id: "h_call_en".into(),
                role: "system".into(),
                text: "Tool: get_current_time".into(),
                reasoning_text: String::new(),
                image_urls: vec![],
                state: Some(timeline_state_tool("h_call_en", true)),
                is_tool: true,
                tool_call_id: None,
                tool_name: Some("get_current_time".into()),
                created_at: 1,
            },
            StoredMessage {
                id: "h_result".into(),
                role: "system".into(),
                text: "get_current_time".into(),
                reasoning_text: "now".into(),
                image_urls: vec![],
                state: Some(timeline_state_tool("h_result", true)),
                is_tool: true,
                tool_call_id: Some("tc_en".into()),
                tool_name: Some("get_current_time".into()),
                created_at: 2,
            },
            assistant_msg("a-srv", "ok"),
        ];
        let merged = merge_session_tail(server, &local);
        let tools: Vec<_> = merged.iter().filter(|m| m.is_tool).collect();
        assert_eq!(
            tools.len(),
            1,
            "ids={:?}",
            merged.iter().map(|m| &m.id).collect::<Vec<_>>()
        );
        assert_eq!(tools[0].id, "h_result");
    }

    #[test]
    fn golden_drops_ephemeral_sse_tools_after_turn_complete() {
        let local = vec![tool_msg("sse-1"), tool_msg("h_99_1")];
        let server = vec![tool_msg("h_0_0")];
        let merged = merge_session_tail(server, &local);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].id, "h_0_0");
    }

    #[test]
    fn golden_keeps_sse_tool_while_loading() {
        let local = vec![
            StoredMessage {
                id: "a-loading".into(),
                role: "assistant".into(),
                text: String::new(),
                reasoning_text: String::new(),
                image_urls: vec![],
                state: Some(StoredMessageState::Loading),
                is_tool: false,
                tool_call_id: None,
                tool_name: None,
                created_at: 0,
            },
            tool_msg("sse-1"),
        ];
        let server = vec![tool_msg("h_0_0")];
        let merged = merge_session_tail(server, &local);
        assert!(merged.iter().any(|m| m.id == "sse-1"));
    }
}
