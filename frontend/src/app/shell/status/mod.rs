//! 状态栏子域（`#597` shell 域的第一步）。
//!
//! 底栏整条由这里组成：条本身（`bar`）、运行时取数（`fetch_state` / `tasks_wiring` /
//! `tasks_state`）、上下文用量芯片（`context`）、权限芯片（`permission_chip`）、
//! 会话模式选段（`session_mode_seg`）、角色菜单（`agent_role_menu`）、视图切换芯片（`view_chip`）。
//!
//! 迁移方式：从 `app/` 根级整批搬入，**只改路径不改逻辑**。两条只能靠编译发现的改动：
//! - 原 `super::<app 级模块>` 全部改成 `crate::app::…`（层级深了一级）；
//! - 原 `pub(super)`（= 可见于 `app`）改成 `pub(in crate::app)`，否则会被静默收紧到本目录。

pub(crate) mod agent_role_menu;
pub(crate) mod bar;
pub(crate) mod context;
pub(crate) mod fetch_state;
pub(crate) mod permission_chip;
pub(crate) mod session_mode_seg;
pub(crate) mod tasks_state;
pub(crate) mod tasks_wiring;
pub(crate) mod view_chip;
