//! `LaneApp` 的收尾入口：让一条 lane 停下手上**所有**事（issue #346 / #348）。
//!
//! 从 `shell.rs` 挪出来只为 `fn-nloc-ratchet` 的单文件 920 行上限：这里跟「有几条 lane、
//! 当前聚焦哪条」无关，只跟「这一轮怎么收尾」有关。
//!
//! 两类取消必须一起做，少任一个都留下「屏幕上说停了、实际还在跑」的窗口：
//!
//! - 回合（[`super::worker::StreamCancel`]）：撤审批浮层 + 计数 1，worker 先
//!   `POST /chat/stream/{job}/cancel` 再收尾 SSE；
//! - 非回合 job（[`super::worker::JobCancel`]）：`select!` 直接中断，不必等
//!   `REQUEST_TIMEOUT`（30 s/请求）撞完。

use super::LaneApp;
use super::state::LineKind;

impl LaneApp<'_> {
    /// 让这条 lane 停下手上所有事：撤审批浮层 + 请求取消一次 + 中断在跑的 job。
    ///
    /// **这是唯一的收尾入口**（issue #346）：`/lane cancel`、`/lane detach`、Ctrl+C、
    /// `/quit` 都只调它。少任一步都会造出新 bug：
    ///
    /// - **撤掉浮层**：`drop(answer)` 让 gate 立刻走 `Disconnected → Deny`，不必干等
    ///   `APPROVAL_WAIT`（600 s）超时才收尾 —— 少这一步，线程与 runtime 照样白占一整个
    ///   审批窗口，而屏幕上早已写着「正在取消回合…」；
    /// - **`cancel()`**：计数 1 唤醒 `wait_for_cancel`，worker 先
    ///   `POST /chat/stream/{job}/cancel` 让 serve 停掉 job，再收尾 SSE；
    /// - **`job_cancel()`**（issue #348）：**无条件**施加。没有回合在跑时，这条 lane 也
    ///   可能正卡在一次 HTTP 拉取里（`RefreshSessions` 还是两次串行请求），detach / quit
    ///   不该为它白等。空闲时是空操作 —— `JobCancel::arm` 每次新建通道，不沿用上一代。
    ///
    /// 返回这一轮原本是否在跑、有没有撤掉浮层、有没有一个 job 在跑，由调用方决定措辞
    /// （`/lane cancel` 说「已请求取消」，`/lane detach` 说「断开时顺手取消」）——
    /// 动作同源，话不必一样。
    pub(super) fn wind_down(&mut self) -> WindDown {
        let job = self.job_cancel.is_armed();
        self.job_cancel.cancel();
        if !self.st.running {
            // 没有在跑的回合：`cancel` 与浮层此时不该在（`on_turn_done` 会清），
            // 真在也是上一轮收尾的残留，不动它。
            return WindDown {
                job,
                ..WindDown::default()
            };
        }
        let overlay = self.st.approval.take().is_some();
        if let Some(cancel) = self.cancel.take() {
            cancel.cancel();
        }
        WindDown {
            running: true,
            overlay,
            job,
        }
    }

    /// 「请求停下并在 transcript 里说明」：Ctrl+C 与 `/quit` 共用的那一步。
    ///
    /// 返回 `true` = 这次**真的**请求了取消；`false` = 本来就空闲、或已经在取消中。
    /// 两个调用点都据此决定「再按一次就走」还是「先等它收尾」，不必各写一遍
    /// `running` / `cancel_sent` 的组合判断。
    pub(super) fn request_stop(&mut self, notice: &str) -> bool {
        if !self.st.running || self.st.cancel_sent {
            return false;
        }
        let stopped = self.wind_down();
        self.st.cancel_sent = true;
        self.st.push_line(LineKind::System, notice);
        if stopped.overlay {
            self.st
                .push_line(LineKind::System, "审批浮层已撤销（按拒绝处理）");
        }
        true
    }
}

/// [`LaneApp::wind_down`] 的结果：只在**措辞**上分叉，调用点都不需要更多信息。
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct WindDown {
    /// 原本有在跑的回合（`false` = 回合那一半什么都没做）。
    pub(super) running: bool,
    /// 原本有审批浮层（`true` = `answer` 已被 drop，gate 按拒绝收尾）。
    pub(super) overlay: bool,
    /// 原本有一个**非回合** job 在跑（`true` = 它的取消令牌已被施加）。
    ///
    /// 与 `running` 正交：回合在跑时也可能排着拉取，但那时只需要说回合那一句，
    /// 拉取会被随后的 `job_tx` 关闭一起收抬。
    pub(super) job: bool,
}
