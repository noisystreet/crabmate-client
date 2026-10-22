//! 设置面板的「选项表 / 枚举下标」纯函数层（从 `settings_panel.rs` 拆出以压住单
//! 文件行数门禁）。
//!
//! 这里只放**不依赖** [`SettingsPanel`](super::SettingsPanel) 状态的定义：枚举选项表、
//! 网关预设表与下标换算。父模块以 `pub(crate) use self::schema::*;`
//! 重导出，因此 `draw` / `app` / 各测试子模块的 `use super::*` 仍能拿到这些名字。
//! （`Editing` 分派类逻辑留在父模块——它是私有的状态机类型。）

use super::super::palette::THEME_OPTIONS;
use super::super::settings::{SESSION_MODES, THINKING_MODES, TOOL_CACHE_DISABLED};

/// 会话模式编辑选项长度 =「跟随 server」+ [`SESSION_MODES`]。
pub(crate) const MODE_OPTIONS_LEN: usize = SESSION_MODES.len() + 1;

/// 思考模式编辑选项长度 =「跟随 server」+ [`THINKING_MODES`]。
pub(crate) const THINK_OPTIONS_LEN: usize = THINKING_MODES.len() + 1;

/// 会话模式编辑选项（`None` = 跟随 server，即清除存储键）；取值以 [`SESSION_MODES`] 为单一来源。
pub(crate) fn mode_options() -> [Option<&'static str>; MODE_OPTIONS_LEN] {
    let mut out = [None; MODE_OPTIONS_LEN];
    for (i, m) in SESSION_MODES.iter().copied().enumerate() {
        out[i + 1] = Some(m);
    }
    out
}

/// 思考模式编辑选项（`None` = 跟随 server，即清除存储键）；取值以 [`THINKING_MODES`] 为单一来源。
pub(crate) fn think_options() -> [Option<&'static str>; THINK_OPTIONS_LEN] {
    let mut out = [None; THINK_OPTIONS_LEN];
    for (i, m) in THINKING_MODES.iter().copied().enumerate() {
        out[i + 1] = Some(m);
    }
    out
}

/// 只读工具缓存编辑选项长度 = 开（跟随 server） + 关（禁用）。
pub(crate) const TOOL_OPTIONS_LEN: usize = 2;

/// 只读工具缓存编辑选项（`None` = 开/跟随 server，即清除 prefs 键；`Some("off")` = 关/禁用）。
pub(crate) fn tool_options() -> [Option<&'static str>; TOOL_OPTIONS_LEN] {
    [None, Some(TOOL_CACHE_DISABLED)]
}

/// 网关预设（与 `frontend/src/client_llm_presets.rs` 的 `CLIENT_LLM_API_BASE_PRESETS`
/// **同源拷贝**；改 URL / 增删条目需两仓同步，避免漂移）。
/// TUI 只回写 `api_base`，不按预设自动填建议模型名（模型名仍保持「空 = 跟随 server」）。
#[derive(Clone, Copy)]
pub(crate) struct ApiPreset {
    pub(crate) id: &'static str,
    pub(crate) url: &'static str,
}

pub(crate) const API_BASE_PRESETS: &[ApiPreset] = &[
    ApiPreset {
        id: "server",
        url: "",
    },
    ApiPreset {
        id: "ollama",
        url: "http://127.0.0.1:11434/v1",
    },
    ApiPreset {
        id: "deepseek",
        url: "https://api.deepseek.com/v1",
    },
    ApiPreset {
        id: "minimax",
        url: "https://api.minimaxi.com/v1",
    },
    ApiPreset {
        id: "zhipu",
        url: "https://open.bigmodel.cn/api/paas/v4",
    },
    ApiPreset {
        id: "moonshot",
        url: "https://api.moonshot.cn/v1",
    },
    ApiPreset {
        id: "custom",
        url: "",
    },
];

pub(crate) const GATEWAY_SERVER_ID: &str = "server";
pub(crate) const GATEWAY_CUSTOM_ID: &str = "custom";

/// 网关预设选项下标 → 值（对应 [`API_BASE_PRESETS`]；`server` 是首位，`custom` 是末位）。
pub(crate) fn gateway_pick_of(value: Option<&str>) -> usize {
    match value.map(str::trim).filter(|s| !s.is_empty()) {
        None => 0,
        Some(v) => API_BASE_PRESETS
            .iter()
            .position(|p| p.id != GATEWAY_SERVER_ID && p.id != GATEWAY_CUSTOM_ID && p.url == v)
            .unwrap_or(API_BASE_PRESETS.len() - 1),
    }
}

/// ←/→ 移动循环类编辑的游标推进。`Editing` 是父模块的私有状态机类型，故该函数
/// 的"选择哪个选项表"分派留在父模块（`SettingsPanel::edit_move`），这里只做下标算术。
pub(crate) fn move_pick(pick: &mut usize, left: bool, len: usize) {
    if left {
        *pick = pick.saturating_sub(1);
    } else if *pick + 1 < len {
        *pick += 1;
    }
}

/// 会话模式选项下标 → 值（按 [`SESSION_MODES`] 顺序；其余（含 None）回 0 = 跟随 server）。
pub(crate) fn mode_index_of(v: Option<&str>) -> usize {
    v.and_then(|m| SESSION_MODES.iter().position(|x| *x == m))
        .map_or(0, |i| i + 1)
}

/// 思考模式选项下标 → 值（按 [`THINKING_MODES`] 顺序；其余（含 None）回 0 = server/跟随）。
pub(crate) fn think_index_of(v: Option<&str>) -> usize {
    v.and_then(|m| THINKING_MODES.iter().position(|x| *x == m))
        .map_or(0, |i| i + 1)
}

/// 只读工具缓存选项下标 → 值（`off` → 1 = 禁用；其它（含 None）回 0 = 开/跟随 server）。
pub(crate) fn tool_index_of(v: Option<&str>) -> usize {
    usize::from(v == Some(TOOL_CACHE_DISABLED))
}

/// 主题选项下标 → 值（按 [`THEME_OPTIONS`] 顺序；未知 slug（含 `system`、
/// Desktop 侧新增主题）回 0 =「默认」——只影响编辑起点，读取路径不改写持久层）。
pub(crate) fn theme_index_of(v: Option<&str>) -> usize {
    let Some(v) = v.map(str::trim).filter(|s| !s.is_empty()) else {
        return 0;
    };
    THEME_OPTIONS
        .iter()
        .position(|opt| *opt == Some(v))
        .unwrap_or(0)
}
