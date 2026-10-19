//! 审批浮层双向握手（M3）：SSE 回合线程的 gate 把请求经 UI 事件通道发给事件循环
//! 并阻塞在应答通道上等决策；事件循环保持运行收按键。Esc/n=拒绝、Enter/y/o=一次、
//! a=始终（与 repl 的 Tty 提示语义一致）。

use std::sync::mpsc::{self, Sender};

use crate::serve::{ApprovalDecision, ApprovalGate, CommandApprovalData, TermError};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::worker::UiEvent;

/// 一条待 UI 决策的审批请求（携带回传通道）。
pub struct ApprovalPrompt {
    pub req: CommandApprovalData,
    pub answer: Sender<ApprovalDecision>,
}

/// 全屏浮层审批 gate（运行在 SSE 回合任务内）。
///
/// `decide` 把请求发给 UI 事件循环，然后阻塞等决策；UI 通道关闭（退出/崩溃）时
/// 回 `Deny` 兜底，避免 serve 侧审批会话悬挂（与文本模式读行失败路径一致）。
pub struct OverlayApprovalGate {
    pub tx: Sender<UiEvent>,
}

impl ApprovalGate for OverlayApprovalGate {
    fn decide(&mut self, req: &CommandApprovalData) -> Result<ApprovalDecision, TermError> {
        let (answer_tx, answer_rx) = mpsc::channel::<ApprovalDecision>();
        self.tx
            .send(UiEvent::Approval {
                prompt: ApprovalPrompt {
                    req: req.clone(),
                    answer: answer_tx,
                },
            })
            .map_err(|_| TermError::Message("ui channel closed".into()))?;
        Ok(answer_rx.recv().unwrap_or(ApprovalDecision::Deny))
    }
}

/// 审批浮层里可选的三个动作：`←` / `→` 移动，`Enter` 确认。
///
/// 枚举顺序即屏幕上从左到右的顺序，也是方向键的遍历方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ApprovalChoice {
    /// 只放行这一次。默认选中 —— 最常见的回答，回车直接走这条。
    #[default]
    Once,
    /// 加入 allowlist，之后同类命令不再询问。
    Always,
    /// 拒绝（回合继续，不取消）。
    Deny,
}

impl ApprovalChoice {
    /// 展示顺序（= 左右键的遍历顺序）。
    pub const ALL: [ApprovalChoice; 3] = [Self::Once, Self::Always, Self::Deny];

    /// 选项文字：浮层里直接画这三块。
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Once => "仅此一次",
            Self::Always => "始终允许",
            Self::Deny => "拒绝",
        }
    }

    /// 提交时对应的决策。
    #[must_use]
    pub fn decision(self) -> ApprovalDecision {
        match self {
            Self::Once => ApprovalDecision::AllowOnce,
            Self::Always => ApprovalDecision::AllowAlways,
            Self::Deny => ApprovalDecision::Deny,
        }
    }

    fn index(self) -> isize {
        match self {
            Self::Once => 0,
            Self::Always => 1,
            Self::Deny => 2,
        }
    }

    /// 左右移动一格。
    ///
    /// **到头夹住而不循环**：循环的话「按住右键」会从「拒绝」直接跳回「仅此一次」，
    /// 多按一次就把拒绝变成了放行 —— 这个方向上宁可少走一步。
    #[must_use]
    pub fn shift(self, delta: isize) -> Self {
        Self::ALL[(self.index() + delta).clamp(0, 2) as usize]
    }
}

/// 浮层打开期间的按键 → 决策（其余按键忽略，防叠栈/误输入）。
///
/// `choice` 是方向键移动的那个光标：方向键只改它、不提交；`Enter` 才按它提交。
pub fn decision_for_key(key: &KeyEvent, choice: &mut ApprovalChoice) -> Option<ApprovalDecision> {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        // Ctrl+C / Ctrl+D：与文本模式同语义地把审批当拒绝（回合取消需再次 Ctrl+C）。
        return matches!(key.code, KeyCode::Char('c') | KeyCode::Char('d'))
            .then_some(ApprovalDecision::Deny);
    }
    match key.code {
        // ←/→（以及 vim 的 h/l）：只移动选择。
        KeyCode::Left | KeyCode::Char('h') => {
            *choice = choice.shift(-1);
            None
        }
        KeyCode::Right | KeyCode::Char('l') => {
            *choice = choice.shift(1);
            None
        }
        // Enter 提交「当前选中的那一项」，不再写死 AllowOnce。
        KeyCode::Enter => Some(choice.decision()),
        KeyCode::Esc => Some(ApprovalDecision::Deny),
        // 字母快捷键保留：肌肉记忆与脚本化操作仍可用，不依赖光标位置。
        KeyCode::Char('y') | KeyCode::Char('o') => Some(ApprovalDecision::AllowOnce),
        KeyCode::Char('a') => Some(ApprovalDecision::AllowAlways),
        KeyCode::Char('n') | KeyCode::Char('d') => Some(ApprovalDecision::Deny),
        _ => None,
    }
}

/// 决策后写入 transcript 的结果词。
pub fn decision_summary(decision: &ApprovalDecision) -> &'static str {
    match decision {
        ApprovalDecision::Deny => "已拒绝",
        ApprovalDecision::AllowOnce => "已允许（仅此一次）",
        ApprovalDecision::AllowAlways => "已允许（始终）",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use std::sync::mpsc;

    fn req(command: &str) -> CommandApprovalData {
        CommandApprovalData {
            command: command.to_string(),
            args: "".to_string(),
            allowlist_key: None,
        }
    }

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    /// 从默认选中项出发按一个键（不关心光标移动的用例走这个）。
    fn decide(code: KeyCode, modifiers: KeyModifiers) -> Option<ApprovalDecision> {
        let mut choice = ApprovalChoice::default();
        decision_for_key(&key(code, modifiers), &mut choice)
    }

    #[test]
    fn decision_keys_map_like_repl_aliases() {
        assert_eq!(
            decide(KeyCode::Enter, KeyModifiers::NONE),
            Some(ApprovalDecision::AllowOnce)
        );
        assert_eq!(
            decide(KeyCode::Char('y'), KeyModifiers::NONE),
            Some(ApprovalDecision::AllowOnce)
        );
        assert_eq!(
            decide(KeyCode::Char('o'), KeyModifiers::NONE),
            Some(ApprovalDecision::AllowOnce)
        );
        assert_eq!(
            decide(KeyCode::Char('a'), KeyModifiers::NONE),
            Some(ApprovalDecision::AllowAlways)
        );
        assert_eq!(
            decide(KeyCode::Esc, KeyModifiers::NONE),
            Some(ApprovalDecision::Deny)
        );
        assert_eq!(
            decide(KeyCode::Char('n'), KeyModifiers::NONE),
            Some(ApprovalDecision::Deny)
        );
        assert_eq!(
            decide(KeyCode::Char('d'), KeyModifiers::NONE),
            Some(ApprovalDecision::Deny)
        );
    }

    #[test]
    fn control_c_denies_and_other_keys_ignored() {
        assert_eq!(
            decide(KeyCode::Char('c'), KeyModifiers::CONTROL),
            Some(ApprovalDecision::Deny)
        );
        assert_eq!(decide(KeyCode::Char('x'), KeyModifiers::NONE), None);
        assert_eq!(decide(KeyCode::Tab, KeyModifiers::NONE), None);
    }

    #[test]
    fn arrows_move_the_choice_and_enter_submits_whichever_is_selected() {
        let mut choice = ApprovalChoice::default();
        // 直接调用（不用闭包）：闭包会长期持有 `&mut choice`，后面就读不到它了。
        let key_of = |code: KeyCode| key(code, KeyModifiers::NONE);

        // 默认停在「仅此一次」：回车直接放行一次。
        assert_eq!(
            decision_for_key(&key_of(KeyCode::Enter), &mut choice),
            Some(ApprovalDecision::AllowOnce)
        );

        // → 移到「始终允许」：方向键只移动，不提交。
        assert_eq!(decision_for_key(&key_of(KeyCode::Right), &mut choice), None);
        assert_eq!(choice, ApprovalChoice::Always);
        assert_eq!(
            decision_for_key(&key_of(KeyCode::Enter), &mut choice),
            Some(ApprovalDecision::AllowAlways)
        );

        // → 到「拒绝」（vim 的 l 同样右移）。
        assert_eq!(
            decision_for_key(&key_of(KeyCode::Char('l')), &mut choice),
            None
        );
        assert_eq!(choice, ApprovalChoice::Deny);
        assert_eq!(
            decision_for_key(&key_of(KeyCode::Enter), &mut choice),
            Some(ApprovalDecision::Deny)
        );

        // 右端夹住：不会循环跳回「仅此一次」，否则多按一次就把拒绝变成放行。
        assert_eq!(decision_for_key(&key_of(KeyCode::Right), &mut choice), None);
        assert_eq!(choice, ApprovalChoice::Deny);

        // ← 回退（h 同样左移），左端同样夹住。
        assert_eq!(decision_for_key(&key_of(KeyCode::Left), &mut choice), None);
        assert_eq!(
            decision_for_key(&key_of(KeyCode::Char('h')), &mut choice),
            None
        );
        assert_eq!(choice, ApprovalChoice::Once);
        assert_eq!(decision_for_key(&key_of(KeyCode::Left), &mut choice), None);
        assert_eq!(choice, ApprovalChoice::Once);
    }

    /// 每次新审批都把光标复位：上一条停在哪，不该成为下一条的默认回答。
    #[test]
    fn begin_approval_resets_the_cursor_to_the_default_choice() {
        let mut st = crate::tui_mode::state::UiState::new();
        let (tx, _rx) = mpsc::channel();
        st.begin_approval(&req("rm"), tx);
        st.approval_choice = ApprovalChoice::Always;
        let (tx, _rx) = mpsc::channel();
        st.begin_approval(&req("mv"), tx);
        assert_eq!(st.approval_choice, ApprovalChoice::Once);
    }

    #[test]
    fn overlay_gate_round_trips_via_channel() {
        let (tx, rx) = mpsc::channel::<UiEvent>();
        let mut gate = OverlayApprovalGate { tx };
        let worker = std::thread::spawn(move || gate.decide(&req("rm")));
        // UI 侧收到请求
        let UiEvent::Approval { prompt } = rx.recv().expect("prompt") else {
            panic!("expected approval prompt");
        };
        assert_eq!(prompt.req.command, "rm");
        prompt.answer.send(ApprovalDecision::AllowAlways).unwrap();
        assert_eq!(
            worker.join().unwrap().unwrap(),
            ApprovalDecision::AllowAlways
        );
    }

    #[test]
    fn overlay_gate_denies_when_ui_drops() {
        let (tx, rx) = mpsc::channel::<UiEvent>();
        let mut gate = OverlayApprovalGate { tx };
        let worker = std::thread::spawn(move || gate.decide(&req("rm")));
        let UiEvent::Approval { prompt } = rx.recv().expect("prompt") else {
            panic!("expected approval prompt");
        };
        drop(prompt.answer); // UI 退出未答复
        assert_eq!(worker.join().unwrap().unwrap(), ApprovalDecision::Deny);
    }

    #[test]
    fn summary_words_for_decisions() {
        assert_eq!(decision_summary(&ApprovalDecision::Deny), "已拒绝");
        assert_eq!(
            decision_summary(&ApprovalDecision::AllowOnce),
            "已允许（仅此一次）"
        );
        assert_eq!(
            decision_summary(&ApprovalDecision::AllowAlways),
            "已允许（始终）"
        );
    }
}
