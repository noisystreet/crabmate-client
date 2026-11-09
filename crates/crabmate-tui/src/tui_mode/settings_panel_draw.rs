//! 设置面板内容构建与行渲染（content / field_line / 值列 / 导航 / 底栏文案）。
//!
//! `settings_panel.rs` 的 `#[path]` 子模块：压住面板文件行数门禁。逻辑与测试仍
//! 在父模块组装，这里直接 `use super::*;` 访问父模块私有项。

use super::super::hotkeys::{self, HotkeyGroup};
use super::super::palette::FieldTone;
use super::super::settings_panel_env::{self as env, EnvSwitch};
use super::super::text::wrap_display;
use super::*;
use crate::serve::BearerSource;

/// 主题「未设置」在面板里的显示名（下拉项、字段值、底栏共用一份措辞）。
const THEME_NONE_LABEL: &str = "默认（透明）";

impl SettingsPanel {
    /// 构建一帧面板内容：说明行 + 字段行 + 顶栏/导航/底栏元数据。
    pub fn content(&self, ctx: &PanelCtx<'_>, width: usize) -> PanelContent {
        let mut lines: Vec<Line<'static>> = Vec::new();
        // 行归属与 `lines` 一一对应：鼠标按「内容区第 r 行」查它，两边一旦不齐就会
        // 点一行选中相邻那行。说明行也占一格，只是不归属字段。
        let mut row_owner: Vec<RowOwner> = Vec::new();
        // 分区说明行：对应 desktop 内容区标题下的 `settings-content-desc`。
        // 只读三栏不能沿用「留空保存 = 跟随 server」——它们根本没有可保存的值。
        lines.push(Line::from(Span::styled(
            truncate_display(section_hint(self.section), width),
            ctx.pal.tone_style(FieldTone::Unset),
        )));
        row_owner.push(RowOwner::Prose);
        let mut cursor = None;
        for (i, field) in self.section.fields().iter().enumerate() {
            let selected = i == self.row;
            let (rows, edit_col) = self.field_lines(ctx, *field, width, selected);
            if let Some(col) = edit_col {
                // 编辑态永远是单行，光标落在这一组的首行。
                cursor = Some((lines.len(), col));
            }
            // 折出来的续行也算这个字段（`part` 只用于区分首行/续行）。
            row_owner.extend((0..rows.len()).map(|part| RowOwner::Field {
                field: *field,
                part,
            }));
            lines.extend(rows);
            // 主题 9 个候选、也全是展示用选项：编辑中就在行下方把列表摊开，
            // 比在底栏读一行「当前循环到哪」再连按 ←→ 直观。
            if *field == FieldId::Theme {
                let dropdown = self.theme_dropdown(width, ctx.pal);
                row_owner.extend(dropdown.iter().map(|(_, owner)| *owner));
                lines.extend(dropdown.into_iter().map(|(line, _)| line));
            }
        }
        PanelContent {
            lines,
            row_owner,
            cursor,
            title: self.section.label().to_string(),
            badge: settings_badge(self),
            nav: nav_items(self),
            // 底栏单独返回：绘制侧把它贴到页面底部，而不是跟字段行一起往上堆。
            footer: footer_lines(self, width, ctx.pal),
        }
    }

    /// 单字段行：标签 + 值。
    ///
    /// 只读说明行可能**折成多行**：80 列终端上这栏的值就是事实（键位、开关的生效值），
    /// 被 `truncate_display` 截掉尾部等于丢事实（#433）。文本编辑态仍是一行，
    /// 并返回光标列。
    fn field_lines(
        &self,
        ctx: &PanelCtx<'_>,
        field: FieldId,
        width: usize,
        selected: bool,
    ) -> (Vec<Line<'static>>, Option<usize>) {
        let label = pad_cells(field_label(field), LABEL_PAD);
        let area = width.saturating_sub(LABEL_PAD);
        let base = if selected {
            Style::new().add_modifier(Modifier::REVERSED)
        } else {
            Style::new()
        };
        let label_style = base.patch(ctx.pal.tone_style(FieldTone::Muted));
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
            let line = Line::from(vec![
                Span::styled(label, label_style),
                Span::styled(visible, base.patch(ctx.pal.tone_style(FieldTone::Value))),
            ]);
            return (vec![line], Some(LABEL_PAD + shown_cursor));
        }
        let (text, tone) = self.value_cell(ctx, field);
        let value_style = base.patch(ctx.pal.tone_style(tone));
        let rows = wrap_note(&text, area)
            .into_iter()
            .enumerate()
            .map(|(i, chunk)| {
                // 续行把标签列留白：读者一眼看出这是同一行的续行，而不是新字段。
                let head = if i == 0 {
                    Span::styled(label.clone(), label_style)
                } else {
                    Span::styled(" ".repeat(LABEL_PAD), label_style)
                };
                Line::from(vec![head, Span::styled(chunk, value_style)])
            })
            .collect();
        (rows, None)
    }

    /// 主题下拉列表：仅在主题行处于编辑态时非空。
    ///
    /// 对齐到标签列之后，当前项反色 + `▸`，其余灰字；`None` 显示 [`THEME_NONE_LABEL`]，
    /// 与 `theme_cell` 用同一份措辞，免得同一样东西两种叫法。
    ///
    /// 候选多于 [`THEME_DROPDOWN_MAX_ROWS`] 时开窗滚动，窗口跟着 `pick` 走：绘制侧
    /// `render_settings_body` 是 `take(inner.height)` 硬裁，全铺的话小终端里被裁掉的
    /// 正是尾部，选中项可能整个消失 —— 而那条路径没有任何滚动机制能救它。
    /// 返回 `(行, 归属)`：归属让鼠标能点到「第几个候选」，省略号行标 `Prose`
    /// （点它没有动作）。两样在同一处产出，省得鼠标侧重算窗口边界。
    fn theme_dropdown(&self, width: usize, pal: &Palette) -> Vec<(Line<'static>, RowOwner)> {
        let Some(Editing::Theme { pick }) = &self.editing else {
            return Vec::new();
        };
        let total = THEME_OPTIONS.len();
        let (start, end) = dropdown_window(*pick, total, THEME_DROPDOWN_MAX_ROWS);
        let indent = " ".repeat(LABEL_PAD + 2);
        let mut out = Vec::with_capacity(end - start + 2);
        if start > 0 {
            let line = dropdown_ellipsis(&indent, start, width, pal);
            out.push((line, RowOwner::Prose));
        }
        for i in start..end {
            let line = theme_option_line(&indent, i, i == *pick, width, pal);
            out.push((line, RowOwner::ThemeOption(i)));
        }
        if end < total {
            let line = dropdown_ellipsis(&indent, total - end, width, pal);
            out.push((line, RowOwner::Prose));
        }
        out
    }

    /// 字段值列文本与颜色：staged（~）＞ override（*）＞ user-data ＞ serve 默认 ＞ 跟随 server。
    ///
    /// 先分派「有自己一套文案」的字段（密钥 / 工具缓存 / 主题 / 会话存储 / MCP），其余
    /// 统一走 [`Self::layered_cell`]。拆成两跳是为了 CCN 门禁：一行一个 `if` 加回
    /// 三层 `match` 会直接越过上限。
    fn value_cell(&self, ctx: &PanelCtx<'_>, field: FieldId) -> (String, FieldTone) {
        if let Some(cell) = self.readonly_cell(ctx, field) {
            return cell;
        }
        if let Some(cell) = self.special_cell(ctx, field) {
            return cell;
        }
        self.layered_cell(ctx, field)
    }

    /// 只读行的值：连接栏读本连接 / serve 事实，快捷键栏由键位表生成，本地行为栏
    /// 读 `CM_TUI_*` 的生效值与行为事实。`None` = 不是只读行。
    fn readonly_cell(&self, ctx: &PanelCtx<'_>, field: FieldId) -> Option<(String, FieldTone)> {
        if let Some(cell) = conn_cell(ctx, field) {
            return Some(cell);
        }
        // 快捷键：值就是 `hotkeys` 表里那一组条目（新加热键不必改这里的代码）。
        if let Some(group) = shortcut_group(field) {
            return Some((hotkeys::group_line(group), FieldTone::Value));
        }
        // `CM_TUI_*` 开关：值是运行时判定函数的结论，不是这里再解析一遍环境变量。
        if let Some(switch) = EnvSwitch::of(field) {
            return Some((env::row_value(switch), FieldTone::Value));
        }
        let d = ctx.serve_defaults;
        Some(match field {
            FieldId::McpServers => self.mcp_servers_cell(ctx),
            FieldId::ConnModel => serve_fact(d.and_then(|d| d.model.clone())),
            FieldId::ConnPermission => {
                serve_fact(d.and_then(|d| d.effective_level().map(str::to_string)))
            }
            FieldId::LocalSecret => prose_cell(LOCAL_SECRET_NOTE),
            FieldId::LocalSave => prose_cell(LOCAL_SAVE_NOTE),
            FieldId::LocalMarkdown => prose_cell(LOCAL_MARKDOWN_NOTE),
            _ => return None,
        })
    }

    /// 有独立文案的字段（密钥 / 工具缓存 / 主题 / 会话存储 / MCP 两项）。`None` = 走三层合成。
    fn special_cell(&self, ctx: &PanelCtx<'_>, field: FieldId) -> Option<(String, FieldTone)> {
        Some(match field {
            FieldId::ApiKey => self.secret_cell(),
            FieldId::ToolCache => self.tool_cache_cell(ctx),
            FieldId::Theme => self.theme_cell(ctx),
            FieldId::SessionSqlite => self.store_cell(ctx),
            FieldId::McpGlobalEnabled => self.mcp_enabled_cell(ctx),
            FieldId::McpToolTimeout => self.mcp_timeout_cell(ctx),
            _ => return None,
        })
    }

    /// 三层合成值（override ＞ user-data ＞ serve 默认）；空白一律视为未设置。
    fn layered_cell(&self, ctx: &PanelCtx<'_>, field: FieldId) -> (String, FieldTone) {
        match self.staged(field) {
            FieldAction::Write(Some(v)) => {
                (format!("{}~", value_text(field, v)), FieldTone::Pending)
            }
            FieldAction::Write(None) => {
                // 清除后回落：显示 serve 默认（若有）。
                let (_, _, remote) = ctx.sources(field);
                match remote.and_then(normalize_str) {
                    Some(v) => (format!("{}~", value_text(field, &v)), FieldTone::Pending),
                    None => ("(跟随 server)~".to_string(), FieldTone::Pending),
                }
            }
            FieldAction::Skip => match ctx.effective(field) {
                EffectiveView {
                    layer: Layer::Override,
                    value,
                } => (
                    format!("{}*", value_text(field, &value.unwrap_or_default())),
                    FieldTone::Override,
                ),
                EffectiveView {
                    layer: Layer::Stored,
                    value,
                } => (
                    value_text(field, &value.unwrap_or_default()),
                    FieldTone::Value,
                ),
                EffectiveView {
                    layer: Layer::Default,
                    value,
                } => (
                    value_text(field, &value.unwrap_or_default()),
                    FieldTone::Muted,
                ),
                EffectiveView {
                    layer: Layer::Follow,
                    value: _,
                } => ("(跟随 server)".to_string(), FieldTone::Unset),
            },
        }
    }

    /// 只读工具缓存行值：staged（~）＞ user-data 禁用（白字）＞ 开/跟随 server。
    fn tool_cache_cell(&self, ctx: &PanelCtx<'_>) -> (String, FieldTone) {
        match self.staged(FieldId::ToolCache) {
            FieldAction::Write(Some(_)) => ("关（禁用缓存）~".to_string(), FieldTone::Pending),
            FieldAction::Write(None) => ("开（跟随 server）~".to_string(), FieldTone::Pending),
            FieldAction::Skip => match ctx.effective(FieldId::ToolCache) {
                EffectiveView {
                    layer: Layer::Stored,
                    value: Some(_),
                } => ("关（禁用缓存）".to_string(), FieldTone::Value),
                _ => ("开（跟随 server）".to_string(), FieldTone::Unset),
            },
        }
    }

    /// 主题行值：staged（~）＞ user-data 已存 slug ＞ 默认（透明）。
    ///
    /// 空值不叫「跟随 server」：`prefs.theme` 缺失时 TUI 回落到自己的产品默认
    /// （透明底色），与 serve 配置无关，写成「跟随 server」会误导。
    fn theme_cell(&self, ctx: &PanelCtx<'_>) -> (String, FieldTone) {
        match self.staged(FieldId::Theme) {
            FieldAction::Write(Some(v)) => (format!("{v}~"), FieldTone::Pending),
            FieldAction::Write(None) => (format!("{THEME_NONE_LABEL}~"), FieldTone::Pending),
            FieldAction::Skip => match ctx.effective(FieldId::Theme).value {
                Some(v) => (v, FieldTone::Value),
                None => (THEME_NONE_LABEL.to_string(), FieldTone::Unset),
            },
        }
    }

    /// 会话存储（SQLite）行值：staged（~）＞ `/status` 当前态 ＞ 「该 serve 未提供」。
    ///
    /// 未知不显示成「关」：那会让用户以为面板知道当前状态，而按下去要动的是另一个值。
    fn store_cell(&self, ctx: &PanelCtx<'_>) -> (String, FieldTone) {
        match self.staged(FieldId::SessionSqlite) {
            FieldAction::Write(Some(v)) => {
                (format!("{}~", store_label(Some(v))), FieldTone::Pending)
            }
            _ => match ctx.effective(FieldId::SessionSqlite).value {
                Some(v) => (store_label(Some(&v)).to_string(), FieldTone::Value),
                None => ("(该 serve 未提供)".to_string(), FieldTone::Unset),
            },
        }
    }

    /// MCP 全局开关行值：staged（~）＞ mcp-servers 文件里的布尔 ＞ 「该 serve 未提供」。
    fn mcp_enabled_cell(&self, ctx: &PanelCtx<'_>) -> (String, FieldTone) {
        match self.staged(FieldId::McpGlobalEnabled) {
            FieldAction::Write(Some(v)) => {
                (format!("{}~", toggle_label(Some(v))), FieldTone::Pending)
            }
            _ => match ctx.effective(FieldId::McpGlobalEnabled).value {
                Some(v) => (toggle_label(Some(&v)).to_string(), FieldTone::Value),
                None => ("(该 serve 未提供)".to_string(), FieldTone::Unset),
            },
        }
    }

    /// MCP 工具超时行值：staged（~）＞ 持久层秒数 ＞ 「该 serve 未提供」。
    fn mcp_timeout_cell(&self, ctx: &PanelCtx<'_>) -> (String, FieldTone) {
        match self.staged(FieldId::McpToolTimeout) {
            FieldAction::Write(Some(v)) => (format!("{v} 秒~"), FieldTone::Pending),
            // `commit_text` 已把留空提交落成 `Skip`，这一支只在外部构造时可达；仍不带
            // `~`：它的语义是「不改」，不是「待保存」。
            FieldAction::Write(None) => ("(不改)".to_string(), FieldTone::Unset),
            FieldAction::Skip => match ctx.persisted.and_then(|p| p.mcp_tool_timeout_secs) {
                Some(s) => (format!("{s} 秒"), FieldTone::Value),
                None => ("(该 serve 未提供)".to_string(), FieldTone::Unset),
            },
        }
    }

    /// MCP 服务器**只读**信息行：只报数量，编辑入口指向 Desktop/Web。
    fn mcp_servers_cell(&self, ctx: &PanelCtx<'_>) -> (String, FieldTone) {
        match ctx
            .persisted
            .and_then(|p| p.mcp_server_count.zip(p.mcp_enabled_count))
        {
            Some((total, on)) => (
                format!("{total} 个（{on} 启用）· 在 Desktop/Web 编辑"),
                FieldTone::Value,
            ),
            None => ("(该 serve 未提供)".to_string(), FieldTone::Unset),
        }
    }

    /// API 密钥行值：staged 写 = 掩码 + `~`；staged 清除 = `清除~`；未编辑按已设态显示。
    fn secret_cell(&self) -> (String, FieldTone) {
        match &self.secret {
            FieldAction::Write(Some(v)) => (
                format!("••••（{} 字符）~", v.chars().count()),
                FieldTone::Pending,
            ),
            FieldAction::Write(None) => ("清除~".to_string(), FieldTone::Pending),
            FieldAction::Skip if self.secret_set => {
                ("已设（钥匙串）".to_string(), FieldTone::Value)
            }
            FieldAction::Skip => ("未设（跟随 serve）".to_string(), FieldTone::Unset),
        }
    }
}

/// 只读行拿不到 serve 值时的统一落款（不假装知道）。
fn serve_fact(v: Option<String>) -> (String, FieldTone) {
    match v {
        Some(t) => (t, FieldTone::Value),
        None => ("(该 serve 未提供)".to_string(), FieldTone::Unset),
    }
}

/// 本地行为栏的三条行为事实（不是设置项，也没有可写的落点）。
const LOCAL_SECRET_NOTE: &str = "API 密钥只进本机钥匙串（不写 serve user-data）";
const LOCAL_SAVE_NOTE: &str = "保存 = 先 GET、再改自己的键、再全量 PUT（不覆盖 Desktop/Web 字段）";
const LOCAL_MARKDOWN_NOTE: &str = "/editor 打开 .md 走 markdown 预览（只读渲染）";

/// 行为事实行的值：白字（不是「未设置」，也不是错误）+ 原文。
fn prose_cell(text: &'static str) -> (String, FieldTone) {
    (text.to_string(), FieldTone::Value)
}

/// 快捷键分区行 ↦ [`HotkeyGroup`]（`None` = 不是快捷键行）。
///
/// 分组与字段的对应写在这里一处：键位表本身只管「哪一组有哪些键」。
fn shortcut_group(field: FieldId) -> Option<HotkeyGroup> {
    match field {
        FieldId::KeyGlobal => Some(HotkeyGroup::Global),
        FieldId::KeyChat => Some(HotkeyGroup::Chat),
        FieldId::KeySettings => Some(HotkeyGroup::Panel),
        FieldId::KeyEditor => Some(HotkeyGroup::Editor),
        _ => None,
    }
}

/// 分区说明行：每栏一句「这栏是什么」，而不是统一的 `*`/`~` 标记说明。
///
/// 只读三栏的价值在「读事实」，把它们和可写栏用同一句「留空保存 = 跟随 server」
/// 盖过去，会让人以为它们也能编辑。
fn section_hint(section: Section) -> &'static str {
    match section {
        Section::Connection => "标记：本连接的事实（只读；Bearer 只报来源不报值）",
        Section::Shortcuts => "标记：键位来自热键表（新加热键自动出现在这里与 /help）",
        Section::Local => "标记：CM_TUI_* 开关的当前生效值（改需重启）；整栏只读",
        _ => "标记：* = override · ~ = 未保存 · 留空保存 = 跟随 server",
    }
}

/// 连接栏的事实行：`api_base` / lane 名与序号 / Bearer 来源。`None` = 不是这几行。
///
/// 拿不到就写「未提供」而不是留空：三行里最关键的就是「我到底连了哪里、带了什么凭据」。
fn conn_cell(ctx: &PanelCtx<'_>, field: FieldId) -> Option<(String, FieldTone)> {
    let unknown = |what: &str| (format!("({what} 未提供)"), FieldTone::Unset);
    Some(match field {
        FieldId::ConnApiBase => match ctx.conn {
            Some(c) if !c.api_base.is_empty() => (c.api_base.clone(), FieldTone::Value),
            _ => unknown("api_base"),
        },
        FieldId::ConnLane => match ctx.conn {
            Some(c) => (
                format!("{}（第 {}/{} 条）", c.label, c.pos.index, c.pos.total),
                FieldTone::Value,
            ),
            None => unknown("lane"),
        },
        FieldId::ConnBearer => match ctx.conn {
            Some(c) => (bearer_text(c.bearer).to_string(), FieldTone::Value),
            None => unknown("Bearer"),
        },
        _ => return None,
    })
}

/// Web Bearer 来源的人读措辞。**只报来源**：值本身永不上屏（与 API 密钥行同口径）。
fn bearer_text(source: BearerSource) -> &'static str {
    match source {
        BearerSource::Cli => "启动参数或环境（--bearer / CM_WEB_API_BEARER_TOKEN）",
        BearerSource::Keyring => "本机钥匙串（桌面壳槽；连的不是同一台 serve 时用 --bearer 覆盖）",
        BearerSource::Missing => "未提供（serve 开了 Bearer 鉴权就会 401）",
    }
}

/// 只读值的折行：先按 ` · ` 分项贪心装行，单项本身超宽再按显示列硬折。
///
/// 只读栏的值**就是事实**（键位、开关的生效值）：80 列终端上被 `truncate_display`
/// 截掉尾部的尾几个键，正是 #433 要消灭的东西，所以这列折行而不是截断。
pub(super) fn wrap_note(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    let mut out: Vec<String> = Vec::new();
    let mut line = String::new();
    for item in text.split(NOTE_SEP) {
        let item = item.trim();
        if item.is_empty() || push_item(&mut line, item, width) {
            continue;
        }
        // 这一项装不下：先把已装的这一行收尾，再把这一项（可能仍超宽）折开。
        if !line.is_empty() {
            out.push(std::mem::take(&mut line));
        }
        let mut chunks = wrap_display(item, width);
        line = chunks.pop().unwrap_or_default();
        out.append(&mut chunks);
    }
    if !line.is_empty() {
        out.push(line);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// 只读值的分项分隔符（面板行内统一用它分段，折行也只在这里断）。
const NOTE_SEP: &str = " · ";

/// 试把一项追加到当前行（带分隔符）；装得下返回 `true`。
fn push_item(line: &mut String, item: &str, width: usize) -> bool {
    let extra = if line.is_empty() {
        UnicodeWidthStr::width(item)
    } else {
        UnicodeWidthStr::width(NOTE_SEP) + UnicodeWidthStr::width(item)
    };
    if UnicodeWidthStr::width(line.as_str()) + extra > width {
        return false;
    }
    if !line.is_empty() {
        line.push_str(NOTE_SEP);
    }
    line.push_str(item);
    true
}

/// 二值开关的极简措辞（MCP 全局开关用）：`开` / `关`。
fn toggle_label(v: Option<&str>) -> &'static str {
    if v.map(str::trim) == Some(STORE_OFF) {
        "关"
    } else {
        "开"
    }
}

/// 会话存储开关的显示措辞（行值与底栏预览共用，避免同一个值两种叫法）。
fn store_label(v: Option<&str>) -> &'static str {
    match v {
        Some(STORE_ON) => "开（SQLite）",
        Some(STORE_OFF) => "关（文件存储）",
        _ => "(该 serve 未提供)",
    }
}

/// 行值显示文本：API Base 命中预设 URL 时显示预设 id（如 `deepseek`），其余原样。
fn value_text(field: FieldId, v: &str) -> String {
    if field == FieldId::ApiBase {
        api_base_display(v)
    } else {
        v.to_string()
    }
}

/// API Base 显示名：与任一具名预设 URL 完全一致 → 预设 id；否则原样 URL。
fn api_base_display(v: &str) -> String {
    crabmate_client_api::client_llm_presets::display_id_for_url(v)
        .map_or_else(|| v.to_string(), str::to_string)
}

/// 左侧导航项：按 [`Section::ALL`] 顺序给出「分区名 + 是否选中」。
///
/// 选中态的视觉（反色底块）由绘制侧负责，这里只产出数据，便于状态层直接断言。
fn nav_items(panel: &SettingsPanel) -> Vec<(String, bool)> {
    Section::ALL
        .iter()
        .map(|s| (s.label().to_string(), *s == panel.section))
        .collect()
}

/// 顶栏状态徽标：关闭确认 ＞ 保存中 ＞ 未保存 ＞ 只读；都没有则 `None`。
fn settings_badge(panel: &SettingsPanel) -> Option<String> {
    if panel.confirm_close {
        return Some("确认放弃？".to_string());
    }
    if panel.is_saving() {
        return Some("保存中…".to_string());
    }
    if panel.is_dirty() {
        return Some("未保存".to_string());
    }
    if panel.read_only {
        return Some("只读".to_string());
    }
    None
}

/// 面板底栏两行：动态提示 + 按键说明（每行文案各自独立成函数，控制 CCN ≤ 10）。
fn footer_lines(panel: &SettingsPanel, width: usize, pal: &Palette) -> Vec<Line<'static>> {
    let hint = footer_hint(panel);
    let (msg, level) = footer_msg(panel);
    let mut out = vec![Line::from(Span::styled(
        truncate_display(&msg, width),
        pal.notice_style(level),
    ))];
    out.push(Line::from(Span::styled(
        truncate_display(&hint, width),
        pal.tone_style(FieldTone::Unset),
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
        // 主题是下拉列表：上下逐项走，不是左右循环。
        Some(Editing::Theme { .. }) => "[↑/↓] 选择  [Enter] 确定  [Esc] 取消".to_string(),
        Some(Editing::Mode { .. })
        | Some(Editing::Think { .. })
        | Some(Editing::Tool { .. })
        | Some(Editing::Toggle { .. })
        | Some(Editing::Gateway { .. }) => "[←/→] 循环  [Enter] 确定  [Esc] 取消".to_string(),
        Some(Editing::Text { .. }) => "[Enter] 确定  [Esc] 取消编辑".to_string(),
        // 分区切换宣传 ←→（`Tab` / `Shift+Tab` 仍是同义别名，只是底栏宽度有限，
        // 两栏布局里方向键更直观 —— 见 `handle_key`）。
        None if panel.read_only => "[↑↓] 浏览  [←→] 分区  [Esc] 关闭".to_string(),
        None => "[↑↓] 移动  [Enter] 编辑  [←→] 分区  [F2] 保存  [Esc] 关闭".to_string(),
    }
}

/// 底栏动态提示（枚举循环预览 / 编辑引导 / 浏览态状态）。
fn footer_msg(panel: &SettingsPanel) -> (String, NoticeLevel) {
    match &panel.editing {
        Some(Editing::Mode { pick }) => mode_cycle_text(*pick),
        Some(Editing::Think { pick }) => think_cycle_text(*pick),
        Some(Editing::Tool { pick }) => tool_cycle_text(*pick),
        Some(Editing::Toggle { field, pick }) => toggle_cycle_text(*field, *pick),
        Some(Editing::Theme { pick }) => theme_cycle_text(*pick),
        Some(Editing::Gateway { pick, .. }) => gateway_cycle_text(*pick),
        Some(Editing::Text { field, .. }) => match &panel.note {
            Some((note, level)) => (note.clone(), *level),
            None => (
                format!("编辑「{}」：输入 · Backspace · ←→", field_label(*field)),
                NoticeLevel::Muted,
            ),
        },
        None => browse_msg(panel),
    }
}

/// 会话模式 ←→ 循环的可视化预览。
fn mode_cycle_text(pick: usize) -> (String, NoticeLevel) {
    let mut text = String::from("会话模式：");
    for (i, opt) in mode_options().iter().enumerate() {
        if i == pick {
            text.push('▸');
        }
        text.push_str(opt.unwrap_or("(跟随 server)"));
        text.push(' ');
    }
    (text, NoticeLevel::Muted)
}

/// 思考模式 ←→ 循环的可视化预览。
fn think_cycle_text(pick: usize) -> (String, NoticeLevel) {
    let mut text = String::from("思考模式：");
    for (i, opt) in think_options().iter().enumerate() {
        if i == pick {
            text.push('▸');
        }
        text.push_str(opt.unwrap_or("(跟随 server)"));
        text.push(' ');
    }
    (text, NoticeLevel::Muted)
}

/// 只读工具缓存 ←→ 循环的可视化预览。
fn tool_cycle_text(pick: usize) -> (String, NoticeLevel) {
    let mut text = String::from("只读工具缓存：");
    for (i, opt) in tool_options().iter().enumerate() {
        if i == pick {
            text.push('▸');
        }
        let label = if *opt == Some(TOOL_CACHE_DISABLED) {
            "关(随轮 ttl=0)"
        } else {
            "开(跟随 server)"
        };
        text.push_str(label);
        text.push(' ');
    }
    (text, NoticeLevel::Muted)
}

/// 二值开关 ←→ 循环的可视化预览（按字段选标题与措辞，两处共用一份结构）。
fn toggle_cycle_text(field: FieldId, pick: usize) -> (String, NoticeLevel) {
    let (title, labels) = if field == FieldId::SessionSqlite {
        (
            "会话存储：",
            [store_label(Some(STORE_ON)), store_label(Some(STORE_OFF))],
        )
    } else {
        (
            "MCP：",
            [toggle_label(Some(STORE_ON)), toggle_label(Some(STORE_OFF))],
        )
    };
    let mut text = String::from(title);
    for (i, label) in labels.iter().enumerate() {
        if i == pick {
            text.push('▸');
        }
        text.push_str(label);
        text.push(' ');
    }
    (text, NoticeLevel::Muted)
}

/// 主题编辑态的底栏提示：只报当前选中项。
///
/// 候选列表已经在字段行下方摊开（`theme_dropdown`），这里再拼整串只会让高亮项被
/// `truncate_display` 截掉 —— 底栏的职责是「说清现在选到哪」，不是复述列表。
fn theme_cycle_text(pick: usize) -> (String, NoticeLevel) {
    let label = THEME_OPTIONS
        .get(pick)
        .and_then(|opt| *opt)
        .unwrap_or(THEME_NONE_LABEL);
    (format!("主题：{label}"), NoticeLevel::Muted)
}

/// 网关预设 ←→ 循环的可视化预览（URL 一并显示，避免只看 id 不知落点）。
fn gateway_cycle_text(pick: usize) -> (String, NoticeLevel) {
    let mut text = String::from("网关：");
    for (i, p) in API_BASE_PRESETS.iter().enumerate() {
        if i == pick {
            text.push('▸');
        }
        text.push_str(p.id);
        if !p.url.is_empty() {
            text.push('(');
            text.push_str(p.url);
            text.push(')');
        }
        text.push(' ');
    }
    (text, NoticeLevel::Muted)
}

/// 浏览态提示行（非编辑、非模式循环时）。
fn browse_msg(panel: &SettingsPanel) -> (String, NoticeLevel) {
    if panel.confirm_close {
        return (
            "未保存改动：y 放弃 / Esc 返回".to_string(),
            NoticeLevel::Warn,
        );
    }
    if panel.is_saving() {
        return ("保存中…".to_string(), NoticeLevel::Info);
    }
    if let Some((note, level)) = &panel.note {
        return (note.clone(), *level);
    }
    if panel.read_only {
        return (
            "回合进行中：只读（结束后自动解锁）".to_string(),
            NoticeLevel::Warn,
        );
    }
    if is_readonly_section(panel.section) {
        return (
            "本栏只读：值就是当前事实（Enter 无动作）".to_string(),
            NoticeLevel::Muted,
        );
    }
    if panel.is_dirty() {
        return ("未保存改动：S 保存".to_string(), NoticeLevel::Warn);
    }
    (
        "留空保存 = 清除（跟随 server）".to_string(),
        NoticeLevel::Muted,
    )
}

/// 主题下拉一次最多显示几行候选。
///
/// 9 项全铺时整块内容要 11 行（说明 + 字段 + 9），而 `render_settings_body` 是
/// `take(inner.height)` 硬裁 —— 小窗口（tmux 分屏、矮终端）里被裁掉的恰好是尾部，
/// 选中项可以整个消失，且没有任何滚动机制能救它。封顶后候选行数固定，再配合
/// [`dropdown_window`] 保证 `pick` 永远落在可见区间内；展开后总行数上界为
/// `THEME_DROPDOWN_MAX_ROWS + 2`（首尾各一行省略提示）。
const THEME_DROPDOWN_MAX_ROWS: usize = 7;

/// 以 `pick` 为中心的可见区间 `[start, end)`，长度不超过 `max`。
///
/// 贴边时向内收而不是强行居中，这样**候选区间**长度恒为 `max`（总数不足时取总数），
/// 只有首尾省略行会随位置多出一行 —— 展开后总行数的上界仍是 `max + 2`。
fn dropdown_window(pick: usize, total: usize, max: usize) -> (usize, usize) {
    if total <= max {
        return (0, total);
    }
    let start = pick.saturating_sub(max / 2).min(total - max);
    (start, start + max)
}

/// 窗口外那一侧的省略行（`rest` = 还有几项没显示）。
fn dropdown_ellipsis(indent: &str, rest: usize, width: usize, pal: &Palette) -> Line<'static> {
    let text = format!("{indent}  ⋯ 还有 {rest} 项");
    Line::from(Span::styled(
        truncate_display(&text, width),
        pal.tone_style(FieldTone::Unset),
    ))
}

/// 下拉里的一项：`▸` + 名称，选中项反色。
///
/// 截断只作用于名称：整行（含缩进）一起截的话，窄终端下缩进会先吃掉宽度，
/// 主题名只剩半截甚至空串。
fn theme_option_line(
    indent: &str,
    i: usize,
    here: bool,
    width: usize,
    pal: &Palette,
) -> Line<'static> {
    let label = THEME_OPTIONS[i].unwrap_or(THEME_NONE_LABEL);
    let avail = width.saturating_sub(indent.len() + 2);
    let marker = if here { "▸ " } else { "  " };
    let text = format!("{indent}{marker}{}", truncate_display(label, avail));
    let style = if here {
        Style::new().add_modifier(Modifier::REVERSED)
    } else {
        pal.tone_style(FieldTone::Unset)
    };
    Line::from(Span::styled(text, style))
}
