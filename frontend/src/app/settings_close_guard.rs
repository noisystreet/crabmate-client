//! 设置界面「关闭前确认未保存更改」。
//!
//! 设置页 / 设置弹窗在挂载时各注册一个关闭处理器；全局 Escape 与组件内关闭按钮
//! 统一走注册处理器，避免「Escape 不丢弃草稿」与「按钮静默丢弃」两条分叉语义。
//! 未注册时（启动早期等）回退为直接关闭（与历史行为一致）。

use std::cell::RefCell;
use std::sync::Arc;

use crate::i18n::{self, Locale};

thread_local! {
    static SETTINGS_PAGE_CLOSE: RefCell<Option<Arc<dyn Fn() + Send + Sync>>> = const { RefCell::new(None) };
    static SETTINGS_MODAL_CLOSE: RefCell<Option<Arc<dyn Fn() + Send + Sync>>> = const { RefCell::new(None) };
}

/// 注册设置页关闭处理器（设置页挂载时调用，可重复覆盖）。
pub fn register_settings_page_close_handler(f: Arc<dyn Fn() + Send + Sync>) {
    SETTINGS_PAGE_CLOSE.with(|c| *c.borrow_mut() = Some(f));
}

/// 注册设置弹窗关闭处理器（弹窗挂载时调用，可重复覆盖）。
pub fn register_settings_modal_close_handler(f: Arc<dyn Fn() + Send + Sync>) {
    SETTINGS_MODAL_CLOSE.with(|c| *c.borrow_mut() = Some(f));
}

/// 请求关闭设置页。已注册处理器时由其接管（返回 `true`）；否则返回 `false`，调用方直接关闭。
pub fn request_settings_page_close() -> bool {
    SETTINGS_PAGE_CLOSE.with(|c| {
        if let Some(f) = c.borrow().clone() {
            f();
            true
        } else {
            false
        }
    })
}

/// 请求关闭设置弹窗（语义同 [`request_settings_page_close`]）。
pub fn request_settings_modal_close() -> bool {
    SETTINGS_MODAL_CLOSE.with(|c| {
        if let Some(f) = c.borrow().clone() {
            f();
            true
        } else {
            false
        }
    })
}

/// 关闭前确认「放弃未保存更改」（仅在脏表单时由注册处理器触发）。
pub async fn confirm_discard_unsaved(locale: Locale) -> bool {
    crate::confirm_dialog::confirm_in_page(
        i18n::settings_discard_unsaved_confirm(locale),
        i18n::settings_discard_changes(locale),
        i18n::ide_confirm_cancel(locale),
    )
    .await
}
