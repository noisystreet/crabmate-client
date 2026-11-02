//! 左右侧栏的列宽。布局（`layout`）与右栏状态栈（`status_stack` / `render`）共用这一份，
//! 免得两处各算一遍导致「状态栈底部空一行」或「点击落到别的行」。
//!
//! 拆成两个默认值而不是一个：左栏只放会话列表与提示行，26 列够用；右栏底部还叠着纵向状态栈，
//! 其中 `ctx 116.9k / 1000k (11.7%)` 需要「值 21 列 + label 6 列」—— 左右同为 26 时内宽只有
//! 24，这一行会被截成 `… (1…`，看上去就像用量溢出。
//!
//! 宽度在运行期可调（issue #407）：[`PaneWidths`] 存**用户的选择**，[`PaneWidths::clamped`]
//! 按当前终端宽度把它夹进安全范围。渲染与鼠标命中都只认夹取后的那一份（[`PaneLayout`]）——
//! 两边各夹一次就会漂移，症状是「点到的列与画出的列差一列」，比画错更难归因。

/// 左会话列默认宽度。
pub(crate) const SIDEBAR_WIDTH: u16 = 26;

/// 右工作区列默认宽度（比左栏宽，理由见模块注释）。
pub(crate) const WORKSPACE_PANE_WIDTH: u16 = 34;

/// 左栏可调下限：再窄就放不下「`▶ ` + 标题前十几列」，会话列表退化成一片省略号。
///
/// 取 20 而不是默认值 26 是有意的：底部按键提示（`↑↓ Enter用 Del删 n新 r刷`，约 24 列）
/// 在这个窄档会被 `render_pane` 截断。但那是用户**主动**拖窄的结果 —— 宁可少显示几个键，
/// 也不该让「想腾点地方给正文」的人完全无路可走。
pub(crate) const MIN_SIDEBAR_COLS: u16 = 20;

/// 左栏可调上限：再宽就开始抢正文，而会话标题在 60 列内早已放得下。
pub(crate) const MAX_SIDEBAR_COLS: u16 = 60;

/// 右栏可调下限：`宽度 - 2` 的内宽仍放得下目录树的一层缩进 + 文件名。
pub(crate) const MIN_WORKSPACE_COLS: u16 = 20;

/// 右栏可调上限（理由同 [`MAX_SIDEBAR_COLS`]）。
pub(crate) const MAX_WORKSPACE_COLS: u16 = 80;

/// 聊天列保底宽度：两栏合计最多占「终端宽度 - 本值」。
///
/// 有它才有 [`PaneWidths::clamped`] 的「先压右栏、再压左栏」：没有保底，用户把两栏拖宽
/// 之后一缩窗口，正文就被挤成一条竖线。
pub(crate) const MIN_CHAT_COLS: u16 = 40;

/// 两栏的列宽（用户的选择，**未**夹取）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PaneWidths {
    /// 左会话列。
    pub left: u16,
    /// 右工作区列。
    pub right: u16,
}

impl Default for PaneWidths {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl PaneWidths {
    /// 默认宽度：与最初的两个常量同值，所以这次改动本身不改版面。
    pub(crate) const DEFAULT: Self = Self {
        left: SIDEBAR_WIDTH,
        right: WORKSPACE_PANE_WIDTH,
    };

    /// 按终端宽度夹取：两栏各自落在 `[MIN, MAX]`，且合计不超过 `终端宽度 - MIN_CHAT_COLS`。
    ///
    /// 超出预算时**先压右栏、再压左栏**：右栏底部是状态栈（挤窄只是把 `ctx` 行截断），
    /// 左栏底部是 lane 列表（挤窄后连「现在连的哪台 serve」都读不全），而会话列表是两栏里
    /// 用户最先愿意放弃的那部分。两边都到下限仍不够时**保持下限**、把问题留给上层
    /// （低于 `render::SIDEBAR_MIN_WIDTH` 时两栏本来就不显示），不在这里偷偷隐藏 ——
    /// 可见性是 `apply_size` 的唯一职责，两处都判会给出两个答案。
    ///
    /// 幂等：对已夹取的值再调一次不变。调用方因此不必记得「先夹再传」——
    /// `layout::panes_with_lanes` 内部还会再夹一次，两处结果必然相同。
    #[must_use]
    pub(crate) fn clamped(self, terminal_width: u16) -> Self {
        let left = self.left.clamp(MIN_SIDEBAR_COLS, MAX_SIDEBAR_COLS);
        let right = self.right.clamp(MIN_WORKSPACE_COLS, MAX_WORKSPACE_COLS);
        // 预算 = 两栏能合占的最大列数。`saturating_sub` 保证极窄终端下算成 0 而不是下溢。
        let budget = terminal_width.saturating_sub(MIN_CHAT_COLS);
        if left.saturating_add(right) <= budget {
            return Self { left, right };
        }
        // 第一轮：右栏让到「扣掉左栏后剩下的」，但不低于它自己的下限。
        let right = budget.saturating_sub(left).max(MIN_WORKSPACE_COLS);
        if left.saturating_add(right) <= budget {
            return Self { left, right };
        }
        // 第二轮：右栏已到底，左栏再让到「扣掉右栏后剩下的」，同样不低于下限。
        let left = budget.saturating_sub(right).max(MIN_SIDEBAR_COLS);
        Self { left, right }
    }
}

/// 一帧的两栏版面：可见性 + 夹取后的列宽。
///
/// 合成一个结构而不是两个入参：调用点很容易只更新宽度、忘了 `visible` 也跟着走
/// （例如把宽度换成夹取后的值，却仍用旧的可见性拼 `Constraint`）。合成后
/// `layout::panes_with_lanes` 只有一个「版面」入参，两栏 / 状态栈 / lane 区共用同一份结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PaneLayout {
    /// 两栏是否显示（窄屏隐藏；由 `apply_size` 按 `render::SIDEBAR_MIN_WIDTH` 派生）。
    pub visible: bool,
    /// 夹取后的两栏宽度。
    pub widths: PaneWidths,
}

impl PaneLayout {
    /// 按终端宽度夹取宽度；`visible` 原样带过。
    #[must_use]
    pub(crate) fn clamped(self, terminal_width: u16) -> Self {
        Self {
            visible: self.visible,
            widths: self.widths.clamped(terminal_width),
        }
    }
}

impl super::state::UiState {
    /// 本帧的两栏版面（可见性 + 用户选择的宽度，**未**夹取）。
    ///
    /// 需要「按终端宽度夹取后的值」时在调用点 `.clamped(宽)`：夹取幂等，所以
    /// `layout::panes_with_lanes` 内部再夹一次也不会给出第二个答案。
    ///
    /// 放在本模块而不是 `state.rs`：后者贴着 920 行/文件的 `fn-nloc-ratchet` 门禁，
    /// 而这个取值器只依赖 `sidebar_visible` / `pane` 两个字段。
    #[must_use]
    pub(crate) fn pane_layout(&self) -> PaneLayout {
        PaneLayout {
            visible: self.sidebar_visible,
            widths: self.pane,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_SIDEBAR_COLS, MAX_WORKSPACE_COLS, MIN_CHAT_COLS, MIN_SIDEBAR_COLS, MIN_WORKSPACE_COLS,
        PaneLayout, PaneWidths, SIDEBAR_WIDTH, WORKSPACE_PANE_WIDTH,
    };

    #[test]
    fn defaults_match_the_historical_constants() {
        assert_eq!(PaneWidths::DEFAULT.left, SIDEBAR_WIDTH);
        assert_eq!(PaneWidths::DEFAULT.right, WORKSPACE_PANE_WIDTH);
        assert_eq!(PaneWidths::default(), PaneWidths::DEFAULT);
    }

    /// 宽终端里用户的选择原样生效 —— 默认值尤其不能被夹动，否则「什么都没改却换了版面」。
    #[test]
    fn wide_terminal_keeps_the_requested_widths() {
        assert_eq!(
            PaneWidths::DEFAULT.clamped(200),
            PaneWidths::DEFAULT,
            "200 列下默认宽度远未超预算"
        );
        let wide = PaneWidths {
            left: 50,
            right: 70,
        };
        assert_eq!(wide.clamped(200), wide);
    }

    #[test]
    fn each_pane_is_clamped_into_its_own_range() {
        let tiny = PaneWidths { left: 1, right: 1 };
        assert_eq!(tiny.clamped(200).left, MIN_SIDEBAR_COLS);
        assert_eq!(tiny.clamped(200).right, MIN_WORKSPACE_COLS);

        let huge = PaneWidths {
            left: u16::MAX,
            right: u16::MAX,
        };
        let got = huge.clamped(1000);
        assert_eq!(got.left, MAX_SIDEBAR_COLS);
        assert!(got.right <= MAX_WORKSPACE_COLS);
    }

    /// 预算不够时**先压右栏**：右栏挤窄只是状态栈行被截断，左栏挤窄读不出会话标题。
    #[test]
    fn right_pane_gives_way_before_the_left_one() {
        // 120 列 → 两栏预算 80；请求 60 + 60 = 120。左栏 60 保留，右栏降到下限 20。
        let got = PaneWidths {
            left: 60,
            right: 60,
        }
        .clamped(120);
        assert_eq!(got.left, 60, "右栏还没到底，左栏不该先让");
        assert_eq!(got.right, MIN_WORKSPACE_COLS);
        assert!(got.left as u32 + got.right as u32 + MIN_CHAT_COLS as u32 <= 120);
    }

    /// 右栏让到「扣掉左栏后剩下的」就够时**不必让到底**：缺口有多少让多少，
    /// 免得白白把状态栈挤成最短的那一档。
    #[test]
    fn right_pane_stops_as_soon_as_the_budget_fits() {
        // 130 列 → 预算 90；上限 60 + 80 = 140。右栏让到 30 正好用满，左栏一列不动。
        let got = PaneWidths {
            left: MAX_SIDEBAR_COLS,
            right: MAX_WORKSPACE_COLS,
        }
        .clamped(130);
        assert_eq!(got.left, MAX_SIDEBAR_COLS);
        assert_eq!(got.right, 30);
        assert_eq!(
            got.left as u32 + got.right as u32 + MIN_CHAT_COLS as u32,
            130,
            "正好用满预算，不多占正文一列"
        );
    }

    /// 右栏一个人让不出预算时，左栏接着让 —— 聊天列的保底由两栏共同守住。
    #[test]
    fn left_pane_takes_over_when_the_right_one_hits_its_floor() {
        // 100 列 → 预算 60；左栏 60 已占满，右栏让到下限 20 仍超 ⇒ 左栏降到 40。
        let got = PaneWidths {
            left: MAX_SIDEBAR_COLS,
            right: MAX_WORKSPACE_COLS,
        }
        .clamped(100);
        assert_eq!(got.right, MIN_WORKSPACE_COLS, "右栏先让步到底");
        assert_eq!(got.left, 40, "剩下的缺口由左栏补");
        assert_eq!(
            got.left as u32 + got.right as u32 + MIN_CHAT_COLS as u32,
            100,
            "正好用满预算，不多占正文一列"
        );
    }

    /// 极窄终端（低到连两个下限都放不下）：两边各自停在自身下限，**不**返回 0 ——
    /// 可见性由 `apply_size` 判，夹取函数只负责「别给出非法值」。
    #[test]
    fn absurdly_narrow_terminal_keeps_both_lower_bounds() {
        let got = PaneWidths::DEFAULT.clamped(0);
        assert_eq!(got.left, MIN_SIDEBAR_COLS);
        assert_eq!(got.right, MIN_WORKSPACE_COLS);
    }

    /// 幂等：夹过一次的宽度再夹一次不变。`draw` 与 `panes` 各夹一次的前提就是这个。
    #[test]
    fn clamping_is_idempotent() {
        for width in [0, 40, 80, 120, 200, 400] {
            let once = PaneWidths {
                left: 55,
                right: 75,
            }
            .clamped(width);
            assert_eq!(once.clamped(width), once, "width={width}");
        }
    }

    #[test]
    fn layout_clamp_keeps_visibility_untouched() {
        let layout = PaneLayout {
            visible: false,
            widths: PaneWidths {
                left: 99,
                right: 99,
            },
        };
        let got = layout.clamped(120);
        assert!(!got.visible, "夹取宽度不该顺手改可见性");
        assert!(got.widths.left <= MAX_SIDEBAR_COLS);
    }
}
