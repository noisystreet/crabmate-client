//! TUI 设置面板（W1）UI 状态机与面板内容构建：分区导航 + 字段行编辑 +
//! 脏/确认关闭/保存动作（按键语义纯状态层，测试友好）。三层合成与合并 PUT
//! 的纯逻辑在 [`super::settings`]，本文件只负责交互状态与可视行生成。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use super::SessionPrefs;
use super::render::{cell_window, horizontal_window, truncate_display};
use super::serve_defaults::ServeDefaults;
use super::settings::{
    EffectiveView, FieldAction, Layer, LlmSave, PersistedSettings, PrefsSave, SESSION_MODES,
    THINKING_MODES, TOOL_CACHE_DISABLED, effective_value, is_valid_context_tokens,
    is_valid_temperature, normalize_str, validate_api_base,
};

/// 会话模式编辑选项长度 =「跟随 server」+ [`SESSION_MODES`]。
const MODE_OPTIONS_LEN: usize = SESSION_MODES.len() + 1;

/// 思考模式编辑选项长度 =「跟随 server」+ [`THINKING_MODES`]。
const THINK_OPTIONS_LEN: usize = THINKING_MODES.len() + 1;

/// 会话模式编辑选项（`None` = 跟随 server，即清除存储键）；取值以 [`SESSION_MODES`] 为单一来源。
fn mode_options() -> [Option<&'static str>; MODE_OPTIONS_LEN] {
    let mut out = [None; MODE_OPTIONS_LEN];
    for (i, m) in SESSION_MODES.iter().copied().enumerate() {
        out[i + 1] = Some(m);
    }
    out
}

/// 思考模式编辑选项（`None` = 跟随 server，即清除存储键）；取值以 [`THINKING_MODES`] 为单一来源。
fn think_options() -> [Option<&'static str>; THINK_OPTIONS_LEN] {
    let mut out = [None; THINK_OPTIONS_LEN];
    for (i, m) in THINKING_MODES.iter().copied().enumerate() {
        out[i + 1] = Some(m);
    }
    out
}

/// 只读工具缓存编辑选项长度 = 开（跟随 server） + 关（禁用）。
const TOOL_OPTIONS_LEN: usize = 2;

/// 只读工具缓存编辑选项（`None` = 开/跟随 server，即清除 prefs 键；`Some("off")` = 关/禁用）。
fn tool_options() -> [Option<&'static str>; TOOL_OPTIONS_LEN] {
    [None, Some(TOOL_CACHE_DISABLED)]
}

/// 网关预设（与 `frontend/src/client_llm_presets.rs` 的 `CLIENT_LLM_API_BASE_PRESETS`
/// **同源拷贝**；改 URL / 增删条目需两仓同步，避免漂移）。
/// TUI 只回写 `api_base`，不按预设自动填建议模型名（模型名仍保持「空 = 跟随 server」）。
#[derive(Clone, Copy)]
struct ApiPreset {
    id: &'static str,
    url: &'static str,
}

const API_BASE_PRESETS: &[ApiPreset] = &[
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

const GATEWAY_SERVER_ID: &str = "server";
const GATEWAY_CUSTOM_ID: &str = "custom";

/// 网关预设选项下标 → 值（对应 [`API_BASE_PRESETS`]；`server` 是首位，`custom` 是末位）。
fn gateway_pick_of(value: Option<&str>) -> usize {
    match value.map(str::trim).filter(|s| !s.is_empty()) {
        None => 0,
        Some(v) => API_BASE_PRESETS
            .iter()
            .position(|p| p.id != GATEWAY_SERVER_ID && p.id != GATEWAY_CUSTOM_ID && p.url == v)
            .unwrap_or(API_BASE_PRESETS.len() - 1),
    }
}
/// 字段行标签列宽（cell；含标签与值之间间隔）。
const LABEL_PAD: usize = 13;

/// F2：设置面板开关（与 Esc 同语义地关闭；空闲时由 mod 层打开）。
pub(super) fn is_f2_key(key: &KeyEvent) -> bool {
    key.code == KeyCode::F(2) && !key.modifiers.contains(KeyModifiers::CONTROL)
}

/// 面板分区：模型 / 会话（扁平字段，写 serve user-data）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Llm,
    Session,
}

impl Section {
    const ALL: [Section; 2] = [Section::Llm, Section::Session];

    const fn label(self) -> &'static str {
        match self {
            Section::Llm => "模型",
            Section::Session => "会话",
        }
    }

    const fn fields(self) -> &'static [FieldId] {
        match self {
            Section::Llm => &[
                FieldId::Model,
                FieldId::ApiBase,
                FieldId::Temperature,
                FieldId::ContextTokens,
                FieldId::ThinkingMode,
                FieldId::ApiKey,
            ],
            Section::Session => &[FieldId::Role, FieldId::SessionMode, FieldId::ToolCache],
        }
    }
}

/// 面板管理的字段：模型分区 6 个 + 会话分区 3 个。`Temperature/ThinkingMode/
/// ContextTokens` 写 user-data `client_llm.*`；`ApiKey` 只写本机钥匙串（见 TuiApp
/// 接线）；`ToolCache` 写 prefs `disable_readonly_tool_ttl_cache`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldId {
    Model,
    ApiBase,
    Temperature,
    ContextTokens,
    ThinkingMode,
    ApiKey,
    Role,
    SessionMode,
    ToolCache,
}

const fn field_label(field: FieldId) -> &'static str {
    match field {
        FieldId::Model => "模型名",
        FieldId::ApiBase => "API Base",
        FieldId::Temperature => "温度",
        FieldId::ContextTokens => "上下文 tokens",
        FieldId::ThinkingMode => "思考模式",
        FieldId::ApiKey => "API 密钥",
        FieldId::Role => "Agent role",
        FieldId::SessionMode => "会话模式",
        FieldId::ToolCache => "只读工具缓存",
    }
}

/// 保存结果属于哪一组（llm-overrides / prefs）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveGroup {
    Llm,
    Prefs,
}

/// 行编辑状态：文本编辑（含缓冲与光标）、模式/思考/工具缓存枚举循环、网关预设循环。
#[derive(Debug, Clone, PartialEq, Eq)]
enum Editing {
    Text {
        field: FieldId,
        buf: Vec<char>,
        cursor: usize,
    },
    Mode {
        pick: usize,
    },
    Think {
        pick: usize,
    },
    Tool {
        pick: usize,
    },
    /// 网关预设循环：`pick` 对应 [`API_BASE_PRESETS`]；`prefill` 在选「自定义 URL」时
    /// 作为文本编辑初值（当前生效值，避免重输长 URL）。
    Gateway {
        pick: usize,
        prefill: String,
    },
}

/// 编辑态按键动作（先在不可变借用下解析，再执行以避开自引用借用）。
enum EditAction {
    Cancel,
    CommitText,
    CommitMode,
    CommitThink,
    CommitTool,
    CommitGateway,
    Insert(char),
    Backspace,
    MoveLeft,
    MoveRight,
}

/// 面板按键处理结果（状态已内部更新；副作用交给调用方）。
#[derive(Debug)]
pub(super) enum PanelEffect {
    None,
    /// 请求保存（三组可能各有动作；空组由调用方跳过）。负载走 Box：字段数涨到
    /// 超 clippy large-enum-variant 阈值，而该 effect 只是 UI→app 的一次性传递。
    Save {
        llm: Box<LlmSave>,
        prefs: Box<PrefsSave>,
        /// API 密钥保存动作（写钥匙串，不进 `LlmSave`；`Skip` = 无动作）。
        secret: FieldAction,
    },
    /// 请求关闭面板（已通过脏确认或本来干净）。
    Close,
}

/// 设置面板交互状态机。
pub struct SettingsPanel {
    /// 打开瞬间是否有回合在跑（只读：可浏览，禁编辑/保存）。
    read_only: bool,
    /// 当前分区。
    section: Section,
    /// 分区内行光标（字段下标）。
    row: usize,
    /// 已编辑（staged）的 llm 侧字段；`Write` = 待保存。
    llm: LlmSave,
    /// 已编辑（staged）的 prefs 侧字段；`Write` = 待保存。
    prefs: PrefsSave,
    /// 已编辑（staged）的 API 密钥动作（不随 `LlmSave` 走 user-data，见 TuiApp）。
    secret: FieldAction,
    /// API 密钥当前是否已设（CLI/env override 或钥匙串有值）；只读显示用。
    secret_set: bool,
    /// 正在编辑的字段（None = 浏览态）。
    editing: Option<Editing>,
    /// 有关闭确认待答复（脏字段时 Esc/F2 先弹确认）。
    confirm_close: bool,
    /// llm 组保存请求在途（结果未回）。
    saving_llm: bool,
    /// prefs 组保存请求在途（结果未回）。
    saving_prefs: bool,
    /// 底部提示行（编辑错误 / 只读提示 / 保存结果等）。
    note: Option<(String, Color)>,
}

impl Default for SettingsPanel {
    fn default() -> Self {
        Self::new(false)
    }
}

impl SettingsPanel {
    #[must_use]
    pub fn new(read_only: bool) -> Self {
        Self {
            read_only,
            section: Section::Llm,
            row: 0,
            llm: LlmSave::default(),
            prefs: PrefsSave::default(),
            secret: FieldAction::Skip,
            secret_set: false,
            editing: None,
            confirm_close: false,
            saving_llm: false,
            saving_prefs: false,
            note: None,
        }
    }

    /// 是否有任一组保存请求在途（期间禁止编辑/保存/关闭）。
    fn is_saving(&self) -> bool {
        self.saving_llm || self.saving_prefs
    }

    /// 是否有未保存的 staged 改动（含 API 密钥动作）。
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.llm.any() || self.prefs.any() || self.secret.is_write()
    }

    /// API 密钥行当前是否"已设"（CLI/env override 或钥匙串有值）；由 TuiApp 打开
    /// 面板及密钥保存成功后调用。只影响显示与清除提示，不代表 staged。
    pub(super) fn set_secret_set(&mut self, v: bool) {
        self.secret_set = v;
    }

    /// API 密钥保存动作落地后回调（`written` = 钥匙串里当前是否有值）：清 staged 并
    /// 同步"已设"显示；失败时改动保留（由 TuiApp 提示，可重试）。
    pub(super) fn secret_saved(&mut self, written: bool) {
        self.secret = FieldAction::Skip;
        self.secret_set = written;
    }

    fn current_field(&self) -> FieldId {
        self.section.fields()[self.row]
    }

    fn staged(&self, field: FieldId) -> &FieldAction {
        match field {
            FieldId::Model => &self.llm.model,
            FieldId::ApiBase => &self.llm.api_base,
            FieldId::Temperature => &self.llm.temperature,
            FieldId::ContextTokens => &self.llm.context_tokens,
            FieldId::ThinkingMode => &self.llm.thinking,
            FieldId::ApiKey => &self.secret,
            FieldId::Role => &self.prefs.role,
            FieldId::SessionMode => &self.prefs.session_mode,
            FieldId::ToolCache => &self.prefs.tool_cache,
        }
    }

    fn set_staged(&mut self, field: FieldId, action: FieldAction) {
        match field {
            FieldId::Model => self.llm.model = action,
            FieldId::ApiBase => self.llm.api_base = action,
            FieldId::Temperature => self.llm.temperature = action,
            FieldId::ContextTokens => self.llm.context_tokens = action,
            FieldId::ThinkingMode => self.llm.thinking = action,
            FieldId::ApiKey => self.secret = action,
            FieldId::Role => self.prefs.role = action,
            FieldId::SessionMode => self.prefs.session_mode = action,
            FieldId::ToolCache => self.prefs.tool_cache = action,
        }
    }

    /// 底部提示行（给 TuiApp 在保存结果到达时写提示）。
    pub(super) fn set_note(&mut self, text: String, color: Color) {
        self.note = Some((text, color));
    }

    /// 一组保存结果返回。`ok` 时清空该组 staged 与在途标记；失败保留 staged 供重试。
    /// 返回该组结果是否让"全部已发请求"落地完毕（供调用方发总结行）。
    pub(super) fn save_group_result(&mut self, group: SaveGroup, ok: bool) -> bool {
        match group {
            SaveGroup::Llm => {
                self.saving_llm = false;
                if ok {
                    self.llm.clear();
                }
            }
            SaveGroup::Prefs => {
                self.saving_prefs = false;
                if ok {
                    self.prefs.clear();
                }
            }
        }
        !self.is_saving()
    }

    /// 回合结束（`TurnDone`）：若是回合进行中打开的只读面板，转为可编辑并提示；
    /// 已编辑过的面板不受影响。
    pub(super) fn unlock_after_turn(&mut self) {
        if self.read_only {
            self.read_only = false;
            self.set_note("回合结束：面板已解锁".to_string(), Color::LightGreen);
        }
    }

    /// 浏览态按键：↑↓ 移动行、Tab 切分区、Enter 编辑、S 保存、Esc/F2 关闭（脏先确认）。
    pub(super) fn handle_key(&mut self, key: &KeyEvent, ctx: &PanelCtx<'_>) -> PanelEffect {
        if self.editing.is_some() {
            self.edit_key(key);
            return PanelEffect::None;
        }
        if self.confirm_close {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.confirm_close = false;
                    return PanelEffect::Close;
                }
                KeyCode::Esc | KeyCode::F(2) | KeyCode::Char('n') | KeyCode::Char('N') => {
                    self.confirm_close = false;
                }
                _ => {}
            }
            return PanelEffect::None;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            return PanelEffect::None;
        }
        // 保存在途：禁止编辑/关闭/重复保存。
        if self.is_saving() {
            return PanelEffect::None;
        }
        self.note = None;
        match key.code {
            KeyCode::Up => {
                self.move_row(true);
            }
            KeyCode::Down => {
                self.move_row(false);
            }
            KeyCode::Tab => {
                self.cycle_section(true);
            }
            KeyCode::BackTab => {
                self.cycle_section(false);
            }
            KeyCode::Enter => self.begin_edit(ctx),
            KeyCode::Char('s') | KeyCode::Char('S') => return self.request_save(),
            KeyCode::Esc | KeyCode::F(2) => {
                if self.is_dirty() {
                    self.confirm_close = true;
                } else {
                    return PanelEffect::Close;
                }
            }
            _ => {}
        }
        PanelEffect::None
    }

    /// 分区内上下移动行光标（clamp）。
    fn move_row(&mut self, up: bool) {
        let len = self.section.fields().len();
        if up {
            self.row = self.row.saturating_sub(1);
        } else if self.row + 1 < len {
            self.row += 1;
        }
    }

    /// Tab / Shift+Tab 循环切换分区（行光标回到该分区首行）。
    fn cycle_section(&mut self, forward: bool) {
        let len = Section::ALL.len();
        let idx = self.section as usize;
        let next = if forward {
            (idx + 1) % len
        } else {
            (idx + len - 1) % len
        };
        self.section = Section::ALL[next];
        self.row = 0;
    }

    /// Enter：进入当前字段的编辑（文本缓冲预填生效值；会话模式/思考模式/工具缓存/网关
    /// 预设进枚举循环；API 密钥从空开始；自定义网关在预设循环里 Enter 落到文本编辑）。
    fn begin_edit(&mut self, ctx: &PanelCtx<'_>) {
        if self.read_only {
            self.set_note(
                "回合进行中：只读（结束后自动解锁）".to_string(),
                Color::LightYellow,
            );
            return;
        }
        let field = self.current_field();
        if field == FieldId::SessionMode {
            let pick = match self.staged(field) {
                FieldAction::Write(Some(v)) => mode_index_of(Some(v)),
                FieldAction::Write(None) => 0,
                FieldAction::Skip => match ctx.effective(field).value {
                    Some(v) => mode_index_of(Some(&v)),
                    None => 0,
                },
            };
            self.editing = Some(Editing::Mode { pick });
            return;
        }
        if field == FieldId::ThinkingMode {
            let pick = match self.staged(field) {
                FieldAction::Write(Some(v)) => think_index_of(Some(v)),
                FieldAction::Write(None) => 0,
                FieldAction::Skip => match ctx.effective(field).value {
                    Some(v) => think_index_of(Some(&v)),
                    None => 0,
                },
            };
            self.editing = Some(Editing::Think { pick });
            return;
        }
        if field == FieldId::ToolCache {
            let pick = match self.staged(field) {
                FieldAction::Write(Some(v)) => tool_index_of(Some(v)),
                FieldAction::Write(None) => 0,
                FieldAction::Skip => match ctx.effective(field).value {
                    Some(v) => tool_index_of(Some(&v)),
                    None => 0,
                },
            };
            self.editing = Some(Editing::Tool { pick });
            return;
        }
        // API Base：网关预设循环（server / ollama / … / 自定义 URL）。自定义在预设里 Enter
        // 后落到文本编辑（缓冲预填当前生效值，免重输长 URL）。
        if field == FieldId::ApiBase {
            let prefill = match self.staged(field) {
                FieldAction::Write(Some(v)) => v.clone(),
                FieldAction::Write(None) => String::new(),
                FieldAction::Skip => ctx.effective(field).value.unwrap_or_default(),
            };
            let pick = gateway_pick_of(Some(&prefill));
            self.editing = Some(Editing::Gateway { pick, prefill });
            return;
        }
        // API 密钥不在三层合成内：编辑缓冲总从空开始（不回显既有值/已设态）。
        if field == FieldId::ApiKey {
            self.editing = Some(Editing::Text {
                field,
                buf: Vec::new(),
                cursor: 0,
            });
            return;
        }
        let prefill = match self.staged(field) {
            FieldAction::Write(Some(v)) => v.clone(),
            FieldAction::Write(None) => String::new(),
            FieldAction::Skip => ctx.effective(field).value.unwrap_or_default(),
        };
        let buf: Vec<char> = prefill.chars().collect();
        let cursor = buf.len();
        self.editing = Some(Editing::Text { field, buf, cursor });
    }

    /// 编辑态按键：文本字符输入/Backspace/←→ 光标、Enter 提交、Esc 取消；
    /// ↑↓ 与 Tab 忽略（避免误跳行/误切分区）。
    fn edit_key(&mut self, key: &KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            return;
        }
        let action = match &self.editing {
            Some(Editing::Text { .. }) => match key.code {
                KeyCode::Esc => Some(EditAction::Cancel),
                KeyCode::Enter => Some(EditAction::CommitText),
                KeyCode::Char(c) => Some(EditAction::Insert(c)),
                KeyCode::Backspace => Some(EditAction::Backspace),
                KeyCode::Left => Some(EditAction::MoveLeft),
                KeyCode::Right => Some(EditAction::MoveRight),
                _ => None,
            },
            Some(Editing::Mode { .. }) => match key.code {
                KeyCode::Esc => Some(EditAction::Cancel),
                KeyCode::Enter => Some(EditAction::CommitMode),
                KeyCode::Left => Some(EditAction::MoveLeft),
                KeyCode::Right => Some(EditAction::MoveRight),
                _ => None,
            },
            Some(Editing::Think { .. }) => match key.code {
                KeyCode::Esc => Some(EditAction::Cancel),
                KeyCode::Enter => Some(EditAction::CommitThink),
                KeyCode::Left => Some(EditAction::MoveLeft),
                KeyCode::Right => Some(EditAction::MoveRight),
                _ => None,
            },
            Some(Editing::Tool { .. }) => match key.code {
                KeyCode::Esc => Some(EditAction::Cancel),
                KeyCode::Enter => Some(EditAction::CommitTool),
                KeyCode::Left => Some(EditAction::MoveLeft),
                KeyCode::Right => Some(EditAction::MoveRight),
                _ => None,
            },
            Some(Editing::Gateway { .. }) => match key.code {
                KeyCode::Esc => Some(EditAction::Cancel),
                KeyCode::Enter => Some(EditAction::CommitGateway),
                KeyCode::Left => Some(EditAction::MoveLeft),
                KeyCode::Right => Some(EditAction::MoveRight),
                _ => None,
            },
            None => None,
        };
        let Some(action) = action else {
            return;
        };
        match action {
            EditAction::Cancel => self.editing = None,
            EditAction::CommitText => self.commit_text(),
            EditAction::CommitMode => self.commit_mode(),
            EditAction::CommitThink => self.commit_think(),
            EditAction::CommitTool => self.commit_tool(),
            EditAction::CommitGateway => self.commit_gateway(),
            EditAction::Insert(c) => self.edit_insert(c),
            EditAction::Backspace => self.edit_backspace(),
            EditAction::MoveLeft => self.edit_move(true),
            EditAction::MoveRight => self.edit_move(false),
        }
    }

    /// 文本字段提交：trim 后空串 = 清除；API Base 需 http(s):// 前缀、温度需
    /// 0.0..=2.0、上下文 tokens 需 ≤ 10_000_000 整数（非法则留在编辑态提示）；
    /// 密钥空提交 = 清除。
    fn commit_text(&mut self) {
        let Some(Editing::Text { field, buf, .. }) = self.editing.take() else {
            return;
        };
        let text: String = buf.into_iter().collect();
        let text = text.trim().to_string();
        if field == FieldId::ApiBase && !validate_api_base(&text) {
            let buf: Vec<char> = text.chars().collect();
            let cursor = buf.len();
            self.editing = Some(Editing::Text { field, buf, cursor });
            self.set_note("API Base 需 http(s):// 开头".to_string(), Color::LightRed);
            return;
        }
        if field == FieldId::Temperature && !is_valid_temperature(&text) {
            let buf: Vec<char> = text.chars().collect();
            let cursor = buf.len();
            self.editing = Some(Editing::Text { field, buf, cursor });
            self.set_note(
                "温度需 0.0..=2.0（留空保存 = 跟随 server）".to_string(),
                Color::LightRed,
            );
            return;
        }
        if field == FieldId::ContextTokens && !is_valid_context_tokens(&text) {
            let buf: Vec<char> = text.chars().collect();
            let cursor = buf.len();
            self.editing = Some(Editing::Text { field, buf, cursor });
            self.set_note(
                "上下文需 0..=10_000_000 整数（留空保存 = 跟随 server）".to_string(),
                Color::LightRed,
            );
            return;
        }
        let value = if text.is_empty() { None } else { Some(text) };
        self.set_staged(field, FieldAction::Write(value));
    }

    /// 会话模式枚举确认：写入当前选项（`None` = 跟随 server）。
    fn commit_mode(&mut self) {
        let Some(Editing::Mode { pick }) = self.editing.take() else {
            return;
        };
        let value = mode_options()[pick].map(str::to_string);
        self.set_staged(FieldId::SessionMode, FieldAction::Write(value));
    }

    /// 思考模式枚举确认：写入当前选项（`None` = server / 跟随）。
    fn commit_think(&mut self) {
        let Some(Editing::Think { pick }) = self.editing.take() else {
            return;
        };
        let value = think_options()[pick].map(str::to_string);
        self.set_staged(FieldId::ThinkingMode, FieldAction::Write(value));
    }

    /// 只读工具缓存确认：写入当前选项（`None` = 开/跟随 server；`off` = 关/禁用）。
    fn commit_tool(&mut self) {
        let Some(Editing::Tool { pick }) = self.editing.take() else {
            return;
        };
        let value = tool_options()[pick].map(str::to_string);
        self.set_staged(FieldId::ToolCache, FieldAction::Write(value));
    }

    /// 网关预设确认：`server` = 清除（跟随 server）；具名预设写其 URL；`custom`
    /// 转文本编辑（缓冲预填当前生效值，Enter 后再走 URL 校验）。
    fn commit_gateway(&mut self) {
        let Some(Editing::Gateway { pick, prefill }) = self.editing.take() else {
            return;
        };
        let preset = API_BASE_PRESETS[pick];
        if preset.id == GATEWAY_CUSTOM_ID {
            let buf: Vec<char> = prefill.chars().collect();
            let cursor = buf.len();
            self.editing = Some(Editing::Text {
                field: FieldId::ApiBase,
                buf,
                cursor,
            });
            return;
        }
        let value = (preset.id != GATEWAY_SERVER_ID).then(|| preset.url.to_string());
        self.set_staged(FieldId::ApiBase, FieldAction::Write(value));
    }

    /// 文本编辑：光标处插入字符。
    fn edit_insert(&mut self, c: char) {
        let Some(Editing::Text { buf, cursor, .. }) = &mut self.editing else {
            return;
        };
        if *cursor >= buf.len() {
            buf.push(c);
            *cursor = buf.len();
        } else {
            buf.insert(*cursor, c);
            *cursor += 1;
        }
    }

    /// 文本编辑：删除光标前一字符。
    fn edit_backspace(&mut self) {
        let Some(Editing::Text { buf, cursor, .. }) = &mut self.editing else {
            return;
        };
        if *cursor > 0 {
            *cursor -= 1;
            buf.remove(*cursor);
        }
    }

    /// 编辑态 ←/→：文本字段移动光标；模式/思考/工具/网关枚举循环切选项（↑↓ 不动）。
    fn edit_move(&mut self, left: bool) {
        match &mut self.editing {
            Some(Editing::Text { buf, cursor, .. }) => {
                if left {
                    *cursor = cursor.saturating_sub(1);
                } else if *cursor < buf.len() {
                    *cursor += 1;
                }
            }
            Some(other) => move_cycle_pick(other, left),
            None => {}
        }
    }

    /// S 保存：staged 全空时提示；否则标记在途并返回保存请求（含两组暂存动作 + 密钥）。
    fn request_save(&mut self) -> PanelEffect {
        if self.read_only {
            self.set_note("回合进行中：不能保存".to_string(), Color::LightYellow);
            return PanelEffect::None;
        }
        if !self.is_dirty() {
            self.set_note("无改动可保存".to_string(), Color::LightYellow);
            return PanelEffect::None;
        }
        let llm = self.llm.clone();
        let prefs = self.prefs.clone();
        let secret = self.secret.clone();
        self.saving_llm = llm.any();
        self.saving_prefs = prefs.any();
        PanelEffect::Save {
            llm: Box::new(llm),
            prefs: Box::new(prefs),
            secret,
        }
    }
}

/// ←/→ 移动循环类编辑的光标（Mode / Think / Tool / Gateway）。
fn move_cycle_pick(editing: &mut Editing, left: bool) {
    match editing {
        Editing::Mode { pick } => move_pick(pick, left, mode_options().len()),
        Editing::Think { pick } => move_pick(pick, left, think_options().len()),
        Editing::Tool { pick } => move_pick(pick, left, tool_options().len()),
        Editing::Gateway { pick, .. } => move_pick(pick, left, API_BASE_PRESETS.len()),
        Editing::Text { .. } => {}
    }
}

/// 把 `pick` 在 `0..len` 里按方向移动一格（clamp，不循环回卷）。
fn move_pick(pick: &mut usize, left: bool, len: usize) {
    if left {
        *pick = pick.saturating_sub(1);
    } else if *pick + 1 < len {
        *pick += 1;
    }
}

/// 会话模式选项下标 → 值（按 [`SESSION_MODES`] 顺序；其余（含 None）回 0 = 跟随 server）。
fn mode_index_of(v: Option<&str>) -> usize {
    v.and_then(|m| SESSION_MODES.iter().position(|x| *x == m))
        .map_or(0, |i| i + 1)
}

/// 思考模式选项下标 → 值（按 [`THINKING_MODES`] 顺序；其余（含 None）回 0 = server/跟随）。
fn think_index_of(v: Option<&str>) -> usize {
    v.and_then(|m| THINKING_MODES.iter().position(|x| *x == m))
        .map_or(0, |i| i + 1)
}

/// 只读工具缓存选项下标 → 值（`off` → 1 = 禁用；其它（含 None）回 0 = 开/跟随 server）。
fn tool_index_of(v: Option<&str>) -> usize {
    usize::from(v == Some(TOOL_CACHE_DISABLED))
}

/// 面板显示所需的三层上下文（override / 持久层 / serve 默认）。
pub struct PanelCtx<'a> {
    pub overrides: &'a SessionPrefs,
    pub persisted: Option<&'a PersistedSettings>,
    pub serve_defaults: Option<&'a ServeDefaults>,
}

impl PanelCtx<'_> {
    /// 字段的三层来源（override、user-data、serve 默认；空白由合成时归一），供合成/预填共用。
    /// 温度/上下文/思考/工具缓存无本地 override（TUI 无斜杠）与 serve 默认；API 密钥不参与
    /// 三层（走钥匙串）。
    fn sources(&self, field: FieldId) -> (Option<&str>, Option<&str>, Option<&str>) {
        let p = self.persisted;
        let d = self.serve_defaults;
        let local = match field {
            FieldId::Model => self.overrides.model.as_deref(),
            FieldId::ApiBase => self.overrides.api_base.as_deref(),
            FieldId::Temperature
            | FieldId::ContextTokens
            | FieldId::ThinkingMode
            | FieldId::ToolCache
            | FieldId::ApiKey => None,
            FieldId::Role => self.overrides.agent_role.as_deref(),
            FieldId::SessionMode => self.overrides.session_mode.as_deref(),
        };
        let stored = match field {
            FieldId::Model => p.and_then(|p| p.model.as_deref()),
            FieldId::ApiBase => p.and_then(|p| p.api_base.as_deref()),
            FieldId::Temperature => p.and_then(|p| p.temperature.as_deref()),
            FieldId::ContextTokens => p.and_then(|p| p.context_tokens.as_deref()),
            FieldId::ThinkingMode => p.and_then(|p| p.thinking.as_deref()),
            FieldId::ApiKey => None,
            FieldId::Role => p.and_then(|p| p.role.as_deref()),
            FieldId::SessionMode => p.and_then(|p| p.session_mode.as_deref()),
            // 工具缓存：禁用（disable=true）记为 "off"，其余（None/Some(false)）= 跟随 server。
            FieldId::ToolCache => p
                .and_then(|p| p.tool_cache_disabled)
                .and_then(|d| d.then_some(TOOL_CACHE_DISABLED)),
        };
        let remote = match field {
            FieldId::Model => d.and_then(|d| d.model.as_deref()),
            // API Base / 温度 / 上下文 / 思考 / 工具缓存没有 /status 默认来源：三层只到 user-data。
            FieldId::ApiBase
            | FieldId::Temperature
            | FieldId::ContextTokens
            | FieldId::ThinkingMode
            | FieldId::ToolCache
            | FieldId::ApiKey => None,
            FieldId::Role => d.and_then(|d| d.role.as_deref()),
            FieldId::SessionMode => d.and_then(|d| d.mode.as_deref()),
        };
        (local, stored, remote)
    }

    /// 三层合成（override ＞ user-data ＞ serve 默认）。
    fn effective(&self, field: FieldId) -> EffectiveView {
        let (l, s, r) = self.sources(field);
        effective_value(l, s, r)
    }
}

/// 面板一帧内容（render 只负责把它画到浮层里）。
pub struct PanelContent {
    pub lines: Vec<Line<'static>>,
    /// 文本编辑光标：内容行下标 + cell 列（None = 不设硬件光标）。
    pub cursor: Option<(usize, usize)>,
}

/// 把字符串按 cell 补齐到至少 `cells`（超出则原样返回）。
fn pad_cells(s: &str, cells: usize) -> String {
    let w = UnicodeWidthStr::width(s);
    let pad = cells.saturating_sub(w);
    format!("{s}{}", " ".repeat(pad))
}

// 内容构建与行渲染（content / field_line / 值列 / 导航 / 底栏文案）独立成文件：压行数门禁。
#[path = "settings_panel_draw.rs"]
mod draw;

#[path = "settings_app.rs"]
mod app;

#[cfg(test)]
#[path = "settings_panel_tests.rs"]
mod tests;

// W2 新增字段（温度/思考模式/API 密钥）单测独立成文件：控制 tests 文件规模，避免
// lizard 对超大测试文件的函数体合并跨度超过 fn-nloc 门禁。
#[cfg(test)]
#[path = "settings_panel_w2_tests.rs"]
mod tests_w2;

// W2 收尾字段（上下文 tokens / 只读工具缓存 / 网关预设）单测独立成文件（同上）。
#[cfg(test)]
#[path = "settings_panel_w3_tests.rs"]
mod tests_w3;
