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

use crate::serve::BearerSource;

/// Bearer 的来源判定（设置面板「连接」栏展示用；只报来源，不报值）。
///
/// 两个入参就够：显式值（`--bearer` 与 `CM_WEB_API_BEARER_TOKEN` 在 clap 层已合并为
/// `Cli.bearer`）和 [`resolve_bearer`] 的最终结果。显式非空 → 启动参数；否则有值
/// → 钥匙串回退命中；都没有 → 未提供。**显式给空值**也是「未提供」而不是「钥匙串」：
/// 那是「明确不带、也不回退」的写法，与 `--no-keyring` 同一个语义。
///
/// 纯函数：值本身逆推不出来（空值可能是显式空、也可能是读不到钥匙串），所以来源必须
/// 在解析的同一处定下来，不能让面板再猜一遍。
pub(crate) fn bearer_source(explicit: Option<&str>, resolved: Option<&str>) -> BearerSource {
    let explicit_set = explicit.map(str::trim).is_some_and(|v| !v.is_empty());
    let resolved_set = resolved.map(str::trim).is_some_and(|v| !v.is_empty());
    match (explicit_set, resolved_set) {
        (true, _) => BearerSource::Cli,
        (false, true) => BearerSource::Keyring,
        (false, false) => BearerSource::Missing,
    }
}

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
///
/// 三态而不是两态：`Ok(None)` 是「没有这个槽」，`Err` 是「钥匙串读不了」（后端不可用 /
/// 被锁 / 无 D-Bus）。混成一个 `Option` 会把后者报成前者，而两者对 401 的下一步动作
/// 相反 —— 一个该去桌面连一次，一个该去修钥匙串。口径与 `crabmate-connect` 的
/// `read_account` 一致。
fn read_account(account: &str) -> Result<Option<String>, String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, account)
        .map_err(|e| format!("系统钥匙串不可用: {e}"))?;
    match entry.get_password() {
        Ok(secret) => {
            let trimmed = secret.trim();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                Ok(Some(trimmed.to_string()))
            }
        }
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("读取系统钥匙串失败: {e}")),
    }
}

/// 宽松版：读不到就是 `None`。
///
/// 只给模型密钥那条路径用 —— 它只关心「有没有值」，且未命中时本来就不出声（见
/// `resolve_llm_key`）。Bearer 路径必须用 [`read_account`]：那里要把「不可用」与
/// 「没存过」分开报。
pub(crate) fn keyring_secret(account: &str) -> Option<String> {
    read_account(account).ok().flatten()
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

/// 钥匙串读不了时的一行 stderr。
///
/// 与 [`miss_note`] 分开：那条说的是「没存过」，这条说的是「读不了」。两者都表现为
/// 「未取到 Web Bearer」，但下一步动作相反 —— 混成一句就把排查又推回起点。
#[must_use]
fn unavailable_note(accounts: &[String], error: &str) -> String {
    format!(
        "[crabmate-tui] 读不到桌面壳钥匙串（{error}）；试过 {}；若 serve 启用 Bearer 鉴权，请改用 --bearer / CM_WEB_API_BEARER_TOKEN，或修好系统钥匙串后重试",
        accounts.join(", ")
    )
}

/// 回退读桌面壳钥匙串里的 Web Bearer，并说明结果取自哪个槽 / 为何没取到。
fn read_shell_bearer_slot() -> Option<String> {
    let accounts = bearer_account_candidates();
    match first_keyring_hit(&accounts, read_account) {
        Ok(Some((account, secret))) => {
            eprintln!("{}", hit_note(&account));
            Some(secret)
        }
        Ok(None) => {
            eprintln!("{}", miss_note(&accounts));
            None
        }
        Err(error) => {
            eprintln!("{}", unavailable_note(&accounts, &error));
            None
        }
    }
}

/// 按顺序读候选槽，返回首个命中的 `(账户名, 值)`。
///
/// 任一槽读取失败即 `Err`，不再往下探：钥匙串后端不可用是全局故障，下一个槽不会更好；
/// 而把同一次故障说成「两个槽都没有」，正是这条链路要消灭的归因错误。
fn first_keyring_hit(
    accounts: &[String],
    read: impl Fn(&str) -> Result<Option<String>, String>,
) -> Result<Option<(String, String)>, String> {
    for account in accounts {
        if let Some(secret) = read(account)? {
            return Ok(Some((account.clone(), secret)));
        }
    }
    Ok(None)
}

#[cfg(test)]
#[path = "shell_keyring_tests.rs"]
mod tests;
