//! 工作区根目录选择 / 提交（顶栏只读路径与「项目」菜单共用）。

use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::api::post_workspace_set;
use crate::api::user_data::put_current_web_sessions;
use crate::app_prefs::SidePanelView;
use crate::chat_session_state::ChatSessionSignals;
use crate::i18n::{self, Locale};
use crate::session_export::tauri_pick_workspace_folder;
use crate::session_workspace_partition::{
    begin_workspace_session_persist_block, clear_workspace_session_persist_block,
    ensure_sessions_for_workspace, memory_sessions_match_workspace,
    request_empty_session_after_next_partition, session_persist_allowed,
};
use crate::stream_text_overlay::sessions_snapshot_with_stream_overlay_merged;
use crate::tauri_shell::tauri_shell_available;
use crate::user_data_bootstrap::{remember_workspace_root, workspace_recent_menu_label};
use crate::workspace_shell::reload_workspace_panel;

use super::workspace_panel_state::WorkspacePanelSignals;

/// 切换工作区后的会话交接策略。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorkspaceSessionHandoff {
    /// 加载新桶并恢复该桶 `active_session_id`；空桶则默认空会话。
    RestoreBucketActive,
    /// Clone 等：在新桶上强制切到空会话。
    PreferEmptySession,
}

/// 顶栏 / 菜单共用的工作区根选择句柄。
#[derive(Clone, Copy)]
pub struct WorkspaceRootPickHandle {
    pub locale: RwSignal<Locale>,
    pub chat: ChatSessionSignals,
    /// 聊天 composer 草稿（Clone 强制空会话时需清空）。
    pub composer_draft: RwSignal<String>,
    pub ws: WorkspacePanelSignals,
    pub side_panel_view: RwSignal<SidePanelView>,
}

pub(crate) fn workspace_inputs_blocked(ws: WorkspacePanelSignals) -> bool {
    ws.workspace_set_busy.get() || ws.workspace_pick_busy.get() || ws.workspace_loading.get()
}

/// 将当前活动会话（含流式 overlay）写回**当前**工作区桶。
pub(crate) async fn flush_current_workspace_sessions(
    chat: ChatSessionSignals,
    loc: Locale,
) -> Result<(), String> {
    if !session_persist_allowed() {
        // 切仓窗口内内存可能仍属旧桶，而服务端 current 已是新桶——禁止 PUT。
        return Ok(());
    }
    let aid = chat.active_id.get_untracked();
    if aid.is_empty() {
        return Ok(());
    }
    let list = chat.sessions.get_untracked();
    let merged = sessions_snapshot_with_stream_overlay_merged(
        list.as_slice(),
        chat.stream_text_overlay.get_untracked().as_ref(),
    );
    put_current_web_sessions(&merged, Some(aid.as_str()), loc).await
}

/// 提交工作区根；成功返回 `true`（调用方可用于关闭弹窗）。
pub(crate) async fn commit_workspace_root(
    chat: ChatSessionSignals,
    ws: WorkspacePanelSignals,
    path: String,
    loc: Locale,
) -> bool {
    let path = path.clone();
    if let Err(e) = flush_current_workspace_sessions(chat, loc).await {
        ws.workspace_set_err.set(Some(e));
        ws.workspace_set_busy.set(false);
        return false;
    }
    begin_workspace_session_persist_block();
    let ok = match post_workspace_set(Some(path.clone()), loc).await {
        Ok(_) => {
            finish_workspace_root_ui(
                chat,
                ws,
                path,
                loc,
                WorkspaceSessionHandoff::RestoreBucketActive,
                None,
            )
            .await;
            true
        }
        Err(e) => {
            clear_workspace_session_persist_block();
            ws.workspace_set_err.set(Some(e));
            false
        }
    };
    ws.workspace_set_busy.set(false);
    ok
}

/// 切换工作区成功后的 UI：记最近路径、刷新树，并等待会话分桶落地。
///
/// **不**在换桶前改写当前活动会话的 `workspace_root`（避免串桶写脏）。
pub(crate) async fn finish_workspace_root_ui(
    chat: ChatSessionSignals,
    ws: WorkspacePanelSignals,
    path: String,
    loc: Locale,
    handoff: WorkspaceSessionHandoff,
    composer_draft: Option<RwSignal<String>>,
) {
    let force_empty = matches!(handoff, WorkspaceSessionHandoff::PreferEmptySession);
    if force_empty {
        request_empty_session_after_next_partition();
    }
    remember_workspace_root(&path, ws.recent_workspace_roots);
    reload_workspace_panel(
        ws.workspace_loading,
        ws.workspace_err,
        ws.workspace_path_draft,
        ws.workspace_data,
        ws.workspace_subtree_expanded,
        ws.workspace_subtree_cache,
        ws.workspace_subtree_loading,
        loc,
    )
    .await;
    await_workspace_session_handoff(
        chat,
        ws.workspace_path_draft,
        composer_draft,
        &path,
        loc,
        force_empty,
    )
    .await;
}

/// 等待分桶 Effect 将内存会话切到目标工作区；超时则显式 `ensure_sessions_for_workspace`。
///
/// PreferEmpty：禁止在「内存仍属旧桶」时 in-memory 插空会话（会触发串桶 PUT）。
async fn await_workspace_session_handoff(
    chat: ChatSessionSignals,
    session_workspace_path: RwSignal<String>,
    composer_draft: Option<RwSignal<String>>,
    path: &str,
    loc: Locale,
    force_empty: bool,
) {
    use crate::session_workspace_partition::{
        active_session_is_blank_for_workspace, apply_empty_session_in_memory,
        take_pending_empty_session_after_partition,
    };

    const POLL_MS: u32 = 20;
    // Android / 远程：clone+reload+load sessions 常超过 1.5s。
    const MAX_POLLS: u32 = 500;

    for _ in 0..MAX_POLLS {
        let matched = memory_sessions_match_workspace(path) && session_persist_allowed();
        if matched {
            if !force_empty || active_session_is_blank_for_workspace(chat, path) {
                let _ = take_pending_empty_session_after_partition();
                return;
            }
            // 新桶已落地但尚未空会话：仅在内存已属新桶时本地补空。
            let draft = composer_draft.unwrap_or_else(|| RwSignal::new(String::new()));
            apply_empty_session_in_memory(chat, draft, path, loc);
            let _ = take_pending_empty_session_after_partition();
            return;
        }
        TimeoutFuture::new(POLL_MS).await;
    }

    // 超时：显式加载目标桶（勿 take pending 抢分桶任务；ensure 自己处理 force_empty）。
    let draft = composer_draft.unwrap_or_else(|| RwSignal::new(String::new()));
    ensure_sessions_for_workspace(chat, draft, session_workspace_path, path, loc, force_empty)
        .await;
}

fn spawn_server_side_workspace_pick(
    locale: RwSignal<Locale>,
    ws: WorkspacePanelSignals,
    side_panel_view: RwSignal<SidePanelView>,
) {
    ws.workspace_pick_busy.set(true);
    let loc = locale.get_untracked();
    spawn_local(async move {
        match crate::api::fetch_workspace_projects(loc).await {
            Ok(resp) if resp.enabled => {
                side_panel_view.set(SidePanelView::Workspace);
                ws.workspace_project_modal_open.set(true);
            }
            Ok(_) => {
                // 未配置项目池：最近列表 + 路径输入（勿用 window.prompt，移动端常不可用）
                ws.workspace_browser_pick_modal_open.set(true);
            }
            Err(e) => {
                ws.workspace_set_err.set(Some(e));
            }
        }
        ws.workspace_pick_busy.set(false);
    });
}

impl WorkspaceRootPickHandle {
    /// 本机 loopback serve 的桌面壳：系统文件夹对话框；否则（远程 serve / 浏览器 / Android）：
    /// 项目池弹窗，或最近列表 + 路径输入弹窗。
    pub fn spawn_pick_or_reveal(self) {
        let Self {
            locale,
            chat,
            ws,
            side_panel_view,
            ..
        } = self;
        ws.workspace_set_err.set(None);
        if workspace_inputs_blocked(ws) {
            return;
        }
        let use_native_folder_pick = tauri_shell_available()
            && crate::api::api_base_host_is_loopback(&crate::api::api_base_url());
        if !use_native_folder_pick {
            spawn_server_side_workspace_pick(locale, ws, side_panel_view);
            return;
        }
        ws.workspace_pick_busy.set(true);
        let loc = locale.get_untracked();
        spawn_local(async move {
            match tauri_pick_workspace_folder().await {
                Ok(None) => {}
                Ok(Some(raw)) => {
                    let p = raw.trim().to_string();
                    if !p.is_empty() {
                        ws.workspace_path_draft.set(p.clone());
                        ws.workspace_set_busy.set(true);
                        let _ = commit_workspace_root(chat, ws, p, loc).await;
                    }
                }
                Err(e) => {
                    ws.workspace_set_err.set(Some(e));
                }
            }
            ws.workspace_pick_busy.set(false);
        });
    }

    #[must_use]
    pub fn pick_busy_tracked(&self) -> bool {
        workspace_inputs_blocked(self.ws)
    }

    #[must_use]
    pub fn menu_label(&self) -> &'static str {
        let loc = self.locale.get();
        if self.ws.workspace_pick_busy.get() {
            i18n::ws_browse_busy_title(loc)
        } else {
            i18n::ide_menu_open_workspace(loc)
        }
    }

    /// 从最近列表打开已记录路径（不打开系统对话框）。
    pub fn spawn_open_recent(self, path: String) {
        let Self {
            locale, chat, ws, ..
        } = self;
        ws.workspace_set_err.set(None);
        let p = path.trim().to_string();
        if p.is_empty() || workspace_inputs_blocked(ws) {
            return;
        }
        ws.workspace_path_draft.set(p.clone());
        ws.workspace_set_busy.set(true);
        let loc = locale.get_untracked();
        spawn_local(async move {
            let _ = commit_workspace_root(chat, ws, p, loc).await;
        });
    }
}

/// 顶栏正中：工作区根路径只读标题（切换目录见「项目」菜单）。
/// 项目池模式只展示末段目录名；完整路径放在 `title` 悬停提示。
#[component]
pub(crate) fn ShellTopbarWorkspaceRoot(pick: WorkspaceRootPickHandle) -> impl IntoView {
    let WorkspaceRootPickHandle { locale, ws, .. } = pick;
    view! {
        <div class="shell-topbar-workspace" data-testid="shell-topbar-workspace">
            <span
                class="shell-topbar-workspace-title"
                data-testid="workspace-root-title"
                prop:aria-label=move || i18n::ws_root_label(locale.get())
                prop:title=move || {
                    let path = ws.workspace_path_draft.get();
                    if path.trim().is_empty() {
                        i18n::ws_path_title_hint(locale.get()).to_string()
                    } else {
                        path
                    }
                }
            >
                {move || {
                    let path = ws.workspace_path_draft.get();
                    if path.trim().is_empty() {
                        return i18n::ws_path_empty(locale.get()).to_string();
                    }
                    if ws.workspace_pool_enabled.get() {
                        workspace_recent_menu_label(&path)
                    } else {
                        path
                    }
                }}
            </span>
            <Show when=move || ws.workspace_set_err.get().is_some()>
                <span class="shell-topbar-workspace-error" role="alert" prop:title=move || {
                    ws.workspace_set_err.get().unwrap_or_default()
                }>
                    {move || ws.workspace_set_err.get().unwrap_or_default()}
                </span>
            </Show>
        </div>
    }
}
