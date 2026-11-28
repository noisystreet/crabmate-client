//! IDE 域（`#597` 域目录迁移的第四个域）：编辑器列、标签栏、查找条、顶栏菜单、IDE 设置页。
//!
//! 目录约定：
//! - 本层平铺的 8 个文件是编辑器列本身与它的附属：`editor_body` / `editor_pane` / `tabs_bar` /
//!   `find_bar` / `layout_switch` / `new_file_modal` / `confirm_dialog` / `session`；
//! - `menu_bar/`：IDE 顶栏菜单（文件 / 编辑 / 视图三组 + 菜单 id 与 props）；
//! - `settings_page/`：IDE 设置页（分区视图 / 草稿 / 三个 section 块）。
//!
//! 迁移方式：从 `app/` 根级整批搬入，**只改路径不改逻辑**。两条跨层改动：
//! - 原 `super::<app 级模块>` → `crate::app::…`（`ide/session.rs` 里的 `workspace::panel_state` 一处）；
//! - `app/mod.rs` 里 `use ide_layout_switch::…` → `use ide::layout_switch::…`（裸 `use`，不匹配路径模式）。
//!
//! `pub(super)` 全部位于 `menu_bar/` 与 `settings_page/` 的**子文件**里，两个目录整体搬家后父模块未变，
//! 因此语义不变、未做改动（与 PR-4 的 `nav_rail_*` / `clone_modal` 同一情形）。

pub(crate) mod confirm_dialog;
pub(crate) mod editor_body;
pub(crate) mod editor_pane;
pub(crate) mod find_bar;
pub(crate) mod layout_switch;
pub(crate) mod menu_bar;
pub(crate) mod new_file_modal;
pub(crate) mod session;
pub(crate) mod settings_page;
pub(crate) mod tabs_bar;
