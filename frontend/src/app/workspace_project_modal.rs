//! 项目池模式：列出 / 新建 / 打开命名工作区（远程浏览器无需手输绝对路径）。

use std::sync::Arc;

use gloo_timers::future::TimeoutFuture;
use leptos::html::Input;
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::app::focusable_menu::FocusableModalPanel;
use crate::app::workspace_root_actions::WorkspaceRootPickHandle;
use crate::i18n;

use super::workspace_project_modal_body::{
    WorkspaceProjectModalBody, spawn_reload_workspace_projects,
};
use super::workspace_project_modal_parts::{
    WorkspaceProjectNewRow, WorkspaceProjectOpenArgs, spawn_workspace_project_open,
};

#[derive(Clone, Copy)]
pub struct WorkspaceProjectModalSignals {
    pub open: RwSignal<bool>,
    pub workspace_pick: WorkspaceRootPickHandle,
}

#[component]
fn WorkspaceProjectModalPanel(signals: WorkspaceProjectModalSignals) -> impl IntoView {
    let WorkspaceProjectModalSignals {
        open,
        workspace_pick,
    } = signals;
    let WorkspaceRootPickHandle {
        locale,
        chat,
        ws,
        side_panel_view,
        ..
    } = workspace_pick;

    let projects = RwSignal::new(Vec::<String>::new());
    let pool_path = RwSignal::new(None::<String>);
    let loading = RwSignal::new(true);
    let load_err = RwSignal::new(None::<String>);
    let action_err = RwSignal::new(None::<String>);
    let new_name = RwSignal::new(String::new());
    let name_input_ref = NodeRef::<Input>::new();

    Effect::new(move |_| {
        if !open.get() {
            return;
        }
        action_err.set(None);
        spawn_reload_workspace_projects(locale, projects, pool_path, loading, load_err);
        let name_ref = name_input_ref.clone();
        spawn_local(async move {
            TimeoutFuture::new(0).await;
            if let Some(el) = name_ref.get() {
                let _ = el.focus();
            }
        });
    });

    let close = move || open.set(false);
    let open_args = WorkspaceProjectOpenArgs {
        open,
        locale,
        chat,
        ws,
        side_panel_view,
        action_err,
    };
    let on_open_existing = Arc::new(move |name: String| {
        spawn_workspace_project_open(open_args, name, false);
    });
    let on_create = Arc::new(move |name: String| {
        spawn_workspace_project_open(open_args, name, true);
    });

    view! {
        <FocusableModalPanel
            class="modal"
            dialog_role="dialog"
            labelledby="workspace-project-modal-title"
            testid="workspace-project-modal"
            on_escape=Callback::new(move |_| close())
        >
            <div class="modal-head">
                <h2 class="modal-title" id="workspace-project-modal-title">
                    {move || i18n::ws_project_modal_title(locale.get())}
                </h2>
                <span class="modal-head-spacer"></span>
                <button type="button" class="btn btn-ghost btn-sm" on:click=move |_| close()>
                    {move || i18n::settings_close(locale.get())}
                </button>
            </div>
            <div class="modal-body">
                <WorkspaceProjectModalBody
                    locale=locale
                    ws=ws
                    projects=projects
                    pool_path=pool_path
                    loading=loading
                    load_err=load_err
                    action_err=action_err
                    on_open=on_open_existing.clone()
                />
                <WorkspaceProjectNewRow
                    locale=locale
                    ws=ws
                    new_name=new_name
                    name_input_ref=name_input_ref
                    on_create=on_create.clone()
                />
            </div>
        </FocusableModalPanel>
    }
}

#[component]
fn WorkspaceProjectModalBackdrop(signals: WorkspaceProjectModalSignals) -> impl IntoView {
    let open = signals.open;
    view! {
        <div class="modal-backdrop" on:click=move |_| open.set(false)>
            <WorkspaceProjectModalPanel signals=signals />
        </div>
    }
}

pub fn workspace_project_modal_view(signals: WorkspaceProjectModalSignals) -> impl IntoView {
    let open = signals.open;
    view! {
        <Show when=move || open.get()>
            <WorkspaceProjectModalBackdrop signals=signals />
        </Show>
    }
}
