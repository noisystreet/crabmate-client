//! lane 出站事件的**路由**：来源 lane（[`LaneId`]）+ 归属会话（[`TurnSession`]）→ 谁处理。
//!
//! 为什么需要归属：回合跑着的时候用户可能切到别的会话，而 SSE 增量仍在到达。没有归属，
//! `LaneApp::on_msg` 只能写「当前会话的 transcript」——那正是把 A 的回答画进 B 的屏上。
//! Desktop 用的是同一套办法：`ChatStreamTransportLane::Bound { session_id }` 把流绑到会话、
//! `StreamTextOverlay { session_id, .. }` 把正文缓冲绑到会话，展示层按「承载该消息的会话 id」
//! 取而不是按当前 `active_id`。
//!
//! 三个部件：
//!
//! - [`TurnSession`]：一条出站事件归谁（非回合 / 某回合的某会话）。**不能**用
//!   `Option<String>` 表达 —— `None` 在「不是回合事件」与「回合但首轮未绑定会话」两边都合法，
//!   压成一个 `Option` 会让后者被当成全局事件写进别人的 transcript。
//! - [`LaneTx::with_session`]：回合的发送端派生一份贴了归属的，回合代码（`UiSink` /
//!   审批 gate / 收尾）一行都不用改就全部带上归属 —— 漏一处就是静默串台。
//! - [`split_turn`] + `TuiApp::on_lane_event`：在**一处**解包并路由，于是后台提示 /
//!   同 serve fan-out / 未读点这些既有判定看到的都是内层事件，不必知道包装层存在。
//!
//! 单独成模块的直接原因是行数门禁（`fn-nloc` 单文件 ≤ 920）：`worker.rs` 与 `tui_mode/mod.rs`
//! 在 `main` 上都已经正好 920 行，任何净增都会立刻转红。

use std::sync::mpsc::Sender;

use crate::serve::TermError;

use super::inbound::Inbound;
use super::worker::{LaneId, UiEvent};
use super::{LaneApp, TuiApp};

/// 一条出站事件**归谁**：非回合事件，还是某一轮回合（带它归属的会话）。
///
/// 与 Desktop 的 `Idle` / `Bound { session_id }` 两态同构 —— 那边也是靠两个变体而不是
/// 「`Option<session_id>`」来避免同一个歧义。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TurnSession {
    /// 非回合事件（会话刷新 / 工作区 / 历史分页 / ctl…）：写当前会话，不需要归属。
    Global,
    /// 某一轮回合；`None` = 发出时还没绑定会话（`/conv new` 之后的首轮）。
    Of(Option<String>),
}

/// 带来源 lane 的事件发送端：**lane 自己的出站事件一律经它发**。
///
/// 通道收 [`Inbound`]，`LaneTx` 是 [`Inbound::FromLane`] 的**唯一构造入口**：「忘记
/// 包一层来源」这个错误曾经只靠注释约定，漏一次就把 lane-2 的回合结束写进 lane-1 的
/// transcript —— 静默串台，不崩溃、不报错（issue #347）。外壳动作走 [`Inbound::Shell`]，
/// 那条路径不经过这里。
///
/// **这不是封装边界**：持有 `Sender<Inbound>` 的地方（`LaneApp::tx` / `LaneTemplate::tx`
/// / `spawn_worker` 的参数）仍能手写 `FromLane` 或把 lane 事件塞进 `Input`。类型消除的是
/// 旧形态（`Sender<UiEvent>` + 全凭约定打标签），不是所有绕过路径。
#[derive(Clone)]
pub(super) struct LaneTx {
    lane: LaneId,
    tx: Sender<Inbound>,
    /// [`TurnSession::Global`] = 非回合事件（刷新 / 工作区 / 历史分页…），按 lane 直接发；
    /// [`TurnSession::Of`] = 这条发送端属于某一轮回合，事件要包成 [`UiEvent::Turn`]。
    session: TurnSession,
}

impl LaneTx {
    pub(super) fn new(lane: LaneId, tx: Sender<Inbound>) -> Self {
        Self {
            lane,
            tx,
            session: TurnSession::Global,
        }
    }

    /// 派生一个「贴了回合归属」的发送端：之后经它发出的事件都带 `session`。
    ///
    /// 回合代码（`run_turn_job` / `UiSink` / 审批 gate）因此不必逐条记得写归属 ——
    /// 与 `LaneTx` 存在的理由相同：漏一处就是静默串台，而静默串台比崩溃难查得多。
    pub(super) fn with_session(&self, session: Option<String>) -> Self {
        Self {
            lane: self.lane,
            tx: self.tx.clone(),
            session: TurnSession::Of(session),
        }
    }

    /// 打上来源 lane 后发出；通道关闭即视为 UI 已退出。
    pub(super) fn send(&self, ev: UiEvent) -> Result<(), TermError> {
        let ev = match &self.session {
            // 已经包过的不再套第二层（`Turn` 只由这里产生，重复包装只会让解包漏一层）。
            TurnSession::Of(session) if !matches!(&ev, UiEvent::Turn { .. }) => UiEvent::Turn {
                session: session.clone(),
                ev: Box::new(ev),
            },
            _ => ev,
        };
        self.tx
            .send(Inbound::FromLane(self.lane, ev))
            .map_err(|_| TermError::Message("ui channel closed".into()))
    }
}

/// 拆掉 [`UiEvent::Turn`] 的包装：`(归属, 内层事件)`；非回合事件得到 [`TurnSession::Global`]。
pub(super) fn split_turn(ev: UiEvent) -> (TurnSession, UiEvent) {
    match ev {
        UiEvent::Turn { session, ev } => (TurnSession::Of(session), *ev),
        other => (TurnSession::Global, other),
    }
}

impl LaneApp<'_> {
    /// 回合类事件入口：`session` 是它**归属的会话**。
    ///
    /// 这一步只做「解包 + 交回既有分派」：回合期间还允许切会话之前，归属必然等于
    /// 当前会话，于是行为与改动前逐字相同（可回归的前提）。
    pub(super) fn on_turn_event(&mut self, _session: TurnSession, ev: UiEvent) {
        self.on_msg(ev);
    }
}

impl TuiApp<'_> {
    /// 一条 lane 的出站事件：拆归属 → 三件全局动作 → 交给那条 lane。
    ///
    /// 三件全局动作的顺序是既有口径（issue #337 / #349 / 未读点），一起放在这里是因为
    /// 它们都要先看到**内层**事件；`mod.rs` 只留一行调用，省下的行数正好给归属逻辑用。
    ///
    /// 目标 lane 已关闭时丢弃：绝不把增量写进别的 transcript —— 串台比丢一条增量难查得多。
    pub(super) fn on_lane_event(&mut self, id: LaneId, ev: UiEvent) {
        let (session, ev) = split_turn(ev);
        // 后台 lane 弹审批先在本 lane 留一句提示：浮层只画当前 lane，不说一声就是最长
        // 600s 的静默停摆。提示归提示，事件仍要分派给**它自己**那条 lane。
        self.announce_background_approval(id, &ev);
        // 同一台 serve 的其它 lane 也要拿到这份事实：刷新已按 serve 去重，回包得复制过去，
        // 否则被跳过的那条会话列表就再也不更新。
        self.fanout_shared_refresh(id, &ev);
        // 后台 lane 说了话、用户还没看：点亮左栏那一行（切过去即熄）。
        self.mark_background_unread(id, &ev);
        if let Some(lane) = self.lanes.iter_mut().find(|l| l.id == id) {
            lane.on_turn_event(session, ev);
        }
    }
}
