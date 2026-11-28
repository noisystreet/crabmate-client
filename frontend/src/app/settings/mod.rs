//! 设置域（`#597` 域目录迁移的第一个域）。
//!
//! 目录约定：本目录只放设置相关的规则与视图接线，按子域再分——
//! - `page/`：设置页外壳（视图、顶栏、导航与 hash 路由、布局、脏标记快照）；
//! - `models_registry/`：模型注册表（预设列表 / 提交 / 持久化 / 删除确认）；
//! - 其余是各分区与共用小件：GitHub / MCP（含工具列表与探测）/ 权限 / 选段 / 开关 / 提交校验 / 关页守卫。
//!
//! 迁移方式：从 `app/` 根级整批搬入时**只改路径、不改逻辑**（`app/mod.rs` 的 16 条 `mod` 收敛成
//! 一条 `mod settings;`），所以每个文件的内部实现与单测保持原样；体积预算见
//! `scripts/layout_budget.txt` 的 `frontend/src/app/settings` 条目。
//!
//! 可见性沿用搬入前的实际可达范围：`settings` 本身对 `app` 私有，子模块 `pub(crate)` 供
//! `app/` 内的兄弟模块引用（原先它们也是「`app` 私有、`app` 子树可达」）。

pub(crate) mod close_guard;
pub(crate) mod commit;
pub(crate) mod form_state;
pub(crate) mod github_block;
pub(crate) mod mcp_block;
pub(crate) mod mcp_block_toolbar;
pub(crate) mod mcp_json_import;
pub(crate) mod mcp_server_row;
pub(crate) mod mcp_server_row_actions;
pub(crate) mod mcp_status;
pub(crate) mod mcp_tools_list;
pub(crate) mod models_registry;
pub(crate) mod page;
pub(crate) mod permission_block;
pub(crate) mod sections;
pub(crate) mod toggle_switch;
