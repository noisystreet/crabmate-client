//! 工作区域（`#597` 域目录迁移的第二个域）。
//!
//! 目录约定：本目录只放工作区域相关的视图与接线，按子域再分——
//! - `panel.rs`：侧栏工作区面板（树、刷新、插入路径到合成器）；
//! - `panel_state.rs`：面板信号（`WorkspacePanelSignals`，被 `app_signals` 与 crate 根的
//!   `session_workspace_bind` 共用，故作 `pub(crate)`）；
//! - `root_actions.rs`：工作区根路径的挑选 / 切换 / 顶栏只读标题；
//! - `project_modal*`：项目池弹窗（视图 / 主体 / 子部件，按行数余量拆出的三片）；
//! - `browser_pick_modal.rs`：浏览器式目录挑选弹窗；
//! - `clone_modal/`：克隆 / 新建工作区的弹窗（含 `parts`）。
//!
//! **命名注意**：`workspace_*` 这个前缀在 crate 根还有一整族无关模块（`workspace_tree` /
//! `workspace_shell` / `workspace_context_menu` / `workspace_fs_ops` / `workspace_file_drop*`、
//! `api::http_workspace_*`、`i18n::workspace_*`）——它们不在本目录，也不随本次迁移移动。
//! 另外 `workspace_panel` / `to_workspace_panel()` / `reload_workspace_panel()` 是**字段与方法名**，
//! 与模块路径无关，不要一起改。
//!
//! 迁移方式：从 `app/` 根级整批搬入时只改路径、不改逻辑（`app/mod.rs` 的 8 条 `mod` 收敛成
//! 一条 `pub(crate) mod workspace;`）；体积预算见 `scripts/layout_budget.txt` 的
//! `frontend/src/app/workspace` 条目。

pub(crate) mod browser_pick_modal;
pub(crate) mod clone_modal;
pub(crate) mod panel;
pub(crate) mod panel_state;
pub(crate) mod project_modal;
pub(crate) mod project_modal_body;
pub(crate) mod project_modal_parts;
pub(crate) mod root_actions;
