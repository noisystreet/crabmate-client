//! 与焦点无关的顶层热键判定。
//!
//! 单独成文件有两个理由：`mod.rs` 贴着 920 行门禁，而 `on_key` 那条 if 链贴着
//! lizard 的 CCN≤10 —— 把「哪个键对应哪个热键」收成一次查表，事件循环层只按结果
//! 分派，新增热键不会再往 if 链上加分支。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::settings_panel::is_f2_key;

/// 与焦点无关的热键（输入框 / 会话栏 / 工作区树下都可用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TopHotkey {
    /// F2：设置面板开关（打开时等同 Esc 的关闭语义，脏字段先确认）。
    Settings,
    /// F3：右栏底部状态栈展开 / 折叠（窄屏没有状态栈，切换无效果也不报错）。
    StatusStack,
    /// Ctrl+Y：复制最近一条助手回复（`yank` 的传统键位，终端一般不截获）。
    Copy,
}

/// 判定热键。
///
/// 用 `(code, modifiers)` 精确匹配而不是 `if` 链：match 的臂不计入圈复杂度，
/// 所以这里能同时容纳多个热键，而调用方的 if 链保持原样。
pub(super) fn of(key: &KeyEvent) -> Option<TopHotkey> {
    if is_f2_key(key) {
        return Some(TopHotkey::Settings);
    }
    match (key.code, key.modifiers) {
        (KeyCode::F(3), KeyModifiers::NONE) => Some(TopHotkey::StatusStack),
        (KeyCode::Char('y'), KeyModifiers::CONTROL) => Some(TopHotkey::Copy),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn ctrl_y_is_copy() {
        assert_eq!(
            of(&key(KeyCode::Char('y'), KeyModifiers::CONTROL)),
            Some(TopHotkey::Copy)
        );
    }

    #[test]
    fn plain_y_stays_with_the_sidebar() {
        // 裸 `y` 在会话栏是「确认删除」，不能被顶层热键表抢走。
        assert_eq!(of(&key(KeyCode::Char('y'), KeyModifiers::NONE)), None);
    }

    #[test]
    fn f3_toggles_the_stack_and_ctrl_f3_is_left_alone() {
        assert_eq!(
            of(&key(KeyCode::F(3), KeyModifiers::NONE)),
            Some(TopHotkey::StatusStack)
        );
        // 带修饰键的 F3 是别的终端语义，不抢。
        assert_eq!(of(&key(KeyCode::F(3), KeyModifiers::CONTROL)), None);
    }

    #[test]
    fn f2_opens_the_settings_panel() {
        assert_eq!(
            of(&key(KeyCode::F(2), KeyModifiers::NONE)),
            Some(TopHotkey::Settings)
        );
    }
}
