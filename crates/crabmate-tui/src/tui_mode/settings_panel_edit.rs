//! 设置面板的**编辑态**（`settings_panel.rs` 的 `#[path]` 子模块）：把字段行从
//! 「浏览」切进「编辑」之后的按键分派、提交校验与文本缓冲原语。
//!
//! 单独成文件的理由只是 `fn-nloc-ratchet` 的单文件 920 行上限：`settings_panel.rs`
//! 在加 W3 字段前已 897 行。这里只搬**状态机**（[`Editing`] / `EditAction` 与其分派），
//! 三层合成仍归 [`super::super::settings`]，内容渲染仍归 [`super::draw`]。
//!
//! 子模块能访问父模块的私有字段与方法，所以 `impl SettingsPanel` 写在这里不必把
//! `editing` / `set_staged` 改成 `pub(crate)`。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::super::palette::{NoticeLevel, THEME_OPTIONS};
use super::super::settings::{
    FieldAction, is_valid_context_tokens, is_valid_temperature, validate_api_base,
};
use super::{
    API_BASE_PRESETS, FieldId, GATEWAY_CUSTOM_ID, GATEWAY_SERVER_ID, SettingsPanel, mode_options,
    move_pick, think_options, tool_options,
};

/// 行编辑状态：文本编辑（含缓冲与光标）、模式/思考/工具缓存枚举循环、网关预设循环。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Editing {
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
    /// 主题循环：`pick` 对应 [`THEME_OPTIONS`]（下标 0 = 默认 / 清除 prefs 键）。
    Theme {
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
    CommitTheme,
    CommitGateway,
    Insert(char),
    Backspace,
    MoveLeft,
    MoveRight,
    /// 下拉列表里向上/向下移动（主题用；到两端停住，不环绕）。
    PickUp,
    PickDown,
}

impl SettingsPanel {
    /// 编辑态按键：文本字符输入/Backspace/←→ 光标、Enter 提交、Esc 取消；
    /// ↑↓ 与 Tab 忽略（避免误跳行/误切分区）。
    pub(super) fn edit_key(&mut self, key: &KeyEvent) {
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
            Some(Editing::Theme { .. }) => match key.code {
                KeyCode::Esc => Some(EditAction::Cancel),
                KeyCode::Enter => Some(EditAction::CommitTheme),
                // 主题是「下拉」而不是「循环」：候选有 9 个，↑↓ 逐项走过一遍比记住顺序
                // 连按 ←→ 更符合直觉。←→ 保留，习惯循环的人不必改手。
                KeyCode::Up => Some(EditAction::PickUp),
                KeyCode::Down => Some(EditAction::PickDown),
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
            EditAction::CommitTheme => self.commit_theme(),
            EditAction::CommitGateway => self.commit_gateway(),
            EditAction::Insert(c) => self.edit_insert(c),
            EditAction::Backspace => self.edit_backspace(),
            EditAction::MoveLeft => self.edit_move(true),
            EditAction::MoveRight => self.edit_move(false),
            EditAction::PickUp => self.pick_step(true),
            EditAction::PickDown => self.pick_step(false),
        }
    }

    /// 文本字段提交：trim 后空串 = 清除；API Base 需 http(s):// 前缀、温度需
    /// 0.0..=2.0、上下文长度需 ≤ 10_000_000 整数（非法则留在编辑态提示）；
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
            self.set_note(
                "API Base 需 http(s):// 开头".to_string(),
                NoticeLevel::Error,
            );
            return;
        }
        if field == FieldId::Temperature && !is_valid_temperature(&text) {
            let buf: Vec<char> = text.chars().collect();
            let cursor = buf.len();
            self.editing = Some(Editing::Text { field, buf, cursor });
            self.set_note(
                "温度需 0.0..=2.0（留空保存 = 跟随 server）".to_string(),
                NoticeLevel::Error,
            );
            return;
        }
        if field == FieldId::ContextTokens && !is_valid_context_tokens(&text) {
            let buf: Vec<char> = text.chars().collect();
            let cursor = buf.len();
            self.editing = Some(Editing::Text { field, buf, cursor });
            self.set_note(
                "上下文长度需 0..=10_000_000 整数（留空保存 = 跟随 server）".to_string(),
                NoticeLevel::Error,
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

    /// 主题确认：写入当前选项（下标 0 = `None` = 清除 prefs 键、回到产品默认透明）。
    fn commit_theme(&mut self) {
        let Some(Editing::Theme { pick }) = self.editing.take() else {
            return;
        };
        let value = THEME_OPTIONS[pick].map(str::to_string);
        self.set_staged(FieldId::Theme, FieldAction::Write(value));
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

    /// 编辑态 ←/→：文本字段移动光标；模式/思考/工具/主题/网关枚举移动选项（↑↓ 不动）。
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

    /// 主题下拉光标：↑↓ 与 ←→ 走同一个 clamp 移动（[`move_pick`]）。
    ///
    /// 不复用 `move_cycle_pick` 那层分派，是因为下拉只认 `Theme`；但移动规则必须
    /// 与它一致，否则同一个 `pick` 会因按键不同而语义分叉。
    fn pick_step(&mut self, up: bool) {
        let Some(Editing::Theme { pick }) = &mut self.editing else {
            return;
        };
        move_pick(pick, up, THEME_OPTIONS.len());
    }
}

/// ←/→ 移动循环类编辑的光标（Mode / Think / Tool / Theme / Gateway）。
///
/// 分派留在编辑态这一层：参数是私有的 [`Editing`]，选项表与下标算术在
/// [`super::schema`]。
fn move_cycle_pick(editing: &mut Editing, left: bool) {
    match editing {
        Editing::Mode { pick } => move_pick(pick, left, mode_options().len()),
        Editing::Think { pick } => move_pick(pick, left, think_options().len()),
        Editing::Tool { pick } => move_pick(pick, left, tool_options().len()),
        Editing::Theme { pick } => move_pick(pick, left, THEME_OPTIONS.len()),
        Editing::Gateway { pick, .. } => move_pick(pick, left, API_BASE_PRESETS.len()),
        Editing::Text { .. } => {}
    }
}
