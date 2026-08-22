//! 工作区文件树：磁盘新建/删除异步操作。

use std::sync::Arc;

use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::api::{
    delete_workspace_dir, delete_workspace_file, fetch_workspace_dir_archive,
    fetch_workspace_file_download, post_workspace_dir, post_workspace_file_move,
    post_workspace_file_write_opts,
};
use crate::i18n::Locale;
use crate::ide_save::spawn_create_and_open_file;
use crate::ide_tabs::{
    force_close_tabs_for_deleted_entry, ide_tab_basename, retarget_tabs_after_file_move,
    tab_list_path_is_dirty,
};
use crate::session_export::trigger_download_bytes;
use crate::workspace_context_menu::{
    WorkspaceContextMenuActions, WorkspaceInlineCreateKind, WorkspaceTreeRefreshHint,
};
use crate::workspace_file_drop::WORKSPACE_UPLOAD_MAX_BYTES;
use crate::workspace_tree::workspace_parent_rel;

fn refresh_after_create(actions: &WorkspaceContextMenuActions, parent_rel: String) {
    (actions.refresh_after_mutation)(WorkspaceTreeRefreshHint {
        parent_rel,
        deleted_rel: None,
    });
}

fn refresh_after_delete(actions: &WorkspaceContextMenuActions, deleted_rel: String) {
    let parent_rel = workspace_parent_rel(deleted_rel.as_str());
    (actions.refresh_after_mutation)(WorkspaceTreeRefreshHint {
        parent_rel,
        deleted_rel: Some(deleted_rel),
    });
}

pub fn spawn_create_workspace_file(
    rel: String,
    locale: RwSignal<Locale>,
    workspace_err: RwSignal<Option<String>>,
    actions: WorkspaceContextMenuActions,
) {
    let parent_rel = workspace_parent_rel(rel.as_str());
    spawn_local(async move {
        let loc = locale.get_untracked();
        match post_workspace_file_write_opts(rel, String::new(), true, false, loc).await {
            Ok(()) => {
                workspace_err.set(None);
                refresh_after_create(&actions, parent_rel);
            }
            Err(e) => workspace_err.set(Some(e)),
        }
    });
}

pub fn spawn_create_workspace_dir(
    rel: String,
    locale: RwSignal<Locale>,
    workspace_err: RwSignal<Option<String>>,
    actions: WorkspaceContextMenuActions,
) {
    let parent_rel = workspace_parent_rel(rel.as_str());
    spawn_local(async move {
        let loc = locale.get_untracked();
        match post_workspace_dir(rel, true, loc).await {
            Ok(()) => {
                workspace_err.set(None);
                refresh_after_create(&actions, parent_rel);
            }
            Err(e) => workspace_err.set(Some(e)),
        }
    });
}

/// 浏览器 `<a download>` / 桌面另存为对话框用的文件名（仅最后一段）。
#[must_use]
pub fn workspace_download_basename(rel: &str) -> String {
    let name = ide_tab_basename(rel);
    let name = name.trim();
    if name.is_empty() || name == "." || name == ".." {
        "download".to_string()
    } else {
        name.to_string()
    }
}

fn open_tab_text_for_path(actions: &WorkspaceContextMenuActions, rel: &str) -> Option<String> {
    let (tabs, editor) = actions.ide_tabs?;
    tabs.persist_editor_into_active(editor.ide_text, editor.ide_baseline);
    tabs.tabs
        .get_untracked()
        .into_iter()
        .find(|t| t.path == rel)
        .map(|t| t.text)
}

async fn workspace_bytes_for_local_save(
    rel: &str,
    loc: Locale,
    actions: &WorkspaceContextMenuActions,
) -> Result<Vec<u8>, String> {
    if let Some(text) = open_tab_text_for_path(actions, rel) {
        return Ok(text.into_bytes());
    }
    fetch_workspace_file_download(rel, loc).await
}

/// 把工作区文件下载到本机（桌面另存为；Android 系统另存为；浏览器 `<a download>`）。
/// 磁盘内容走 `GET /workspace/file/download`（原样字节：PDF/二进制/文本）。
/// 若该路径已在 IDE 打开，用当前缓冲区的 UTF-8 字节（含未写回 serve 的编辑）。
pub fn spawn_save_workspace_file_to_device(
    rel: String,
    locale: RwSignal<Locale>,
    workspace_err: RwSignal<Option<String>>,
    actions: WorkspaceContextMenuActions,
) {
    spawn_local(async move {
        let loc = locale.get_untracked();
        let filename = workspace_download_basename(rel.as_str());
        let content = match workspace_bytes_for_local_save(rel.as_str(), loc, &actions).await {
            Ok(c) => c,
            Err(e) => {
                workspace_err.set(Some(e));
                return;
            }
        };
        if content.len() as u64 > WORKSPACE_UPLOAD_MAX_BYTES {
            workspace_err.set(Some(crate::i18n::workspace_save_too_large(
                loc,
                filename.as_str(),
            )));
            return;
        }
        match trigger_download_bytes(&filename, &content, loc) {
            Ok(()) => workspace_err.set(None),
            Err(e) => workspace_err.set(Some(e)),
        }
    });
}

/// 目录 zip 下载文件名：根为 `workspace.zip`；目录名后加 `.zip`（不剥已有 `.zip` 后缀）。
#[must_use]
pub fn workspace_archive_download_name(rel: &str) -> String {
    let stem = workspace_download_basename(rel);
    if rel.trim().trim_matches('/').is_empty() {
        return "workspace.zip".to_string();
    }
    if stem.ends_with(".zip") {
        stem
    } else {
        format!("{stem}.zip")
    }
}

/// 把工作区目录下载为 zip（`GET /workspace/dir/archive`）。空 `rel` 为工作区根。
pub fn spawn_save_workspace_dir_to_device(
    rel: String,
    locale: RwSignal<Locale>,
    workspace_err: RwSignal<Option<String>>,
    _actions: WorkspaceContextMenuActions,
) {
    spawn_local(async move {
        let loc = locale.get_untracked();
        let filename = workspace_archive_download_name(rel.as_str());
        let content = match fetch_workspace_dir_archive(rel.as_str(), loc).await {
            Ok(c) => c,
            Err(e) => {
                workspace_err.set(Some(e));
                return;
            }
        };
        if content.len() as u64 > WORKSPACE_UPLOAD_MAX_BYTES {
            workspace_err.set(Some(crate::i18n::workspace_save_too_large(
                loc,
                filename.as_str(),
            )));
            return;
        }
        match trigger_download_bytes(&filename, &content, loc) {
            Ok(()) => workspace_err.set(None),
            Err(e) => workspace_err.set(Some(e)),
        }
    });
}

fn refresh_after_rename(actions: &WorkspaceContextMenuActions, from_rel: &str, to_rel: &str) {
    if let Some((tabs, editor)) = actions.ide_tabs {
        retarget_tabs_after_file_move(tabs, from_rel, to_rel, editor);
    }
    let parent_rel = workspace_parent_rel(from_rel);
    (actions.refresh_after_mutation)(WorkspaceTreeRefreshHint {
        parent_rel,
        deleted_rel: Some(from_rel.to_string()),
    });
}

fn dest_tab_is_dirty(actions: &WorkspaceContextMenuActions, to_rel: &str) -> bool {
    let Some((tabs, editor)) = actions.ide_tabs else {
        return false;
    };
    tabs.persist_editor_into_active(editor.ide_text, editor.ide_baseline);
    tab_list_path_is_dirty(&tabs.tabs.get_untracked(), to_rel)
}

fn rename_overwrite_confirm_message(loc: Locale, to_rel: &str, dest_dirty: bool) -> String {
    if dest_dirty {
        crate::i18n::workspace_tree_rename_overwrite_dirty_confirm(loc, to_rel)
    } else {
        crate::i18n::workspace_tree_rename_overwrite_confirm(loc, to_rel)
    }
}

async fn overwrite_move_after_confirm(
    from_rel: &str,
    to_rel: &str,
    cid: Option<&str>,
    loc: Locale,
    actions: &WorkspaceContextMenuActions,
) -> Result<bool, String> {
    let dest_dirty = dest_tab_is_dirty(actions, to_rel);
    if !crate::confirm_dialog::confirm_in_page(
        rename_overwrite_confirm_message(loc, to_rel, dest_dirty).as_str(),
        crate::i18n::workspace_tree_rename_overwrite_ok(loc),
        crate::i18n::ide_confirm_cancel(loc),
    )
    .await
    {
        return Ok(false);
    }
    post_workspace_file_move(from_rel, to_rel, true, cid, loc)
        .await
        .map_err(|e| e.message)?;
    Ok(true)
}

/// 工作区内重命名文件（`POST /workspace/file/move`）。目标已存在时确认覆盖。
pub fn spawn_rename_workspace_file(
    from_rel: String,
    to_rel: String,
    locale: RwSignal<Locale>,
    workspace_err: RwSignal<Option<String>>,
    actions: WorkspaceContextMenuActions,
) {
    spawn_local(async move {
        let loc = locale.get_untracked();
        if from_rel == to_rel {
            workspace_err.set(None);
            return;
        }
        let cid = actions.bound_conversation_id();
        let moved = post_workspace_file_move(
            from_rel.as_str(),
            to_rel.as_str(),
            false,
            cid.as_deref(),
            loc,
        )
        .await;
        let result = match moved {
            Ok(()) => Ok(true),
            Err(e) if e.is_conflict() => {
                overwrite_move_after_confirm(
                    from_rel.as_str(),
                    to_rel.as_str(),
                    cid.as_deref(),
                    loc,
                    &actions,
                )
                .await
            }
            Err(e) => Err(e.message),
        };
        match result {
            Ok(true) => {
                workspace_err.set(None);
                refresh_after_rename(&actions, from_rel.as_str(), to_rel.as_str());
            }
            Ok(false) => {}
            Err(e) => workspace_err.set(Some(e)),
        }
    });
}

pub fn spawn_delete_workspace_entry(
    rel: String,
    is_dir: bool,
    locale: RwSignal<Locale>,
    workspace_err: RwSignal<Option<String>>,
    actions: WorkspaceContextMenuActions,
) {
    spawn_local(async move {
        let loc = locale.get_untracked();
        let deleted = rel.clone();
        let result = if is_dir {
            delete_workspace_dir(rel.as_str(), true, loc).await
        } else {
            delete_workspace_file(rel.as_str(), loc).await
        };
        match &result {
            Ok(()) => {
                workspace_err.set(None);
                if let Some((tabs, editor)) = actions.ide_tabs {
                    force_close_tabs_for_deleted_entry(tabs, deleted.as_str(), is_dir, editor);
                }
                refresh_after_delete(&actions, deleted);
            }
            Err(e) => workspace_err.set(Some(e.clone())),
        }
    });
}

/// 行内输入确认：校验名称并创建文件或目录。
pub fn commit_inline_create(
    name: String,
    parent_rel: &str,
    kind: WorkspaceInlineCreateKind,
    locale: RwSignal<Locale>,
    workspace_err: RwSignal<Option<String>>,
    actions: WorkspaceContextMenuActions,
) {
    let name = name.trim().to_string();
    if !crate::workspace_context_menu::name_segment_valid(&name) {
        workspace_err.set(Some(
            crate::i18n::workspace_tree_name_invalid(locale.get_untracked()).to_string(),
        ));
        return;
    }
    let rel = crate::workspace_tree::workspace_child_rel(parent_rel, &name);
    let parent_for_refresh = parent_rel.to_string();
    workspace_err.set(None);
    match kind {
        WorkspaceInlineCreateKind::File => {
            if let Some((tabs, editor)) = actions.ide_tabs {
                let after_create = {
                    let refresh = Arc::clone(&actions.refresh_after_mutation);
                    let parent = parent_for_refresh.clone();
                    Arc::new(move || {
                        refresh(WorkspaceTreeRefreshHint {
                            parent_rel: parent.clone(),
                            deleted_rel: None,
                        })
                    })
                };
                spawn_create_and_open_file(
                    crate::ide_save::IdeSaveContext {
                        tabs,
                        ide_path: editor.ide_path,
                        ide_text: editor.ide_text,
                        ide_baseline: editor.ide_baseline,
                        ide_err: tabs.err,
                    },
                    locale,
                    rel,
                    Some(after_create),
                    actions
                        .ide_confirm
                        .expect("IDE inline create requires confirm"),
                );
            } else {
                spawn_create_workspace_file(rel, locale, workspace_err, actions);
            }
        }
        WorkspaceInlineCreateKind::Dir => {
            spawn_create_workspace_dir(rel, locale, workspace_err, actions);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::workspace_download_basename;
    use crate::i18n::{Locale, workspace_tree_ctx_save_to_device};

    #[test]
    fn download_basename_uses_last_path_segment() {
        assert_eq!(workspace_download_basename("src/lib.rs"), "lib.rs");
        assert_eq!(workspace_download_basename("lib.rs"), "lib.rs");
        assert_eq!(workspace_download_basename(r"src\lib.rs"), "lib.rs");
    }

    #[test]
    fn download_basename_keeps_cjk() {
        assert_eq!(workspace_download_basename("笔记/说明.txt"), "说明.txt");
        assert_eq!(workspace_download_basename("你好"), "你好");
    }

    #[test]
    fn download_basename_rejects_empty_and_dots() {
        assert_eq!(workspace_download_basename(""), "download");
        assert_eq!(workspace_download_basename("/"), "download");
        assert_eq!(workspace_download_basename("."), "download");
        assert_eq!(workspace_download_basename("foo/.."), "download");
    }

    #[test]
    fn archive_download_name_root_and_dir() {
        use super::workspace_archive_download_name;
        assert_eq!(workspace_archive_download_name(""), "workspace.zip");
        assert_eq!(workspace_archive_download_name("src"), "src.zip");
        assert_eq!(workspace_archive_download_name("笔记/docs"), "docs.zip");
        assert_eq!(workspace_archive_download_name("notes.zip"), "notes.zip");
    }

    #[test]
    fn save_to_device_label_is_bilingual() {
        assert_eq!(
            workspace_tree_ctx_save_to_device(Locale::ZhHans),
            "保存到本机…"
        );
        assert_eq!(
            workspace_tree_ctx_save_to_device(Locale::En),
            "Save to this device…"
        );
        assert_eq!(
            crate::i18n::workspace_tree_ctx_rename_file(Locale::ZhHans),
            "重命名…"
        );
        assert_eq!(
            crate::i18n::workspace_tree_ctx_rename_file(Locale::En),
            "Rename…"
        );
        assert_eq!(
            crate::i18n::workspace_tree_rename_overwrite_confirm(Locale::ZhHans, "b.rs"),
            "b.rs 已存在，要覆盖吗？"
        );
        assert_eq!(
            crate::i18n::workspace_tree_rename_overwrite_confirm(Locale::En, "b.rs"),
            "b.rs already exists. Overwrite?"
        );
        assert!(
            crate::i18n::workspace_tree_rename_overwrite_dirty_confirm(Locale::ZhHans, "b.rs")
                .contains("未保存")
        );
        assert!(
            crate::i18n::workspace_tree_rename_overwrite_dirty_confirm(Locale::En, "b.rs")
                .contains("unsaved")
        );
        assert_eq!(
            crate::i18n::workspace_save_too_large(Locale::ZhHans, "说明.txt"),
            "说明.txt 超过 16 MiB，无法保存到本机"
        );
        assert_eq!(
            crate::i18n::workspace_save_too_large(Locale::En, "说明.txt"),
            "说明.txt is over 16 MiB and cannot be saved to this device"
        );
        assert_eq!(crate::i18n::workspace_upload_ok(Locale::ZhHans), "上传");
        assert_eq!(crate::i18n::workspace_upload_ok(Locale::En), "Upload");
    }

    #[test]
    fn android_save_failed_alert_is_bilingual() {
        assert_eq!(
            crate::i18n::export_android_save_failed(Locale::ZhHans),
            "无法打开系统保存对话框，文件未保存到本机。"
        );
        assert_eq!(
            crate::i18n::export_android_save_failed(Locale::En),
            "Could not open the system save dialog; the file was not saved."
        );
    }
}
