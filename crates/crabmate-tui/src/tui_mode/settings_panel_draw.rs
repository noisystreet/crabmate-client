//! 设置面板内容构建与行渲染（content / field_line / 值列 / 导航 / 底栏文案）。
//!
//! `settings_panel.rs` 的 `#[path]` 子模块：压住面板文件行数门禁。逻辑与测试仍
//! 在父模块组装，这里直接 `use super::*;` 访问父模块私有项。

use super::*;

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
    /// API 密钥不走三层合成（staged / 已设态两态）；工具缓存有独立两态行文案。
    fn value_cell(&self, ctx: &PanelCtx<'_>, field: FieldId) -> (String, Color) {
        if field == FieldId::ApiKey {
            return self.secret_cell();
        }
        if field == FieldId::ToolCache {
            return self.tool_cache_cell(ctx);
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
    for p in API_BASE_PRESETS {
        if p.id != GATEWAY_SERVER_ID && p.id != GATEWAY_CUSTOM_ID && p.url == v {
            return p.id.to_string();
        }
    }
    v.to_string()
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
        Some(Editing::Mode { .. })
        | Some(Editing::Think { .. })
        | Some(Editing::Tool { .. })
        | Some(Editing::Gateway { .. }) => "[←/→] 循环  [Enter] 确定  [Esc] 取消".to_string(),
        Some(Editing::Text { .. }) => "[Enter] 确定  [Esc] 取消编辑".to_string(),
        None if panel.read_only => "[↑↓] 浏览  [Tab] 分区  [Esc/F2] 关闭".to_string(),
        None => "[↑↓] 移动  [Enter] 编辑  [Tab] 分区  [S] 保存  [Esc/F2] 关闭".to_string(),
    }
}

/// 底栏动态提示（枚举循环预览 / 编辑引导 / 浏览态状态）。
fn footer_msg(panel: &SettingsPanel) -> (String, Color) {
    match &panel.editing {
        Some(Editing::Mode { pick }) => mode_cycle_text(*pick),
        Some(Editing::Think { pick }) => think_cycle_text(*pick),
        Some(Editing::Tool { pick }) => tool_cycle_text(*pick),
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
