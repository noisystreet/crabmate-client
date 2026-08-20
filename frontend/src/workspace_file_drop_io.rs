//! 工作区树：本机文件 drop 后确认并 `PUT /workspace/file/raw`。

use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

use crate::api::put_workspace_file_raw;
use crate::confirm_dialog::confirm_in_page;
use crate::i18n::{self, Locale};
use crate::ide_disk_sync::spawn_sync_ide_tabs_from_disk;
use crate::workspace_context_menu::{WorkspaceContextMenuActions, WorkspaceTreeRefreshHint};
use crate::workspace_file_drop::{
    WorkspaceUploadPlanError, plan_workspace_uploads, workspace_accept_os_file_drag,
    workspace_upload_confirm_names,
};

fn data_transfer_types(dt: &web_sys::DataTransfer) -> Vec<String> {
    let types = dt.types();
    let mut out = Vec::new();
    for i in 0..types.length() {
        if let Some(t) = types.get(i).as_string() {
            out.push(t);
        }
    }
    out
}

pub(crate) fn workspace_os_file_dragover(ev: &web_sys::DragEvent) -> bool {
    let Some(dt) = ev.data_transfer() else {
        return false;
    };
    if !workspace_accept_os_file_drag(&data_transfer_types(&dt)) {
        return false;
    }
    ev.prevent_default();
    ev.stop_propagation();
    dt.set_drop_effect("copy");
    true
}

fn dest_dir_from_event(ev: &web_sys::DragEvent) -> String {
    let Some(target) = ev.target() else {
        return String::new();
    };
    let el = target.dyn_ref::<web_sys::Element>().cloned().or_else(|| {
        target
            .dyn_ref::<web_sys::Node>()
            .and_then(|n| n.parent_element())
    });
    let Some(el) = el else {
        return String::new();
    };
    el.closest("[data-ws-drop-dest]")
        .ok()
        .flatten()
        .and_then(|n| n.get_attribute("data-ws-drop-dest"))
        .unwrap_or_default()
}

fn file_nested_rel(f: &web_sys::File) -> String {
    js_sys::Reflect::get(f, &wasm_bindgen::JsValue::from_str("webkitRelativePath"))
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_default()
}

fn collect_file_meta(list: &web_sys::FileList) -> Vec<(String, String, u64)> {
    let n = list.length();
    let mut out = Vec::with_capacity(n as usize);
    for i in 0..n {
        let Some(f) = list.item(i) else {
            continue;
        };
        let size = f.size() as u64;
        out.push((f.name(), file_nested_rel(&f), size));
    }
    out
}

fn collect_files(list: &web_sys::FileList) -> Vec<web_sys::File> {
    let n = list.length();
    let mut out = Vec::with_capacity(n as usize);
    for i in 0..n {
        if let Some(f) = list.item(i) {
            out.push(f);
        }
    }
    out
}

fn plan_error_message(err: WorkspaceUploadPlanError, loc: Locale) -> String {
    match err {
        WorkspaceUploadPlanError::Empty => String::new(),
        WorkspaceUploadPlanError::TooMany => i18n::workspace_upload_too_many(
            loc,
            crate::workspace_file_drop::WORKSPACE_UPLOAD_MAX_FILES,
        ),
        WorkspaceUploadPlanError::TooLarge { name } => i18n::workspace_upload_too_large(loc, &name),
        WorkspaceUploadPlanError::BadName { name } => {
            format!("{} ({name})", i18n::workspace_tree_name_invalid(loc))
        }
    }
}

async fn file_bytes(f: &web_sys::File) -> Result<Vec<u8>, String> {
    let buf = JsFuture::from(f.array_buffer())
        .await
        .map_err(|e| format!("{e:?}"))?;
    let arr = js_sys::Uint8Array::new(&buf);
    let mut out = vec![0u8; arr.length() as usize];
    arr.copy_to(&mut out);
    Ok(out)
}

/// 处理工作区树上的 OS 文件 `drop`：确认后逐个 `PUT /workspace/file/raw`。
pub(crate) fn handle_workspace_os_file_drop(
    ev: web_sys::DragEvent,
    locale: RwSignal<Locale>,
    workspace_err: RwSignal<Option<String>>,
    actions: WorkspaceContextMenuActions,
) {
    let Some(dt) = ev.data_transfer() else {
        return;
    };
    if !workspace_accept_os_file_drag(&data_transfer_types(&dt)) {
        return;
    }
    ev.prevent_default();
    ev.stop_propagation();
    let dest_dir = dest_dir_from_event(&ev);
    let Some(list) = dt.files() else {
        return;
    };
    let meta = collect_file_meta(&list);
    let files = collect_files(&list);
    match plan_workspace_uploads(dest_dir.as_str(), &meta) {
        Ok(plan) => spawn_upload_plan(dest_dir, plan, files, locale, workspace_err, actions),
        Err(WorkspaceUploadPlanError::Empty) => {}
        Err(e) => workspace_err.set(Some(plan_error_message(e, locale.get_untracked()))),
    }
}

fn refresh_after_upload(
    actions: &WorkspaceContextMenuActions,
    dest_dir: String,
    locale: RwSignal<Locale>,
) {
    (actions.refresh_after_mutation)(WorkspaceTreeRefreshHint {
        parent_rel: dest_dir,
        deleted_rel: None,
    });
    let Some((tabs, editor)) = actions.ide_tabs.clone() else {
        return;
    };
    let Some(confirm) = actions.ide_confirm else {
        return;
    };
    spawn_sync_ide_tabs_from_disk(tabs, editor, locale, confirm);
}

fn spawn_upload_plan(
    dest_dir: String,
    plan: Vec<crate::workspace_file_drop::WorkspaceUploadPlanItem>,
    files: Vec<web_sys::File>,
    locale: RwSignal<Locale>,
    workspace_err: RwSignal<Option<String>>,
    actions: WorkspaceContextMenuActions,
) {
    spawn_local(async move {
        let loc = locale.get_untracked();
        let names: Vec<String> = plan.iter().map(|p| p.name.clone()).collect();
        let listed = workspace_upload_confirm_names(&names, 5);
        let msg =
            i18n::workspace_upload_confirm(loc, dest_dir.as_str(), listed.as_str(), plan.len());
        if !confirm_in_page(
            msg.as_str(),
            i18n::workspace_upload_ok(loc),
            i18n::ide_confirm_cancel(loc),
        )
        .await
        {
            return;
        }
        let mut wrote_any = false;
        for (item, file) in plan.into_iter().zip(files) {
            match upload_one_file(item.rel.as_str(), &file, loc).await {
                Ok(()) => wrote_any = true,
                Err(UploadOneError::OverwriteDeclined) => {
                    workspace_err.set(Some(
                        i18n::workspace_upload_overwrite_cancelled(loc).to_string(),
                    ));
                    if wrote_any {
                        refresh_after_upload(&actions, dest_dir, locale);
                    }
                    return;
                }
                Err(UploadOneError::Other(e)) => {
                    workspace_err.set(Some(e));
                    if wrote_any {
                        refresh_after_upload(&actions, dest_dir, locale);
                    }
                    return;
                }
            }
        }
        workspace_err.set(None);
        refresh_after_upload(&actions, dest_dir, locale);
    });
}

enum UploadOneError {
    OverwriteDeclined,
    Other(String),
}

async fn upload_one_file(
    rel: &str,
    file: &web_sys::File,
    loc: Locale,
) -> Result<(), UploadOneError> {
    let bytes = file_bytes(file).await.map_err(UploadOneError::Other)?;
    match put_workspace_file_raw(rel, &bytes, true, loc).await {
        Ok(()) => Ok(()),
        Err(e) if e.is_conflict() => {
            let ok = confirm_in_page(
                i18n::workspace_upload_overwrite_confirm(loc, rel).as_str(),
                i18n::workspace_upload_overwrite_ok(loc),
                i18n::ide_confirm_cancel(loc),
            )
            .await;
            if !ok {
                return Err(UploadOneError::OverwriteDeclined);
            }
            put_workspace_file_raw(rel, &bytes, false, loc)
                .await
                .map_err(|e| UploadOneError::Other(e.message))
        }
        Err(e) => Err(UploadOneError::Other(e.message)),
    }
}
