//! 与焦点无关的顶层热键判定。
//!
//! 单独成文件有两个理由：`mod.rs` 贴着 920 行门禁，而 `on_key` 那条 if 链贴着
//! lizard 的 CCN≤10 —— 把「哪个键对应哪个热键」收成一次查表，事件循环层只按结果
//! 分派，新增热键不会再往 if 链上加分支。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::settings_panel::is_settings_open_key;

/// 与焦点无关的热键（输入框 / 会话栏 / 工作区树下都可用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TopHotkey {
    /// F6：设置面板开关（打开时等同 Esc 的关闭语义，脏字段先确认）。
    /// 面板内保存用 F2，两者分工不同：F2 不再是开关别名。
    Settings,
    /// F3：右栏底部状态栈展开 / 折叠（窄屏没有状态栈，切换无效果也不报错）。
    StatusStack,
    /// Ctrl+Y：复制最近一条助手回复（`yank` 的传统键位，终端一般不截获）。
    Copy,
    /// Alt+→：切到下一条 lane。多 serve 并行时只换视图，不影响任何在跑的回合。
    ///
    /// 单 lane 时是 no-op（外层 `TuiApp::switch_lane` 在 `len <= 1` 直接返回）。
    NextLane,
    /// Alt+←：切到上一条 lane（循环）。
    PrevLane,
    /// F4：切到**下一条在等审批**的 lane（循环，没有就留在原地并提示）。
    ///
    /// 后台 lane 的审批浮层画不到屏幕上（每帧只画当前 lane），左栏那一行的 `▲` 是唯一
    /// 线索；三条以上时靠 [`Self::NextLane`] 逐个试等于让用户自己找。
    NextApproval,
    /// Alt+1..8：**直达**第 n 条 lane（1-based，序号与左栏列表自上而下一致）。
    ///
    /// 与 [`Self::NextLane`] 的区别是「绝对目标」而不是「相对移动」：八条连接时从第 1
    /// 条到第 8 条，按方向键要七次，按这一个键就够。越界**不切视图**，只留一句提示。
    GotoLane(usize),
}

/// 判定热键。
///
/// 用 `(code, modifiers)` 精确匹配而不是 `if` 链：match 的臂不计入圈复杂度，
/// 所以这里能同时容纳多个热键，而调用方的 if 链保持原样。
pub(super) fn of(key: &KeyEvent) -> Option<TopHotkey> {
    if is_settings_open_key(key) {
        return Some(TopHotkey::Settings);
    }
    match (key.code, key.modifiers) {
        (KeyCode::F(3), KeyModifiers::NONE) => Some(TopHotkey::StatusStack),
        // F4：跳到在等审批的那条 lane。F 系热键已用 F2（面板保存）/F3（状态栈）/F6（设置），
        // 不与输入框的 Ctrl 组合抢键。
        (KeyCode::F(4), KeyModifiers::NONE) => Some(TopHotkey::NextApproval),
        (KeyCode::Char('y'), KeyModifiers::CONTROL) => Some(TopHotkey::Copy),
        // Ctrl+N：切下一条 lane。与 Alt+→ **同一条**实现（同一个 `switch_lane(1)`），
        // 不是第二套「下一条」规则 —— 两个键给出同一结果，才不需要记两套语义。
        (KeyCode::Char('n'), KeyModifiers::CONTROL) => Some(TopHotkey::NextLane),
        // Alt+方向：切换 lane。裸方向键归输入框 / 列表，必须精确匹配 ALT。
        (KeyCode::Right, KeyModifiers::ALT) => Some(TopHotkey::NextLane),
        (KeyCode::Left, KeyModifiers::ALT) => Some(TopHotkey::PrevLane),
        // Alt+1..8：直达。**同样精确匹配 ALT**（不放宽成 `contains`）：`Alt+Shift+数字`
        // 是另一回事，而且 `Alt+Enter` / `Alt+方向` 的修饰符匹配不能因为加数字被顺手放宽。
        // 上限 8 与 `shell::MAX_LANES` 同值，更靠后的数字键不必认。
        (KeyCode::Char(c @ '1'..='8'), KeyModifiers::ALT) => {
            c.to_digit(10).map(|n| TopHotkey::GotoLane(n as usize))
        }
        _ => None,
    }
}

/// Ctrl+C：取消回合（再按一次强退）。
pub(super) fn is_ctrl_c(key: &KeyEvent) -> bool {
    key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)
}

/// Ctrl+C / Ctrl+D（审批浮层里的"中断式拒绝"；回合取消需在浮层关闭后再按一次）。
pub(super) fn is_ctrl_abort(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('d'))
}

/// Ctrl+O：提交输入（与 Enter 同义，多行编辑里更好按）。
pub(super) fn is_ctrl_o(key: &KeyEvent) -> bool {
    key.code == KeyCode::Char('o') && key.modifiers.contains(KeyModifiers::CONTROL)
}

/// Ctrl+W：输入焦点下进入左栏工作区目录树。
pub(super) fn is_ctrl_w(key: &KeyEvent) -> bool {
    key.code == KeyCode::Char('w') && key.modifiers.contains(KeyModifiers::CONTROL)
}

/// Ctrl+E：思考内容展开 / 折叠。
pub(super) fn is_ctrl_e(key: &KeyEvent) -> bool {
    key.code == KeyCode::Char('e') && key.modifiers.contains(KeyModifiers::CONTROL)
}

/// 任意 `Ctrl+<code>`（Home / End 等）。
pub(super) fn is_ctrl(key: &KeyEvent, code: KeyCode) -> bool {
    key.code == code && key.modifiers.contains(KeyModifiers::CONTROL)
}

/// Alt+Enter：输入框内换行（Enter 是提交）。
pub(super) fn is_alt_enter(key: &KeyEvent) -> bool {
    key.code == KeyCode::Enter && key.modifiers.contains(KeyModifiers::ALT)
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

    /// F4 只认裸键：带修饰键的 F4 属别的终端语义，而裸 `4` 就是输入框里的数字。
    #[test]
    fn f4_jumps_to_the_next_approval() {
        assert_eq!(
            of(&key(KeyCode::F(4), KeyModifiers::NONE)),
            Some(TopHotkey::NextApproval)
        );
        assert_eq!(of(&key(KeyCode::F(4), KeyModifiers::CONTROL)), None);
        assert_eq!(of(&key(KeyCode::Char('4'), KeyModifiers::NONE)), None);
    }

    #[test]
    fn f6_opens_the_settings_panel() {
        assert_eq!(
            of(&key(KeyCode::F(6), KeyModifiers::NONE)),
            Some(TopHotkey::Settings)
        );
    }

    #[test]
    fn f2_and_punctuation_stay_out_of_the_settings_hotkey() {
        // F2 归面板内的「保存」；`Ctrl+,` 在 VTE 系终端与裸逗号同码，索性不绑。
        assert_eq!(of(&key(KeyCode::F(2), KeyModifiers::NONE)), None);
        assert_eq!(of(&key(KeyCode::Char(','), KeyModifiers::CONTROL)), None);
        assert_eq!(of(&key(KeyCode::Char(','), KeyModifiers::NONE)), None);
    }

    #[test]
    fn alt_digits_goto_the_lane_with_that_number() {
        for n in 1..=8u32 {
            let digit = char::from_digit(n, 10).expect("1..=8 都是数字");
            assert_eq!(
                of(&key(KeyCode::Char(digit), KeyModifiers::ALT)),
                Some(TopHotkey::GotoLane(n as usize)),
                "Alt+{digit}"
            );
        }
    }

    /// 只认**裸** Alt+数字：裸数字归输入框（要能打数字），`Ctrl+数字` 是别处的约定，
    /// `Alt+Shift+数字` 又是一种组合，`0` / `9` 超出 `MAX_LANES = 8`。
    /// 放宽任一条，都会让别的键悄悄变成切 lane。
    #[test]
    fn only_bare_alt_digits_one_to_eight_goto_a_lane() {
        for modifiers in [
            KeyModifiers::NONE,
            KeyModifiers::CONTROL,
            KeyModifiers::ALT | KeyModifiers::SHIFT,
        ] {
            assert_eq!(
                of(&key(KeyCode::Char('3'), modifiers)),
                None,
                "{modifiers:?} 不该被当成直达"
            );
        }
        assert_eq!(of(&key(KeyCode::Char('0'), KeyModifiers::ALT)), None);
        assert_eq!(of(&key(KeyCode::Char('9'), KeyModifiers::ALT)), None);
    }

    /// `Ctrl+N` 与 `Alt+→` 返回**同一个**热键，所以下游只有一条「下一条」实现。
    #[test]
    fn ctrl_n_is_the_same_next_lane_as_alt_right() {
        assert_eq!(
            of(&key(KeyCode::Char('n'), KeyModifiers::CONTROL)),
            of(&key(KeyCode::Right, KeyModifiers::ALT)),
        );
        // 裸 `n` 仍然是输入框里的字母。
        assert_eq!(of(&key(KeyCode::Char('n'), KeyModifiers::NONE)), None);
    }

    #[test]
    fn alt_arrows_switch_lanes_but_bare_arrows_do_not() {
        assert_eq!(
            of(&key(KeyCode::Right, KeyModifiers::ALT)),
            Some(TopHotkey::NextLane)
        );
        assert_eq!(
            of(&key(KeyCode::Left, KeyModifiers::ALT)),
            Some(TopHotkey::PrevLane)
        );
        // 裸方向键归输入框光标 / 列表选择，顶层绝不抢。
        assert_eq!(of(&key(KeyCode::Right, KeyModifiers::NONE)), None);
        assert_eq!(of(&key(KeyCode::Left, KeyModifiers::NONE)), None);
        // Shift+Alt+方向也不认：精确匹配修饰键，避免和「按词选择」类终端行为打架。
        assert_eq!(
            of(&key(
                KeyCode::Right,
                KeyModifiers::ALT | KeyModifiers::SHIFT
            )),
            None
        );
    }
}
