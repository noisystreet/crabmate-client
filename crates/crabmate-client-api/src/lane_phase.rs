//! 一条 lane 的**生命周期阶段**（前台 / 后台 / 关闭中 / 已关闭）。
//!
//! 与 [`crate::connections::ConnectionStatus`] **正交**：那个回答「这条 lane 的回合
//! 在做什么」（Idle / Running / WaitingApproval / Quitting），这个回答「这条 lane
//! 本身处于哪个阶段」。所以「后台 + 回合在跑」是合法组合 —— 那正是「后台降频」与
//! 「后台审批怎么被看见」要判的那一格。
//!
//! # 为什么要先立这张表
//!
//! 单窗口多 lane（#561）的三个开放问题看起来是三件事，实际是同一张转移表上的三个
//! 查询：
//!
//! | 问题 | 落点 |
//! |---|---|
//! | 后台副作用降频 | `!is_foreground()` 时，定时类副作用让位 |
//! | 关闭一条 lane | `CloseRequested` → `Closing` → `Closed`；「中止在途回合」复用既有的用户中止链路，不引入第三种关闭语义 |
//! | 后台审批可见性 | 只有 `can_present_modal()` 弹浮层；后台走「事件 → 多个呈现通道」 |
//!
//! 先立表的好处是**它不需要第二条 lane 就有消费者**（转移函数是纯逻辑、可单测），
//! 而那三个问题各自降为「给定 phase 该怎么表现」，不再散落成三处判断。
//!
//! # 跨端
//!
//! TUI 的 `TuiApp.active: usize` 可以直接映射：当前那条 = [`LanePhase::Foreground`]，
//! 其余 = [`LanePhase::Background`]。一条 lane 的终端会话里它恒为 `Foreground`。
//! 所以放在共享层而不是前端 —— 两端的「切 lane 时该做什么」不该各写一遍。
//!
//! # 与 `Quitting` 的区别
//!
//! [`crate::connections::ConnectionStatus::Quitting`] 是「客户端要退出、等本轮结束」
//! （TUI 的 `/quit`）；本模块的 [`LanePhase::Closing`] 是「**这一条** lane 要销毁」。
//! 按「复用中止链路」的口径，桌面的关闭走的是中止（不是 Quitting），所以两者不合并。

/// lane 处于哪个阶段。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LanePhase {
    /// 前台：用户正在看的那一条（一个窗口里最多一条）。
    #[default]
    Foreground,
    /// 后台：还活着、回合可以继续跑，但屏幕上看不到它。
    Background,
    /// 关闭中：已请求销毁，正在收尾（中止在途回合 + 释放 lane 级状态）。
    Closing,
    /// 已关闭：终态，不再接受任何会改变它的事件。
    Closed,
}

/// 喂给转移表的事件 —— 都是「**事实已发生**」，不是「请求」。
///
/// 这个区分是有意的：`CloseRequested` 表示「用户确实点了关闭」这个事实；而「该不该
/// 弹确认框」是 UI 的另一个决定，不进这张表（否则转移函数要懂 i18n 与弹层）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaneEvent {
    /// 前台归属变化；`foreground = true` 表示这条 lane 成为前台。
    FocusChanged { foreground: bool },
    /// 用户要求关闭这条 lane（点关闭、关掉它对应的窗口）。
    CloseRequested,
    /// 收尾完成（中止链路已结束、lane 级状态已释放）。
    CloseSettled,
}

impl LanePhase {
    /// 推进一个事件。
    ///
    /// 三条不变量（都由下面的单测钉住）：
    ///
    /// 1. **`Closed` 吸收一切**：否则一条已销毁的 lane 会被迟到的聚焦/事件「复活」。
    /// 2. **`Closing` 不接受聚焦**：把一个将被销毁的 lane 又抬到前台，用户会看到
    ///    一个正在消失的界面。
    /// 3. **迟到的 `CloseSettled` 被忽略**：活动态收到「收尾完成」说明事件乱序，改状态
    ///    只会更糟。
    #[must_use]
    pub fn next(self, event: LaneEvent) -> Self {
        use LaneEvent::{CloseRequested, CloseSettled, FocusChanged};
        match (self, event) {
            // 终态。
            (Self::Closed, _) => Self::Closed,
            // 关闭中只认「收尾完成」。
            (Self::Closing, CloseSettled) => Self::Closed,
            (Self::Closing, _) => Self::Closing,
            // 关闭请求从任一活动态进入 Closing（重复请求幂等）。
            (_, CloseRequested) => Self::Closing,
            // 活动态收到迟到的收尾完成：忽略。
            (_, CloseSettled) => self,
            (Self::Foreground, FocusChanged { foreground: true }) => Self::Foreground,
            (Self::Foreground, FocusChanged { foreground: false }) => Self::Background,
            (Self::Background, FocusChanged { foreground: true }) => Self::Foreground,
            (Self::Background, FocusChanged { foreground: false }) => Self::Background,
        }
    }

    /// 是否前台（用户正看着它）。
    #[must_use]
    pub fn is_foreground(self) -> bool {
        matches!(self, Self::Foreground)
    }

    /// 后台工作（在途回合 / SSE 回调）是否该继续。
    ///
    /// `Closing` 之后**不**继续：`CloseRequested` 的语义就是「复用中止链路」，所以
    /// 进入 `Closing` 时在途回合已在收尾，不该再被算成「它在后台干活」（否则降频与
    /// 未读置位的判断都会多算一条正在消失的 lane）。
    #[must_use]
    pub fn keeps_background_work(self) -> bool {
        matches!(self, Self::Foreground | Self::Background)
    }

    /// 能否弹需要用户决策的浮层（审批 / 澄清）。
    ///
    /// 只有前台能弹：后台那条的浮层画在屏幕外，弹了等于没有 —— TUI 侧同一个理由
    /// （issue #337），桌面多窗口同理（`desktop_multi_window.md` §5）。后台的审批
    /// 走「事件 → 多个呈现通道」（栏位 `▲` / 系统通知 / 视图内提示），不是弹框。
    #[must_use]
    pub fn can_present_modal(self) -> bool {
        self.is_foreground()
    }

    /// 是否已不在活动期（关闭中或已关闭）。
    ///
    /// 用于「不再计入 lane 数、不再上报状态、不再接受切换」—— 与
    /// [`Self::keeps_background_work`] 分开是因为后者问的是「工作要不要继续」，
    /// 这里是「它还存不存在」：`Closing` 两者都是 `false`/`true` 的不同组合。
    #[must_use]
    pub fn is_gone(self) -> bool {
        matches!(self, Self::Closing | Self::Closed)
    }
}

#[cfg(test)]
mod tests {
    use super::{LaneEvent, LanePhase};
    use LaneEvent::{CloseRequested, CloseSettled, FocusChanged};
    use LanePhase::{Background, Closed, Closing, Foreground};

    #[test]
    fn focus_moves_between_foreground_and_background() {
        assert_eq!(
            Foreground.next(FocusChanged { foreground: false }),
            Background
        );
        assert_eq!(
            Background.next(FocusChanged { foreground: true }),
            Foreground
        );
        // 幂等：重复的同一方向事件不改状态。
        assert_eq!(
            Foreground.next(FocusChanged { foreground: true }),
            Foreground
        );
        assert_eq!(
            Background.next(FocusChanged { foreground: false }),
            Background
        );
    }

    #[test]
    fn a_close_request_enters_closing_from_any_active_phase() {
        for phase in [Foreground, Background] {
            assert_eq!(phase.next(CloseRequested), Closing, "{phase:?}");
            // 重复请求幂等。
            assert_eq!(phase.next(CloseRequested).next(CloseRequested), Closing);
        }
    }

    #[test]
    fn closing_refuses_focus_and_finishes_on_settle() {
        assert_eq!(Closing.next(FocusChanged { foreground: true }), Closing);
        assert_eq!(Closing.next(FocusChanged { foreground: false }), Closing);
        assert_eq!(Closing.next(CloseSettled), Closed);
    }

    #[test]
    fn closed_absorbs_everything() {
        for event in [
            FocusChanged { foreground: true },
            FocusChanged { foreground: false },
            CloseRequested,
            CloseSettled,
        ] {
            assert_eq!(
                Closed.next(event),
                Closed,
                "已销毁的 lane 不该被 {event:?} 复活"
            );
        }
    }

    #[test]
    fn a_late_settle_in_an_active_phase_is_ignored() {
        // 活动态收到「收尾完成」说明事件乱序：改状态只会更糟。
        for phase in [Foreground, Background] {
            assert_eq!(phase.next(CloseSettled), phase);
        }
    }

    #[test]
    fn only_the_foreground_is_presentable_and_foreground() {
        for phase in [Foreground, Background, Closing, Closed] {
            assert_eq!(phase.is_foreground(), phase == Foreground);
            assert_eq!(phase.can_present_modal(), phase == Foreground);
        }
    }

    #[test]
    fn background_work_stops_once_closing_starts() {
        assert!(Foreground.keeps_background_work());
        assert!(Background.keeps_background_work(), "后台仍在跑回合");
        assert!(!Closing.keeps_background_work(), "关闭中已在收尾");
        assert!(!Closed.keeps_background_work());
    }

    #[test]
    fn gone_and_keeps_work_are_independent_questions() {
        // 同一格 `Closing` 的两种答案：工作不再继续，而它也确实要消失了。
        assert!(Closing.is_gone());
        assert!(!Closing.keeps_background_work());
        assert!(!Foreground.is_gone());
        assert!(Foreground.keeps_background_work());
    }

    #[test]
    fn the_default_phase_is_foreground() {
        // 新建的 lane 就是用户正要看的那条。
        assert_eq!(LanePhase::default(), Foreground);
    }
}
