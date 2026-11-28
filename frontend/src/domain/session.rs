//! 会话域的**纯规则**：消息序号、用户气泡就地改写、按用户消息截断。
//!
//! 本模块只对 `&mut [ChatSession]` / `&[StoredMessage]` 做变换：不读时钟、不生成 id、
//! 不碰 DOM。规则与后端语义对齐的部分（`before_user_ordinal`）在文档与测试里点明。
//!
//! 由 `crate::session_ops` 用 `pub use` 转发，因此各端调用点无需改动。

use crate::storage::{ChatSession, StoredMessage};

/// 第 `idx` 条消息若为普通用户消息，返回其在本会话中的 **0-based 用户序号**
/// （与 `POST /chat/branch` 的 `before_user_ordinal` 一致）。
///
/// 工具结果气泡（`is_tool`）不计入序号，也不视为用户消息 —— 与后端一致。
#[must_use]
pub fn user_ordinal_for_message_index(messages: &[StoredMessage], idx: usize) -> Option<u64> {
    let m = messages.get(idx)?;
    if m.role != "user" || m.is_tool {
        return None;
    }
    let mut ord = 0_u64;
    for (i, x) in messages.iter().enumerate() {
        if i >= idx {
            break;
        }
        if x.role == "user" && !x.is_tool {
            ord = ord.saturating_add(1);
        }
    }
    Some(ord)
}

/// 就地改用户气泡正文（不截断、不再生）。随后应走
/// `truncate_at_user_message_and_prepare_regenerate`。
///
/// 返回 `false` 表示会话或消息不存在，或目标不是普通用户消息 —— 调用方据此决定是否提示。
pub fn set_user_message_text(
    sessions: &mut [ChatSession],
    active_id: &str,
    user_msg_id: &str,
    new_text: &str,
) -> bool {
    let Some(s) = sessions.iter_mut().find(|sess| sess.id == active_id) else {
        return false;
    };
    let Some(um) = s.messages.iter_mut().find(|m| m.id == user_msg_id) else {
        return false;
    };
    if um.role != "user" || um.is_tool {
        return false;
    }
    um.text = new_text.to_string();
    true
}

/// 截断到指定用户消息之前（含该条及之后全部移除），不追加助手泡。
///
/// 用于 `POST /chat/branch` 成功后的本地镜像，或 `404` 降级路径 —— 两者必须得到同一结果，
/// 否则「服务端已截断、本地没截断」会让下一次水合把旧尾条带回来。
pub fn truncate_at_user_message_branch_local(
    sessions: &mut [ChatSession],
    active_id: &str,
    user_msg_id: &str,
) -> bool {
    let Some(s) = sessions.iter_mut().find(|sess| sess.id == active_id) else {
        return false;
    };
    let Some(idx) = s.messages.iter().position(|m| m.id == user_msg_id) else {
        return false;
    };
    let um = match s.messages.get(idx) {
        Some(m) => m,
        None => return false,
    };
    if um.role != "user" || um.is_tool {
        return false;
    }
    s.messages.truncate(idx);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::CURRENT_LAYOUT_SCHEMA_VERSION;

    fn msg(id: &str, role: &str, text: &str) -> StoredMessage {
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

    fn session_with(id: &str, messages: Vec<StoredMessage>) -> ChatSession {
        ChatSession {
            id: id.into(),
            layout_schema_version: CURRENT_LAYOUT_SCHEMA_VERSION,
            title: "t".into(),
            draft: String::new(),
            pinned: false,
            starred: false,
            server_conversation_id: None,
            server_revision: None,
            workspace_root: None,
            history_total: None,
            history_window_start: None,
            history_has_older: None,
            messages,
            updated_at: 0,
        }
    }

    #[test]
    fn user_ordinal_matches_backend_semantics() {
        let messages = vec![
            msg("1", "user", "a"),
            msg("2", "assistant", "b"),
            msg("3", "user", "c"),
        ];
        assert_eq!(user_ordinal_for_message_index(&messages, 0), Some(0));
        assert_eq!(user_ordinal_for_message_index(&messages, 2), Some(1));
        // 助手行没有用户序号
        assert!(user_ordinal_for_message_index(&messages, 1).is_none());
        // 越界不 panic
        assert!(user_ordinal_for_message_index(&messages, 3).is_none());
    }

    #[test]
    fn user_ordinal_skips_tool_result_rows() {
        let mut tool_row = msg("2", "user", "tool out");
        tool_row.is_tool = true;
        let messages = vec![msg("1", "user", "a"), tool_row, msg("3", "user", "c")];
        // 带 is_tool 的行既不算序号，也不能作为目标行
        assert_eq!(user_ordinal_for_message_index(&messages, 2), Some(1));
        assert!(user_ordinal_for_message_index(&messages, 1).is_none());
    }

    #[test]
    fn truncate_branch_local_drops_from_user_onwards() {
        let mut sessions = vec![session_with(
            "s1",
            vec![
                msg("u0", "user", "first"),
                msg("a0", "assistant", "ok"),
                msg("u1", "user", "retry me"),
            ],
        )];
        assert!(truncate_at_user_message_branch_local(
            &mut sessions,
            "s1",
            "u1"
        ));
        // 与后端一致：`before_user_ordinal`=1 时保留第 0 条用户及其后直到下一条用户之前（含中间助手）。
        assert_eq!(sessions[0].messages.len(), 2);
        assert_eq!(sessions[0].messages[0].id, "u0");
        assert_eq!(sessions[0].messages[1].id, "a0");
    }

    #[test]
    fn truncate_branch_local_rejects_missing_targets() {
        let mut sessions = vec![session_with("s1", vec![msg("u0", "user", "first")])];
        // 会话不存在
        assert!(!truncate_at_user_message_branch_local(
            &mut sessions,
            "nope",
            "u0"
        ));
        // 消息不存在
        assert!(!truncate_at_user_message_branch_local(
            &mut sessions,
            "s1",
            "nope"
        ));
        // 目标不是用户消息
        let mut s2 = vec![session_with("s1", vec![msg("a0", "assistant", "hi")])];
        assert!(!truncate_at_user_message_branch_local(&mut s2, "s1", "a0"));
        // 失败路径不改动原数据
        assert_eq!(sessions[0].messages.len(), 1);
        assert_eq!(s2[0].messages.len(), 1);
    }

    #[test]
    fn set_user_message_text_updates_only_that_user_line() {
        let mut sessions = vec![session_with("s1", vec![msg("u0", "user", "old")])];
        assert!(set_user_message_text(&mut sessions, "s1", "u0", "new"));
        assert_eq!(sessions[0].messages[0].text, "new");
        // 目标不存在时不改、返回 false
        assert!(!set_user_message_text(&mut sessions, "s1", "missing", "x"));
        assert_eq!(sessions[0].messages[0].text, "new");
        // 助手行不能被当作「用户气泡」改写
        let mut other = vec![session_with("s1", vec![msg("a0", "assistant", "keep")])];
        assert!(!set_user_message_text(&mut other, "s1", "a0", "x"));
        assert_eq!(other[0].messages[0].text, "keep");
    }
}
