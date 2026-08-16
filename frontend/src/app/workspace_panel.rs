//! Workspace 侧栏：拉取目录树与在切换到 Workspace 视图时自动刷新；以及树双击 / 拖放插入 **`@相对路径`** 到输入框。

use std::sync::Arc;

use gloo_timers::future::TimeoutFuture;
use leptos::html::Textarea;
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::app_prefs::SidePanelView;
use crate::i18n::Locale;
use crate::workspace_context_menu::WorkspaceTreeRefreshHint;
use crate::workspace_shell::{refresh_workspace_panel_after_mutation, reload_workspace_panel};

use super::workspace_panel_state::WorkspacePanelSignals;

/// 返回与历史行为一致的 `reload_workspace_panel` 封装（供 SSE `on_workspace_changed`、侧栏等复用）。
pub(super) fn make_refresh_workspace(
    ws: WorkspacePanelSignals,
    locale: Locale,
) -> Arc<dyn Fn() + Send + Sync> {
    Arc::new(move || {
        spawn_local(async move {
            reload_workspace_panel(
                ws.workspace_loading,
                ws.workspace_err,
                ws.workspace_path_draft,
                ws.workspace_data,
                ws.workspace_subtree_expanded,
                ws.workspace_subtree_cache,
                ws.workspace_subtree_loading,
                locale,
            )
            .await;
        });
    })
}

/// 文件树新建/删除后刷新（保留展开状态、不重载骨架屏）。
pub(super) fn make_refresh_workspace_after_mutation(
    ws: WorkspacePanelSignals,
    locale: Locale,
) -> Arc<dyn Fn(WorkspaceTreeRefreshHint) + Send + Sync> {
    Arc::new(move |hint: WorkspaceTreeRefreshHint| {
        spawn_local(async move {
            refresh_workspace_panel_after_mutation(
                ws.workspace_err,
                ws.workspace_path_draft,
                ws.workspace_data,
                ws.workspace_subtree_expanded,
                ws.workspace_subtree_cache,
                ws.workspace_subtree_loading,
                locale,
                &hint,
            )
            .await;
        });
    })
}

/// 工作区树双击文件时，将 **`@{rel}`** 插入 composer 草稿并聚焦输入框。
pub(super) fn make_insert_workspace_path_into_composer(
    draft: RwSignal<String>,
    status_err: RwSignal<Option<String>>,
    locale: RwSignal<Locale>,
    composer_input_ref: NodeRef<Textarea>,
) -> Arc<dyn Fn(String) + Send + Sync> {
    Arc::new(move |rel: String| {
        if rel.chars().any(|c| c.is_whitespace()) {
            status_err.set(Some(
                crate::i18n::composer_ws_path_whitespace_err(locale.get_untracked()).to_string(),
            ));
            return;
        }
        let rel = rel.trim().trim_start_matches('/').replace('\\', "/");
        if rel.is_empty() {
            return;
        }
        // 与镜像高亮 / 服务端展开约定：插入 `@rel`（气泡链接文字只显示 rel，不含协议）。
        let token = format!("@{rel}");
        let mut guard = draft.get_untracked();
        let needs_space = guard
            .chars()
            .next_back()
            .is_some_and(|c| !c.is_whitespace());
        if needs_space {
            guard.push(' ');
        }
        guard.push_str(&token);
        guard.push(' ');
        draft.set(guard);
        status_err.set(None);
        let cref = composer_input_ref.clone();
        spawn_local(async move {
            TimeoutFuture::new(0).await;
            if let Some(el) = cref.get() {
                let _ = el.focus();
            }
        });
    })
}

/// 侧栏变为 Workspace 时是否应自动 `GET /workspace`。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct WorkspaceVisibleRefreshState {
    pub was_initialized: bool,
    pub skip_cold_start_fetch: bool,
}

/// 冷启动时侧栏已是 Workspace：跳过第一次自动拉取（不打开上次目录树）。
/// 之后从其它视图切回 Workspace，或启动时侧栏不是 Workspace 再打开，仍刷新。
#[must_use]
pub(crate) fn take_workspace_visible_auto_refresh(
    initialized: bool,
    is_workspace_panel: bool,
    state: &mut WorkspaceVisibleRefreshState,
) -> bool {
    let just_initialized = initialized && !state.was_initialized;
    if just_initialized && is_workspace_panel {
        state.skip_cold_start_fetch = true;
    }
    state.was_initialized = initialized;
    if !initialized || !is_workspace_panel {
        return false;
    }
    if state.skip_cold_start_fetch {
        state.skip_cold_start_fetch = false;
        return false;
    }
    true
}

/// 初始化完成且侧栏为 Workspace 时拉取一次（冷启动已是 Workspace 则跳过）。
pub(super) fn wire_workspace_refresh_when_visible(
    side_panel_view: RwSignal<SidePanelView>,
    initialized: RwSignal<bool>,
    refresh_workspace: Arc<dyn Fn() + Send + Sync>,
) {
    let refresh_state = StoredValue::new(WorkspaceVisibleRefreshState::default());
    Effect::new({
        let refresh_workspace = Arc::clone(&refresh_workspace);
        move |_| {
            let mut state = refresh_state.get_value();
            let should = take_workspace_visible_auto_refresh(
                initialized.get(),
                matches!(side_panel_view.get(), SidePanelView::Workspace),
                &mut state,
            );
            refresh_state.set_value(state);
            if should {
                refresh_workspace();
            }
        }
    });
}

#[cfg(test)]
mod workspace_visible_refresh_tests {
    use super::{WorkspaceVisibleRefreshState, take_workspace_visible_auto_refresh};

    #[test]
    fn skips_first_workspace_fetch_when_already_open_at_init() {
        let mut state = WorkspaceVisibleRefreshState::default();
        assert!(!take_workspace_visible_auto_refresh(
            false, true, &mut state
        ));
        assert!(!take_workspace_visible_auto_refresh(true, true, &mut state));
        assert!(!take_workspace_visible_auto_refresh(
            true, false, &mut state
        ));
        assert!(take_workspace_visible_auto_refresh(true, true, &mut state));
    }

    #[test]
    fn refreshes_when_workspace_opened_after_init() {
        let mut state = WorkspaceVisibleRefreshState::default();
        assert!(!take_workspace_visible_auto_refresh(
            false, false, &mut state
        ));
        assert!(!take_workspace_visible_auto_refresh(
            true, false, &mut state
        ));
        assert!(take_workspace_visible_auto_refresh(true, true, &mut state));
    }
}
