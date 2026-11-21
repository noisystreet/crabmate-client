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

use crabmate_client_api::secrets::{WEB_API_BEARER_KEYRING_ACCOUNT, scoped_bearer_account};

use crate::secret_backend::{read_trimmed, write_trimmed};

/// 读某个窗口的 Bearer；无条目或空串返回 `None`。
///
/// **迁移**：旧版只存一个不带 label 的槽。新槽为空时回退读旧槽，命中就搬到新槽并删掉
/// 旧槽 —— 否则升级后每个窗口都「丢了」凭据、得重连一次；而留着旧槽会把同一份凭据
/// 分发给每一个窗口，那正是这次要修的串号。
pub fn read_connect_bearer_for(label: &str) -> Result<Option<String>, String> {
    let scoped = scoped_bearer_account(label);
    if let Some(found) = read_trimmed(&scoped)? {
        return Ok(Some(found));
    }
    let Some(legacy) = read_trimmed(WEB_API_BEARER_KEYRING_ACCOUNT)? else {
        return Ok(None);
    };
    write_trimmed(&scoped, &legacy)?;
    if let Err(e) = write_trimmed(WEB_API_BEARER_KEYRING_ACCOUNT, "") {
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
    write_trimmed(&scoped_bearer_account(label), bearer)
}

/// 只清除某个窗口的槽。
///
/// 断开的窗口只该带走自己的凭据 —— 别的窗口还连着它们各自的 `serve`。
pub fn clear_connect_bearer_for(label: &str) -> Result<(), String> {
    write_trimmed(&scoped_bearer_account(label), "")
}

#[cfg(test)]
mod tests {
    use crabmate_client_api::secrets::{WEB_API_BEARER_KEYRING_ACCOUNT, scoped_bearer_account};

    use super::{
        clear_connect_bearer_for, read_connect_bearer_for, write_connect_bearer_for,
        write_connect_bearer_on_connect,
    };
    use crate::secret_backend::{read_trimmed, testing, write_trimmed};

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

    #[test]
    fn bearer_round_trips_through_the_scoped_slot() {
        let _backend = testing::install();
        write_connect_bearer_for("main", "  bearer-abc\n").unwrap();

        assert_eq!(
            read_connect_bearer_for("main").unwrap().as_deref(),
            Some("bearer-abc")
        );
    }

    #[test]
    fn one_window_bearer_does_not_leak_into_another() {
        // #391 的回归：后连窗口覆盖先连窗口时运行时看不出来（凭据已在各窗口内存里），
        // 症状要到下一次刷新 / 重开窗口才以 401 出现。
        let _backend = testing::install();
        write_connect_bearer_for("main", "bearer-main").unwrap();
        write_connect_bearer_for("prod", "bearer-prod").unwrap();

        assert_eq!(
            read_connect_bearer_for("main").unwrap().as_deref(),
            Some("bearer-main")
        );
        assert_eq!(
            read_connect_bearer_for("prod").unwrap().as_deref(),
            Some("bearer-prod")
        );
    }

    #[test]
    fn clearing_one_window_leaves_the_others_connected() {
        let _backend = testing::install();
        write_connect_bearer_for("main", "bearer-main").unwrap();
        write_connect_bearer_for("prod", "bearer-prod").unwrap();

        clear_connect_bearer_for("prod").unwrap();

        assert_eq!(read_connect_bearer_for("prod").unwrap(), None);
        assert_eq!(
            read_connect_bearer_for("main").unwrap().as_deref(),
            Some("bearer-main")
        );
    }

    #[test]
    fn legacy_slot_migrates_into_the_scoped_slot_and_is_cleared() {
        // 升级路径：旧版只写无后缀槽。读到它要搬进本窗口的槽并删掉旧槽 —— 否则
        // 要么每个窗口都「丢了」凭据、得重连，要么把同一份凭据分发给所有窗口。
        let _backend = testing::install();
        write_trimmed(WEB_API_BEARER_KEYRING_ACCOUNT, "legacy-bearer").unwrap();

        let migrated = read_connect_bearer_for("main").unwrap();
        assert_eq!(migrated.as_deref(), Some("legacy-bearer"));

        assert_eq!(read_trimmed(WEB_API_BEARER_KEYRING_ACCOUNT).unwrap(), None);
        assert_eq!(
            read_trimmed(&scoped_bearer_account("main"))
                .unwrap()
                .as_deref(),
            Some("legacy-bearer")
        );
    }

    #[test]
    fn connecting_without_a_bearer_writes_nothing() {
        let _backend = testing::install();

        assert!(!write_connect_bearer_on_connect("main", "   ").unwrap());
        assert_eq!(read_connect_bearer_for("main").unwrap(), None);
    }
}
