//! 连接用 Web API Bearer：本机系统钥匙串（非 TOML / 非明文 localStorage）。
//!
//! 账户名与服务端 `web_api_bearer` 槽位区分，避免与 serve 主机钥匙串语义混淆。
//! Android 等无钥匙串后端的平台上读写失败时静默降级；移动连接页改用
//! `CrabMateMobile` + `SecureBearerStore`（AndroidKeyStore AES-GCM）。
//!
//! # 为什么按窗口分槽
//!
//! 桌面可以同时开多个窗口，每个窗口连一台 `serve`（#391）。Bearer 若只存一个槽，
//! 后连的窗口会覆盖先连的：运行时看不出来（凭据已在各窗口的 JS 内存里），但**任何一次
//! 页面刷新 / 窗口重开**都会拿着别人的凭据去连 —— 认证失败，而且错得莫名其妙。
//!
//! 这与 `AllowedServeOrigin` 是同一个问题、同一个修法：那个白名单早就是 `label → origin`
//! 的映射，注释里写着「进程级单值会让先连的那个窗口丢掉放行依据」。Bearer 只是当时
//! 漏掉的那一半。
//!
//! 账户名的拼接在共享层（[`crabmate_client_api::secrets::scoped_bearer_account`]），
//! TUI 的钥匙串回退读的是同一份——两处各写一遍就是下一个「读得到但认证不上」。

use crabmate_client_api::secrets::{
    KEYRING_SERVICE, WEB_API_BEARER_KEYRING_ACCOUNT, scoped_bearer_account,
};

fn entry_for(account: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, account).map_err(|e| format!("系统钥匙串不可用: {e}"))
}

fn read_account(account: &str) -> Result<Option<String>, String> {
    match entry_for(account)?.get_password() {
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

fn write_account(account: &str, bearer: &str) -> Result<(), String> {
    let entry = entry_for(account)?;
    let trimmed = bearer.trim();
    if trimmed.is_empty() {
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(format!("清除系统钥匙串失败: {e}")),
        }
    } else {
        entry
            .set_password(trimmed)
            .map_err(|e| format!("写入系统钥匙串失败: {e}"))
    }
}

/// 读某个窗口的 Bearer；无条目或空串返回 `None`。
///
/// **迁移**：旧版只存一个不带 label 的槽。新槽为空时回退读旧槽，命中就搬到新槽并删掉
/// 旧槽 —— 否则升级后每个窗口都「丢了」凭据、得重连一次；而留着旧槽会把同一份凭据
/// 分发给每一个窗口，那正是这次要修的串号。
pub fn read_connect_bearer_for(label: &str) -> Result<Option<String>, String> {
    let scoped = scoped_bearer_account(label);
    if let Some(found) = read_account(&scoped)? {
        return Ok(Some(found));
    }
    let Some(legacy) = read_account(WEB_API_BEARER_KEYRING_ACCOUNT)? else {
        return Ok(None);
    };
    write_account(&scoped, &legacy)?;
    if let Err(e) = write_account(WEB_API_BEARER_KEYRING_ACCOUNT, "") {
        // 删不掉只是脏数据，不影响正确性：读路径优先新槽。
        eprintln!("[crabmate-connect] legacy bearer slot cleanup skipped: {e}");
    }
    Ok(Some(legacy))
}

/// **连接成功**：非空 `bearer` 写入该窗口的槽（覆盖）。
///
/// 返回 `Ok(true)` 表示本次写入；`Ok(false)` 表示跳过（空串）。
pub fn write_connect_bearer_on_connect(label: &str, bearer: &str) -> Result<bool, String> {
    if bearer.trim().is_empty() {
        return Ok(false);
    }
    write_connect_bearer_for(label, bearer)?;
    Ok(true)
}

/// 无条件写入该窗口的槽（测试 / 显式更新）。空串删除条目。
pub fn write_connect_bearer_for(label: &str, bearer: &str) -> Result<(), String> {
    write_account(&scoped_bearer_account(label), bearer)
}

/// 只清除某个窗口的槽。
///
/// 断开的窗口只该带走自己的凭据 —— 别的窗口还连着它们各自的 `serve`。
pub fn clear_connect_bearer_for(label: &str) -> Result<(), String> {
    write_account(&scoped_bearer_account(label), "")
}

#[cfg(test)]
mod tests {
    use crabmate_client_api::secrets::{WEB_API_BEARER_KEYRING_ACCOUNT, scoped_bearer_account};

    #[test]
    fn each_window_gets_its_own_account_name() {
        // 两个窗口必须落在两个账户名上，否则又回到「后写的覆盖先写的」。
        assert_ne!(scoped_bearer_account("main"), scoped_bearer_account("prod"));
        assert_eq!(
            scoped_bearer_account("prod"),
            format!("{WEB_API_BEARER_KEYRING_ACCOUNT}:prod")
        );
    }

    #[test]
    fn the_legacy_account_name_is_not_reused_for_scoped_writes() {
        // 迁移读的是这个无后缀的账户名；若分槽后仍写它，就又变成共享槽了。
        assert_ne!(
            scoped_bearer_account("main"),
            WEB_API_BEARER_KEYRING_ACCOUNT
        );
        assert_ne!(
            scoped_bearer_account(WEB_API_BEARER_KEYRING_ACCOUNT),
            WEB_API_BEARER_KEYRING_ACCOUNT
        );
    }
}
