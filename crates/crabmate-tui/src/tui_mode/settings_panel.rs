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
    effective_value, normalize_str, validate_api_base,
};

/// 会话模式编辑选项长度 =「跟随 server」+ [`SESSION_MODES`]。
const MODE_OPTIONS_LEN: usize = SESSION_MODES.len() + 1;

/// 会话模式编辑选项（`None` = 跟随 server，即清除存储键）；取值以 [`SESSION_MODES`] 为单一来源。
fn mode_options() -> [Option<&'static str>; MODE_OPTIONS_LEN] {
    let mut out = [None; MODE_OPTIONS_LEN];
    for (i, m) in SESSION_MODES.iter().copied().enumerate() {
        out[i + 1] = Some(m);
    }
    out
}
/// 字段行标签列宽（cell；含标签与值之间间隔）。
const LABEL_PAD: usize = 13;

/// F2：设置面板开关（与 Esc 同语义地关闭；空闲时由 mod 层打开）。
pub(super) fn is_f2_key(key: &KeyEvent) -> bool {
    key.code == KeyCode::F(2) && !key.modifiers.contains(KeyModifiers::CONTROL)
}

/// 面板分区：模型 / 会话（W1 只管理这两个分区的扁平字段）。
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
            Section::Llm => &[FieldId::Model, FieldId::ApiBase],
            Section::Session => &[FieldId::Role, FieldId::SessionMode],
        }
    }
}

/// W1 管理的四个字段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldId {
    Model,
    ApiBase,
    Role,
    SessionMode,
}

const fn field_label(field: FieldId) -> &'static str {
    match field {
        FieldId::Model => "模型名",
        FieldId::ApiBase => "API Base",
        FieldId::Role => "Agent role",
        FieldId::SessionMode => "会话模式",
    }
}

/// 保存结果属于哪一组（llm-overrides / prefs）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveGroup {
    Llm,
    Prefs,
}

/// 行编辑状态：文本编辑（含缓冲与光标）或会话模式枚举循环。
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
}

/// 编辑态按键动作（先在不可变借用下解析，再执行以避开自引用借用）。
enum EditAction {
    Cancel,
    CommitText,
    CommitMode,
    Insert(char),
    Backspace,
    MoveLeft,
    MoveRight,
}

/// 面板按键处理结果（状态已内部更新；副作用交给调用方）。
#[derive(Debug)]
pub(super) enum PanelEffect {
    None,
    /// 请求保存（两组可能各有 ≥1 个 Write；空组由调用方跳过）。
    Save {
        llm: LlmSave,
        prefs: PrefsSave,
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

    /// 是否有未保存的 staged 改动。
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.llm.any() || self.prefs.any()
    }

    fn current_field(&self) -> FieldId {
        self.section.fields()[self.row]
    }

    fn staged(&self, field: FieldId) -> &FieldAction {
        match field {
            FieldId::Model => &self.llm.model,
            FieldId::ApiBase => &self.llm.api_base,
            FieldId::Role => &self.prefs.role,
            FieldId::SessionMode => &self.prefs.session_mode,
        }
    }

    fn set_staged(&mut self, field: FieldId, action: FieldAction) {
        match field {
            FieldId::Model => self.llm.model = action,
            FieldId::ApiBase => self.llm.api_base = action,
            FieldId::Role => self.prefs.role = action,
            FieldId::SessionMode => self.prefs.session_mode = action,
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

    /// Enter：进入当前字段的编辑（文本缓冲预填生效值；会话模式进枚举循环）。
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
            None => None,
        };
        let Some(action) = action else {
            return;
        };
        match action {
            EditAction::Cancel => self.editing = None,
            EditAction::CommitText => self.commit_text(),
            EditAction::CommitMode => self.commit_mode(),
            EditAction::Insert(c) => self.edit_insert(c),
            EditAction::Backspace => self.edit_backspace(),
            EditAction::MoveLeft => self.edit_move(true),
            EditAction::MoveRight => self.edit_move(false),
        }
    }

    /// 文本字段提交：trim 后空串 = 清除；API Base 需 http(s):// 前缀（非法则留在编辑态提示）。
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

    /// 编辑态 ←/→：文本字段移动光标；会话模式枚举循环选项（↑↓ 不动）。
    fn edit_move(&mut self, left: bool) {
        match &mut self.editing {
            Some(Editing::Text { buf, cursor, .. }) => {
                if left {
                    *cursor = cursor.saturating_sub(1);
                } else if *cursor < buf.len() {
                    *cursor += 1;
                }
            }
            Some(Editing::Mode { pick }) => {
                if left {
                    *pick = pick.saturating_sub(1);
                } else if *pick + 1 < mode_options().len() {
                    *pick += 1;
                }
            }
            None => {}
        }
    }

    /// S 保存：staged 全空时提示；否则标记在途并返回保存请求（含两组暂存动作）。
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
        self.saving_llm = llm.any();
        self.saving_prefs = prefs.any();
        PanelEffect::Save { llm, prefs }
    }
}

/// 会话模式选项下标 → 值（按 [`SESSION_MODES`] 顺序；其余（含 None）回 0 = 跟随 server）。
fn mode_index_of(v: Option<&str>) -> usize {
    v.and_then(|m| SESSION_MODES.iter().position(|x| *x == m))
        .map_or(0, |i| i + 1)
}

/// 面板显示所需的三层上下文（override / 持久层 / serve 默认）。
pub struct PanelCtx<'a> {
    pub overrides: &'a SessionPrefs,
    pub persisted: Option<&'a PersistedSettings>,
    pub serve_defaults: Option<&'a ServeDefaults>,
}

impl PanelCtx<'_> {
    /// 字段的三层来源（override、user-data、serve 默认；空白由合成时归一），供合成/预填共用。
    fn sources(&self, field: FieldId) -> (Option<&str>, Option<&str>, Option<&str>) {
        let p = self.persisted;
        let d = self.serve_defaults;
        let local = match field {
            FieldId::Model => self.overrides.model.as_deref(),
            FieldId::ApiBase => self.overrides.api_base.as_deref(),
            FieldId::Role => self.overrides.agent_role.as_deref(),
            FieldId::SessionMode => self.overrides.session_mode.as_deref(),
        };
        let stored = match field {
            FieldId::Model => p.and_then(|p| p.model.as_deref()),
            FieldId::ApiBase => p.and_then(|p| p.api_base.as_deref()),
            FieldId::Role => p.and_then(|p| p.role.as_deref()),
            FieldId::SessionMode => p.and_then(|p| p.session_mode.as_deref()),
        };
        let remote = match field {
            FieldId::Model => d.and_then(|d| d.model.as_deref()),
            // API Base 没有 /status 默认来源：三层只到 user-data。
            FieldId::ApiBase => None,
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

impl SettingsPanel {
    /// 构建一帧面板内容：分区导航 + 标记说明 + 字段行 + 底栏。
    pub fn content(&self, ctx: &PanelCtx<'_>, width: usize) -> PanelContent {
        let mut lines: Vec<Line<'static>> = Vec::new();
        lines.push(nav_line(self, width));
        lines.push(Line::from(Span::styled(
            truncate_display(
                "标记：* = override · ~ = 未保存 · 留空保存 = 跟随 server",
                width,
            ),
            Style::new().fg(Color::DarkGray),
        )));
        let mut cursor = None;
        for (i, field) in self.section.fields().iter().enumerate() {
            let selected = i == self.row;
            let (line, edit_col) = self.field_line(ctx, *field, width, selected);
            if let Some(col) = edit_col {
                cursor = Some((lines.len(), col));
            }
            lines.push(line);
        }
        // 字段行与底栏之间留一空行。
        lines.push(Line::from(""));
        lines.extend(footer_lines(self, width));
        PanelContent { lines, cursor }
    }

    /// 单字段行：标签 + 值（编辑中的文本字段显示缓冲并给出光标列）。
    fn field_line(
        &self,
        ctx: &PanelCtx<'_>,
        field: FieldId,
        width: usize,
        selected: bool,
    ) -> (Line<'static>, Option<usize>) {
        let label = pad_cells(field_label(field), LABEL_PAD);
        let area = width.saturating_sub(LABEL_PAD);
        let base = if selected {
            Style::new().add_modifier(Modifier::REVERSED)
        } else {
            Style::new()
        };
        let label_span = Span::styled(label.clone(), base.fg(Color::Gray));
        if let Some(Editing::Text {
            field: ef,
            buf,
            cursor,
        }) = &self.editing
            && *ef == field
        {
            let full: String = buf.iter().collect();
            let prefix: String = buf[..*cursor].iter().collect();
            let cursor_cell = UnicodeWidthStr::width(prefix.as_str());
            let (hstart, shown_cursor) = horizontal_window(&full, cursor_cell, area);
            let visible = cell_window(&full, hstart, area);
            let value_span = Span::styled(visible, base.fg(Color::White));
            let line = Line::from(vec![label_span, value_span]);
            return (line, Some(LABEL_PAD + shown_cursor));
        }
        let (text, color) = self.value_cell(ctx, field);
        let visible = truncate_display(&text, area);
        let value_span = Span::styled(visible, base.fg(color));
        (Line::from(vec![label_span, value_span]), None)
    }

    /// 字段值列文本与颜色：staged（~）＞ override（*）＞ user-data ＞ serve 默认 ＞ 跟随 server。
    fn value_cell(&self, ctx: &PanelCtx<'_>, field: FieldId) -> (String, Color) {
        match self.staged(field) {
            FieldAction::Write(Some(v)) => (format!("{v}~"), Color::LightYellow),
            FieldAction::Write(None) => {
                // 清除后回落：显示 serve 默认（若有）。
                let (_, _, remote) = ctx.sources(field);
                match remote.and_then(normalize_str) {
                    Some(v) => (format!("{v}~"), Color::LightYellow),
                    None => ("(跟随 server)~".to_string(), Color::LightYellow),
                }
            }
            FieldAction::Skip => match ctx.effective(field) {
                EffectiveView {
                    layer: Layer::Override,
                    value,
                } => (format!("{}*", value.unwrap_or_default()), Color::LightCyan),
                EffectiveView {
                    layer: Layer::Stored,
                    value,
                } => (value.unwrap_or_default(), Color::White),
                EffectiveView {
                    layer: Layer::Default,
                    value,
                } => (value.unwrap_or_default(), Color::Gray),
                EffectiveView {
                    layer: Layer::Follow,
                    value: _,
                } => ("(跟随 server)".to_string(), Color::DarkGray),
            },
        }
    }
}

/// 分区导航（选中项高亮反色）。
fn nav_line(panel: &SettingsPanel, width: usize) -> Line<'static> {
    let mut parts = vec![Span::styled("分区：", Style::new().fg(Color::DarkGray))];
    for section in Section::ALL {
        let selected = section == panel.section;
        let mark = if selected { "▸ " } else { "  " };
        let text = format!("{mark}{}   ", section.label());
        let style = if selected {
            Style::new()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD | Modifier::REVERSED)
        } else {
            Style::new().fg(Color::DarkGray)
        };
        parts.push(Span::styled(truncate_display(&text, width), style));
    }
    Line::from(parts)
}

/// 面板底栏两行：动态提示 + 按键说明（每行文案各自独立成函数，控制 CCN ≤ 10）。
fn footer_lines(panel: &SettingsPanel, width: usize) -> Vec<Line<'static>> {
    let hint = footer_hint(panel);
    let (msg, mcolor) = footer_msg(panel);
    let mut out = vec![Line::from(Span::styled(
        truncate_display(&msg, width),
        Style::new().fg(mcolor),
    ))];
    out.push(Line::from(Span::styled(
        truncate_display(&hint, width),
        Style::new().fg(Color::DarkGray),
    )));
    out
}

/// 底栏按键说明（按状态取一行）。
fn footer_hint(panel: &SettingsPanel) -> String {
    if panel.confirm_close {
        return "[y] 放弃并关闭  [Esc] 返回".to_string();
    }
    if panel.is_saving() {
        return String::new();
    }
    match &panel.editing {
        Some(Editing::Mode { .. }) => "[←/→] 循环  [Enter] 确定  [Esc] 取消".to_string(),
        Some(Editing::Text { .. }) => "[Enter] 确定  [Esc] 取消编辑".to_string(),
        None if panel.read_only => "[↑↓] 浏览  [Tab] 分区  [Esc/F2] 关闭".to_string(),
        None => "[↑↓] 移动  [Enter] 编辑  [Tab] 分区  [S] 保存  [Esc/F2] 关闭".to_string(),
    }
}

/// 底栏动态提示（模式循环预览 / 编辑引导 / 浏览态状态）。
fn footer_msg(panel: &SettingsPanel) -> (String, Color) {
    match &panel.editing {
        Some(Editing::Mode { pick }) => mode_cycle_text(*pick),
        Some(Editing::Text { field, .. }) => match &panel.note {
            Some((note, color)) => (note.clone(), *color),
            None => (
                format!("编辑「{}」：输入 · Backspace · ←→", field_label(*field)),
                Color::Gray,
            ),
        },
        None => browse_msg(panel),
    }
}

/// 会话模式 ←→ 循环的可视化预览。
fn mode_cycle_text(pick: usize) -> (String, Color) {
    let mut text = String::from("会话模式：");
    for (i, opt) in mode_options().iter().enumerate() {
        if i == pick {
            text.push('▸');
        }
        text.push_str(opt.unwrap_or("(跟随 server)"));
        text.push(' ');
    }
    (text, Color::Gray)
}

/// 浏览态提示行（非编辑、非模式循环时）。
fn browse_msg(panel: &SettingsPanel) -> (String, Color) {
    if panel.confirm_close {
        return ("未保存改动：y 放弃 / Esc 返回".to_string(), Color::Yellow);
    }
    if panel.is_saving() {
        return ("保存中…".to_string(), Color::LightCyan);
    }
    if let Some((note, color)) = &panel.note {
        return (note.clone(), *color);
    }
    if panel.read_only {
        return (
            "回合进行中：只读（结束后自动解锁）".to_string(),
            Color::LightYellow,
        );
    }
    if panel.is_dirty() {
        return ("未保存改动：S 保存".to_string(), Color::LightYellow);
    }
    ("留空保存 = 清除（跟随 server）".to_string(), Color::Gray)
}

#[path = "settings_app.rs"]
mod app;

#[cfg(test)]
#[path = "settings_panel_tests.rs"]
mod tests;
