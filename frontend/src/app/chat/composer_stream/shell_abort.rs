//! [`super::handles::ComposerStreamShell`] 上 `AbortController` 与用户取消标志的 Mutex 集中读写，
//! 避免 `attach` / 回调 / 停止按钮各处重复 `lock().unwrap()`。
//! 槽位定义见 **[`crate::app::app_signals::StreamControlSignals`]**（**`abort_cell`** / **`user_cancelled_stream`**）；此处仅封装业务语义。
//! `spawn_local` 尾逻辑请用 [`user_cancelled_flag`]，勿在闭包外再 `Arc::clone`  Mutex 手动锁。

use leptos::prelude::Update;

use super::super::handles::ComposerStreamShell;

fn bump_stream_abort_epoch(shell: &ComposerStreamShell) {
    shell.stream.stream_abort_epoch.update(|n| {
        *n = n.wrapping_add(1);
    });
}

/// 发起新流前：中止上一控制器、清除「用户取消」标记（随后应 [`store_abort_controller`]）。
pub(crate) fn reset_abort_state_for_new_attach(shell: &ComposerStreamShell) {
    let prev = shell.stream.abort_cell.lock().unwrap().take();
    if prev.is_some() {
        bump_stream_abort_epoch(shell);
    }
    if let Some(ac) = prev {
        ac.abort();
    }
    *shell.stream.user_cancelled_stream.lock().unwrap() = false;
}

/// 只置用户取消标志并 bump epoch（**不** abort）。尚无 `job_id` 时用来等 `x-stream-job-id`。
pub(crate) fn mark_user_cancelled(shell: &ComposerStreamShell) {
    *shell.stream.user_cancelled_stream.lock().unwrap() = true;
    bump_stream_abort_epoch(shell);
}

pub(super) fn store_abort_controller(shell: &ComposerStreamShell, ac: web_sys::AbortController) {
    *shell.stream.abort_cell.lock().unwrap() = Some(ac);
    bump_stream_abort_epoch(shell);
}

pub(crate) fn clear_abort_slot(shell: &ComposerStreamShell) {
    let taken = shell.stream.abort_cell.lock().unwrap().take();
    if taken.is_some() {
        bump_stream_abort_epoch(shell);
    }
}

/// 取出并 `abort` 在途控制器（幂等）。
pub(crate) fn abort_in_flight_stream(shell: &ComposerStreamShell) {
    if let Some(ac) = shell.stream.abort_cell.lock().unwrap().take() {
        ac.abort();
        bump_stream_abort_epoch(shell);
    }
}

pub(crate) fn user_cancelled_flag(shell: &ComposerStreamShell) -> bool {
    *shell.stream.user_cancelled_stream.lock().unwrap()
}

pub(crate) fn spawn_post_chat_stream_cancel(job_id: u64, loc: crate::i18n::Locale) {
    leptos::task::spawn_local(async move {
        let _ = crate::api::post_chat_stream_cancel(job_id, loc).await;
    });
}
