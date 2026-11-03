//! 非回合 job 的取消封装（从 `worker.rs` 拆出：那是本仓最长的文件之一，新增内容
//! 会直接顶破 `fn-nloc-ratchet` 的 920 行上限）。
//!
//! 这里只有 `select!` 与「被打断后补什么事件」两件事，`JobCancel` / `UiEvent` /
//! `LaneTx` 仍归 `worker.rs`，本模块只借用。

use std::future::Future;

use super::{JobCancel, LaneTx, UiEvent};

/// 在 job 外面套一层取消：`select!` 上 [`JobCancel`]，被打断时发一条 system 行。
///
/// 这是**唯一**给非回合 job 用的 `block_on` 出口 —— 16 个 job 一个都不用自己写
/// `select!`，也就不会漏掉某个（漏一个就是 #348 那条「白等一个超时窗口」）。
///
/// 中断时那行是 best-effort：lane 已 detach 时它会被 `on_msg` 丢弃（按 `LaneId` 找不到
/// 就丢），这正是想要的 —— 不再往已摘掉的 lane 里写东西。
pub(super) fn run_cancellable<F>(
    rt: &tokio::runtime::Runtime,
    cancel: &JobCancel,
    label: &str,
    job: F,
    out: &LaneTx,
) where
    F: Future<Output = ()>,
{
    run_cancellable_with(rt, cancel, label, job, out, None);
}

/// 保存类 job 的取消：除了那行提示，还补一条 [`UiEvent::SaveInterrupted`]。
///
/// 保存 job 的「结果」不止是给人看的文本 —— 设置面板还挂着一个「保存中」标记。取消
/// 意味着 `Saved*` 回包再也不会来，所以必须有人告诉面板可以放手了（否则它会卡在
/// `is_saving` 上，连 Esc 都关不掉）；其余 12 个 job 没有这种状态耦合。
pub(super) fn run_save_job<F>(
    rt: &tokio::runtime::Runtime,
    cancel: &JobCancel,
    label: &str,
    job: F,
    out: &LaneTx,
) where
    F: Future<Output = ()>,
{
    run_cancellable_with(rt, cancel, label, job, out, Some(UiEvent::SaveInterrupted));
}

fn run_cancellable_with<F>(
    rt: &tokio::runtime::Runtime,
    cancel: &JobCancel,
    label: &str,
    job: F,
    out: &LaneTx,
    on_cancel: Option<UiEvent>,
) where
    F: Future<Output = ()>,
{
    let rx = cancel.arm();
    let interrupted = rt.block_on(async {
        tokio::select! {
            () = job => false,
            _ = rx => true,
        }
    });
    cancel.disarm();
    if interrupted {
        let _ = out.send(UiEvent::System(format!("已中断：{label}")));
        if let Some(ev) = on_cancel {
            let _ = out.send(ev);
        }
    }
}
