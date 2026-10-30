//! 审批浮层双向握手（M3）：SSE 回合线程的 gate 把请求经 UI 事件通道发给事件循环
//! 并阻塞在应答通道上等决策；事件循环保持运行收按键。Esc/n=拒绝、Enter/y/o=一次、
//! a=始终（与 repl 的 Tty 提示语义一致）。

use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::Duration;

use crate::serve::{ApprovalDecision, ApprovalGate, CommandApprovalData, TermError};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::LaneApp;
use super::render;
use super::state::ApprovalOverlay;
use super::worker::{LaneTx, UiEvent};
use ratatui::layout::Rect;

/// 一条待 UI 决策的审批请求（携带回传通道）。
pub struct ApprovalPrompt {
    pub req: CommandApprovalData,
    pub answer: Sender<ApprovalDecision>,
}

/// 等用户按键的上限。
///
/// 人会走开，所以给得宽；但**必须有上限**。`recv()` 无限等会把整个 worker 线程停在这里，
/// 之后每一条消息都排在它后面 —— 界面照常能打字，却再也不会有回应，而且没有任何提示，
/// 看上去就是卡死。超时按「拒绝」收尾（保守方向：不放行未确认的命令），并留一条系统行。
pub const APPROVAL_WAIT: Duration = Duration::from_secs(600);

/// 全屏浮层审批 gate（运行在 SSE 回合任务内）。
///
/// `decide` 把请求发给 UI 事件循环，然后阻塞等决策；UI 通道关闭（退出/崩溃）时
/// 回 `Deny` 兜底，避免 serve 侧审批会话悬挂（与文本模式读行失败路径一致）。
pub struct OverlayApprovalGate {
    /// 打标签的出口：浮层与超时提示都属于**发起这条 SSE 的 lane** ——
    /// 裸发会落到当前聚焦的那条，多 lane 下就是弹在别人的屏幕上。
    pub out: LaneTx,
    /// 等决策的上限；生产用 [`APPROVAL_WAIT`]，单测可传毫秒级以便覆盖超时路径。
    pub wait: Duration,
}

impl ApprovalGate for OverlayApprovalGate {
    fn decide(&mut self, req: &CommandApprovalData) -> Result<ApprovalDecision, TermError> {
        let (answer_tx, answer_rx) = mpsc::channel::<ApprovalDecision>();
        self.out.send(UiEvent::Approval {
            prompt: ApprovalPrompt {
                req: req.clone(),
                answer: answer_tx,
            },
        })?;
        match answer_rx.recv_timeout(self.wait) {
            Ok(decision) => Ok(decision),
            Err(RecvTimeoutError::Timeout) => {
                // 通知 UI 撤掉浮层：它手里那条 `answer` 此刻已断开，再按键只会静默失败，
                // 而屏幕上还显示着「等你确认」，与真实状态相反。
                let _ = self.out.send(UiEvent::ApprovalExpired {
                    notice: format!(
                        "审批等待超过 {} 分钟，已按「拒绝」处理",
                        self.wait.as_secs().div_ceil(60).max(1)
                    ),
                });
                Ok(ApprovalDecision::Deny)
            }
            // UI 侧丢弃了回传通道（退出/崩溃）：与超时同向兜底。
            Err(RecvTimeoutError::Disconnected) => Ok(ApprovalDecision::Deny),
        }
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

/// 展开态下 `PgUp` / `PgDn` 一步滚几行。
const APPROVAL_PAGE_STEP: usize = 8;

/// 浮层「看完整命令」的按键：`Ctrl+E` 展开/收起；展开后 `↑/↓`（vim 的 `k`/`j`）逐行、
/// `PgUp`/`PgDn` 翻页、`Home`/`End` 到首尾。返回 `true` 表示已消费——这些键既不能
/// 参与决策，也不能漏到输入框。
///
/// `bounds` 是命令区的（总行数, 可显示行数），由渲染侧同一套折行算出。上界必须精确：
/// 否则 `End` 会停在渲染时才被截掉的越界值上，之后按 `↑` 看起来像失灵。
pub(super) fn approval_view_key(
    key: &KeyEvent,
    overlay: &mut ApprovalOverlay,
    bounds: (usize, usize),
) -> bool {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        // Ctrl+E：审批期间它本来空闲（thinking 折叠只在回合内才有意义）。
        if !matches!(key.code, KeyCode::Char('e')) {
            return false;
        }
        overlay.expanded = !overlay.expanded;
        overlay.scroll = 0;
        return true;
    }
    if !overlay.expanded {
        return false;
    }
    let (total, body) = bounds;
    let max_scroll = total.saturating_sub(body);
    let next = match key.code {
        KeyCode::Up | KeyCode::Char('k') => overlay.scroll.saturating_sub(1),
        KeyCode::Down | KeyCode::Char('j') => overlay.scroll.saturating_add(1),
        KeyCode::PageUp => overlay.scroll.saturating_sub(APPROVAL_PAGE_STEP),
        KeyCode::PageDown => overlay.scroll.saturating_add(APPROVAL_PAGE_STEP),
        KeyCode::Home => 0,
        KeyCode::End => max_scroll,
        _ => return false,
    };
    overlay.scroll = next.min(max_scroll);
    true
}

/// 浮层画在「聊天主体」上，而 `last_size` 是整屏：这里按固定 6 行估掉 composer
/// 边框 + 内容 + 状态栏。估大的方向只让滚动上界略宽，渲染侧还会再夹一次。
const APPROVAL_CHROME_ROWS: u16 = 6;

// 用显式生命周期而不是 `impl LaneApp<'_>`：lizard 的 Rust 解析器把 `'_` 当成
// 未闭合的字符字面量，会从这一行开始把后面所有函数体吞进同一个函数（fn-nloc 会误报）。
impl<'a> LaneApp<'a> {
    /// 审批浮层期间的按键：先给「看完整命令」（`Ctrl+E` 展开、展开后滚动），再落到决策。
    ///
    /// 顺序不能反：展开态下 `↑/↓` 是滚动、`PgUp/PgDn` 是翻页；若先问 [`decision_for_key`]，
    /// 方向键会被当成「移动选项」，滚动就永远不生效。
    pub(super) fn on_approval_key(&mut self, key: &KeyEvent) {
        let bounds = self.approval_view_bounds();
        let handled = self
            .st
            .approval
            .as_mut()
            .is_some_and(|ap| approval_view_key(key, ap, bounds));
        if handled {
            return;
        }
        if let Some(decision) = decision_for_key(key, &mut self.st.approval_choice) {
            let via_ctrl_abort = super::hotkeys::is_ctrl_abort(key);
            self.answer_approval(decision, via_ctrl_abort);
        }
    }

    /// 浮层命令区的（总行数, 可显示行数）：与渲染同一套折行，用来夹住滚动上界。
    fn approval_view_bounds(&self) -> (usize, usize) {
        let Some(ap) = self.st.approval.as_ref() else {
            return (0, 0);
        };
        let (w, h) = self.last_size;
        let area = Rect::new(0, 0, w, h.saturating_sub(APPROVAL_CHROME_ROWS));
        render::approval_scroll_bounds(area, ap, ap.expanded)
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
#[path = "approve_view_tests.rs"]
mod approve_view_tests;

#[cfg(test)]
mod tests {
    use super::super::worker::LaneId;
    use super::*;
    use crossterm::event::KeyModifiers;
    use std::sync::mpsc;

    fn req(command: &str) -> CommandApprovalData {
        CommandApprovalData {
            command: command.to_string(),
            args: "".to_string(),
            allowlist_key: None,
            permission_level: None,
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
        let mut gate = OverlayApprovalGate {
            out: LaneTx::new(LaneId(1), tx),
            wait: APPROVAL_WAIT,
        };
        let worker = std::thread::spawn(move || gate.decide(&req("rm")));
        // UI 侧收到请求
        let UiEvent::Lane(_, inner) = rx.recv().expect("prompt") else {
            panic!("expected a lane-tagged approval prompt");
        };
        let UiEvent::Approval { prompt } = *inner else {
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
        let mut gate = OverlayApprovalGate {
            out: LaneTx::new(LaneId(1), tx),
            wait: APPROVAL_WAIT,
        };
        let worker = std::thread::spawn(move || gate.decide(&req("rm")));
        let UiEvent::Lane(_, inner) = rx.recv().expect("prompt") else {
            panic!("expected a lane-tagged approval prompt");
        };
        let UiEvent::Approval { prompt } = *inner else {
            panic!("expected approval prompt");
        };
        drop(prompt.answer); // UI 退出未答复
        assert_eq!(worker.join().unwrap().unwrap(), ApprovalDecision::Deny);
    }

    /// 用户始终不回答（浮层没被处理 / 人走开）：必须有一个上限把 worker 放回来，
    /// 否则整条 worker 线程永久停在这里，后续消息全部排队，且屏幕上没有任何提示。
    #[test]
    fn overlay_gate_times_out_as_deny() {
        let (tx, rx) = mpsc::channel::<UiEvent>();
        let mut gate = OverlayApprovalGate {
            out: LaneTx::new(LaneId(1), tx),
            wait: Duration::from_millis(30),
        };
        let worker = std::thread::spawn(move || gate.decide(&req("rm")));
        let UiEvent::Lane(_, inner) = rx.recv().expect("prompt") else {
            panic!("expected a lane-tagged approval prompt");
        };
        let UiEvent::Approval { prompt } = *inner else {
            panic!("expected approval prompt");
        };
        // 通道保持打开但不答复 —— 正是「UI 收到了却没人处理」那种状态。
        let decision = worker.join().unwrap().unwrap();
        assert_eq!(decision, ApprovalDecision::Deny, "超时按拒绝收尾");
        drop(prompt);
        let UiEvent::Lane(_, inner) = rx.recv().expect("timeout notice") else {
            panic!("expected a lane-tagged timeout notice");
        };
        let UiEvent::ApprovalExpired { notice } = *inner else {
            panic!("expected an ApprovalExpired after timeout");
        };
        assert!(notice.contains("拒绝"), "提示应说明按拒绝处理: {notice}");
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
