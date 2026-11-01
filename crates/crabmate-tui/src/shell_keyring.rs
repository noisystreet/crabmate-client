//! 桌面壳钥匙串里的凭据槽：TUI 侧的读取口径。
//!
//! 桌面壳（`desktop-tauri` + `crabmate-connect`）把 Web Bearer 存在
//! `<account>:<窗口 label>` 上（多窗口后按窗口分槽），迁移时还会把早先那个不带后缀的
//! 槽搬进分槽槽再**清空**它。TUI 若只认裸槽，桌面跑过一次之后就永远读到 `None` ——
//! 而且失败是静默的：`web_api_credential_pair("")` 返回 `None`，请求不带任何认证头，
//! serve 回 401。症状指向服务端配置，真因却在客户端槽名。
//!
//! 所以这里按「分槽主窗口槽 → 裸槽」顺序探测，并把命中的槽名写进 stderr；全部未命中
//! 也出声（否则下次排查还是从 401 开始猜）。

use crabmate_client_api::secrets::{
    DESKTOP_MAIN_WINDOW_LABEL, KEYRING_SERVICE, WEB_API_BEARER_KEYRING_ACCOUNT,
    scoped_bearer_account,
};

/// `--bearer` / `CM_WEB_API_BEARER_TOKEN` 缺省（未提供）时回退读桌面壳钥匙串的 Web Bearer 槽。
pub(crate) fn resolve_bearer(explicit: Option<&str>, no_keyring: bool) -> Option<String> {
    with_shell_keyring_fallback(explicit, no_keyring, read_shell_bearer_slot)
}

/// 显式值优先：`Some(非空)` 用显式值；`Some(空)` 表示"明确不带、不回退"；
/// 仅 `None`（未提供）且未禁用回退时才交给 `read`（钥匙串）补位。
pub(crate) fn with_shell_keyring_fallback(
    cli_value: Option<&str>,
    no_keyring: bool,
    read: impl FnOnce() -> Option<String>,
) -> Option<String> {
    match cli_value {
        Some(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        None if no_keyring => None,
        None => read(),
    }
}

/// 读系统钥匙串条目（service 与桌面壳一致：`com.crabmate.credentials`）。
/// 无条目 / 钥匙串不可用 / 空值返回 `None`（调用方静默回退）。
pub(crate) fn keyring_secret(account: &str) -> Option<String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, account).ok()?;
    let raw = entry.get_password().ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Bearer 槽的探测顺序：分槽后的主窗口槽，再是分槽前的裸槽。
///
/// 顺序不能反：桌面的迁移会把裸槽搬空，它只对「从未用新版桌面连接过」的安装有效；
/// 反过来只读裸槽就是 401 的成因。
#[must_use]
fn bearer_account_candidates() -> Vec<String> {
    vec![
        scoped_bearer_account(DESKTOP_MAIN_WINDOW_LABEL),
        WEB_API_BEARER_KEYRING_ACCOUNT.to_string(),
    ]
}

/// 命中槽时的一行 stderr。
///
/// 措辞刻意**不**写成「已回退…」这种完成态：`:main` 是桌面主窗口最近一次连接的那台
/// serve 的凭据，与本次 `--api-base` 不保证同一台。写成成功语气会让「读到了但仍 401」
/// 看起来已经修好 —— 而误导排查方向正是本轮要消灭的东西。
#[must_use]
fn hit_note(account: &str) -> String {
    format!(
        "[crabmate-tui] Web Bearer 未指定，已从桌面壳钥匙串槽 {account} 取到一个；若本次 serve 与桌面主窗口正在连的不是同一台，请改用 --bearer"
    )
}

/// 未命中槽时的一行 stderr。
///
/// 中性陈述、不带无条件祈使：serve 不需要 Bearer（同 Origin 本地开发）是常态，那里
/// 「请用 `--bearer`」是错误建议。这条仍然要打印 —— 它正是排查 401 时最缺的信息。
#[must_use]
fn miss_note(accounts: &[String]) -> String {
    format!(
        "[crabmate-tui] 未取到 Web Bearer：桌面壳钥匙串里没有可用槽（试过 {}）；serve 启用 Bearer 鉴权时请用 --bearer / CM_WEB_API_BEARER_TOKEN",
        accounts.join(", ")
    )
}

/// 回退读桌面壳钥匙串里的 Web Bearer，并说明结果取自哪个槽。
fn read_shell_bearer_slot() -> Option<String> {
    let accounts = bearer_account_candidates();
    match first_keyring_hit(&accounts, keyring_secret) {
        Some((account, secret)) => {
            eprintln!("{}", hit_note(&account));
            Some(secret)
        }
        None => {
            eprintln!("{}", miss_note(&accounts));
            None
        }
    }
}

/// 按顺序读候选槽，返回首个命中的 `(账户名, 值)`。
fn first_keyring_hit(
    accounts: &[String],
    read: impl Fn(&str) -> Option<String>,
) -> Option<(String, String)> {
    accounts
        .iter()
        .find_map(|account| read(account).map(|secret| (account.clone(), secret)))
}

#[cfg(test)]
#[path = "shell_keyring_tests.rs"]
mod tests;
