//! 项目池弹窗：列表区与加载状态（降低主面板圈复杂度）。

use std::sync::Arc;

use leptos::prelude::*;

use crate::app::workspace_panel_state::WorkspacePanelSignals;
use crate::i18n::{self, Locale};

use super::workspace_project_modal_parts::WorkspaceProjectListRows;

/// 顶部横幅区（池路径 + 各类错误），独立以降低 body CCN。
#[component]
fn WorkspaceProjectModalBanners(
    locale: RwSignal<Locale>,
    pool_path: RwSignal<Option<String>>,
    loading: RwSignal<bool>,
    load_err: RwSignal<Option<String>>,
    action_err: RwSignal<Option<String>>,
) -> impl IntoView {
    view! {
        <Show when=move || pool_path.get().is_some()>
            <p class="modal-hint workspace-project-pool-path">
                {move || pool_path.get().unwrap_or_default()}
            </p>
        </Show>
        <Show when=move || load_err.get().is_some()>
            <p class="workspace-project-error" role="alert" data-testid="workspace-project-load-err">
                {move || load_err.get().unwrap_or_default()}
            </p>
        </Show>
        <Show when=move || action_err.get().is_some()>
            <p class="workspace-project-error" role="alert" data-testid="workspace-project-action-err">
                {move || action_err.get().unwrap_or_default()}
            </p>
        </Show>
        <Show when=move || loading.get()>
            <p class="modal-hint">{move || i18n::ws_project_loading(locale.get())}</p>
        </Show>
    }
}

#[component]
pub(crate) fn WorkspaceProjectModalBody(
    locale: RwSignal<Locale>,
    ws: WorkspacePanelSignals,
    projects: RwSignal<Vec<String>>,
    pool_path: RwSignal<Option<String>>,
    loading: RwSignal<bool>,
    load_err: RwSignal<Option<String>>,
    action_err: RwSignal<Option<String>>,
    on_open: Arc<dyn Fn(String) + Send + Sync>,
) -> impl IntoView {
    view! {
        <WorkspaceProjectModalBanners
            locale=locale
            pool_path=pool_path
            loading=loading
            load_err=load_err
            action_err=action_err
        />
        <Show when=move || !loading.get() && load_err.get().is_none()>
            <WorkspaceProjectListRows
                locale=locale
                ws=ws
                projects=projects
                on_open=Arc::clone(&on_open)
            />
        </Show>
    }
}

pub(crate) fn spawn_reload_workspace_projects(
    locale: RwSignal<Locale>,
    projects: RwSignal<Vec<String>>,
    pool_path: RwSignal<Option<String>>,
    loading: RwSignal<bool>,
    load_err: RwSignal<Option<String>>,
) {
    loading.set(true);
    load_err.set(None);
    let loc = locale.get_untracked();
    leptos::task::spawn_local(async move {
        match crate::api::fetch_workspace_projects(loc).await {
            Ok(resp) => {
                pool_path.set(resp.pool_path);
                projects.set(resp.projects);
                loading.set(false);
            }
            Err(e) => {
                load_err.set(Some(e));
                loading.set(false);
            }
        }
    });
}
