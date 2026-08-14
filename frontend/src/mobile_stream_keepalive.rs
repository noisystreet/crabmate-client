//! Android 流式前台保活 / 审批通知（ADR-0002）的 WASM 侧编排。
//! 无 `CrabMateMobile` 桥时全部 no-op（桌面 / 浏览器）。

use leptos::prelude::*;

use crate::i18n::{self, Locale};
use crate::mobile_remote;

pub const APPROVAL_PREVIEW_MAX_CHARS: usize = 80;

/// 与原生通知栏一致的命令预览截断（单测对齐 Kotlin `StreamKeepAliveText`）。
#[must_use]
pub fn truncate_approval_preview(command: &str, args: &str) -> String {
    let joined = format!("{} {}", command.trim(), args.trim());
    let collapsed: String = joined.split_whitespace().collect::<Vec<_>>().join(" ");
    let count = collapsed.chars().count();
    if count <= APPROVAL_PREVIEW_MAX_CHARS {
        return collapsed;
    }
    let take: String = collapsed.chars().take(APPROVAL_PREVIEW_MAX_CHARS).collect();
    format!("{take}…")
}

/// 仅在原生已确认拒绝通知权限时写状态栏（`prompting` / `ok` 不报错）。
#[must_use]
pub fn keepalive_start_sets_notify_warning(result: &str) -> bool {
    result == "need_permission"
}

/// 权限对话框结果：拒绝则提示；授权则清掉本条提示（不覆盖其它错误）。
pub fn apply_keepalive_permission_result(
    granted: bool,
    locale: Locale,
    status_err: RwSignal<Option<String>>,
) {
    let msg = i18n::android_stream_keepalive_notify_needed(locale);
    if granted {
        status_err.update(|e| {
            if e.as_deref() == Some(msg) {
                *e = None;
            }
        });
        return;
    }
    if status_err.get_untracked().is_some() {
        return;
    }
    status_err.set(Some(msg.to_string()));
}

/// `/chat/stream` attach 开始（含软续传）：拉起 FGS；仅已拒绝通知权限时写入状态栏。
pub fn on_stream_attach_started(locale: RwSignal<Locale>, status_err: RwSignal<Option<String>>) {
    wire_permission_banner(locale, status_err);
    let result =
        mobile_remote::mobile_start_stream_keep_alive(locale.get_untracked().storage_slug());
    if !keepalive_start_sets_notify_warning(&result) {
        return;
    }
    apply_keepalive_permission_result(false, locale.get_untracked(), status_err);
}

pub fn on_stream_attach_finished() {
    mobile_remote::mobile_stop_stream_keep_alive();
}

pub fn on_command_approval(command: &str, args: &str, locale: Locale) {
    let preview = truncate_approval_preview(command, args);
    mobile_remote::mobile_notify_stream_approval(&preview, "", locale.storage_slug());
}

pub fn on_approval_resolved() {
    mobile_remote::mobile_clear_stream_approval_notification();
}

fn wire_permission_banner(locale: RwSignal<Locale>, status_err: RwSignal<Option<String>>) {
    crate::mobile_remote::install_keepalive_permission_handler(move |granted| {
        apply_keepalive_permission_result(granted, locale.get_untracked(), status_err);
    });
}

#[cfg(test)]
mod tests {
    use super::{
        APPROVAL_PREVIEW_MAX_CHARS, keepalive_start_sets_notify_warning, truncate_approval_preview,
    };

    #[test]
    fn preview_collapses_whitespace() {
        assert_eq!(
            truncate_approval_preview(" rm ", "  -rf   /tmp "),
            "rm -rf /tmp"
        );
    }

    #[test]
    fn preview_truncates_char_count() {
        let cmd = "a".repeat(50);
        let args = "b".repeat(50);
        let out = truncate_approval_preview(&cmd, &args);
        assert_eq!(out.chars().count(), APPROVAL_PREVIEW_MAX_CHARS + 1);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn start_result_warns_only_when_denied() {
        assert!(keepalive_start_sets_notify_warning("need_permission"));
        assert!(!keepalive_start_sets_notify_warning("prompting"));
        assert!(!keepalive_start_sets_notify_warning("ok"));
        assert!(!keepalive_start_sets_notify_warning(""));
    }
}
