//! 项目池弹窗子视图与打开/创建逻辑（从 `workspace_project_modal` 拆出以降低圈复杂度）。

use std::sync::Arc;

use leptos::html::Input;
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::api::post_workspace_project;
use crate::app::workspace_panel_state::WorkspacePanelSignals;
use crate::app::workspace_root_actions::{
    WorkspaceSessionHandoff, finish_workspace_root_ui, flush_current_workspace_sessions,
};
use crate::app_prefs::SidePanelView;
use crate::chat_session_state::ChatSessionSignals;
use crate::i18n::{self, Locale};
use crate::session_workspace_partition::{
    begin_workspace_session_persist_block, clear_workspace_session_persist_block,
};

#[derive(Clone, Copy)]
pub(crate) struct WorkspaceProjectOpenArgs {
    pub open: RwSignal<bool>,
    pub locale: RwSignal<Locale>,
    pub chat: ChatSessionSignals,
    pub ws: WorkspacePanelSignals,
    pub side_panel_view: RwSignal<SidePanelView>,
    pub action_err: RwSignal<Option<String>>,
}

pub(crate) fn spawn_workspace_project_open(
    args: WorkspaceProjectOpenArgs,
    name: String,
    create: bool,
) {
    let WorkspaceProjectOpenArgs {
        open,
        locale,
        chat,
        ws,
        side_panel_view,
        action_err,
    } = args;
    if ws.workspace_set_busy.get() || ws.workspace_pick_busy.get() {
        return;
    }
    let name = name.trim().to_string();
    if name.is_empty() {
        action_err.set(Some(
            i18n::ws_path_required(locale.get_untracked()).to_string(),
        ));
        return;
    }
    action_err.set(None);
    ws.workspace_set_err.set(None);
    ws.workspace_set_busy.set(true);
    side_panel_view.set(SidePanelView::Workspace);
    let loc = locale.get_untracked();
    spawn_local(async move {
        if let Err(e) = flush_current_workspace_sessions(chat, loc).await {
            action_err.set(Some(e));
            ws.workspace_set_busy.set(false);
            return;
        }
        begin_workspace_session_persist_block();
        match post_workspace_project(&name, create, loc).await {
            Ok(resp) if resp.ok => {
                ws.workspace_path_draft.set(resp.path.clone());
                action_err.set(None);
                ws.workspace_set_err.set(None);
                finish_workspace_root_ui(
                    chat,
                    ws,
                    resp.path,
                    loc,
                    WorkspaceSessionHandoff::RestoreBucketActive,
                    None,
                )
                .await;
                ws.workspace_set_busy.set(false);
                open.set(false);
            }
            Ok(resp) => {
                clear_workspace_session_persist_block();
                action_err.set(
                    resp.error
                        .or(Some(i18n::api_err_workspace_set_failed(loc).to_string())),
                );
                ws.workspace_set_busy.set(false);
            }
            Err(e) => {
                clear_workspace_session_persist_block();
                action_err.set(Some(e));
                ws.workspace_set_busy.set(false);
            }
        }
    });
}

#[component]
pub(crate) fn WorkspaceProjectListRows(
    locale: RwSignal<Locale>,
    ws: WorkspacePanelSignals,
    projects: RwSignal<Vec<String>>,
    on_open: Arc<dyn Fn(String) + Send + Sync>,
) -> impl IntoView {
    view! {
        <div class="workspace-project-list">
            {move || {
                let list = projects.get();
                if list.is_empty() {
                    view! {
                        <p class="modal-hint">{i18n::ws_project_empty(locale.get_untracked())}</p>
                    }
                    .into_any()
                } else {
                    list.into_iter()
                        .map(|name| {
                            let name_for_click = name.clone();
                            let on_open = Arc::clone(&on_open);
                            view! {
                                <button
                                    type="button"
                                    class="btn btn-secondary btn-sm workspace-project-row"
                                    data-testid="workspace-project-open"
                                    prop:disabled=move || ws.workspace_set_busy.get()
                                    on:click=move |_| on_open(name_for_click.clone())
                                >
                                    {name}
                                    <span class="workspace-project-open-label">
                                        {i18n::ws_project_open(locale.get_untracked())}
                                    </span>
                                </button>
                            }
                        })
                        .collect_view()
                        .into_any()
                }
            }}
        </div>
    }
}

#[component]
pub(crate) fn WorkspaceProjectNewRow(
    locale: RwSignal<Locale>,
    ws: WorkspacePanelSignals,
    new_name: RwSignal<String>,
    name_input_ref: NodeRef<Input>,
    on_create: Arc<dyn Fn(String) + Send + Sync>,
) -> impl IntoView {
    view! {
        <div class="workspace-project-new">
            <label class="settings-label" for="workspace-project-new-input">
                {move || i18n::ws_project_new_label(locale.get())}
            </label>
            <div class="workspace-project-new-row">
                <input
                    id="workspace-project-new-input"
                    class="input"
                    type="text"
                    node_ref=name_input_ref
                    prop:placeholder=move || i18n::ws_project_new_placeholder(locale.get())
                    prop:value=move || new_name.get()
                    on:input=move |ev| new_name.set(event_target_value(&ev))
                    on:keydown={
                        let on_create = Arc::clone(&on_create);
                        move |ev: web_sys::KeyboardEvent| {
                            if ev.key() == "Enter" {
                                on_create(new_name.get_untracked());
                            }
                        }
                    }
                />
                <button
                    type="button"
                    class="btn btn-primary btn-sm"
                    data-testid="workspace-project-create"
                    prop:disabled=move || ws.workspace_set_busy.get() || new_name.get().trim().is_empty()
                    on:click={
                        let on_create = Arc::clone(&on_create);
                        move |_| on_create(new_name.get_untracked())
                    }
                >
                    {move || i18n::ws_project_create_open(locale.get())}
                </button>
            </div>
        </div>
    }
}
