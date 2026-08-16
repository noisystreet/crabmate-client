//! 会话列表：首启从 **`/user-data`** 载入、`GET /web-ui` 一次同步、变更写回服务端。

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::api::fetch_web_ui_config;
use crate::api::fetch_workspace;
use crate::api::user_data::{put_current_web_sessions, put_current_web_sessions_keepalive};
use crate::chat_session_state::ChatSessionSignals;

use super::session_hydrate::bump_session_hydrate_nonce;
use crate::api::client_llm_storage::hydrate_client_llm_from_server;
use crate::i18n::Locale;
use crate::session_workspace_partition::{
    prepare_cold_start_session_list, record_memory_sessions_partition,
};
use crate::stream_text_overlay::sessions_snapshot_with_stream_overlay_merged;
use crate::user_data_bootstrap::load_web_sessions;
use crate::user_prefs_sync::wire_load_user_prefs_from_server;
use crate::user_prefs_sync_state::user_prefs_load_attempt_finished;

const PERSIST_SESSIONS_DEBOUNCE_MS: u32 = 400;

/// 流结束后立即发起一次可跨页面刷新的持久化；常规变更仍由防抖 Effect 兜底。
pub(crate) fn persist_chat_sessions_at_stream_end(chat: ChatSessionSignals, loc: Locale) {
    use crate::session_workspace_partition::{session_persist_epoch, session_persist_put_ok};
    if !crate::session_workspace_partition::session_persist_allowed() {
        return;
    }
    let epoch = session_persist_epoch();
    let aid = chat.active_id.get_untracked();
    if aid.is_empty() {
        return;
    }
    let list = chat.sessions.get_untracked();
    let merged = sessions_snapshot_with_stream_overlay_merged(
        list.as_slice(),
        chat.stream_text_overlay.get_untracked().as_ref(),
    );
    spawn_local(async move {
        if !session_persist_put_ok(epoch) {
            return;
        }
        let _ = put_current_web_sessions_keepalive(&merged, Some(aid.as_str()), loc).await;
    });
}

/// 首次渲染时从 `/user-data` 加载会话列表；活动会话固定为空聊天（不恢复上次会话 / 工作区绑定）。
pub fn wire_initial_sessions_from_storage(app: crate::app::app_signals::AppSignals) {
    let initialized = app.initialized;
    let sessions = app.chat.sessions;
    let active_id = app.chat.active_id;
    let draft = app.chat_composer.draft;
    let locale = app.shell_ui.locale;
    let chat = app.chat;

    wire_load_user_prefs_from_server(app.clone());
    let prefs_sync_phase = app.workspace.user_prefs_sync_phase;
    Effect::new(move |_| {
        if initialized.get() {
            return;
        }
        let loc = locale.get_untracked();
        spawn_local(async move {
            // LLM 覆盖 / 密钥状态 与 会话列表并行，缩短门闸；prefs（含只读 TTL）另路由行。
            let ((), ((list, _aid), ws_outcome)) = futures_util::future::join(
                hydrate_client_llm_from_server(loc),
                futures_util::future::join(load_web_sessions(loc), fetch_workspace(None, loc)),
            )
            .await;
            if let Ok(wd) = ws_outcome {
                record_memory_sessions_partition(wd.path.as_str());
            } else {
                // 工作区探测失败也登记空路径桶，避免 initialized 后误走破坏性同仓 GET。
                record_memory_sessions_partition("");
            }
            let (list, pick, d) = prepare_cold_start_session_list(list, loc);
            sessions.set(list);
            active_id.set(pick);
            draft.set(d);
            // 等 prefs 落地后再 `initialized`，避免首条聊天用默认只读 TTL（false→附带 0）。
            for _ in 0..250 {
                if user_prefs_load_attempt_finished(prefs_sync_phase.get_untracked()) {
                    break;
                }
                TimeoutFuture::new(20).await;
            }
            initialized.set(true);
            bump_session_hydrate_nonce(chat);
        });
    });
}

/// 初始化完成后拉取一次 **`GET /web-ui`**，同步 Markdown / 助手过滤开关。
pub fn wire_web_ui_config_once_after_init(
    initialized: RwSignal<bool>,
    web_ui_config_loaded: RwSignal<bool>,
    markdown_render: RwSignal<bool>,
    apply_assistant_display_filters: RwSignal<bool>,
    locale: RwSignal<Locale>,
) {
    Effect::new({
        move |_| {
            if !initialized.get() || web_ui_config_loaded.get() {
                return;
            }
            web_ui_config_loaded.set(true);
            let locale_val = locale.get_untracked();
            spawn_local(async move {
                if let Ok(c) = fetch_web_ui_config(locale_val).await {
                    markdown_render.set(c.markdown_render);
                    apply_assistant_display_filters.set(c.apply_assistant_display_filters);
                }
            });
        }
    });
}

/// 会话或活动 id 变化时写回 **`PUT /user-data/workspaces/current/sessions`**（防抖）。
pub fn wire_persist_chat_sessions(
    initialized: RwSignal<bool>,
    chat: ChatSessionSignals,
    locale: RwSignal<Locale>,
) {
    let sessions = chat.sessions;
    let active_id = chat.active_id;
    let stream_text_overlay = chat.stream_text_overlay;
    let stream_overlay_revision = chat.stream_overlay_revision;
    let debounce_tick = StoredValue::new(Arc::new(AtomicU64::new(0)));
    Effect::new(move |_| {
        if !initialized.get() {
            return;
        }
        let _ = sessions.get();
        let _ = active_id.get();
        let _ = stream_overlay_revision.get();
        let ctr = debounce_tick.get_value();
        let prev = ctr.fetch_add(1, Ordering::Relaxed);
        let tick = prev.wrapping_add(1);
        let ctr2 = Arc::clone(&ctr);
        spawn_local(async move {
            TimeoutFuture::new(PERSIST_SESSIONS_DEBOUNCE_MS).await;
            if ctr2.load(Ordering::Relaxed) != tick {
                return;
            }
            if !initialized.get_untracked() {
                return;
            }
            use crate::session_workspace_partition::{
                session_persist_epoch, session_persist_put_ok,
            };
            if !crate::session_workspace_partition::session_persist_allowed() {
                return;
            }
            let epoch = session_persist_epoch();
            let list = sessions.get_untracked();
            let aid = active_id.get_untracked();
            if aid.is_empty() {
                return;
            }
            let merged = sessions_snapshot_with_stream_overlay_merged(
                list.as_slice(),
                stream_text_overlay.get_untracked().as_ref(),
            );
            let loc = locale.get_untracked();
            if !session_persist_put_ok(epoch) {
                return;
            }
            let _ = put_current_web_sessions(&merged, Some(aid.as_str()), loc).await;
        });
    });
}
