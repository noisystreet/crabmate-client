// 会话 / 消息模型与派生判定已下沉到 `crabmate-client-api`：Desktop 与 TUI 共用一份实现，
// 避免「空白会话 / 可丢弃空白行 / 复用或新建 / 冷启动复用」这类语义各写一遍再漂移
// （TUI 曾把缺失标题当作默认标题，而 Web 的 `is_default_session_title("")` 返回 false）。
//
// re-export 是为了让 `crate::storage::X` 的既有调用点（59 个文件）无需改动。
pub use crabmate_client_api::session_model::{
    CURRENT_LAYOUT_SCHEMA_VERSION, ChatSession, DEFAULT_CHAT_SESSION_TITLE,
    LEGACY_LAYOUT_SCHEMA_VERSION, StoredMessage, StoredMessageState, TIMELINE_UI_STATE_KEY,
    V2_COMMENTARY_ROW_ID_PREFIX, V2_FINAL_ANSWER_ROW_ID, V2_THINK_ROW_ID_PREFIX,
};

/// 进程重启后不再有挂起的 SSE；本地持久化的助手 `loading` 占位若不清理会永久显示「生成中」。
/// 在从 `/user-data` 恢复会话列表时调用（见 `wire_initial_sessions_from_storage`）。
pub fn clear_stale_assistant_loading_states(messages: &mut [StoredMessage]) {
    use crate::message_loading::is_loading_plain_assistant;
    for m in messages.iter_mut() {
        if is_loading_plain_assistant(m) {
            m.state = None;
        }
    }
}

/// 从 `/user-data` 恢复会话时：清助手 `Loading`，并收口无 SSE 可配对的僵尸工具「执行中」。
pub fn clear_stale_stream_loading_states(messages: &mut [StoredMessage], loc: crate::i18n::Locale) {
    clear_stale_assistant_loading_states(messages);
    crate::message_loading::finalize_loading_tool_placeholders(
        messages,
        loc,
        crate::message_loading::ToolLoadingFinalizeKind::OrphanStale,
    );
}

/// 与 `GET /workspace` 返回的 `path` 对齐的规范化字符串（与服务端分桶一致）。
#[must_use]
pub fn normalize_workspace_partition_path(path: &str) -> String {
    path.trim().trim_end_matches('/').to_string()
}

/// 新会话行 id。时间与熵在这里注入（Web 用 `js_sys`），格式由共享层统一。
pub fn make_session_id() -> String {
    crabmate_client_api::new_session_id(
        js_sys::Date::now() as i64,
        (js_sys::Math::random() * 1_000_000_000.0) as u32,
    )
}

pub fn ensure_at_least_one(
    mut sessions: Vec<ChatSession>,
    default_title: String,
) -> (Vec<ChatSession>, String) {
    if !sessions.is_empty() {
        let id = sessions[0].id.clone();
        return (sessions, id);
    }
    let s = crabmate_client_api::new_empty_session(
        make_session_id(),
        default_title,
        js_sys::Date::now() as i64,
        None,
    );
    let id = s.id.clone();
    sessions.push(s);
    (sessions, id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_roundtrip_preserves_wire_strings() {
        let m = StoredMessage {
            id: "m".into(),
            role: "assistant".into(),
            text: String::new(),
            reasoning_text: String::new(),
            image_urls: vec![],
            state: Some(StoredMessageState::Loading),
            is_tool: false,
            tool_call_id: None,
            tool_name: None,
            created_at: 0,
        };
        let json = serde_json::to_string(&m).expect("serialize");
        let back: StoredMessage = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.state, Some(StoredMessageState::Loading));
    }

    #[test]
    fn from_wire_classifies_timeline_json() {
        let raw = r#"{"k":"cm_tl","t":"tool","msg":"x","ok":true}"#.to_string();
        let st = StoredMessageState::from_wire(raw.clone());
        assert!(matches!(st, StoredMessageState::TimelineUiJson(s) if s == raw));
    }

    #[test]
    fn clear_stale_assistant_loading_clears_assistant_only() {
        let mut msgs = vec![
            StoredMessage {
                id: "a".into(),
                role: "assistant".into(),
                text: "partial".into(),
                reasoning_text: String::new(),
                image_urls: vec![],
                state: Some(StoredMessageState::Loading),
                is_tool: false,
                tool_call_id: None,
                tool_name: None,
                created_at: 0,
            },
            StoredMessage {
                id: "u".into(),
                role: "user".into(),
                text: "hi".into(),
                reasoning_text: String::new(),
                image_urls: vec![],
                state: Some(StoredMessageState::Loading),
                is_tool: false,
                tool_call_id: None,
                tool_name: None,
                created_at: 0,
            },
        ];
        clear_stale_assistant_loading_states(&mut msgs);
        assert!(msgs[0].state.is_none());
        assert_eq!(msgs[1].state, Some(StoredMessageState::Loading));
    }

    #[test]
    fn clear_stale_stream_loading_finalizes_orphan_tool() {
        let mut msgs = vec![StoredMessage {
            id: "t".into(),
            role: "system".into(),
            text: "工具：http_fetch".into(),
            reasoning_text: "tool: http_fetch\nstatus: running".into(),
            image_urls: vec![],
            state: Some(StoredMessageState::Loading),
            is_tool: true,
            tool_call_id: Some("c1".into()),
            tool_name: Some("http_fetch".into()),
            created_at: 0,
        }];
        clear_stale_stream_loading_states(&mut msgs, crate::i18n::Locale::ZhHans);
        assert!(msgs[0].state.is_none());
        assert!(msgs[0].reasoning_text.contains("interrupted (stale)"));
        assert!(msgs[0].text.contains("已中断"));
    }

    #[test]
    fn normalize_workspace_partition_trims_slash() {
        assert_eq!(normalize_workspace_partition_path("/tmp/ws/"), "/tmp/ws");
    }

    #[test]
    fn legacy_session_without_layout_version_defaults_to_v1() {
        let session: ChatSession = serde_json::from_str(r#"{"id":"s","messages":[]}"#)
            .expect("deserialize legacy session");
        assert_eq!(session.layout_schema_version, LEGACY_LAYOUT_SCHEMA_VERSION);
        assert!(!session.has_v2_layout_projection());
    }

    #[test]
    fn stable_commentary_key_upgrades_cache_to_v2() {
        let mut session: ChatSession = serde_json::from_str(
            r#"{"id":"s","messages":[{"id":"turn-commentary-t1","role":"assistant"}]}"#,
        )
        .expect("deserialize pre-versioned v2 session");
        session.normalize_layout_schema_version();
        assert_eq!(session.layout_schema_version, CURRENT_LAYOUT_SCHEMA_VERSION);
        assert!(session.has_v2_layout_projection());
    }
}
