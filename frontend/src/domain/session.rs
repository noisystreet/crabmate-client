//! 会话域的**纯规则**：消息序号、用户气泡就地改写、按用户消息截断。
//!
//! 本模块只对 `&mut [ChatSession]` / `&[StoredMessage]` 做变换：不读时钟、不生成 id、
//! 不碰 DOM。规则与后端语义对齐的部分（`before_user_ordinal`）在文档与测试里点明。
//!
//! 由 `crate::session_ops` 用 `pub use` 转发，因此各端调用点无需改动。

use std::collections::HashMap;

use crate::i18n::{self, Locale};
use crate::message_loading::{
    ToolLoadingFinalizeKind, finalize_loading_tool_placeholders, is_loading_plain_assistant,
    messages_have_loading_tool,
};
use crate::storage::{ChatSession, StoredMessage, StoredMessageState};

/// 第 `idx` 条消息若为普通用户消息，返回其在本会话中的 **0-based 用户序号**
/// （与 `POST /chat/branch` 的 `before_user_ordinal` 一致）。
///
/// 工具结果气泡（`is_tool`）不计入序号，也不视为用户消息 —— 与后端一致。
///
/// 只有本函数带 `#[must_use]`：它的返回值就是调用方要的东西（序号），丢了必然写错。
/// 而 `truncate_*` / `set_user_message_text` 的 `bool` 在既有调用点上有意被 `let _ =` 忽略
/// （见 `app::chat::message_row_actions`），给它们加 `#[must_use]` 会直接把 clippy 打红。
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

/// 新建的 loading 助手泡（两处「截断后重挂尾泡」共用的形状，避免两处字段漂移）。
fn loading_assistant_message(id: &str, now_ms: i64) -> StoredMessage {
    StoredMessage {
        id: id.to_string(),
        role: "assistant".to_string(),
        text: String::new(),
        reasoning_text: String::new(),
        image_urls: vec![],
        state: Some(StoredMessageState::Loading),
        is_tool: false,
        tool_call_id: None,
        tool_name: None,
        created_at: now_ms,
    }
}

/// 保留指定用户气泡（同 id），删除其后的消息，并挂上新的 loading 助手泡；
/// 返回用户原文、用户图片与新助手 id。
///
/// **时间与 id 由调用方注入**：本层不读时钟、不生成随机 id。Web 侧注入
/// `js_sys::Date::now()` 与 `make_message_id()`。正因为解耦了这两个副作用，这条
/// 「重新生成」路径（最容易把消息序列搞错的几条之一）才有 host 单测。
pub fn truncate_at_user_message_and_prepare_regenerate(
    sessions: &mut [ChatSession],
    active_id: &str,
    user_msg_id: &str,
    new_asst_id: &str,
    now_ms: i64,
) -> Option<(String, Vec<String>, String)> {
    let s = sessions.iter_mut().find(|sess| sess.id == active_id)?;
    let idx = s.messages.iter().position(|m| m.id == user_msg_id)?;
    let um = s.messages.get(idx)?;
    if um.role != "user" || um.is_tool {
        return None;
    }
    let user_msg = um.clone();
    let user_text = user_msg.text.clone();
    let user_images = user_msg.image_urls.clone();
    s.messages.truncate(idx);
    s.messages.push(user_msg);
    s.messages
        .push(loading_assistant_message(new_asst_id, now_ms));
    Some((user_text, user_images, new_asst_id.to_string()))
}

/// 去掉失败助手泡及其后消息，挂上新的 loading 助手泡；返回本回合用户原文、用户图片与新助手 id。
///
/// 与 [`truncate_at_user_message_and_prepare_regenerate`] 同样注入时间与 id。前置条件是
/// 失败泡之前必须紧邻一条用户消息 —— 否则无法确定「重试哪一轮」，返回 `None` 而不是猜。
pub fn prepare_retry_failed_assistant_turn(
    sessions: &mut [ChatSession],
    active_id: &str,
    failed_asst_id: &str,
    new_asst_id: &str,
    now_ms: i64,
) -> Option<(String, Vec<String>, String)> {
    let s = sessions.iter_mut().find(|sess| sess.id == active_id)?;
    let idx = s.messages.iter().position(|m| {
        m.id == failed_asst_id
            && m.role == "assistant"
            && !m.is_tool
            && m.state.as_ref().is_some_and(|s| s.is_error())
    })?;
    if idx == 0 {
        return None;
    }
    if s.messages[idx - 1].role != "user" {
        return None;
    }
    let user_text = s.messages[idx - 1].text.clone();
    let user_images = s.messages[idx - 1].image_urls.clone();
    s.messages.truncate(idx);
    s.messages
        .push(loading_assistant_message(new_asst_id, now_ms));
    Some((user_text, user_images, new_asst_id.to_string()))
}

/// `POST /chat/branch` 成功后把服务端新 revision 镜像到活动会话。
///
/// 会话不存在（用户已切走、或本地条目已被回收）时返回 `false` —— 静默写入一个不存在的会话
/// 会让「分支已成功、本地没跟上」看不出来，下一次水合把旧尾条带回来时就只剩一个无因可查的现象。
pub fn apply_branch_revision(sessions: &mut [ChatSession], active_id: &str, revision: u64) -> bool {
    let Some(s) = sessions.iter_mut().find(|s| s.id == active_id) else {
        return false;
    };
    s.server_revision = Some(revision);
    true
}

/// `/clear`：只清本地这段 transcript 与分页窗口，**不**动 serve 上的会话记录。
///
/// 五个字段必须一起清：只清 `messages` 会让旧的分页水位继续生效，下一次「加载更早」
/// 从错误的 offset 开始（与 `history_*` 三个字段的写入侧同源）。
pub fn clear_session_local(sessions: &mut [ChatSession], session_id: &str) -> bool {
    let Some(s) = sessions.iter_mut().find(|s| s.id == session_id) else {
        return false;
    };
    s.messages.clear();
    s.draft.clear();
    s.history_total = None;
    s.history_window_start = None;
    s.history_has_older = None;
    true
}

/// 会话内仍 `Loading` 的普通助手泡 id（可排除一个），按时间线顺序。
///
/// 调用方按这份 id 先把流式旁路缓冲 flush 进消息，再调
/// [`stop_superseded_assistant_loading_rows`] —— overlay 是一个信号（副作用），
/// 只能留在接线侧，本层只认消息本身。
pub fn superseded_loading_assistant_ids(
    sessions: &[ChatSession],
    session_id: &str,
    keep_asst_id: &str,
) -> Vec<String> {
    let Some(s) = sessions.iter().find(|s| s.id == session_id) else {
        return Vec::new();
    };
    s.messages
        .iter()
        .filter(|m| is_loading_plain_assistant(m) && m.id != keep_asst_id)
        .map(|m| m.id.clone())
        .collect()
}

/// 会话内**最后一条**仍 `Loading` 的普通助手泡 id（中止时要 flush 的那条）。
pub fn last_loading_assistant_id(sessions: &[ChatSession], session_id: &str) -> Option<String> {
    let s = sessions.iter().find(|s| s.id == session_id)?;
    s.messages
        .iter()
        .rev()
        .find(|m| is_loading_plain_assistant(m))
        .map(|m| m.id.clone())
}

/// 单条 `Loading` 助手泡收口为「已停止」：清 `Loading`，空正文补文案、已有正文追加后缀。
///
/// `reasoning_counts_as_content` 是两条路径的真实差异：中止路径把「只到了思维链」的泡也算有内容
/// （否则一句「已中断」会把已经流出来的 reasoning 正文抹掉）；顶替路径只看正文。
fn stop_loading_assistant_row(
    m: &mut StoredMessage,
    loc: Locale,
    reasoning_counts_as_content: bool,
) {
    m.state = None;
    let has_content = !m.text.trim().is_empty()
        || (reasoning_counts_as_content && !m.reasoning_text.trim().is_empty());
    if has_content {
        m.text.push_str(i18n::stream_stopped_suffix(loc));
    } else {
        m.text = i18n::stream_stopped_inline(loc).to_string();
    }
}

/// 把同会话内除 `keep_asst_id` 外仍 `Loading` 的普通助手泡收口为「已停止」，返回收口条数。
///
/// 新一轮流已 `push` 新尾泡时，上一轮被 abort 后迟到的回调会留下僵尸尾泡；
/// 收口它们，`session_has_loading_tool_message` 一类判据才会回到「没有在途」。
pub fn stop_superseded_assistant_loading_rows(
    sessions: &mut [ChatSession],
    session_id: &str,
    keep_asst_id: &str,
    loc: Locale,
) -> usize {
    let Some(s) = sessions.iter_mut().find(|s| s.id == session_id) else {
        return 0;
    };
    let mut stopped = 0;
    for m in s.messages.iter_mut() {
        if !is_loading_plain_assistant(m) || m.id == keep_asst_id {
            continue;
        }
        stop_loading_assistant_row(m, loc, true);
        stopped += 1;
    }
    stopped
}

/// 用户点了「停止」：最后一条 `Loading` 助手泡收口，并把残留的工具占位一并收口为「已终止」。
///
/// 两件事一起做才是「停止」的语义：只收助手泡、留着工具 `Loading`，状态栏会继续显示在途。
pub fn finalize_user_abort_messages(messages: &mut [StoredMessage], loc: Locale) {
    if let Some(m) = messages
        .iter_mut()
        .rev()
        .find(|m| is_loading_plain_assistant(m))
    {
        stop_loading_assistant_row(m, loc, false);
    }
    finalize_loading_tool_placeholders(messages, loc, ToolLoadingFinalizeKind::UserStopped);
}

/// 同上，但按会话 id 定位；会话不存在时返回 `false`（什么都没改）。
pub fn finalize_user_abort_in_session(
    sessions: &mut [ChatSession],
    session_id: &str,
    loc: Locale,
) -> bool {
    let Some(s) = sessions.iter_mut().find(|s| s.id == session_id) else {
        return false;
    };
    finalize_user_abort_messages(&mut s.messages, loc);
    true
}

/// 该会话仍有 `Loading` 工具行时收口为「已终止」；没有则一条不动，返回是否收口过。
///
/// 返回 `bool` 而不只是 `()`：调用方据此判断这次停止是否真的改了会话。
pub fn stop_loading_tool_rows_if_any(
    sessions: &mut [ChatSession],
    session_id: &str,
    loc: Locale,
) -> bool {
    let Some(s) = sessions.iter_mut().find(|s| s.id == session_id) else {
        return false;
    };
    if !messages_have_loading_tool(&s.messages) {
        return false;
    }
    finalize_loading_tool_placeholders(&mut s.messages, loc, ToolLoadingFinalizeKind::UserStopped);
    true
}

/// 把水合前保存的 `reasoning_text` 按消息 id 回灌；返回 `(消息 id, 文本长度)`。
///
/// 返回这份列表而不是就地打日志：日志要 `web_sys::console`，带副作用的东西一旦进了本层，
/// host 单测立刻失效（这正是 #598 §2 记录的那处成因）。
pub fn restore_reasoning_texts(
    sessions: &mut [ChatSession],
    session_id: &str,
    preserved: &HashMap<String, String>,
) -> Vec<(String, usize)> {
    let Some(s) = sessions.iter_mut().find(|s| s.id == session_id) else {
        return Vec::new();
    };
    let mut restored = Vec::new();
    for m in s.messages.iter_mut() {
        if let Some(rt) = preserved.get(&m.id) {
            restored.push((m.id.clone(), rt.len()));
            m.reasoning_text = rt.clone();
        }
    }
    restored
}

/// 测试夹具：两个测试模块共用（原先各自定义一份，字段漂移会静默失效）。
#[cfg(test)]
mod test_support {
    use super::*;
    use crate::storage::CURRENT_LAYOUT_SCHEMA_VERSION;

    /// 带附件的消息。「带图重发」时附件必须原样带回 —— 用空图片构造会让那条断言恒真。
    pub(super) fn msg_with(id: &str, role: &str, text: &str, image_urls: &[&str]) -> StoredMessage {
        StoredMessage {
            id: id.into(),
            role: role.into(),
            text: text.into(),
            reasoning_text: String::new(),
            image_urls: image_urls.iter().map(|u| (*u).to_string()).collect(),
            state: None,
            is_tool: false,
            tool_call_id: None,
            tool_name: None,
            created_at: 0,
        }
    }

    pub(super) fn msg(id: &str, role: &str, text: &str) -> StoredMessage {
        msg_with(id, role, text, &[])
    }

    pub(super) fn failed_assistant(id: &str) -> StoredMessage {
        let mut m = msg(id, "assistant", "boom");
        m.state = Some(StoredMessageState::Error);
        m
    }

    pub(super) fn session_with(id: &str, messages: Vec<StoredMessage>) -> ChatSession {
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
}

#[cfg(test)]
mod tests {
    use super::test_support::{msg, session_with};
    use super::*;

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

    #[test]
    fn branch_revision_writes_only_the_target_session() {
        let mut sessions = vec![session_with("s1", vec![]), session_with("s2", vec![])];
        assert!(apply_branch_revision(&mut sessions, "s2", 7));
        assert_eq!(sessions[0].server_revision, None);
        assert_eq!(sessions[1].server_revision, Some(7));
        // 会话已不在（用户切走 / 本地条目被回收）→ false，且不改动任何会话
        assert!(!apply_branch_revision(&mut sessions, "gone", 9));
        assert_eq!(sessions[1].server_revision, Some(7));
    }

    #[test]
    fn clear_session_local_resets_transcript_and_pagination_window() {
        let mut sessions = vec![session_with("s1", vec![msg("u0", "user", "hi")])];
        sessions[0].draft = "半句".into();
        sessions[0].history_total = Some(40);
        sessions[0].history_window_start = Some(20);
        sessions[0].history_has_older = Some(true);
        assert!(clear_session_local(&mut sessions, "s1"));
        assert!(sessions[0].messages.is_empty());
        assert!(sessions[0].draft.is_empty());
        // 三个分页水位必须一起清：留着会让下一次「加载更早」从错的 offset 开始
        assert_eq!(sessions[0].history_total, None);
        assert_eq!(sessions[0].history_window_start, None);
        assert_eq!(sessions[0].history_has_older, None);
    }

    #[test]
    fn clear_session_local_ignores_unknown_session() {
        let mut sessions = vec![session_with("s1", vec![msg("u0", "user", "hi")])];
        assert!(!clear_session_local(&mut sessions, "gone"));
        // 不存在时一条也不动（不能退化成「清空所有会话」）
        assert_eq!(sessions[0].messages.len(), 1);
    }
}

/// 注入版「重新生成 / 重试」规则的边界。
///
/// 这两条路径原先在函数体内直接读 `js_sys::Date::now()` 并生成随机 id，因此**没有任何单测**；
/// 把时间与 id 改成注入后，第一次可以在 host target 钉住消息序列。
#[cfg(test)]
mod regenerate_tests {
    use super::test_support::{failed_assistant, msg, msg_with, session_with};
    use super::*;

    #[test]
    fn regenerate_keeps_user_row_then_appends_loading_assistant() {
        let mut sessions = vec![session_with(
            "s1",
            vec![
                msg_with("u0", "user", "keep", &["blob:one", "blob:two"]),
                msg("a0", "assistant", "old"),
                msg("a1", "assistant", "tail"),
            ],
        )];
        let (text, imgs, asst) = truncate_at_user_message_and_prepare_regenerate(
            &mut sessions,
            "s1",
            "u0",
            "new-a",
            111,
        )
        .expect("user row exists");
        assert_eq!(text, "keep");
        // 带图重发：附件必须原样带回，否则「重新生成」会丢附件（旧测试用空图，该断言恒真）
        assert_eq!(imgs, vec!["blob:one".to_string(), "blob:two".to_string()]);
        assert_eq!(asst, "new-a");
        // 用户行保留 + 新 loading 助手；其后的旧行全部丢弃
        assert_eq!(sessions[0].messages.len(), 2);
        assert_eq!(sessions[0].messages[0].id, "u0");
        // 会话里保留的那条用户消息也必须仍带图
        assert_eq!(
            sessions[0].messages[0].image_urls,
            vec!["blob:one".to_string(), "blob:two".to_string()]
        );
        assert_eq!(sessions[0].messages[1].id, "new-a");
        assert_eq!(
            sessions[0].messages[1].state,
            Some(StoredMessageState::Loading)
        );
        // 注入的时间原样落到新尾泡上（不再读 js 时钟）
        assert_eq!(sessions[0].messages[1].created_at, 111);
    }

    #[test]
    fn regenerate_rejects_missing_or_non_user_target() {
        let mut sessions = vec![session_with(
            "s1",
            vec![msg("u0", "user", "x"), msg("a0", "assistant", "y")],
        )];
        assert!(
            truncate_at_user_message_and_prepare_regenerate(&mut sessions, "nope", "u0", "n", 0)
                .is_none()
        );
        assert!(
            truncate_at_user_message_and_prepare_regenerate(&mut sessions, "s1", "zz", "n", 0)
                .is_none()
        );
        // 助手行不能作为「重新生成」的目标
        assert!(
            truncate_at_user_message_and_prepare_regenerate(&mut sessions, "s1", "a0", "n", 0)
                .is_none()
        );
        // 失败路径不改动原数据
        assert_eq!(sessions[0].messages.len(), 2);
    }

    #[test]
    fn retry_drops_failed_assistant_and_its_tail() {
        let mut sessions = vec![session_with(
            "s1",
            vec![
                msg_with("u0", "user", "question", &["blob:one"]),
                failed_assistant("a0"),
                msg("a1", "assistant", "tail"),
            ],
        )];
        let (text, imgs, asst) =
            prepare_retry_failed_assistant_turn(&mut sessions, "s1", "a0", "new-a", 222)
                .expect("failed row has a preceding user row");
        assert_eq!(text, "question");
        // 重试与重新生成同一条契约：附件必须原样带回
        assert_eq!(imgs, vec!["blob:one".to_string()]);
        assert_eq!(asst, "new-a");
        assert_eq!(sessions[0].messages.len(), 2);
        assert_eq!(sessions[0].messages[1].id, "new-a");
        assert_eq!(sessions[0].messages[1].created_at, 222);
    }

    #[test]
    fn retry_refuses_without_preceding_user_row() {
        // 失败泡前面是助手行 → 无法确定「重试哪一轮」，返回 None 而不是猜
        let mut sessions = vec![session_with(
            "s1",
            vec![msg("a0", "assistant", "first"), failed_assistant("a1")],
        )];
        assert!(prepare_retry_failed_assistant_turn(&mut sessions, "s1", "a1", "n", 0).is_none());
        assert_eq!(sessions[0].messages.len(), 2);

        // 目标不是 error 态 → 不是「重试」对象
        let mut ok = vec![session_with(
            "s1",
            vec![msg("u0", "user", "q"), msg("a0", "assistant", "fine")],
        )];
        assert!(prepare_retry_failed_assistant_turn(&mut ok, "s1", "a0", "n", 0).is_none());
    }
}

/// 中止 / 水合回灌规则的边界（#598 第二轮：把原先写在 `app/chat` 闭包里的分支钉住）。
#[cfg(test)]
mod abort_tests {
    use super::test_support::{msg, session_with};
    use super::*;

    fn tool_loading(id: &str) -> StoredMessage {
        let mut m = msg(id, "system", "工具：git");
        m.is_tool = true;
        m.state = Some(StoredMessageState::Loading);
        m.reasoning_text = "tool: git\nstatus: running".into();
        m
    }

    fn assistant_loading(id: &str, text: &str) -> StoredMessage {
        let mut m = msg(id, "assistant", text);
        m.state = Some(StoredMessageState::Loading);
        m
    }

    #[test]
    fn superseded_ids_skip_the_new_tail_and_non_loading_rows() {
        let sessions = vec![session_with(
            "s1",
            vec![
                assistant_loading("old", "半句"),
                msg("done", "assistant", "已答完"),
                assistant_loading("new", ""),
            ],
        )];
        // 只收「仍 Loading 且不是新尾泡」的那条；已定格的助手行不能被动
        assert_eq!(
            superseded_loading_assistant_ids(&sessions, "s1", "new"),
            vec!["old".to_string()]
        );
        // 会话不存在 → 空列表（不能退化成「所有会话都收」）
        assert!(superseded_loading_assistant_ids(&sessions, "gone", "new").is_empty());
    }

    #[test]
    fn superseded_rows_keep_partial_text_and_append_suffix() {
        let mut sessions = vec![session_with(
            "s1",
            vec![
                assistant_loading("old", "已经写了一半"),
                assistant_loading("new", ""),
            ],
        )];
        assert_eq!(
            stop_superseded_assistant_loading_rows(&mut sessions, "s1", "new", Locale::ZhHans),
            1
        );
        let old = &sessions[0].messages[0];
        assert_eq!(old.state, None);
        assert_eq!(
            old.text,
            format!(
                "已经写了一半{}",
                i18n::stream_stopped_suffix(Locale::ZhHans)
            )
        );
        // 新尾泡一个字都不动
        assert_eq!(sessions[0].messages[1].text, "");
        assert_eq!(
            sessions[0].messages[1].state,
            Some(StoredMessageState::Loading)
        );
    }

    #[test]
    fn superseded_row_with_only_reasoning_is_not_wiped() {
        // 流式只到了思维链：这条也算有内容，否则一句「已终止」会把 reasoning 之外的正文抹平
        let mut m = assistant_loading("old", "");
        m.reasoning_text = "思考了一大段".into();
        let mut sessions = vec![session_with("s1", vec![m, assistant_loading("new", "")])];
        stop_superseded_assistant_loading_rows(&mut sessions, "s1", "new", Locale::ZhHans);
        let old = &sessions[0].messages[0];
        assert_eq!(old.reasoning_text, "思考了一大段");
        assert!(
            old.text
                .contains(i18n::stream_stopped_suffix(Locale::ZhHans))
        );
    }

    #[test]
    fn user_abort_stops_last_assistant_and_tool_rows() {
        let mut messages = vec![assistant_loading("a1", ""), tool_loading("t1")];
        finalize_user_abort_messages(&mut messages, Locale::ZhHans);
        assert_eq!(messages[0].state, None);
        assert_eq!(
            messages[0].text,
            i18n::stream_stopped_inline(Locale::ZhHans)
        );
        // 工具占位也要一起收口，否则状态栏会继续显示在途
        assert_eq!(messages[1].state, None);
        assert!(
            messages[1]
                .reasoning_text
                .contains("status: stopped (user)")
        );
    }

    #[test]
    fn user_abort_with_only_reasoning_keeps_the_reasoning() {
        let mut m = assistant_loading("a1", "");
        m.reasoning_text = "只到了思维链".into();
        let mut messages = vec![m];
        finalize_user_abort_messages(&mut messages, Locale::ZhHans);
        // 中止路径只看正文（`reasoning_counts_as_content = false`）：正文为空就写「已终止」，
        // 但思维链本身一个字都不能丢 —— 这是它与顶替路径的真实差别。
        assert_eq!(
            messages[0].text,
            i18n::stream_stopped_inline(Locale::ZhHans)
        );
        assert_eq!(messages[0].reasoning_text, "只到了思维链");
    }

    #[test]
    fn user_abort_on_unknown_session_changes_nothing() {
        let mut sessions = vec![session_with("s1", vec![assistant_loading("a1", "")])];
        assert!(!finalize_user_abort_in_session(
            &mut sessions,
            "gone",
            Locale::ZhHans
        ));
        assert_eq!(
            sessions[0].messages[0].state,
            Some(StoredMessageState::Loading)
        );
    }

    #[test]
    fn tool_stop_is_reported_only_when_something_was_loading() {
        let mut with_tool = vec![session_with("s1", vec![tool_loading("t1")])];
        assert!(stop_loading_tool_rows_if_any(
            &mut with_tool,
            "s1",
            Locale::ZhHans
        ));
        // 没有 loading 工具 → false，且不改消息
        let mut no_tool = vec![session_with("s1", vec![msg("u0", "user", "hi")])];
        assert!(!stop_loading_tool_rows_if_any(
            &mut no_tool,
            "s1",
            Locale::ZhHans
        ));
        assert!(!stop_loading_tool_rows_if_any(
            &mut no_tool,
            "gone",
            Locale::ZhHans
        ));
    }

    #[test]
    fn reasoning_restore_reports_each_restored_row() {
        let mut sessions = vec![session_with(
            "s1",
            vec![msg("a0", "assistant", "x"), msg("a1", "assistant", "y")],
        )];
        let mut preserved = HashMap::new();
        preserved.insert("a1".to_string(), "想了两句".to_string());
        preserved.insert("gone".to_string(), "无关".to_string());
        let restored = restore_reasoning_texts(&mut sessions, "s1", &preserved);
        // 只报真正落地的那条；长度就是调用方原先在闭包里打的那行日志
        assert_eq!(restored, vec![("a1".to_string(), "想了两句".len())]);
        assert_eq!(sessions[0].messages[1].reasoning_text, "想了两句");
        assert_eq!(sessions[0].messages[0].reasoning_text, "");
        // 会话不存在 → 空列表
        assert!(restore_reasoning_texts(&mut sessions, "gone", &preserved).is_empty());
    }

    #[test]
    fn last_loading_assistant_is_the_tail_row() {
        let sessions = vec![session_with(
            "s1",
            vec![assistant_loading("a1", ""), msg("a2", "assistant", "done")],
        )];
        // 后面已有定格行时，中止要 flush 的仍然是那条 Loading
        assert_eq!(
            last_loading_assistant_id(&sessions, "s1"),
            Some("a1".to_string())
        );
        assert!(last_loading_assistant_id(&sessions, "gone").is_none());
    }
}
