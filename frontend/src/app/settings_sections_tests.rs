//! `settings_sections` 的单测（经 `#[path]` 挂载为 `settings_sections::tests`）。
//!
//! 独立成文件是因为 `settings_sections.rs` 已接近 920 行的单文件上限。

use super::bearer_clear_needs_confirm;

/// 空输入 = 清除；只有「本来就有 token」时才需要确认，避免无后果的二次确认。
#[test]
fn bearer_clear_needs_confirm_only_when_token_present() {
    assert!(bearer_clear_needs_confirm("", true));
    assert!(bearer_clear_needs_confirm("   ", true));
    assert!(bearer_clear_needs_confirm("\n\t", true));
}

/// 输入框还有内容时是在保存而非清除，不得弹清除确认。
#[test]
fn bearer_save_with_token_skips_confirm() {
    assert!(!bearer_clear_needs_confirm("secret", true));
    assert!(!bearer_clear_needs_confirm("  secret  ", true));
}

/// 本来就没配过 token：空输入保存没有任何后果，不弹确认。
#[test]
fn bearer_clear_without_token_skips_confirm() {
    assert!(!bearer_clear_needs_confirm("", false));
}
