//! 工作区树接受本机文件拖放：目标路径与校验（无 DOM / HTTP）。

use leptos::prelude::*;

use crate::workspace_context_menu::name_segment_valid;
use crate::workspace_tree::{CRABMATE_WS_REL_MIME, workspace_child_rel};

/// 与 Server `WORKSPACE_FILE_WRITE_MAX_BYTES` 对齐（16 MiB）。
pub(crate) const WORKSPACE_UPLOAD_MAX_BYTES: u64 = 16 * 1024 * 1024;
pub(crate) const WORKSPACE_UPLOAD_MAX_FILES: usize = 20;

/// `dragover` 是否应按本机文件上传来 `preventDefault`（排除树内 `@rel` 拖拽）。
#[must_use]
pub(crate) fn workspace_accept_os_file_drag(types: &[String]) -> bool {
    let has_files = types.iter().any(|t| t == "Files");
    let internal = types.iter().any(|t| t == CRABMATE_WS_REL_MIME);
    has_files && !internal
}

/// 将目标目录与文件名（或带 `/` 的相对路径）拼成工作区相对路径。
pub(crate) fn workspace_upload_rel(dest_dir: &str, name_or_nested: &str) -> Option<String> {
    let nested = name_or_nested
        .trim()
        .trim_start_matches('/')
        .replace('\\', "/");
    if nested.is_empty() {
        return None;
    }
    let mut acc = dest_dir.trim().trim_start_matches('/').replace('\\', "/");
    if acc == "." {
        acc.clear();
    }
    for part in nested.split('/') {
        if !name_segment_valid(part) {
            return None;
        }
        acc = workspace_child_rel(acc.as_str(), part);
    }
    if acc.is_empty() { None } else { Some(acc) }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorkspaceUploadPlanItem {
    pub rel: String,
    pub name: String,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum WorkspaceUploadPlanError {
    Empty,
    TooMany,
    TooLarge { name: String },
    BadName { name: String },
}

/// 根据目标目录与（文件名, 可选嵌套相对路径, 字节数）生成上传计划。
pub(crate) fn plan_workspace_uploads(
    dest_dir: &str,
    files: &[(String, String, u64)],
) -> Result<Vec<WorkspaceUploadPlanItem>, WorkspaceUploadPlanError> {
    if files.is_empty() {
        return Err(WorkspaceUploadPlanError::Empty);
    }
    if files.len() > WORKSPACE_UPLOAD_MAX_FILES {
        return Err(WorkspaceUploadPlanError::TooMany);
    }
    let mut out = Vec::with_capacity(files.len());
    for (name, nested, size) in files {
        if *size > WORKSPACE_UPLOAD_MAX_BYTES {
            return Err(WorkspaceUploadPlanError::TooLarge { name: name.clone() });
        }
        let pathish = if nested.trim().is_empty() {
            name.as_str()
        } else {
            nested.as_str()
        };
        let Some(rel) = workspace_upload_rel(dest_dir, pathish) else {
            return Err(WorkspaceUploadPlanError::BadName { name: name.clone() });
        };
        out.push(WorkspaceUploadPlanItem {
            rel,
            name: name.clone(),
            size: *size,
        });
    }
    Ok(out)
}

pub(crate) fn workspace_upload_confirm_names(names: &[String], max_list: usize) -> String {
    if names.len() <= max_list {
        return names.join("、");
    }
    let head = names[..max_list].join("、");
    format!("{head}…")
}

/// 在拖放目标上维护进入深度，避免子节点 `dragleave` 误关高亮。
#[derive(Clone, Copy)]
pub(crate) struct WorkspaceDropHighlight {
    pub depth: RwSignal<u32>,
}

impl WorkspaceDropHighlight {
    #[must_use]
    pub fn new() -> Self {
        Self {
            depth: RwSignal::new(0),
        }
    }

    pub fn bump(&self) {
        self.depth.update(|d| *d = d.saturating_add(1));
    }

    pub fn drop_one(&self) {
        self.depth.update(|d| *d = d.saturating_sub(1));
    }

    pub fn clear(&self) {
        self.depth.set(0);
    }

    #[must_use]
    pub fn is_active(&self) -> bool {
        self.depth.get() > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accept_os_files_not_internal_tree_drag() {
        assert!(workspace_accept_os_file_drag(&["Files".into()]));
        assert!(!workspace_accept_os_file_drag(&[
            "Files".into(),
            CRABMATE_WS_REL_MIME.into()
        ]));
        assert!(!workspace_accept_os_file_drag(&["text/plain".into()]));
    }

    #[test]
    fn upload_rel_joins_dest_and_name() {
        assert_eq!(
            workspace_upload_rel("src", "a.rs").as_deref(),
            Some("src/a.rs")
        );
        assert_eq!(workspace_upload_rel("", "a.rs").as_deref(), Some("a.rs"));
        assert_eq!(
            workspace_upload_rel("src", "nested/a.bin").as_deref(),
            Some("src/nested/a.bin")
        );
        assert!(workspace_upload_rel("src", "../x").is_none());
        assert!(workspace_upload_rel("src", "a b.rs").is_none());
        assert_eq!(
            workspace_upload_rel("", "foo..bar.bin").as_deref(),
            Some("foo..bar.bin")
        );
    }

    #[test]
    fn plan_rejects_too_many_and_too_large() {
        let many: Vec<_> = (0..21)
            .map(|i| (format!("f{i}"), String::new(), 1u64))
            .collect();
        assert_eq!(
            plan_workspace_uploads("", &many),
            Err(WorkspaceUploadPlanError::TooMany)
        );
        assert_eq!(
            plan_workspace_uploads(
                "",
                &[(
                    "big.bin".into(),
                    String::new(),
                    WORKSPACE_UPLOAD_MAX_BYTES + 1
                )]
            ),
            Err(WorkspaceUploadPlanError::TooLarge {
                name: "big.bin".into()
            })
        );
        let ok = plan_workspace_uploads("", &[("a.bin".into(), String::new(), 3)]).unwrap();
        assert_eq!(ok[0].rel, "a.bin");
    }
}
