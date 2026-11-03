//! 设置面板的「选项表 / 枚举下标」纯函数层（从 `settings_panel.rs` 拆出以压住单
//! 文件行数门禁）。
//!
//! 这里只放**不依赖** [`SettingsPanel`](super::SettingsPanel) 状态的定义：枚举选项表、
//! 网关预设表与下标换算。父模块以 `pub(crate) use self::schema::*;`
//! 重导出，因此 `draw` / `app` / 各测试子模块的 `use super::*` 仍能拿到这些名字。
//! （`Editing` 分派类逻辑留在父模块——它是私有的状态机类型。）

use super::super::palette::THEME_OPTIONS;
use super::super::settings::{
    SESSION_MODES, STORE_OFF, STORE_ON, THINKING_MODES, TOOL_CACHE_DISABLED,
};

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

/// 网关预设表与「URL → 下标」投影与 Web 共用
/// （`crabmate_client_api::client_llm_presets`）。
/// TUI 只回写 `api_base`，不按预设自动填建议模型名（模型名仍保持「空 = 跟随 server」）。
/// `index_for_url` 承担「空→首位（server）、未知→末位（custom）」语义，
/// 该不变量由共享层单测钉住。
pub(crate) use crabmate_client_api::client_llm_presets::{
    CUSTOM_ID as GATEWAY_CUSTOM_ID, PRESETS as API_BASE_PRESETS, SERVER_ID as GATEWAY_SERVER_ID,
    index_for_url as gateway_pick_of,
};

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

/// 会话存储（SQLite）编辑选项长度：开 / 关（没有「跟随 server」这一档）。
pub(crate) const STORE_OPTIONS_LEN: usize = 2;

/// 会话存储编辑选项（0 = 开，1 = 关）。
pub(crate) fn store_options() -> [Option<&'static str>; STORE_OPTIONS_LEN] {
    [Some(STORE_ON), Some(STORE_OFF)]
}

/// 会话存储选项下标 → 值（`off` → 1 = 关；`on` / 未知 / `None` → 0 = 开）。
///
/// `None` 只可能来自「还没读到 `/status`」；那种情况面板根本不让进编辑态
/// （`PanelCtx::store_toggle_supported` 先拦），所以这里的回落不影响保存语义。
pub(crate) fn store_index_of(v: Option<&str>) -> usize {
    usize::from(v.map(str::trim) == Some(STORE_OFF))
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

/// 面板分区：外观 / 模型 / 会话（扁平字段，写 serve user-data）。
///
/// 顺序与 Desktop 设置页导航一致（外观在模型之前），`ALL` 的下标必须与枚举判别式
/// 对齐——[`SettingsPanel::cycle_section`] 用 `section as usize` 索引。
///
/// 放在本文件而不是父模块：与「分区 → 字段」表同源，且父模块要压 920 行门禁。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Section {
    Appearance,
    Llm,
    Session,
}

impl Section {
    pub(crate) const ALL: [Section; 3] = [Section::Appearance, Section::Llm, Section::Session];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Section::Appearance => "外观",
            Section::Llm => "模型",
            Section::Session => "会话",
        }
    }

    pub(crate) const fn fields(self) -> &'static [FieldId] {
        match self {
            Section::Appearance => &[FieldId::Theme],
            Section::Llm => &[
                FieldId::Model,
                FieldId::ApiBase,
                FieldId::Temperature,
                FieldId::ContextTokens,
                FieldId::ThinkingMode,
                FieldId::ApiKey,
            ],
            Section::Session => &[
                FieldId::Role,
                FieldId::SessionMode,
                FieldId::ToolCache,
                FieldId::SessionSqlite,
            ],
        }
    }
}

/// 面板管理的字段：外观 1 个 + 模型 6 个 + 会话 3 个。`Temperature/ThinkingMode/
/// ContextTokens` 写 user-data `client_llm.*`；`ApiKey` 只写本机钥匙串（见 LaneApp
/// 接线）；`ToolCache` 写 prefs `disable_readonly_tool_ttl_cache`；`Theme` 写 prefs
/// `theme`（与 Desktop 同一 slug 表，保存后立即重新解析配色）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FieldId {
    Theme,
    Model,
    ApiBase,
    Temperature,
    ContextTokens,
    ThinkingMode,
    ApiKey,
    Role,
    SessionMode,
    ToolCache,
    SessionSqlite,
}

pub(crate) const fn field_label(field: FieldId) -> &'static str {
    match field {
        FieldId::Theme => "主题",
        FieldId::Model => "模型名",
        FieldId::ApiBase => "API Base",
        FieldId::Temperature => "温度",
        FieldId::ContextTokens => "上下文长度",
        FieldId::ThinkingMode => "思考模式",
        FieldId::ApiKey => "API 密钥",
        FieldId::Role => "Agent role",
        FieldId::SessionMode => "会话模式",
        FieldId::ToolCache => "只读工具缓存",
        FieldId::SessionSqlite => "会话存储（SQLite）",
    }
}
