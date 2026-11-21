//! **连接级**状态（#561 的第一刀）。
//!
//! 与 [`super::ShellUISignals`]（窗口级：locale / 主题 / 面板宽度 / 布局开关）刻意分开：
//! 这里放的每一项都描述「**这一屏连的是哪台 `serve`**」，切 lane 时**必须**跟着 lane 变，
//! 而不是跟着窗口变。
//!
//! 现在只有 `api_base` 一项。单 lane 下它与留在窗口级聚合里没有任何行为差异 —— 这一步
//! 只是把归属写进类型里，让「每 lane 一份」时有地方放，而不是到时候再从 `ShellUISignals`
//! 里往外挑。
//!
//! **判据**（加新字段前先过一遍）：切到另一条 lane 时，这个值该不该变？
//! 该变 → 放这里；不该变（用户偏好 / 设备相关）→ 留在 `ShellUISignals`。

use leptos::prelude::*;

use crate::api::connection_scope;

#[derive(Clone, Copy)]
pub struct ConnSignals {
    /// 当前这条 lane 连的 `serve` 基址（`api_base_url()` 的**响应式镜像**）。
    ///
    /// 基址的唯一真源是 `api::connection_scope` 的 `ScopeHolder`（所有请求都读那里）；
    /// 这里只是让界面能在「设置 → 连接」改完基址后立刻换字。写入面因此只有一处：
    /// `save_api_base_click`（`settings_sections.rs`）。启动交接（URL hash）**不需要**额外
    /// 写 —— `bootstrap_app_shell` 在 `AppSignals::new` 之前就消费完它，所以 `new()` 里读到
    /// 的 scope 已经是终值，不必先空后闪。
    pub api_base: RwSignal<String>,
}

impl ConnSignals {
    /// `scope` 是这条 lane 的连接作用域（#558 项 1b）：基址的真源在它里面，这里只做
    /// 响应式镜像。
    ///
    /// 前台那一份仍走 [`crate::api::api_base_url`]（含一次性 `localStorage` 水合），
    /// 与改动前**完全等价**；后台 lane 取自己 holder 的值 —— 未水合时为空，由它自己
    /// 的启动路径填（跨 lane 顺手读别人的盘就会串台）。
    pub fn new(scope: &connection_scope::ScopeHolder) -> Self {
        let api_base = if connection_scope::is_active(scope) {
            crate::api::api_base_url()
        } else {
            scope.api_base()
        };
        Self {
            api_base: RwSignal::new(api_base),
        }
    }
}

impl Default for ConnSignals {
    fn default() -> Self {
        Self::new(&connection_scope::active_holder())
    }
}
