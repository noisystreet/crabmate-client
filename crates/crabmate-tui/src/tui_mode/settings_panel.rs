//! TUI 设置面板（W1）UI 状态机与面板内容构建：分区导航 + 字段行编辑 +
//! 脏/确认关闭/保存动作（按键语义纯状态层，测试友好）。三层合成与合并 PUT
//! 的纯逻辑在 [`super::settings`]，本文件只负责交互状态与可视行生成。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use super::SessionPrefs;
use super::palette::{NoticeLevel, Palette, THEME_OPTIONS};
use super::render::truncate_display;
use super::serve_defaults::ServeDefaults;
use super::settings::{
    EffectiveView, FieldAction, Layer, LlmSave, McpSave, PersistedSettings, PrefsSave, STORE_OFF,
    STORE_ON, TOOL_CACHE_DISABLED, effective_value, normalize_str,
};
use super::text::{cell_window, horizontal_window};

/// 只读信息行的占位动作：`staged()` 必须返回引用，所以需要一个静态值。
///
/// 它永远是 `Skip`，因此既不会让面板变脏，也不会进任何一组保存请求。
const READ_ONLY: FieldAction = FieldAction::Skip;

/// 字段行标签列宽（cell；含标签与值之间间隔）。
const LABEL_PAD: usize = 13;

/// F6：设置面板开关（与 Esc 同语义地关闭；空闲时由 mod 层打开）。
///
/// 选 F6 而不是 `Ctrl+,`（VS Code / Desktop 的设置键）：终端只有开键盘增强协议
/// （CSI u / kitty）才分辨得出「Ctrl+逗号」与裸 `,`，而 VTE 系终端（gnome-terminal
/// / xfce4-terminal / tilix，GNOME/vte#2601 仍未实现）会把两者发成同一个 `0x2C`，
/// 那样按键既打不开面板、又会往输入框里插一个逗号。F2–F9 由 VTE 按 terminfo
/// 原样透传，与本 TUI 已有的 F2（保存）/ F3（状态栈）同属一组功能键。
pub(super) fn is_settings_open_key(key: &KeyEvent) -> bool {
    matches!(key.code, KeyCode::F(6)) && !key.modifiers.contains(KeyModifiers::CONTROL)
}

/// F2（或 `S`）：面板内保存。
///
/// F2 不再兼作开关别名，专用于保存：与 `S` 并列，是为了让浏览态既有功能键
/// （不受输入缓冲影响）也有单字母快键。
pub(super) fn is_save_key(key: &KeyEvent) -> bool {
    matches!(
        key.code,
        KeyCode::F(2) | KeyCode::Char('s') | KeyCode::Char('S')
    ) && !key.modifiers.contains(KeyModifiers::CONTROL)
}

/// 保存结果属于哪一组（llm-overrides / prefs / 会话存储开关）。
///
/// 会话存储是**第三组**而不塞进 `Prefs`：它走 `POST /config/session/conversation-store`
/// 而不是全量 DTO 合并 PUT，失败语义（业务拒绝 vs 网络错误）也不同，混进一组会让
/// 「prefs 保存成功」这条提示盖掉一个真正失败的动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveGroup {
    Llm,
    Prefs,
    Store,
    /// MCP 标量（`/user-data/mcp-servers` 的全局开关 / 工具超时）。
    Mcp,
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
        /// 会话存储开关动作（`Write(Some("on"|"off"))`；`Skip` = 无动作）。
        store: FieldAction,
        /// MCP 标量动作（全局开关 / 工具超时；两者都 `Skip` = 无动作）。
        mcp: Box<McpSave>,
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
    /// 已编辑（staged）的会话存储开关；`Write(Some("on"|"off"))` = 待保存。
    ///
    /// 独立于 `PrefsSave`：它不是 user-data 键，保存动作是 `POST /config/session/conversation-store`。
    store: FieldAction,
    /// 已编辑（staged）的 MCP 标量（`/user-data/mcp-servers` 的全局开关 / 工具超时）。
    mcp: McpSave,
    /// 已编辑（staged）的 API 密钥动作（不随 `LlmSave` 走 user-data，见 LaneApp）。
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
    /// 会话存储开关保存请求在途（结果未回）。
    saving_store: bool,
    /// MCP 组保存请求在途（结果未回）。
    saving_mcp: bool,
    /// 底部提示行（编辑错误 / 只读提示 / 保存结果等）。
    note: Option<(String, NoticeLevel)>,
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
            store: FieldAction::Skip,
            mcp: McpSave::default(),
            secret: FieldAction::Skip,
            secret_set: false,
            editing: None,
            confirm_close: false,
            saving_llm: false,
            saving_prefs: false,
            saving_store: false,
            saving_mcp: false,
            note: None,
        }
    }

    /// 是否有任一组保存请求在途（期间禁止编辑/保存/关闭）。
    fn is_saving(&self) -> bool {
        self.saving_llm || self.saving_prefs || self.saving_store || self.saving_mcp
    }

    /// 是否有未保存的 staged 改动（含 API 密钥动作）。
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.llm.any()
            || self.prefs.any()
            || self.secret.is_write()
            || self.store.is_write()
            || self.mcp.any()
    }

    /// API 密钥行当前是否"已设"（CLI/env override 或钥匙串有值）；由 LaneApp 打开
    /// 面板及密钥保存成功后调用。只影响显示与清除提示，不代表 staged。
    pub(super) fn set_secret_set(&mut self, v: bool) {
        self.secret_set = v;
    }

    /// API 密钥保存动作落地后回调（`written` = 钥匙串里当前是否有值）：清 staged 并
    /// 同步"已设"显示；失败时改动保留（由 LaneApp 提示，可重试）。
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
            FieldId::Theme => &self.prefs.theme,
            FieldId::SessionSqlite => &self.store,
            FieldId::McpGlobalEnabled => &self.mcp.global_enabled,
            FieldId::McpToolTimeout => &self.mcp.tool_timeout,
            // 只读信息行：永远没有 staged 动作。
            FieldId::McpServers
            | FieldId::ConnModel
            | FieldId::ConnPermission
            | FieldId::KeyGlobal
            | FieldId::KeyChat
            | FieldId::KeySettings
            | FieldId::KeyEditor
            | FieldId::LocalSecret
            | FieldId::LocalSave
            | FieldId::LocalMarkdown => &READ_ONLY,
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
            FieldId::Theme => self.prefs.theme = action,
            FieldId::SessionSqlite => self.store = action,
            FieldId::McpGlobalEnabled => self.mcp.global_enabled = action,
            FieldId::McpToolTimeout => self.mcp.tool_timeout = action,
            // 只读行不接受写入（面板不会为它们产生动作，这里只是穷尽 match）。
            FieldId::McpServers
            | FieldId::ConnModel
            | FieldId::ConnPermission
            | FieldId::KeyGlobal
            | FieldId::KeyChat
            | FieldId::KeySettings
            | FieldId::KeyEditor
            | FieldId::LocalSecret
            | FieldId::LocalSave
            | FieldId::LocalMarkdown => {}
        }
    }

    /// 底部提示行（给 LaneApp 在保存结果到达时写提示）。
    ///
    /// 只收语义级别，颜色由 [`Palette::notice_style`] 决定。以前调用点直接传
    /// `Color::LightRed` 这类命名色：主题切换影响不到它们，`NO_COLOR` / `TERM=dumb`
    /// 下也照样带色（`mono()` 只抹 palette 的槽，抹不掉调用点硬写的命名色）。
    pub(super) fn set_note(&mut self, text: String, level: NoticeLevel) {
        self.note = Some((text, level));
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
            SaveGroup::Store => {
                self.saving_store = false;
                if ok {
                    self.store = FieldAction::Skip;
                }
            }
            SaveGroup::Mcp => {
                self.saving_mcp = false;
                if ok {
                    self.mcp.clear();
                }
            }
        }
        !self.is_saving()
    }

    /// 取消**全部**在途标记（不动 staged）：请求根本没发出去、或回包不会再来的路径都走这里。
    ///
    /// 单组复位的写法漏掉一组就会让面板永远停在「保存中」——`handle_key` 在 `is_saving`
    /// 处提前返回，那时 `Enter` / `Tab` / `F2` / `Esc` 全部失效（F6 也只是转成 Esc 进
    /// 同一个分支），面板只能靠杀进程关掉。所以这里不给单组版本 —— 回包路径上的
    /// 单组清理由 [`Self::save_group_result`] 负责，两者语义不同：一个是「这一组有结果了」，
    /// 一个是「谁都别再等了」。
    pub(super) fn cancel_all_saves(&mut self) {
        self.saving_llm = false;
        self.saving_prefs = false;
        self.saving_store = false;
        self.saving_mcp = false;
    }

    /// 回合结束（`TurnDone`）：若是回合进行中打开的只读面板，转为可编辑并提示；
    /// 已编辑过的面板不受影响。
    pub(super) fn unlock_after_turn(&mut self) {
        if self.read_only {
            self.read_only = false;
            self.set_note("回合结束：面板已解锁".to_string(), NoticeLevel::Ok);
        }
    }

    /// 浏览态按键：↑↓ 移动行、Tab 切分区、Enter 编辑、F2/S 保存、Esc 关闭（脏先确认）。
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
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
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
            KeyCode::Esc => {
                if self.is_dirty() {
                    self.confirm_close = true;
                } else {
                    return PanelEffect::Close;
                }
            }
            _ if is_save_key(key) => return self.request_save(),
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
        // `ALL` 的下标必须等于枚举判别式（下面按 `idx` 索引它，也按 `section as usize`
        // 反查选中项）。加一条断言把这条隐式契约钉死：调换枚举顺序时立刻炸测试，
        // 而不是等某个分区莫名跳到别处。
        debug_assert_eq!(
            Section::ALL[idx],
            self.section,
            "Section::ALL 的顺序必须与枚举判别式一致"
        );
        let next = if forward {
            (idx + 1) % len
        } else {
            (idx + len - 1) % len
        };
        self.section = Section::ALL[next];
        self.row = 0;
    }

    /// Enter：进入当前字段的编辑（文本缓冲预填生效值；主题/会话模式/思考模式/工具缓存/
    /// 网关预设进枚举循环；API 密钥从空开始；自定义网关在预设循环里 Enter 落到文本编辑）。
    fn begin_edit(&mut self, ctx: &PanelCtx<'_>) {
        if self.read_only {
            self.set_note(
                "回合进行中：只读（结束后自动解锁）".to_string(),
                NoticeLevel::Warn,
            );
            return;
        }
        // 三栏整栏只读（连接 / 快捷键 / 本地行为）：值本身就是说明，没有可编辑项。
        if is_readonly_section(self.section) {
            self.set_note(
                "这一栏是只读说明：值本身就是当前事实".to_string(),
                NoticeLevel::Muted,
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
        if field == FieldId::Theme {
            // 当前生效值不在选项表里（Desktop 侧新增 slug / `system`）时回落到
            // 下标 0 =「默认」：读取不改写，只有用户按 Enter 提交才会落盘。
            let pick = match self.staged(field) {
                FieldAction::Write(Some(v)) => theme_index_of(Some(v)),
                FieldAction::Write(None) => 0,
                FieldAction::Skip => match ctx.effective(field).value {
                    Some(v) => theme_index_of(Some(&v)),
                    None => 0,
                },
            };
            self.editing = Some(Editing::Theme { pick });
            return;
        }
        // 二值开关（会话存储 / MCP 全局开关）：只有 on/off 两态，当前值来自服务端。
        // 值未知或服务端不支持时不让进编辑态 —— 按下去也只会被拒。
        if field == FieldId::SessionSqlite || field == FieldId::McpGlobalEnabled {
            if !ctx.toggle_editable(field) {
                self.set_note(toggle_blocked_note(ctx, field), NoticeLevel::Warn);
                return;
            }
            let pick = match self.staged(field) {
                FieldAction::Write(Some(v)) => toggle_index_of(Some(v)),
                _ => toggle_index_of(ctx.effective(field).value.as_deref()),
            };
            self.editing = Some(Editing::Toggle { field, pick });
            return;
        }
        // MCP 工具超时：从当前值起编（预填），留空提交 = 不改。
        if field == FieldId::McpToolTimeout {
            let prefill = match self.staged(field) {
                FieldAction::Write(Some(v)) => v.clone(),
                FieldAction::Write(None) => String::new(),
                FieldAction::Skip => ctx
                    .persisted
                    .and_then(|p| p.mcp_tool_timeout_secs)
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
            };
            let buf: Vec<char> = prefill.chars().collect();
            let cursor = buf.len();
            self.editing = Some(Editing::Text { field, buf, cursor });
            return;
        }
        // MCP 服务器清单是**只读**信息行：条目里带 env / headers / bearer，不在终端里编辑。
        if field == FieldId::McpServers {
            self.set_note(
                "服务器清单请在 Desktop / Web 设置页编辑（终端只显示数量）".to_string(),
                NoticeLevel::Muted,
            );
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

    /// S 保存：staged 全空时提示；否则标记在途并返回保存请求（含两组暂存动作 + 密钥）。
    fn request_save(&mut self) -> PanelEffect {
        if self.read_only {
            self.set_note("回合进行中：不能保存".to_string(), NoticeLevel::Warn);
            return PanelEffect::None;
        }
        if !self.is_dirty() {
            self.set_note("无改动可保存".to_string(), NoticeLevel::Warn);
            return PanelEffect::None;
        }
        let llm = self.llm.clone();
        let prefs = self.prefs.clone();
        let secret = self.secret.clone();
        let store = self.store.clone();
        let mcp = self.mcp.clone();
        self.saving_llm = llm.any();
        self.saving_prefs = prefs.any();
        self.saving_store = store.is_write();
        self.saving_mcp = mcp.any();
        PanelEffect::Save {
            llm: Box::new(llm),
            prefs: Box::new(prefs),
            secret,
            store,
            mcp: Box::new(mcp),
        }
    }
}

/// 面板显示所需的三层上下文（override / 持久层 / serve 默认）。
pub struct PanelCtx<'a> {
    pub overrides: &'a SessionPrefs,
    pub persisted: Option<&'a PersistedSettings>,
    pub serve_defaults: Option<&'a ServeDefaults>,
    /// 面板配色：值列色调与提示级别都靠它解析，面板自己不再挑颜色。
    pub pal: &'a Palette,
}

impl PanelCtx<'_> {
    /// 字段的三层来源（override、user-data、serve 默认；空白由合成时归一），供合成/预填共用。
    /// 温度/上下文/思考/工具缓存/主题无本地 override（TUI 无斜杠）与 serve 默认；API 密钥不参与
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
            | FieldId::Theme
            | FieldId::ApiKey
            | FieldId::SessionSqlite
            | FieldId::McpGlobalEnabled
            | FieldId::McpToolTimeout
            | FieldId::McpServers
            | FieldId::ConnModel
            | FieldId::ConnPermission
            | FieldId::KeyGlobal
            | FieldId::KeyChat
            | FieldId::KeySettings
            | FieldId::KeyEditor
            | FieldId::LocalSecret
            | FieldId::LocalSave
            | FieldId::LocalMarkdown => None,
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
            FieldId::Theme => p.and_then(|p| p.theme.as_deref()),
            // 会话存储不是 user-data 键：没有持久层这一层。
            FieldId::SessionSqlite => None,
            // MCP 全局开关：值在 user-data 的 mcp-servers 文件里（布尔），映射成 on/off。
            FieldId::McpGlobalEnabled => p
                .and_then(|p| p.mcp_global_enabled)
                .map(|on| if on { STORE_ON } else { STORE_OFF }),
            // 超时要显示「60 秒」这类数值文本，`sources` 只给 `&str`，所以在 `mcp_timeout_cell`
            // 里直接读持久层；服务器数量同理（只读信息行）。
            FieldId::McpToolTimeout | FieldId::McpServers => None,
            // 连接 / 快捷键 / 本地行为三栏整栏只读：值本身就是说明（`draw` 里直接取，
            // 连接那两行还要读 `/status` 与 `/permission`，不在三层合成里）。
            FieldId::ConnModel
            | FieldId::ConnPermission
            | FieldId::KeyGlobal
            | FieldId::KeyChat
            | FieldId::KeySettings
            | FieldId::KeyEditor
            | FieldId::LocalSecret
            | FieldId::LocalSave
            | FieldId::LocalMarkdown => None,
        };
        let remote = match field {
            FieldId::Model => d.and_then(|d| d.model.as_deref()),
            // API Base / 温度 / 上下文 / 思考 / 工具缓存 / 主题没有 /status 默认来源：三层只到 user-data。
            FieldId::ApiBase
            | FieldId::Temperature
            | FieldId::ContextTokens
            | FieldId::ThinkingMode
            | FieldId::ToolCache
            | FieldId::Theme
            | FieldId::ApiKey => None,
            // 会话存储反过来：`/status` 的布尔是唯一来源，没有 override / user-data 层。
            FieldId::SessionSqlite => d
                .and_then(|d| d.store_sqlite_active())
                .map(|on| if on { STORE_ON } else { STORE_OFF }),
            // MCP 两项的当前值在 user-data（不是 `/status`）；服务器数量是只读信息行。
            FieldId::McpGlobalEnabled | FieldId::McpToolTimeout | FieldId::McpServers => None,
            FieldId::ConnModel
            | FieldId::ConnPermission
            | FieldId::KeyGlobal
            | FieldId::KeyChat
            | FieldId::KeySettings
            | FieldId::KeyEditor
            | FieldId::LocalSecret
            | FieldId::LocalSave
            | FieldId::LocalMarkdown => None,
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

    /// 会话存储开关是否可编辑：该 serve 给了当前值，且配置了 SQLite 路径。
    fn store_toggle_supported(&self) -> bool {
        self.serve_defaults
            .is_some_and(ServeDefaults::store_toggle_supported)
    }

    /// 二值开关这一行是否可编辑。
    ///
    /// 两个前提：**知道当前值**（否则「切到另一边」无从表达），且（仅会话存储）服务端
    /// 配置了 SQLite 路径 —— 否则 `sqlite: true` 必被拒。
    fn toggle_editable(&self, field: FieldId) -> bool {
        if self.effective(field).value.is_none() {
            return false;
        }
        field != FieldId::SessionSqlite || self.store_toggle_supported()
    }
}

/// 二值开关被拒时的说明：两种原因（服务端不支持 / 值未知）要分开讲，因为处置不同
/// —— 前者只能换 serve，后者等 `/status` 拉回来就好。
fn toggle_blocked_note(ctx: &PanelCtx<'_>, field: FieldId) -> String {
    if field == FieldId::SessionSqlite {
        return "该 serve 不支持切换会话存储（未配置 SQLite 路径或版本过旧）".to_string();
    }
    // MCP 的「没有值」有两种成因，处置完全不同：端点不存在（换 serve 才行）与这次拉取
    // 失败（重开面板会重拉，见 `LaneApp::open_settings`）。说反了用户就会白跑一趟换 serve。
    if ctx.persisted.is_some_and(|p| p.mcp_fetch_failed) {
        "MCP 配置这次没拉到（网络/解码失败）；重开面板会重试".to_string()
    } else {
        "该 serve 未提供 MCP 配置（版本过旧）".to_string()
    }
}

/// 面板一帧内容（render 只负责把它画到全屏设置页里）。
///
/// 字段按 desktop 设置页的四区骨架拆分：顶栏（[`Self::title`] / [`Self::badge`]）、
/// 左侧导航（[`Self::nav`]）、主内容（[`Self::lines`] / [`Self::cursor`]）、
/// 贴底底栏（[`Self::footer`]）。
#[derive(Default)]
pub struct PanelContent {
    pub lines: Vec<Line<'static>>,
    /// 文本编辑光标：内容行下标 + cell 列（None = 不设硬件光标）。
    pub cursor: Option<(usize, usize)>,
    /// 顶栏标题（当前分区名）。
    pub title: String,
    /// 顶栏状态徽标（未保存 / 保存中 / 只读）；`None` = 不显示。
    pub badge: Option<String>,
    /// 左侧导航项：`(分区名, 是否选中)`，按 [`Section::ALL`] 顺序排列。
    pub nav: Vec<(String, bool)>,
    /// 贴底底栏（状态提示 + 按键说明）。
    ///
    /// 必须与 `lines` 分开：字段行只有六七行，混在一起会让底栏飘在屏幕中间，
    /// 而 desktop 底栏是贴着页面底部固定的。
    pub footer: Vec<Line<'static>>,
}

/// 把字符串按 cell 补齐到至少 `cells`（超出则原样返回）。
fn pad_cells(s: &str, cells: usize) -> String {
    let w = UnicodeWidthStr::width(s);
    let pad = cells.saturating_sub(w);
    format!("{s}{}", " ".repeat(pad))
}

// 选项表 / 枚举下标换算的纯函数层（单独成文件压行数门禁）。重导出让 `draw` /
// `app` / 各测试子模块的 `use super::*` 依旧能拿到 `mode_options` 这些名字。
#[path = "settings_panel_schema.rs"]
mod schema;
pub(crate) use self::schema::*;

// 编辑态（`Editing` / 按键分派 / 提交校验）独立成文件：压行数门禁。
#[path = "settings_panel_edit.rs"]
mod edit;
pub(crate) use self::edit::Editing;

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

// W2 收尾字段（上下文长度 / 只读工具缓存 / 网关预设）单测独立成文件（同上）。
#[cfg(test)]
#[path = "settings_panel_w3_tests.rs"]
mod tests_w3;

// 外观分区（主题）单测独立成文件（同上）。
#[cfg(test)]
#[path = "settings_panel_appearance_tests.rs"]
mod tests_appearance;

// 会话存储（SQLite）开关单测独立成文件：它是第三组保存（非 user-data 快照）。
#[cfg(test)]
#[path = "settings_panel_store_tests.rs"]
mod tests_store;

// MCP 分区单测独立成文件（同上）：只读信息行 + 两个标量。
#[cfg(test)]
#[path = "settings_panel_mcp_tests.rs"]
mod tests_mcp;

// 只读三栏（连接 / 快捷键 / 本地行为）单测独立成文件（同上）。
#[cfg(test)]
#[path = "settings_panel_local_tests.rs"]
mod tests_local;
