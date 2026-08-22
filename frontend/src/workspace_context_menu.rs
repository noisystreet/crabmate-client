//! 工作区文件树右键菜单与行内新建（VS Code 风格：在树中直接输入名称）。

use std::collections::{HashMap, HashSet};

use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::confirm_dialog::confirm_user_message;
use crate::i18n::Locale;
use crate::ide_confirm::IdeConfirmSignals;
use crate::ide_tabs::{IdeTabsEditorSignals, IdeTabsHandle};

/// 工作区树右键菜单锚点（`position: fixed` 使用视口坐标）。
#[derive(Clone, Debug, PartialEq)]
pub struct WorkspaceContextAnchor {
    pub x: f64,
    pub y: f64,
    /// 右键目标相对路径；空白处为 `None`（表示工作区根）。
    pub target_rel: Option<String>,
    pub target_is_dir: bool,
    /// 新建项时的默认父目录（目录自身或其父路径；根为 `""`）。
    pub parent_rel: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceInlineCreateKind {
    File,
    Dir,
}

/// 行内新建：在 `parent_rel` 对应列表末尾显示输入框。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspacePendingCreate {
    pub parent_rel: String,
    pub kind: WorkspaceInlineCreateKind,
    /// 若为 `Some`，行内输入是重命名该文件（`kind` 忽略）。
    pub rename_from_rel: Option<String>,
}

#[derive(Clone, Copy)]
pub struct WorkspaceTreeChromeSignals {
    pub context_menu: RwSignal<Option<WorkspaceContextAnchor>>,
    pub pending_create: RwSignal<Option<WorkspacePendingCreate>>,
}

/// 删除成功后传入，用于清理子树缓存/展开状态。
#[derive(Clone, Debug)]
pub struct WorkspaceTreeRefreshHint {
    pub parent_rel: String,
    pub deleted_rel: Option<String>,
}

#[derive(Clone)]
pub struct WorkspaceContextMenuActions {
    pub refresh_after_mutation: std::sync::Arc<dyn Fn(WorkspaceTreeRefreshHint) + Send + Sync>,
    pub ide_tabs: Option<(IdeTabsHandle, IdeTabsEditorSignals)>,
    pub ide_confirm: Option<IdeConfirmSignals>,
    /// 当前会话绑定的服务端 `conversation_id`（可选；供 `POST /workspace/file/move` 写入变更集）。
    pub conversation_id: std::sync::Arc<dyn Fn() -> Option<String> + Send + Sync>,
}

impl WorkspaceContextMenuActions {
    #[must_use]
    pub fn bound_conversation_id(&self) -> Option<String> {
        (self.conversation_id)()
    }
}

/// 从聊天会话读取当前绑定的 `server_conversation_id`。
pub fn conversation_id_from_chat(
    chat: crate::chat_session_state::ChatSessionSignals,
) -> std::sync::Arc<dyn Fn() -> Option<String> + Send + Sync> {
    std::sync::Arc::new(move || {
        let aid = chat.active_id.get_untracked();
        chat.sessions
            .get_untracked()
            .into_iter()
            .find(|s| s.id == aid)
            .and_then(|s| s.trimmed_server_conversation_id().map(str::to_string))
    })
}

pub fn name_segment_valid(name: &str) -> bool {
    let n = name.trim();
    !n.is_empty()
        && !n
            .chars()
            .any(|c| c.is_whitespace() || c == '/' || c == '\\')
        && n != "."
        && n != ".."
}

/// 右键菜单选择「新建」后：展开父目录（若有）并进入行内命名。
pub fn begin_workspace_inline_create(
    parent_rel: String,
    kind: WorkspaceInlineCreateKind,
    chrome: WorkspaceTreeChromeSignals,
    subtree_expanded: RwSignal<HashSet<String>>,
    subtree_cache: RwSignal<HashMap<String, crate::api::WorkspaceData>>,
    subtree_loading: RwSignal<HashSet<String>>,
    locale: RwSignal<Locale>,
) {
    chrome.context_menu.set(None);
    if !parent_rel.is_empty() {
        crate::workspace_tree::ensure_workspace_dir_open(
            parent_rel.clone(),
            subtree_expanded,
            subtree_cache,
            subtree_loading,
            locale,
        );
    }
    chrome.pending_create.set(Some(WorkspacePendingCreate {
        parent_rel,
        kind,
        rename_from_rel: None,
    }));
}

/// 右键「重命名」：在原文件行位置进入行内改名（仅文件）。
pub fn begin_workspace_inline_rename(from_rel: String, chrome: WorkspaceTreeChromeSignals) {
    chrome.context_menu.set(None);
    let parent_rel = crate::workspace_tree::workspace_parent_rel(from_rel.as_str());
    chrome.pending_create.set(Some(WorkspacePendingCreate {
        parent_rel,
        kind: WorkspaceInlineCreateKind::File,
        rename_from_rel: Some(from_rel),
    }));
}

fn delete_confirm_message(locale: Locale, path: &str, is_dir: bool) -> String {
    if is_dir {
        crate::i18n::workspace_tree_delete_dir_confirm(locale, path, true).to_string()
    } else {
        crate::i18n::workspace_tree_delete_file_confirm(locale, path).to_string()
    }
}

fn workspace_ctx_has_target(menu: RwSignal<Option<WorkspaceContextAnchor>>) -> bool {
    menu.get().and_then(|a| a.target_rel).is_some()
}

fn workspace_ctx_target_is_file(menu: RwSignal<Option<WorkspaceContextAnchor>>) -> bool {
    menu.get()
        .is_some_and(|a| a.target_rel.is_some() && !a.target_is_dir)
}

fn workspace_ctx_target_is_dir_or_root(menu: RwSignal<Option<WorkspaceContextAnchor>>) -> bool {
    menu.get()
        .is_some_and(|a| a.target_is_dir || a.target_rel.is_none())
}

fn start_create_from_open_menu(
    menu: RwSignal<Option<WorkspaceContextAnchor>>,
    kind: WorkspaceInlineCreateKind,
    chrome: WorkspaceTreeChromeSignals,
    subtree_expanded: RwSignal<HashSet<String>>,
    subtree_cache: RwSignal<HashMap<String, crate::api::WorkspaceData>>,
    subtree_loading: RwSignal<HashSet<String>>,
    locale: RwSignal<Locale>,
) {
    let Some(anchor) = menu.get() else {
        return;
    };
    begin_workspace_inline_create(
        anchor.parent_rel,
        kind,
        chrome,
        subtree_expanded,
        subtree_cache,
        subtree_loading,
        locale,
    );
}

fn save_open_menu_entry_to_device(
    menu: RwSignal<Option<WorkspaceContextAnchor>>,
    locale: RwSignal<Locale>,
    workspace_err: RwSignal<Option<String>>,
    actions: StoredValue<WorkspaceContextMenuActions>,
) {
    let Some(anchor) = menu.get() else {
        return;
    };
    let rel = anchor.target_rel.clone().unwrap_or_default();
    let is_dir = anchor.target_is_dir || anchor.target_rel.is_none();
    menu.set(None);
    if is_dir {
        crate::workspace_fs_ops::spawn_save_workspace_dir_to_device(
            rel,
            locale,
            workspace_err,
            actions.get_value(),
        );
    } else {
        crate::workspace_fs_ops::spawn_save_workspace_file_to_device(
            rel,
            locale,
            workspace_err,
            actions.get_value(),
        );
    }
}

fn start_rename_from_open_menu(
    menu: RwSignal<Option<WorkspaceContextAnchor>>,
    chrome: WorkspaceTreeChromeSignals,
) {
    let Some(anchor) = menu.get() else {
        return;
    };
    if anchor.target_is_dir {
        return;
    }
    let Some(rel) = anchor.target_rel.clone() else {
        return;
    };
    begin_workspace_inline_rename(rel, chrome);
}

fn delete_item_label(menu: RwSignal<Option<WorkspaceContextAnchor>>, locale: Locale) -> String {
    match menu.get() {
        Some(a) if a.target_is_dir => {
            crate::i18n::workspace_tree_ctx_delete_dir(locale).to_string()
        }
        Some(_) => crate::i18n::workspace_tree_ctx_delete_file(locale).to_string(),
        None => String::new(),
    }
}

fn confirm_and_delete_from_menu(
    menu: RwSignal<Option<WorkspaceContextAnchor>>,
    locale: RwSignal<Locale>,
    workspace_err: RwSignal<Option<String>>,
    actions: StoredValue<WorkspaceContextMenuActions>,
) {
    let Some(anchor) = menu.get() else {
        return;
    };
    let Some(rel) = anchor.target_rel.clone() else {
        return;
    };
    menu.set(None);
    let loc = locale.get_untracked();
    let msg = delete_confirm_message(loc, rel.as_str(), anchor.target_is_dir);
    let is_dir = anchor.target_is_dir;
    let actions = actions.get_value();
    spawn_local(async move {
        if !confirm_user_message(
            &msg,
            crate::i18n::confirm_delete_ok(loc),
            crate::i18n::ide_confirm_cancel(loc),
        )
        .await
        {
            return;
        }
        crate::workspace_fs_ops::spawn_delete_workspace_entry(
            rel,
            is_dir,
            locale,
            workspace_err,
            actions,
        );
    });
}

#[derive(Clone, Copy)]
struct WorkspaceContextMenuBodyInput {
    workspace_context_menu: RwSignal<Option<WorkspaceContextAnchor>>,
    chrome: WorkspaceTreeChromeSignals,
    subtree_expanded: RwSignal<HashSet<String>>,
    subtree_cache: RwSignal<HashMap<String, crate::api::WorkspaceData>>,
    subtree_loading: RwSignal<HashSet<String>>,
    locale: RwSignal<Locale>,
    workspace_err: RwSignal<Option<String>>,
    actions: StoredValue<WorkspaceContextMenuActions>,
}

#[component]
fn WorkspaceContextMenuBody(input: WorkspaceContextMenuBodyInput) -> impl IntoView {
    let WorkspaceContextMenuBodyInput {
        workspace_context_menu,
        chrome,
        subtree_expanded,
        subtree_cache,
        subtree_loading,
        locale,
        workspace_err,
        actions,
    } = input;
    view! {
        <button
            type="button"
            class="session-ctx-item"
            role="menuitem"
            on:click=move |_| {
                start_create_from_open_menu(
                    workspace_context_menu,
                    WorkspaceInlineCreateKind::File,
                    chrome,
                    subtree_expanded,
                    subtree_cache,
                    subtree_loading,
                    locale,
                );
            }
        >
            {move || crate::i18n::workspace_tree_ctx_new_file(locale.get())}
        </button>
        <button
            type="button"
            class="session-ctx-item"
            role="menuitem"
            on:click=move |_| {
                start_create_from_open_menu(
                    workspace_context_menu,
                    WorkspaceInlineCreateKind::Dir,
                    chrome,
                    subtree_expanded,
                    subtree_cache,
                    subtree_loading,
                    locale,
                );
            }
        >
            {move || crate::i18n::workspace_tree_ctx_new_dir(locale.get())}
        </button>
        <Show when=move || {
            workspace_ctx_target_is_file(workspace_context_menu)
                || workspace_ctx_target_is_dir_or_root(workspace_context_menu)
        }>
            <button
                type="button"
                class="session-ctx-item"
                role="menuitem"
                on:click=move |_| {
                    save_open_menu_entry_to_device(
                        workspace_context_menu,
                        locale,
                        workspace_err,
                        actions,
                    );
                }
            >
                {move || crate::i18n::workspace_tree_ctx_save_to_device(locale.get())}
            </button>
        </Show>
        <Show when=move || workspace_ctx_target_is_file(workspace_context_menu)>
            <button
                type="button"
                class="session-ctx-item"
                role="menuitem"
                on:click=move |_| {
                    start_rename_from_open_menu(workspace_context_menu, chrome);
                }
            >
                {move || crate::i18n::workspace_tree_ctx_rename_file(locale.get())}
            </button>
        </Show>
        <Show when=move || workspace_ctx_has_target(workspace_context_menu)>
            <button
                type="button"
                class="session-ctx-item session-ctx-item-danger"
                role="menuitem"
                on:click=move |_| {
                    confirm_and_delete_from_menu(
                        workspace_context_menu,
                        locale,
                        workspace_err,
                        actions,
                    );
                }
            >
                {move || delete_item_label(workspace_context_menu, locale.get())}
            </button>
        </Show>
    }
}

#[component]
pub fn WorkspaceContextMenuLayer(
    workspace_context_menu: RwSignal<Option<WorkspaceContextAnchor>>,
    workspace_pending_create: RwSignal<Option<WorkspacePendingCreate>>,
    subtree_expanded: RwSignal<HashSet<String>>,
    subtree_cache: RwSignal<HashMap<String, crate::api::WorkspaceData>>,
    subtree_loading: RwSignal<HashSet<String>>,
    locale: RwSignal<Locale>,
    workspace_err: RwSignal<Option<String>>,
    actions: WorkspaceContextMenuActions,
) -> impl IntoView {
    let chrome = WorkspaceTreeChromeSignals {
        context_menu: workspace_context_menu,
        pending_create: workspace_pending_create,
    };
    let actions = StoredValue::new(actions);
    let menu_style = Memo::new(move |_| {
        workspace_context_menu
            .get()
            .map(|a| format!("left:{}px;top:{}px;", a.x, a.y))
            .unwrap_or_default()
    });
    let body = WorkspaceContextMenuBodyInput {
        workspace_context_menu,
        chrome,
        subtree_expanded,
        subtree_cache,
        subtree_loading,
        locale,
        workspace_err,
        actions,
    };
    view! {
        <Show when=move || workspace_context_menu.get().is_some()>
            <div class="session-ctx-layer workspace-ctx-layer">
                <div
                    class="session-ctx-backdrop"
                    aria-hidden="true"
                    on:click=move |_| workspace_context_menu.set(None)
                ></div>
                <crate::app::focusable_menu::FocusableRoleMenu
                    class="session-ctx-menu workspace-ctx-menu"
                    menu_style=menu_style
                >
                    <WorkspaceContextMenuBody input=body />
                </crate::app::focusable_menu::FocusableRoleMenu>
            </div>
        </Show>
    }
}
