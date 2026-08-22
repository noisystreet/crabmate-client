use super::Locale;

// --- 工作区树 ---

pub fn workspace_tree_no_data(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "（无数据）",
        Locale::En => "(No data)",
    }
}

pub fn workspace_tree_aria(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "工作区文件",
        Locale::En => "Workspace files",
    }
}

pub fn workspace_tree_file_row_aria(l: Locale, name: &str) -> String {
    match l {
        Locale::ZhHans => format!("打开文件 {name}"),
        Locale::En => format!("Open file {name}"),
    }
}

pub fn workspace_tree_toggle_dir_title(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "展开或折叠子目录",
        Locale::En => "Expand or collapse subdirectory",
    }
}

pub fn workspace_tree_expand_folder(l: Locale, name: &str) -> String {
    match l {
        Locale::ZhHans => format!("展开子文件夹 {name}"),
        Locale::En => format!("Expand subfolder {name}"),
    }
}

pub fn workspace_tree_collapse_folder(l: Locale, name: &str) -> String {
    match l {
        Locale::ZhHans => format!("折叠子文件夹 {name}"),
        Locale::En => format!("Collapse subfolder {name}"),
    }
}

pub fn workspace_tree_ctx_new_file(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "新建文件…",
        Locale::En => "New file…",
    }
}

pub fn workspace_tree_ctx_new_dir(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "新建文件夹…",
        Locale::En => "New folder…",
    }
}

pub fn workspace_tree_ctx_save_to_device(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "保存到本机…",
        Locale::En => "Save to this device…",
    }
}

pub fn workspace_tree_ctx_rename_file(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "重命名…",
        Locale::En => "Rename…",
    }
}

pub fn workspace_tree_ctx_delete_file(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "删除文件",
        Locale::En => "Delete file",
    }
}

pub fn workspace_tree_ctx_delete_dir(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "删除文件夹",
        Locale::En => "Delete folder",
    }
}

pub fn workspace_tree_delete_file_confirm(l: Locale, path: &str) -> String {
    match l {
        Locale::ZhHans => format!("确定删除文件 {path}？此操作不可恢复。"),
        Locale::En => format!("Delete file {path}? This cannot be undone."),
    }
}

pub fn workspace_tree_delete_dir_confirm(l: Locale, path: &str, recursive: bool) -> String {
    match l {
        Locale::ZhHans if recursive => {
            format!("确定递归删除文件夹 {path} 及其全部内容？此操作不可恢复。")
        }
        Locale::ZhHans => format!("确定删除空文件夹 {path}？"),
        Locale::En if recursive => {
            format!("Recursively delete folder {path} and all contents? This cannot be undone.")
        }
        Locale::En => format!("Delete empty folder {path}?"),
    }
}

pub fn workspace_tree_name_invalid(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "名称无效：不能包含 /、\\ 或空白",
        Locale::En => "Invalid name: must not contain /, \\, or whitespace",
    }
}

pub fn workspace_tree_inline_name_ph_file(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "文件名",
        Locale::En => "File name",
    }
}

pub fn workspace_tree_inline_name_ph_dir(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "文件夹名",
        Locale::En => "Folder name",
    }
}

pub fn workspace_upload_ok(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "上传",
        Locale::En => "Upload",
    }
}

pub fn workspace_upload_overwrite_ok(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "覆盖",
        Locale::En => "Overwrite",
    }
}

pub fn workspace_upload_confirm(l: Locale, dest: &str, names: &str, n: usize) -> String {
    let dest = if dest.is_empty() { "." } else { dest };
    match l {
        Locale::ZhHans => format!("将 {n} 个文件（{names}）上传到 {dest}？"),
        Locale::En => format!("Upload {n} file(s) ({names}) to {dest}?"),
    }
}

pub fn workspace_upload_overwrite_confirm(l: Locale, rel: &str) -> String {
    match l {
        Locale::ZhHans => format!("{rel} 已存在，要覆盖吗？"),
        Locale::En => format!("{rel} already exists. Overwrite?"),
    }
}

pub fn workspace_tree_rename_overwrite_ok(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "覆盖",
        Locale::En => "Overwrite",
    }
}

pub fn workspace_tree_rename_overwrite_confirm(l: Locale, rel: &str) -> String {
    match l {
        Locale::ZhHans => format!("{rel} 已存在，要覆盖吗？"),
        Locale::En => format!("{rel} already exists. Overwrite?"),
    }
}

pub fn workspace_tree_rename_overwrite_dirty_confirm(l: Locale, rel: &str) -> String {
    match l {
        Locale::ZhHans => {
            format!("{rel} 已存在，且编辑器中有未保存的修改；覆盖后这些修改会丢失。要覆盖吗？")
        }
        Locale::En => format!(
            "{rel} already exists and has unsaved editor changes. Overwrite will discard those changes. Continue?"
        ),
    }
}

pub fn workspace_upload_overwrite_cancelled(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "已取消覆盖，其余文件未上传",
        Locale::En => "Overwrite cancelled; remaining files were not uploaded",
    }
}

pub fn workspace_upload_too_many(l: Locale, max: usize) -> String {
    match l {
        Locale::ZhHans => format!("一次最多上传 {max} 个文件"),
        Locale::En => format!("At most {max} files per drop"),
    }
}

pub fn workspace_upload_too_large(l: Locale, name: &str) -> String {
    match l {
        Locale::ZhHans => format!("{name} 超过 16 MiB，无法上传"),
        Locale::En => format!("{name} is over 16 MiB and cannot be uploaded"),
    }
}

pub fn workspace_save_too_large(l: Locale, name: &str) -> String {
    match l {
        Locale::ZhHans => format!("{name} 超过 16 MiB，无法保存到本机"),
        Locale::En => format!("{name} is over 16 MiB and cannot be saved to this device"),
    }
}
