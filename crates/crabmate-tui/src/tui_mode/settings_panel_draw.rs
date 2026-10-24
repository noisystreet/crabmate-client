//! 设置面板内容构建与行渲染（content / field_line / 值列 / 导航 / 底栏文案）。
//!
//! `settings_panel.rs` 的 `#[path]` 子模块：压住面板文件行数门禁。逻辑与测试仍
//! 在父模块组装，这里直接 `use super::*;` 访问父模块私有项。

use super::*;

/// 主题「未设置」在面板里的显示名（下拉项、字段值、底栏共用一份措辞）。
const THEME_NONE_LABEL: &str = "默认（透明）";

impl SettingsPanel {
    /// 构建一帧面板内容：说明行 + 字段行 + 顶栏/导航/底栏元数据。
    pub fn content(&self, ctx: &PanelCtx<'_>, width: usize) -> PanelContent {
        let mut lines: Vec<Line<'static>> = Vec::new();
        // 分区说明行：对应 desktop 内容区标题下的 `settings-content-desc`。
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
            // 主题 9 个候选、也全是展示用选项：编辑中就在行下方把列表摊开，
            // 比在底栏读一行「当前循环到哪」再连按 ←→ 直观。
            if *field == FieldId::Theme {
                lines.extend(self.theme_dropdown(width));
            }
        }
        PanelContent {
            lines,
            cursor,
            title: self.section.label().to_string(),
            badge: settings_badge(self),
            nav: nav_items(self),
            // 底栏单独返回：绘制侧把它贴到页面底部，而不是跟字段行一起往上堆。
            footer: footer_lines(self, width),
        }
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

    /// 主题下拉列表：仅在主题行处于编辑态时非空。
    ///
    /// 对齐到标签列之后，当前项反色 + `▸`，其余灰字；`None` 显示 [`THEME_NONE_LABEL`]，
    /// 与 `theme_cell` 用同一份措辞，免得同一样东西两种叫法。
    ///
    /// 候选多于 [`THEME_DROPDOWN_MAX_ROWS`] 时开窗滚动，窗口跟着 `pick` 走：绘制侧
    /// `render_settings_body` 是 `take(inner.height)` 硬裁，全铺的话小终端里被裁掉的
    /// 正是尾部，选中项可能整个消失 —— 而那条路径没有任何滚动机制能救它。
    fn theme_dropdown(&self, width: usize) -> Vec<Line<'static>> {
        let Some(Editing::Theme { pick }) = &self.editing else {
            return Vec::new();
        };
        let total = THEME_OPTIONS.len();
        let (start, end) = dropdown_window(*pick, total, THEME_DROPDOWN_MAX_ROWS);
        let indent = " ".repeat(LABEL_PAD + 2);
        let mut out = Vec::with_capacity(end - start + 2);
        if start > 0 {
            out.push(dropdown_ellipsis(&indent, start, width));
        }
        for i in start..end {
            out.push(theme_option_line(&indent, i, i == *pick, width));
        }
        if end < total {
            out.push(dropdown_ellipsis(&indent, total - end, width));
        }
        out
    }

    /// 字段值列文本与颜色：staged（~）＞ override（*）＞ user-data ＞ serve 默认 ＞ 跟随 server。
    /// API 密钥不走三层合成（staged / 已设态两态）；工具缓存、主题各有独立行文案。
    fn value_cell(&self, ctx: &PanelCtx<'_>, field: FieldId) -> (String, Color) {
        if field == FieldId::ApiKey {
            return self.secret_cell();
        }
        if field == FieldId::ToolCache {
            return self.tool_cache_cell(ctx);
        }
        if field == FieldId::Theme {
            return self.theme_cell(ctx);
        }
        match self.staged(field) {
            FieldAction::Write(Some(v)) => {
                (format!("{}~", value_text(field, v)), Color::LightYellow)
            }
            FieldAction::Write(None) => {
                // 清除后回落：显示 serve 默认（若有）。
                let (_, _, remote) = ctx.sources(field);
                match remote.and_then(normalize_str) {
                    Some(v) => (format!("{}~", value_text(field, &v)), Color::LightYellow),
                    None => ("(跟随 server)~".to_string(), Color::LightYellow),
                }
            }
            FieldAction::Skip => match ctx.effective(field) {
                EffectiveView {
                    layer: Layer::Override,
                    value,
                } => (
                    format!("{}*", value_text(field, &value.unwrap_or_default())),
                    Color::LightCyan,
                ),
                EffectiveView {
                    layer: Layer::Stored,
                    value,
                } => (value_text(field, &value.unwrap_or_default()), Color::White),
                EffectiveView {
                    layer: Layer::Default,
                    value,
                } => (value_text(field, &value.unwrap_or_default()), Color::Gray),
                EffectiveView {
                    layer: Layer::Follow,
                    value: _,
                } => ("(跟随 server)".to_string(), Color::DarkGray),
            },
        }
    }

    /// 只读工具缓存行值：staged（~）＞ user-data 禁用（白字）＞ 开/跟随 server。
    fn tool_cache_cell(&self, ctx: &PanelCtx<'_>) -> (String, Color) {
        match self.staged(FieldId::ToolCache) {
            FieldAction::Write(Some(_)) => ("关（禁用缓存）~".to_string(), Color::LightYellow),
            FieldAction::Write(None) => ("开（跟随 server）~".to_string(), Color::LightYellow),
            FieldAction::Skip => match ctx.effective(FieldId::ToolCache) {
                EffectiveView {
                    layer: Layer::Stored,
                    value: Some(_),
                } => ("关（禁用缓存）".to_string(), Color::White),
                _ => ("开（跟随 server）".to_string(), Color::DarkGray),
            },
        }
    }

    /// 主题行值：staged（~）＞ user-data 已存 slug ＞ 默认（透明）。
    ///
    /// 空值不叫「跟随 server」：`prefs.theme` 缺失时 TUI 回落到自己的产品默认
    /// （透明底色），与 serve 配置无关，写成「跟随 server」会误导。
    fn theme_cell(&self, ctx: &PanelCtx<'_>) -> (String, Color) {
        match self.staged(FieldId::Theme) {
            FieldAction::Write(Some(v)) => (format!("{v}~"), Color::LightYellow),
            FieldAction::Write(None) => (format!("{THEME_NONE_LABEL}~"), Color::LightYellow),
            FieldAction::Skip => match ctx.effective(FieldId::Theme).value {
                Some(v) => (v, Color::White),
                None => (THEME_NONE_LABEL.to_string(), Color::DarkGray),
            },
        }
    }

    /// API 密钥行值：staged 写 = 掩码 + `~`；staged 清除 = `清除~`；未编辑按已设态显示。
    fn secret_cell(&self) -> (String, Color) {
        match &self.secret {
            FieldAction::Write(Some(v)) => (
                format!("••••（{} 字符）~", v.chars().count()),
                Color::LightYellow,
            ),
            FieldAction::Write(None) => ("清除~".to_string(), Color::LightYellow),
            FieldAction::Skip if self.secret_set => ("已设（钥匙串）".to_string(), Color::White),
            FieldAction::Skip => ("未设（跟随 serve）".to_string(), Color::DarkGray),
        }
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
        // 主题是下拉列表：上下逐项走，不是左右循环。
        Some(Editing::Theme { .. }) => "[↑/↓] 选择  [Enter] 确定  [Esc] 取消".to_string(),
        Some(Editing::Mode { .. })
        | Some(Editing::Think { .. })
        | Some(Editing::Tool { .. })
        | Some(Editing::Gateway { .. }) => "[←/→] 循环  [Enter] 确定  [Esc] 取消".to_string(),
        Some(Editing::Text { .. }) => "[Enter] 确定  [Esc] 取消编辑".to_string(),
        None if panel.read_only => "[↑↓] 浏览  [Tab] 分区  [Esc] 关闭".to_string(),
        None => "[↑↓] 移动  [Enter] 编辑  [Tab] 分区  [F2] 保存  [Esc] 关闭".to_string(),
    }
}

/// 底栏动态提示（枚举循环预览 / 编辑引导 / 浏览态状态）。
fn footer_msg(panel: &SettingsPanel) -> (String, Color) {
    match &panel.editing {
        Some(Editing::Mode { pick }) => mode_cycle_text(*pick),
        Some(Editing::Think { pick }) => think_cycle_text(*pick),
        Some(Editing::Tool { pick }) => tool_cycle_text(*pick),
        Some(Editing::Theme { pick }) => theme_cycle_text(*pick),
        Some(Editing::Gateway { pick, .. }) => gateway_cycle_text(*pick),
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

/// 思考模式 ←→ 循环的可视化预览。
fn think_cycle_text(pick: usize) -> (String, Color) {
    let mut text = String::from("思考模式：");
    for (i, opt) in think_options().iter().enumerate() {
        if i == pick {
            text.push('▸');
        }
        text.push_str(opt.unwrap_or("(跟随 server)"));
        text.push(' ');
    }
    (text, Color::Gray)
}

/// 只读工具缓存 ←→ 循环的可视化预览。
fn tool_cycle_text(pick: usize) -> (String, Color) {
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
    (text, Color::Gray)
}

/// 主题编辑态的底栏提示：只报当前选中项。
///
/// 候选列表已经在字段行下方摊开（`theme_dropdown`），这里再拼整串只会让高亮项被
/// `truncate_display` 截掉 —— 底栏的职责是「说清现在选到哪」，不是复述列表。
fn theme_cycle_text(pick: usize) -> (String, Color) {
    let label = THEME_OPTIONS
        .get(pick)
        .and_then(|opt| *opt)
        .unwrap_or(THEME_NONE_LABEL);
    (format!("主题：{label}"), Color::Gray)
}

/// 网关预设 ←→ 循环的可视化预览（URL 一并显示，避免只看 id 不知落点）。
fn gateway_cycle_text(pick: usize) -> (String, Color) {
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
fn dropdown_ellipsis(indent: &str, rest: usize, width: usize) -> Line<'static> {
    let text = format!("{indent}  ⋯ 还有 {rest} 项");
    Line::from(Span::styled(
        truncate_display(&text, width),
        Style::new().fg(Color::DarkGray),
    ))
}

/// 下拉里的一项：`▸` + 名称，选中项反色。
///
/// 截断只作用于名称：整行（含缩进）一起截的话，窄终端下缩进会先吃掉宽度，
/// 主题名只剩半截甚至空串。
fn theme_option_line(indent: &str, i: usize, here: bool, width: usize) -> Line<'static> {
    let label = THEME_OPTIONS[i].unwrap_or(THEME_NONE_LABEL);
    let avail = width.saturating_sub(indent.len() + 2);
    let marker = if here { "▸ " } else { "  " };
    let text = format!("{indent}{marker}{}", truncate_display(label, avail));
    let style = if here {
        Style::new().add_modifier(Modifier::REVERSED)
    } else {
        Style::new().fg(Color::DarkGray)
    };
    Line::from(Span::styled(text, style))
}
