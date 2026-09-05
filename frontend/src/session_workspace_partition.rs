//! 按工作区根路径分桶的会话：服务端 **`workspace_override`** 变化时加载对应 **`/user-data`** 桶。
//!
//! 空桶不搬迁上一工作区的会话（`ensure_at_least_one` → 默认空会话）。
//! 冷启动见 [`prepare_cold_start_session_list`]：不恢复 `active_session_id`，活动会话为空聊天。
//! Clone 等场景可通过 [`request_empty_session_after_next_partition`] 强制切到空会话。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use leptos::prelude::*;

use crate::api::WorkspaceData;
use crate::chat_session_state::ChatSessionSignals;
use crate::i18n::Locale;
use crate::storage::{
    CURRENT_LAYOUT_SCHEMA_VERSION, ChatSession, clear_stale_stream_loading_states,
    ensure_at_least_one, make_session_id, normalize_workspace_partition_path,
};
use crate::user_data_bootstrap::load_web_sessions;

/// Clone / 显式「打开空会话」：下一次分桶加载时优先活动空会话。
static PENDING_EMPTY_SESSION_AFTER_PARTITION: AtomicBool = AtomicBool::new(false);
/// 分桶异步加载代际：较旧的 `load_web_sessions` 结果不得覆盖较新切换。
static PARTITION_LOAD_GEN: AtomicU64 = AtomicU64::new(0);
/// 切仓后、新桶会话尚未 commit 前禁止 `PUT …/current/sessions`（防串桶）。
static SESSION_PERSIST_BLOCKED: AtomicBool = AtomicBool::new(false);
/// 每次门闩/解闩递增；异步 PUT 快照必须仍对应该代际。
static SESSION_PERSIST_EPOCH: AtomicU64 = AtomicU64::new(0);
/// 内存会话列表已对应的工作区规范化路径（`commit_partition_sessions` 写入）。
static MEMORY_SESSIONS_PARTITION_NORM: Mutex<Option<String>> = Mutex::new(None);

/// 仅登记内存会话对应的工作区桶路径（不清切仓 PUT 门闩）。
fn note_memory_sessions_partition(workspace_path: &str) {
    let norm = normalize_workspace_partition_path(workspace_path);
    if let Ok(mut g) = MEMORY_SESSIONS_PARTITION_NORM.lock() {
        *g = Some(norm);
    }
}

/// 首启从 `/user-data` 载入会话后登记当前工作区桶（避免 `workspace_data` 就绪时重复 GET 覆盖内存）。
///
/// 同时解除 PUT 门闩：首启/显式登记表示内存已对齐 `current`，可写回。
pub fn record_memory_sessions_partition(workspace_path: &str) {
    note_memory_sessions_partition(workspace_path);
    SESSION_PERSIST_BLOCKED.store(false, Ordering::SeqCst);
}

fn bump_session_persist_epoch() -> u64 {
    SESSION_PERSIST_EPOCH
        .fetch_add(1, Ordering::SeqCst)
        .wrapping_add(1)
}

/// 旧桶已 flush、即将 `POST /workspace` 时调用：挡住防抖 PUT 写到新 `current`。
pub fn begin_workspace_session_persist_block() {
    SESSION_PERSIST_BLOCKED.store(true, Ordering::SeqCst);
    let _ = bump_session_persist_epoch();
}

/// 切仓失败回滚时解除门闩（内存仍属旧桶）。
pub fn clear_workspace_session_persist_block() {
    SESSION_PERSIST_BLOCKED.store(false, Ordering::SeqCst);
    let _ = bump_session_persist_epoch();
}

/// 是否允许把内存会话写回服务端 `current` 桶。
#[must_use]
pub fn session_persist_allowed() -> bool {
    !SESSION_PERSIST_BLOCKED.load(Ordering::SeqCst)
}

/// 当前会话持久化代际（异步 PUT 用：快照时记下，写入前必须仍相等）。
#[must_use]
pub fn session_persist_epoch() -> u64 {
    SESSION_PERSIST_EPOCH.load(Ordering::SeqCst)
}

/// `allowed && epoch` 与快照时一致时才可 PUT。
#[must_use]
pub fn session_persist_put_ok(snapshot_epoch: u64) -> bool {
    persist_put_ok_parts(
        session_persist_allowed(),
        session_persist_epoch(),
        snapshot_epoch,
    )
}

#[must_use]
pub(crate) fn persist_put_ok_parts(allowed: bool, current_epoch: u64, snapshot_epoch: u64) -> bool {
    allowed && current_epoch == snapshot_epoch
}

/// 内存会话是否已提交为 `workspace_path` 对应桶。
#[must_use]
pub fn memory_sessions_match_workspace(workspace_path: &str) -> bool {
    let target = normalize_workspace_partition_path(workspace_path);
    MEMORY_SESSIONS_PARTITION_NORM
        .lock()
        .map(|g| g.as_deref() == Some(target.as_str()))
        .unwrap_or(false)
}

/// 标记下一次工作区会话分桶应切到空会话（见 [`prefer_or_create_empty_session`]）。
pub fn request_empty_session_after_next_partition() {
    PENDING_EMPTY_SESSION_AFTER_PARTITION.store(true, Ordering::SeqCst);
}

#[must_use]
pub fn pending_empty_session_after_partition() -> bool {
    PENDING_EMPTY_SESSION_AFTER_PARTITION.load(Ordering::SeqCst)
}

/// 取出并清除「下一分桶要空会话」标记。
#[must_use]
pub fn take_pending_empty_session_after_partition() -> bool {
    PENDING_EMPTY_SESSION_AFTER_PARTITION.swap(false, Ordering::SeqCst)
}

/// 当前活动会话是否已是绑定到 `workspace_path` 的空白会话。
#[must_use]
pub fn active_session_is_blank_for_workspace(
    chat: ChatSessionSignals,
    workspace_path: &str,
) -> bool {
    let target = normalize_workspace_partition_path(workspace_path);
    let aid = chat.active_id.get_untracked();
    if aid.is_empty() {
        return false;
    }
    chat.sessions.with_untracked(|list| {
        let Some(s) = list.iter().find(|s| s.id == aid) else {
            return false;
        };
        if !session_is_blank_chat(s) {
            return false;
        }
        if target.is_empty() {
            return s
                .workspace_root
                .as_deref()
                .map(str::trim)
                .unwrap_or("")
                .is_empty();
        }
        s.workspace_root
            .as_deref()
            .map(normalize_workspace_partition_path)
            .is_some_and(|wr| wr == target)
    })
}

/// 仅采用**本桶**服务端记下的 `active_session_id`；绝不把上一工作区列表拷进空桶。
#[must_use]
pub(crate) fn aid_pick_for_loaded_bucket(
    list: &[ChatSession],
    aid_from_server: Option<String>,
) -> Option<String> {
    aid_from_server.filter(|id| list.iter().any(|s| s.id == *id))
}

#[must_use]
pub(crate) fn session_is_blank_chat(s: &ChatSession) -> bool {
    s.messages.is_empty()
        && s.server_conversation_id
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .is_none()
}

/// 无正文、无草稿、无工作区绑定：冷启动可复用，且不必改写已有行。
#[must_use]
pub(crate) fn session_is_unbound_empty_chat(s: &ChatSession) -> bool {
    session_is_blank_chat(s)
        && s.draft.trim().is_empty()
        && s.workspace_root
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty()
}

fn insert_empty_session(
    mut list: Vec<ChatSession>,
    workspace_root: Option<String>,
    default_title: String,
) -> (Vec<ChatSession>, String) {
    let now = js_sys::Date::now() as i64;
    let s = ChatSession {
        id: make_session_id(),
        layout_schema_version: CURRENT_LAYOUT_SCHEMA_VERSION,
        title: default_title,
        draft: String::new(),
        messages: Vec::new(),
        updated_at: now,
        pinned: false,
        starred: false,
        server_conversation_id: None,
        server_revision: None,
        workspace_root,
        history_total: None,
        history_window_start: None,
        history_has_older: None,
    };
    let id = s.id.clone();
    list.insert(0, s);
    (list, id)
}

/// 冷启动：载入当前桶会话列表，但**不**恢复服务端 `active_session_id`。
/// 只复用「未绑定的空会话」；否则插入新行，不改写已有会话的 `workspace_root` / 草稿。
#[must_use]
pub(crate) fn prepare_cold_start_session_list(
    mut list: Vec<ChatSession>,
    loc: Locale,
) -> (Vec<ChatSession>, String, String) {
    for s in &mut list {
        s.normalize_layout_schema_version();
        clear_stale_stream_loading_states(&mut s.messages, loc);
    }
    let title = crate::i18n::default_session_title(loc).to_string();
    let (list, _) = ensure_at_least_one(list, title.clone());
    if let Some(id) = list
        .iter()
        .find(|s| session_is_unbound_empty_chat(s))
        .map(|s| s.id.clone())
    {
        return (list, id, String::new());
    }
    let (list, pick) = insert_empty_session(list, None, title);
    (list, pick, String::new())
}

/// 优先复用已有空白会话，否则在列表前插入一条空会话并设为活动。
#[must_use]
pub(crate) fn prefer_or_create_empty_session(
    mut list: Vec<ChatSession>,
    workspace_root: Option<String>,
    default_title: String,
) -> (Vec<ChatSession>, String) {
    if let Some(id) = list
        .iter()
        .find(|s| session_is_blank_chat(s))
        .map(|s| s.id.clone())
    {
        if let Some(s) = list.iter_mut().find(|s| s.id == id) {
            if workspace_root.is_some() {
                s.workspace_root = workspace_root;
            }
        }
        return (list, id);
    }
    insert_empty_session(list, workspace_root, default_title)
}

fn align_workspace_roots_to_server_path(list2: &mut [ChatSession], server_path: &str) {
    let server_norm = normalize_workspace_partition_path(server_path);
    if server_norm.is_empty() {
        // 未设置 / 空路径：不写入 workspace_root，保留「默认会话」无绑定态。
        return;
    }
    let canonical = server_path.trim().to_string();
    for s in list2.iter_mut() {
        let needs = match s.workspace_root.as_deref() {
            None => true,
            Some(wr) => {
                let wn = normalize_workspace_partition_path(wr);
                wn.is_empty() || wn != server_norm
            }
        };
        if needs {
            s.workspace_root = Some(canonical.clone());
        }
    }
}

/// 在当前内存会话列表上切到空会话（仅当内存已属于该工作区桶时由交接逻辑调用）。
pub fn apply_empty_session_in_memory(
    chat: ChatSessionSignals,
    draft: RwSignal<String>,
    workspace_path: &str,
    loc: Locale,
) {
    let title = crate::i18n::default_session_title(loc).to_string();
    let root = workspace_root_opt(workspace_path);
    let list = chat.sessions.get_untracked();
    let (list, pick) = prefer_or_create_empty_session(list, root, title);
    let norm = normalize_workspace_partition_path(workspace_path);
    chat.clear_stream_resume_handles();
    chat.stream_text_overlay.set(None);
    chat.session_sync
        .set(crate::session_sync::SessionSyncState::local_only());
    chat.reasoning_preserved.set(HashMap::new());
    chat.session_hydrate_nonce
        .update(|n| *n = n.wrapping_add(1));
    chat.sessions.set(list);
    chat.active_id.set(pick);
    draft.set(String::new());
    if let Ok(mut g) = MEMORY_SESSIONS_PARTITION_NORM.lock() {
        *g = Some(norm);
    }
    SESSION_PERSIST_BLOCKED.store(false, Ordering::SeqCst);
    let _ = bump_session_persist_epoch();
}

fn workspace_root_opt(path: &str) -> Option<String> {
    let t = path.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

fn prepare_partition_session_list(
    mut list: Vec<ChatSession>,
    aid_from_server: Option<String>,
    server_path: &str,
    loc: Locale,
    force_empty: bool,
) -> (Vec<ChatSession>, String, String) {
    align_workspace_roots_to_server_path(&mut list, server_path);
    for s in list.iter_mut() {
        s.normalize_layout_schema_version();
        clear_stale_stream_loading_states(&mut s.messages, loc);
    }
    let title = crate::i18n::default_session_title(loc).to_string();
    let (mut list, def_id) = ensure_at_least_one(list, title.clone());
    align_workspace_roots_to_server_path(&mut list, server_path);
    let aid_for_pick = aid_pick_for_loaded_bucket(&list, aid_from_server);
    let pick = if force_empty {
        let (l, id) = prefer_or_create_empty_session(list, workspace_root_opt(server_path), title);
        list = l;
        id
    } else {
        aid_for_pick
            .filter(|id| list.iter().any(|s| s.id == *id))
            .unwrap_or(def_id)
    };
    let draft = list
        .iter()
        .find(|s| s.id == pick)
        .map(|s| s.draft.clone())
        .unwrap_or_default();
    (list, pick, draft)
}

/// 分桶 GET 结果是否应覆盖内存会话。
///
/// - 真切仓 / force_empty / persist 门闩：允许覆盖（空桶新建）。
/// - 同仓且流式忙碌：禁止（勿清 overlay / 勿 bump hydrate）。
/// - 同仓且内存非空而 GET 空/不含活动会话：视为陈旧竞态，禁止覆盖（对齐旧版 empty-GET 保内存）。
#[must_use]
pub(crate) fn should_apply_partition_load(
    switching: bool,
    stream_busy: bool,
    memory_active_id: &str,
    memory_session_count: usize,
    loaded_contains_active: bool,
    loaded_session_count: usize,
) -> bool {
    if switching {
        return true;
    }
    if stream_busy {
        return false;
    }
    if memory_session_count > 0 && loaded_session_count == 0 {
        return false;
    }
    if memory_session_count > 0 && !memory_active_id.trim().is_empty() && !loaded_contains_active {
        return false;
    }
    true
}

fn memory_stream_busy(chat: ChatSessionSignals) -> bool {
    chat.stream_text_overlay.get_untracked().is_some()
        || chat.stream_bound_resume_handles_untracked().is_some()
}

/// 是否应判定为「分桶切换」。`persist_blocked` 由调用方读取进程级门闩传入，
/// 便于单测直接传值、避免与改写该全局状态的测试产生竞态。
fn partition_switch_context(
    force_empty_gate: bool,
    prev_norm: Option<&str>,
    next_norm: &str,
    persist_blocked: bool,
) -> bool {
    force_empty_gate || prev_norm.is_some_and(|p| p != next_norm) || persist_blocked
}

fn commit_partition_sessions(
    chat: ChatSessionSignals,
    draft: RwSignal<String>,
    session_workspace_path: RwSignal<String>,
    list: Vec<ChatSession>,
    pick: String,
    draft_text: String,
    norm: String,
) {
    chat.clear_stream_resume_handles();
    chat.stream_text_overlay.set(None);
    chat.session_sync
        .set(crate::session_sync::SessionSyncState::local_only());
    chat.reasoning_preserved.set(HashMap::new());
    chat.session_hydrate_nonce
        .update(|n| *n = n.wrapping_add(1));
    chat.sessions.set(list);
    chat.active_id.set(pick);
    draft.set(draft_text);
    session_workspace_path.set(norm.clone());
    if let Ok(mut g) = MEMORY_SESSIONS_PARTITION_NORM.lock() {
        *g = Some(norm);
    }
    SESSION_PERSIST_BLOCKED.store(false, Ordering::SeqCst);
    let _ = bump_session_persist_epoch();
}

/// 显式加载并提交某一工作区桶的会话（`finish_workspace_root_ui` 等待超时或 reload 失败时兜底）。
///
/// 成功 commit 返回 `true`；代际过期返回 `false`（不解闩，由更新的加载任务负责）。
pub async fn ensure_sessions_for_workspace(
    chat: ChatSessionSignals,
    draft: RwSignal<String>,
    session_workspace_path: RwSignal<String>,
    workspace_path: &str,
    loc: Locale,
    force_empty: bool,
) -> bool {
    let norm = normalize_workspace_partition_path(workspace_path);
    let my_gen = PARTITION_LOAD_GEN
        .fetch_add(1, Ordering::AcqRel)
        .wrapping_add(1);
    let (list2, aid2) = load_web_sessions(loc).await;
    if PARTITION_LOAD_GEN.load(Ordering::Acquire) != my_gen {
        return false;
    }
    let force_empty = take_pending_empty_session_after_partition() || force_empty;
    let (list2, pick, draft_text) =
        prepare_partition_session_list(list2, aid2, workspace_path, loc, force_empty);
    if PARTITION_LOAD_GEN.load(Ordering::Acquire) != my_gen {
        if force_empty {
            request_empty_session_after_next_partition();
        }
        return false;
    }
    commit_partition_sessions(
        chat,
        draft,
        session_workspace_path,
        list2,
        pick,
        draft_text,
        norm,
    );
    true
}

/// `wire_workspace_session_storage_partition` 的入参。
#[derive(Clone, Copy)]
pub struct WireWorkspaceSessionPartitionArgs {
    pub initialized: RwSignal<bool>,
    pub workspace_data: RwSignal<Option<WorkspaceData>>,
    pub chat: ChatSessionSignals,
    pub draft: RwSignal<String>,
    pub locale: RwSignal<Locale>,
    pub session_workspace_path: RwSignal<String>,
}

fn partition_effect_should_skip_reload(
    force_empty_gate: bool,
    workspace_path: &str,
    norm: &str,
    prev_slot: &mut Option<String>,
) -> bool {
    if force_empty_gate {
        return false;
    }
    if memory_sessions_match_workspace(workspace_path) {
        *prev_slot = Some(norm.to_string());
        return true;
    }
    if prev_slot.as_deref() == Some(norm) {
        return true;
    }
    false
}

struct PartitionLoadCommitArgs {
    chat: ChatSessionSignals,
    draft: RwSignal<String>,
    session_workspace_path: RwSignal<String>,
    workspace_path: String,
    norm: String,
    loc: Locale,
    my_gen: u64,
    switching: bool,
    /// 仅在成功 commit 后写入，避免 skip/陈旧 GET 永久挡住重试或伪造成功对齐。
    prev_applied: Arc<Mutex<Option<String>>>,
}

async fn load_and_commit_partition_sessions(args: PartitionLoadCommitArgs) {
    let PartitionLoadCommitArgs {
        chat,
        draft,
        session_workspace_path,
        workspace_path,
        norm,
        loc,
        my_gen,
        switching,
        prev_applied,
    } = args;
    let (list2, aid2) = load_web_sessions(loc).await;
    if PARTITION_LOAD_GEN.load(Ordering::Acquire) != my_gen {
        return;
    }
    // 在加载成功且仍是最新代际后再 take，避免旧任务抢走 empty 标记。
    let force_empty = take_pending_empty_session_after_partition();
    let switching = switching || force_empty;
    let memory_active = chat.active_id.get_untracked();
    let memory_count = chat.sessions.get_untracked().len();
    let loaded_has_active =
        !memory_active.is_empty() && list2.iter().any(|s| s.id == memory_active);
    if !should_apply_partition_load(
        switching,
        memory_stream_busy(chat),
        memory_active.as_str(),
        memory_count,
        loaded_has_active,
        list2.len(),
    ) {
        // 同仓陈旧/空 GET 或流式中：保留内存与 overlay；不登记 MEMORY match、不写 prev。
        if force_empty {
            request_empty_session_after_next_partition();
        }
        return;
    }
    let (list2, pick, draft_text) =
        prepare_partition_session_list(list2, aid2, workspace_path.as_str(), loc, force_empty);
    if PARTITION_LOAD_GEN.load(Ordering::Acquire) != my_gen {
        if force_empty {
            request_empty_session_after_next_partition();
        }
        return;
    }
    commit_partition_sessions(
        chat,
        draft,
        session_workspace_path,
        list2,
        pick,
        draft_text,
        norm.clone(),
    );
    if let Ok(mut g) = prev_applied.lock() {
        *g = Some(norm);
    }
}

/// 在 `workspace_data` 的有效根变化时，从服务端加载另一工作区桶的会话列表。
pub fn wire_workspace_session_storage_partition(args: WireWorkspaceSessionPartitionArgs) {
    let WireWorkspaceSessionPartitionArgs {
        initialized,
        workspace_data,
        chat,
        draft,
        locale,
        session_workspace_path,
    } = args;
    let prev_applied = StoredValue::new(Arc::new(Mutex::new(Option::<String>::None)));

    Effect::new(move |_| {
        if !initialized.get() {
            return;
        }
        let Some(wd) = workspace_data.get() else {
            return;
        };
        if wd.error.is_some() {
            return;
        }
        let norm = normalize_workspace_partition_path(&wd.path);
        let force_empty_gate = pending_empty_session_after_partition();
        let prev_cell = prev_applied.get_value();
        let switching = {
            let mut prev_slot = prev_cell.lock().expect("partition prev workspace");
            if partition_effect_should_skip_reload(
                force_empty_gate,
                wd.path.as_str(),
                norm.as_str(),
                &mut prev_slot,
            ) {
                return;
            }
            partition_switch_context(
                force_empty_gate,
                prev_slot.as_deref(),
                norm.as_str(),
                SESSION_PERSIST_BLOCKED.load(Ordering::SeqCst),
            )
        };

        let my_gen = PARTITION_LOAD_GEN
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1);
        let loc = locale.get_untracked();
        let workspace_path = wd.path.clone();
        let prev_applied = Arc::clone(&prev_cell);
        leptos::task::spawn_local(async move {
            load_and_commit_partition_sessions(PartitionLoadCommitArgs {
                chat,
                draft,
                session_workspace_path,
                workspace_path,
                norm,
                loc,
                my_gen,
                switching,
                prev_applied,
            })
            .await;
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StoredMessage;

    fn sess(
        id: &str,
        messages: Vec<StoredMessage>,
        server_conversation_id: Option<&str>,
    ) -> ChatSession {
        ChatSession {
            id: id.into(),
            layout_schema_version: CURRENT_LAYOUT_SCHEMA_VERSION,
            title: "t".into(),
            draft: String::new(),
            messages,
            updated_at: 0,
            pinned: false,
            starred: false,
            server_conversation_id: server_conversation_id.map(str::to_string),
            server_revision: None,
            workspace_root: None,
            history_total: None,
            history_window_start: None,
            history_has_older: None,
        }
    }

    #[test]
    fn persist_put_ok_requires_allow_and_matching_epoch() {
        assert!(persist_put_ok_parts(true, 3, 3));
        assert!(!persist_put_ok_parts(false, 3, 3));
        assert!(!persist_put_ok_parts(true, 4, 3));
    }

    /// 以下测试会改写进程级静态状态（`SESSION_PERSIST_BLOCKED` / 内存分桶登记），
    /// 并行执行时会互相覆盖导致偶发失败，故用同一把锁串行化。
    static PERSIST_STATE_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn record_memory_partition_makes_memory_match_workspace() {
        let _guard = PERSIST_STATE_TEST_LOCK.lock().expect("persist test lock");
        record_memory_sessions_partition("/tmp/proj/");
        assert!(memory_sessions_match_workspace("/tmp/proj"));
        assert!(!memory_sessions_match_workspace("/other"));
    }

    #[test]
    fn note_memory_partition_keeps_persist_block() {
        let _guard = PERSIST_STATE_TEST_LOCK.lock().expect("persist test lock");
        SESSION_PERSIST_BLOCKED.store(true, Ordering::SeqCst);
        note_memory_sessions_partition("/tmp/proj/");
        assert!(memory_sessions_match_workspace("/tmp/proj"));
        assert!(!session_persist_allowed());
        SESSION_PERSIST_BLOCKED.store(false, Ordering::SeqCst);
    }

    #[test]
    fn aid_pick_uses_only_bucket_active_id() {
        let list = vec![sess("a", vec![], None), sess("b", vec![], None)];
        assert_eq!(
            aid_pick_for_loaded_bucket(&list, Some("b".into())),
            Some("b".into())
        );
        assert_eq!(
            aid_pick_for_loaded_bucket(&list, Some("missing".into())),
            None
        );
        assert_eq!(aid_pick_for_loaded_bucket(&[], Some("a".into())), None);
    }

    #[test]
    fn blank_chat_ignores_whitespace_server_id() {
        let blank = sess("x", vec![], None);
        assert!(session_is_blank_chat(&blank));
        let spaced = sess("y", vec![], Some("  "));
        assert!(session_is_blank_chat(&spaced));
        let linked = sess("z", vec![], Some("c_1"));
        assert!(!session_is_blank_chat(&linked));
    }

    #[test]
    fn cold_start_reuses_unbound_empty_without_rewriting_others() {
        let list = vec![
            sess("old", vec![], Some("c_1")),
            sess("blank", vec![], None),
        ];
        let (out, pick, draft) = prepare_cold_start_session_list(list, Locale::En);
        assert_eq!(pick, "blank");
        assert!(draft.is_empty());
        assert_eq!(out.len(), 2);
        let blank = out.iter().find(|s| s.id == "blank").unwrap();
        assert!(session_is_unbound_empty_chat(blank));
    }

    #[test]
    fn unbound_empty_rejects_draft_or_workspace_root() {
        let plain = sess("b", vec![], None);
        assert!(session_is_unbound_empty_chat(&plain));
        let drafted = ChatSession {
            draft: "x".into(),
            ..sess("d", vec![], None)
        };
        assert!(!session_is_unbound_empty_chat(&drafted));
        let bound = ChatSession {
            workspace_root: Some("/ws".into()),
            ..sess("w", vec![], None)
        };
        assert!(!session_is_unbound_empty_chat(&bound));
        assert!(session_is_blank_chat(&drafted));
        assert!(session_is_blank_chat(&bound));
    }

    #[test]
    fn prefer_empty_reuses_blank_without_growing_list() {
        let list = vec![
            sess("old", vec![], Some("c_1")),
            sess("blank", vec![], None),
        ];
        let (out, pick) =
            prefer_or_create_empty_session(list, Some("/tmp/ws".into()), "New chat".into());
        assert_eq!(pick, "blank");
        assert_eq!(out.len(), 2);
        assert_eq!(
            out.iter()
                .find(|s| s.id == "blank")
                .unwrap()
                .workspace_root
                .as_deref(),
            Some("/tmp/ws")
        );
    }

    #[test]
    fn should_apply_partition_load_guards_stale_same_workspace_get() {
        assert!(should_apply_partition_load(true, false, "a", 1, false, 0));
        assert!(!should_apply_partition_load(false, true, "a", 1, true, 1));
        assert!(!should_apply_partition_load(
            false, false, "seed", 2, false, 1
        ));
        assert!(should_apply_partition_load(
            false, false, "seed", 2, true, 2
        ));
        assert!(should_apply_partition_load(false, false, "", 0, false, 0));
        // 同仓空 GET + 内存非空：即使 active 暂时为空也不覆盖
        assert!(!should_apply_partition_load(false, false, "", 2, false, 0));
    }

    #[test]
    fn partition_switch_context_detects_path_change_and_gate() {
        assert!(!partition_switch_context(false, Some("/a"), "/a", false));
        assert!(partition_switch_context(false, Some("/a"), "/b", false));
        assert!(partition_switch_context(true, Some("/a"), "/a", false));
        assert!(!partition_switch_context(false, None, "/a", false));
        // persist 门闩未解除时同路径也判定为切换。
        assert!(partition_switch_context(false, Some("/a"), "/a", true));
    }
}
