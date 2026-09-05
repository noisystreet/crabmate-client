//! 审批浮层双向握手（M3）：SSE 回合线程的 gate 把请求经 UI 事件通道发给事件循环
//! 并阻塞在应答通道上等决策；事件循环保持运行收按键。Esc/n=拒绝、Enter/y/o=一次、
//! a=始终（与 repl 的 Tty 提示语义一致）。

use std::sync::mpsc::{self, Sender};

use crabmate_tui_core::{ApprovalDecision, ApprovalGate, CommandApprovalRequest, TermError};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::UiEvent;

/// 一条待 UI 决策的审批请求（携带回传通道）。
pub struct ApprovalPrompt {
    pub req: CommandApprovalRequest,
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
    fn decide(&mut self, req: &CommandApprovalRequest) -> Result<ApprovalDecision, TermError> {
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

/// 浮层打开期间的按键 → 决策（其余按键忽略，防叠栈/误输入）。
pub fn decision_for_key(key: &KeyEvent) -> Option<ApprovalDecision> {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        // Ctrl+C / Ctrl+D：与文本模式同语义地把审批当拒绝（回合取消需再次 Ctrl+C）。
        return matches!(key.code, KeyCode::Char('c') | KeyCode::Char('d'))
            .then_some(ApprovalDecision::Deny);
    }
    match key.code {
        KeyCode::Enter => Some(ApprovalDecision::AllowOnce),
        KeyCode::Esc => Some(ApprovalDecision::Deny),
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

    fn req(command: &str) -> CommandApprovalRequest {
        CommandApprovalRequest {
            command: command.to_string(),
            args: "".to_string(),
            allowlist_key: None,
        }
    }

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn decision_keys_map_like_repl_aliases() {
        assert_eq!(
            decision_for_key(&key(KeyCode::Enter, KeyModifiers::NONE)),
            Some(ApprovalDecision::AllowOnce)
        );
        assert_eq!(
            decision_for_key(&key(KeyCode::Char('y'), KeyModifiers::NONE)),
            Some(ApprovalDecision::AllowOnce)
        );
        assert_eq!(
            decision_for_key(&key(KeyCode::Char('o'), KeyModifiers::NONE)),
            Some(ApprovalDecision::AllowOnce)
        );
        assert_eq!(
            decision_for_key(&key(KeyCode::Char('a'), KeyModifiers::NONE)),
            Some(ApprovalDecision::AllowAlways)
        );
        assert_eq!(
            decision_for_key(&key(KeyCode::Esc, KeyModifiers::NONE)),
            Some(ApprovalDecision::Deny)
        );
        assert_eq!(
            decision_for_key(&key(KeyCode::Char('n'), KeyModifiers::NONE)),
            Some(ApprovalDecision::Deny)
        );
        assert_eq!(
            decision_for_key(&key(KeyCode::Char('d'), KeyModifiers::NONE)),
            Some(ApprovalDecision::Deny)
        );
    }

    #[test]
    fn control_c_denies_and_other_keys_ignored() {
        assert_eq!(
            decision_for_key(&key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Some(ApprovalDecision::Deny)
        );
        assert_eq!(
            decision_for_key(&key(KeyCode::Char('x'), KeyModifiers::NONE)),
            None
        );
        assert_eq!(
            decision_for_key(&key(KeyCode::Tab, KeyModifiers::NONE)),
            None
        );
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
