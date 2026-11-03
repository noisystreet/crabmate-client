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

/// 二值开关（开 / 关）的编辑选项：会话存储与 MCP 全局开关共用这一份词表。
///
/// 没有 `None` 档：这两处后端都只有 on/off，不存在「跟随 server」。
pub(crate) fn toggle_options() -> [Option<&'static str>; 2] {
    [Some(STORE_ON), Some(STORE_OFF)]
}

/// 二值开关选项下标 → 值（`off` → 1 = 关；`on` / 未知 / `None` → 0 = 开）。
///
/// `None` 只可能来自「还没读到服务端当前值」；那种情况面板根本不让进编辑态，
/// 所以这里的回落不影响保存语义。
pub(crate) fn toggle_index_of(v: Option<&str>) -> usize {
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

/// 面板分区：外观 / 模型 / 会话 / MCP / 连接 / 快捷键 / 本地行为。
///
/// 前四栏可写（写 serve user-data 或本机钥匙串）；后三栏整栏**只读**，值本身就是
/// 说明文案。`ALL` 的下标必须与枚举判别式对齐——[`SettingsPanel::cycle_section`]
/// 用 `section as usize` 索引。
///
/// 放在本文件而不是父模块：与「分区 → 字段」表同源，且父模块要压 920 行门禁。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Section {
    Appearance,
    Llm,
    Session,
    Mcp,
    /// 只读：serve 侧默认模型 / 生效权限档。
    Connection,
    /// 只读：全局 / 面板 / 编辑器快捷键。
    Shortcuts,
    /// 只读：密钥存储、保存语义、Markdown 预览这些「行为事实」。
    Local,
}

impl Section {
    pub(crate) const ALL: [Section; 7] = [
        Section::Appearance,
        Section::Llm,
        Section::Session,
        Section::Mcp,
        Section::Connection,
        Section::Shortcuts,
        Section::Local,
    ];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Section::Appearance => "外观",
            Section::Llm => "模型",
            Section::Session => "会话",
            Section::Mcp => "MCP",
            Section::Connection => "连接",
            Section::Shortcuts => "快捷键",
            Section::Local => "本地行为",
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
            // 只有两个标量可改；`McpServers` 是**只读**信息行（清单编辑留给 Desktop/Web）。
            Section::Mcp => &[
                FieldId::McpGlobalEnabled,
                FieldId::McpToolTimeout,
                FieldId::McpServers,
            ],
            Section::Connection => &[
                FieldId::ConnLane,
                FieldId::ConnApiBase,
                FieldId::ConnBearer,
                FieldId::ConnModel,
                FieldId::ConnPermission,
            ],
            Section::Shortcuts => &[
                FieldId::KeyGlobal,
                FieldId::KeyChat,
                FieldId::KeySettings,
                FieldId::KeyEditor,
            ],
            // 前三条是行为事实（密钥落点 / 保存语义 / Markdown 预览）；其后是 `CM_TUI_*`
            // 环境开关的**当前生效值**（顺序跟 `settings_panel_env::EnvSwitch::ALL`）。
            Section::Local => &[
                FieldId::LocalSecret,
                FieldId::LocalSave,
                FieldId::LocalMarkdown,
                FieldId::EnvMouse,
                FieldId::EnvPaste,
                FieldId::EnvClipboard,
                FieldId::EnvLoop,
                FieldId::EnvColor,
                FieldId::EnvImage,
                FieldId::EnvMotion,
                FieldId::EnvTilde,
            ],
        }
    }
}

/// 整栏只读的分区（连接 / 快捷键 / 本地行为）：没有可编辑字段。
pub(crate) const fn is_readonly_section(section: Section) -> bool {
    matches!(
        section,
        Section::Connection | Section::Shortcuts | Section::Local
    )
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
    McpGlobalEnabled,
    McpToolTimeout,
    /// 只读信息行：服务器数量 / 启用数。清单本身留给 Desktop/Web 设置页。
    McpServers,
    /// 只读：serve 给的默认模型。
    ConnModel,
    /// 只读：本会话生效的权限档（`/permission` 的 min 链结果）。
    ConnPermission,
    /// 只读：全局快捷键一览（F 系功能键）。
    KeyGlobal,
    /// 只读：对话区快捷键（视图 / 思考 / 复制 / 取消）。
    ///
    /// 与 [`FieldId::KeyGlobal`] 分两行而不是挤一行：终端 80 列下挤成一条会被
    /// `truncate_display` 截掉尾部的键位 —— 而这栏的值**就是**事实，截掉即失真。
    KeyChat,
    /// 只读：设置面板内快捷键。
    KeySettings,
    /// 只读：编辑器 / 会话视图快捷键。
    KeyEditor,
    /// 只读：密钥只进本机钥匙串。
    LocalSecret,
    /// 只读：保存语义（先 GET 再改自己的键再全量 PUT）。
    LocalSave,
    /// 只读：`.md` 走 markdown 预览。
    LocalMarkdown,
    /// 只读：本连接的 `api_base`（「连错 serve 了」时第一眼要看的东西）。
    ConnApiBase,
    /// 只读：本连接在左栏里的名字与序号。
    ConnLane,
    /// 只读：Web Bearer 的**来源**（本机钥匙串 / 启动参数或环境 / 未提供），绝不报值。
    ConnBearer,
    /// 只读：`CM_TUI_MOUSE` 的当前生效值。
    EnvMouse,
    /// 只读：`CM_TUI_PASTE` 的当前生效值。
    EnvPaste,
    /// 只读：`CM_TUI_CLIPBOARD` 的当前生效值（`/copy` 会走哪条路）。
    EnvClipboard,
    /// 只读：`CM_TUI_LOOP` 的当前生效值。
    EnvLoop,
    /// 只读：`CM_TUI_COLOR` 的当前生效值。
    EnvColor,
    /// 只读：`CM_TUI_IMAGE` 的当前生效值。
    EnvImage,
    /// 只读：`CM_TUI_REDUCED_MOTION` 的当前生效值。
    EnvMotion,
    /// 只读：`CM_TUI_EXPAND_TILDE` 的当前生效值。
    EnvTilde,
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
        FieldId::McpGlobalEnabled => "MCP 开关",
        FieldId::McpToolTimeout => "工具超时",
        FieldId::McpServers => "服务器",
        FieldId::ConnModel => "serve 模型",
        FieldId::ConnPermission => "权限档",
        FieldId::ConnApiBase => "连接地址",
        FieldId::ConnLane => "lane",
        FieldId::ConnBearer => "Web Bearer",
        FieldId::KeyGlobal => "全局键",
        FieldId::KeyChat => "对话键",
        FieldId::KeySettings => "面板键",
        FieldId::KeyEditor => "编辑器键",
        FieldId::LocalSecret => "密钥存储",
        FieldId::LocalSave => "保存语义",
        FieldId::LocalMarkdown => "Markdown",
        FieldId::EnvMouse => "鼠标捕获",
        FieldId::EnvPaste => "括号粘贴",
        FieldId::EnvClipboard => "复制通道",
        FieldId::EnvLoop => "事件循环",
        FieldId::EnvColor => "色深",
        FieldId::EnvImage => "背景图",
        FieldId::EnvMotion => "动效",
        FieldId::EnvTilde => "~ 展开",
    }
}
