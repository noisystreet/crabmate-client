//! [`super::handles::ComposerStreamShell`] 上 `AbortController` 与用户取消标志的 Mutex 集中读写，
//! 避免 `attach` / 回调 / 停止按钮各处重复加锁样板。
//! 槽位定义见 **[`crate::app::app_signals::StreamControlSignals`]**（**`abort_cell`** / **`user_cancelled_stream`**）；此处仅封装业务语义。
//! `spawn_local` 尾逻辑请用 [`user_cancelled_flag`]，勿在闭包外再 `Arc::clone`  Mutex 手动锁。

use std::sync::{Mutex, MutexGuard};

use leptos::prelude::Update;

use super::super::handles::ComposerStreamShell;

/// 取流控制槽位的锁。
///
/// WASM 单线程下中毒只可能来自持锁者 panic（此时进程已不可信），故不处理中毒；
/// 用 `expect` 把这条不变量写在调用点上，省得每处 `lock()` 都让人猜「为什么不处理」。
fn lock<'a, T>(cell: &'a Mutex<T>) -> MutexGuard<'a, T> {
    cell.lock().expect("流控制 Mutex 未中毒")
}

fn bump_stream_abort_epoch(shell: &ComposerStreamShell) {
    shell.stream.stream_abort_epoch.update(|n| {
        *n = n.wrapping_add(1);
    });
}

/// 发起新流前：中止上一控制器、清除「用户取消」标记（随后应 [`store_abort_controller`]）。
pub(crate) fn reset_abort_state_for_new_attach(shell: &ComposerStreamShell) {
    let prev = lock(&shell.stream.abort_cell).take();
    if prev.is_some() {
        bump_stream_abort_epoch(shell);
    }
    if let Some(ac) = prev {
        ac.abort();
    }
    *lock(&shell.stream.user_cancelled_stream) = false;
}

/// 只置用户取消标志并 bump epoch（**不** abort）。尚无 `job_id` 时用来等 `x-stream-job-id`。
pub(crate) fn mark_user_cancelled(shell: &ComposerStreamShell) {
    *lock(&shell.stream.user_cancelled_stream) = true;
    bump_stream_abort_epoch(shell);
}

pub(super) fn store_abort_controller(shell: &ComposerStreamShell, ac: web_sys::AbortController) {
    *lock(&shell.stream.abort_cell) = Some(ac);
    bump_stream_abort_epoch(shell);
}

pub(crate) fn clear_abort_slot(shell: &ComposerStreamShell) {
    let taken = lock(&shell.stream.abort_cell).take();
    if taken.is_some() {
        bump_stream_abort_epoch(shell);
    }
}

/// 取出并 `abort` 在途控制器（幂等）。
pub(crate) fn abort_in_flight_stream(shell: &ComposerStreamShell) {
    if let Some(ac) = lock(&shell.stream.abort_cell).take() {
        ac.abort();
        bump_stream_abort_epoch(shell);
    }
}

pub(crate) fn user_cancelled_flag(shell: &ComposerStreamShell) -> bool {
    *lock(&shell.stream.user_cancelled_stream)
}

pub(crate) fn spawn_post_chat_stream_cancel(job_id: u64, loc: crate::i18n::Locale) {
    leptos::task::spawn_local(async move {
        let _ = crate::api::post_chat_stream_cancel(job_id, loc).await;
    });
}
