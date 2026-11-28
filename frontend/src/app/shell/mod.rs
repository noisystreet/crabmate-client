//! Shell 域（`#597` 域目录迁移的第三个域）：底栏、左侧连接栏、侧栏工作区列、壳级接线。
//!
//! 目录约定：
//! - `status/`：状态栏（取数、芯片、模式选段、任务指示）；
//! - `rail/`：左侧连接栏（连接行、ctl 重启、lane 组、其它窗口、authority 去重）；
//! - `side_column/`：侧栏工作区列（列本体、尺寸工具条、滚动容器）；
//! - 本层平铺的 4 个文件是壳级接线：`bootstrap` / `ctx` / `init` / `wire_phases`。
//!
//! **未纳入本域**：`app_shell_effects/`（已是独立目录，自带预算；若要并入应是另一件事），
//! 以及 `app_signals/`（信号定义，lane_set / owner-dispose 高风险区）。
//!
//! 迁移方式：只搬不改逻辑。跨层改动的两类（`super::<app 级模块>` → `crate::app::…`、
//! `pub(super)` → `pub(crate)`）在各子域 `mod.rs` 里也各记了一遍。

pub(crate) mod bootstrap;
pub(crate) mod confirm_dialog;
pub(crate) mod ctx;
pub(crate) mod init;
pub(crate) mod mobile_header;
pub(crate) mod prefs_storage;
pub(crate) mod rail;
pub(crate) mod runtime_context;
pub(crate) mod side_column;
pub(crate) mod status;
pub(crate) mod wire_phases;
