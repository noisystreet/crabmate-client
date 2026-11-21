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
    WEB_API_BEARER_KEYRING_ACCOUNT, scoped_bearer_account, scoped_bearer_account_with_fp,
};

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

/// 「一级槽当前对应哪一台」的锚：`<account>:<label>:fp`，值是指纹。
///
/// 没有它就分不清一级槽里的凭据属于哪台 `serve` —— 那正是「切回上一台却拿着新凭据
/// 去连」的成因。锚缺失（升级前写下的一级槽）时按「一级 = 最近一次连接」处理。
fn fp_anchor_account(label: &str) -> String {
    format!("{}:fp", scoped_bearer_account(label))
}

/// 「这个窗口写过的指纹清单」保留条数上限。
const FP_LIST_LIMIT: usize = 16;

/// 该窗口**写过的指纹清单**：`<account>:<label>:fps`，逗号分隔。
///
/// 断开时要把「这个窗口连过的每一台」的槽都清掉，而钥匙串没有「按前缀列举」的接口 ——
/// 所以自己记一张。超出 [`FP_LIST_LIMIT`] 丢最旧的：清单只服务清理，丢掉的旧槽最坏
/// 情况是留一条过期凭据，而下次连回那台时写入会覆盖它。
fn fp_list_account(label: &str) -> String {
    format!("{}:fps", scoped_bearer_account(label))
}

fn read_fp_list(label: &str) -> Result<Vec<String>, String> {
    let raw = read_trimmed(&fp_list_account(label))?.unwrap_or_default();
    Ok(raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect())
}

fn write_fp_list(label: &str, list: &[String]) -> Result<(), String> {
    write_trimmed(&fp_list_account(label), &list.join(","))
}

/// 记下一个写过的指纹（已存在则不重复）。
fn remember_fp(label: &str, fp: &str) -> Result<(), String> {
    let mut list = read_fp_list(label)?;
    if list.iter().any(|f| f == fp) {
        return Ok(());
    }
    list.push(fp.to_string());
    if list.len() > FP_LIST_LIMIT {
        list.remove(0);
    }
    write_fp_list(label, &list)
}

fn forget_fp(label: &str, fp: &str) -> Result<(), String> {
    let list: Vec<String> = read_fp_list(label)?
        .into_iter()
        .filter(|f| f != fp)
        .collect();
    write_fp_list(label, &list)
}

/// 读某个窗口**连某一台时**的 Bearer。
///
/// 回落顺序：连接级槽 → 一级槽（仅当锚指向同一个 `fp`，或锚缺失 = 升级前的数据）。
/// 一级槽属于**别的**连接时返回 `Ok(None)` 而不是借用它的值：借用会让这个窗口的首个
/// 请求「拿着 A 的凭据打 B」，症状是 401 —— 比「没有凭据」难判断得多。
pub fn read_connect_bearer_for_conn(label: &str, fp: &str) -> Result<Option<String>, String> {
    let fp = fp.trim();
    if fp.is_empty() {
        return read_connect_bearer_for(label);
    }
    let scoped = scoped_bearer_account_with_fp(label, fp);
    if let Some(found) = read_trimmed(&scoped)? {
        return Ok(Some(found));
    }
    let anchor = read_trimmed(&fp_anchor_account(label))?;
    if anchor.as_deref().is_some_and(|a| a != fp) {
        return Ok(None);
    }
    let Some(window_level) = read_connect_bearer_for(label)? else {
        return Ok(None);
    };
    // 迁移：把一级值复制到连接级槽。一级保持不动 —— TUI 的回退还要读它。
    write_trimmed(&scoped, &window_level)?;
    write_trimmed(&fp_anchor_account(label), fp)?;
    Ok(Some(window_level))
}

/// **连接成功**：写连接级槽，并同时刷新一级槽与锚。
///
/// 一级槽是 TUI 钥匙串回退读的那一份（语义 =「桌面壳最后连的那台」），必须跟着最近
/// 一次连接走；连接级槽才是「同一窗口连过几台就记几份」。
pub fn write_connect_bearer_for_conn(label: &str, fp: &str, bearer: &str) -> Result<(), String> {
    write_connect_bearer_for(label, bearer)?;
    let fp = fp.trim();
    if fp.is_empty() {
        return Ok(());
    }
    write_trimmed(&scoped_bearer_account_with_fp(label, fp), bearer)?;
    write_trimmed(&fp_anchor_account(label), fp)?;
    remember_fp(label, fp)
}

/// 只清除某个窗口**这一台连接**的凭据。
///
/// 一级槽只在锚指向本连接时才清：断开 A 不该顺手抹掉 TUI 回退要读的 B。
pub fn clear_connect_bearer_for_conn(label: &str, fp: &str) -> Result<(), String> {
    let fp = fp.trim();
    if fp.is_empty() {
        return clear_connect_bearer_for(label);
    }
    write_trimmed(&scoped_bearer_account_with_fp(label, fp), "")?;
    forget_fp(label, fp)?;
    if read_trimmed(&fp_anchor_account(label))?.as_deref() == Some(fp) {
        write_trimmed(&scoped_bearer_account(label), "")?;
        write_trimmed(&fp_anchor_account(label), "")?;
    }
    Ok(())
}

/// 断开连接：清掉该窗口**所有**连接级槽、清单、一级槽与锚。
///
/// 用于「这个窗口不再连任何 `serve`」（连接页的断开按钮）；只断某一台时用
/// [`clear_connect_bearer_for_conn`]。清单记不全（截断）时用锚兜底，所以锚指向的那台
/// 无论是否在清单里都会被清。
pub fn clear_all_connect_bearers_for(label: &str) -> Result<(), String> {
    for fp in read_fp_list(label)? {
        write_trimmed(&scoped_bearer_account_with_fp(label, &fp), "")?;
    }
    write_trimmed(&fp_list_account(label), "")?;
    if let Some(fp) = read_trimmed(&fp_anchor_account(label))? {
        write_trimmed(&scoped_bearer_account_with_fp(label, &fp), "")?;
    }
    write_trimmed(&scoped_bearer_account(label), "")?;
    write_trimmed(&fp_anchor_account(label), "")
}

#[cfg(test)]
mod tests {
    use crabmate_client_api::secrets::{
        WEB_API_BEARER_KEYRING_ACCOUNT, scoped_bearer_account, scoped_bearer_account_with_fp,
    };

    use super::{
        clear_all_connect_bearers_for, clear_connect_bearer_for, clear_connect_bearer_for_conn,
        read_connect_bearer_for, read_connect_bearer_for_conn, write_connect_bearer_for,
        write_connect_bearer_for_conn, write_connect_bearer_on_connect,
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

    #[test]
    fn one_window_keeps_a_credential_per_serve_it_connected_to() {
        // #457：主窗口连 A 再连 B，切回 A 时凭据必须还是 A 的（否则 401）。
        let _backend = testing::install();
        write_connect_bearer_for_conn("main", "aaaa1111", "bearer-a").unwrap();
        write_connect_bearer_for_conn("main", "bbbb2222", "bearer-b").unwrap();

        assert_eq!(
            read_connect_bearer_for_conn("main", "aaaa1111")
                .unwrap()
                .as_deref(),
            Some("bearer-a"),
            "连接级槽不能被后一次连接覆盖"
        );
        assert_eq!(
            read_connect_bearer_for_conn("main", "bbbb2222")
                .unwrap()
                .as_deref(),
            Some("bearer-b")
        );
        // 一级槽跟着「最近一次连接」走，TUI 回退读的就是它。
        assert_eq!(
            read_connect_bearer_for("main").unwrap().as_deref(),
            Some("bearer-b")
        );
    }

    #[test]
    fn the_window_level_slot_is_not_borrowed_for_another_serve() {
        // 一级槽属于 B 时，A 不能借用它：拿 B 的凭据打 A 只会得到 401。
        let _backend = testing::install();
        write_connect_bearer_for_conn("main", "bbbb2222", "").unwrap();
        write_connect_bearer_for("main", "bearer-b").unwrap();

        assert_eq!(
            read_connect_bearer_for_conn("main", "aaaa1111").unwrap(),
            None
        );
    }

    #[test]
    fn a_window_level_only_credential_migrates_once() {
        // 升级路径：只有一级槽（锚缺失）时按「一级 = 最近一次连接」认领并按连接回填。
        let _backend = testing::install();
        write_connect_bearer_for("main", "bearer-legacy").unwrap();

        assert_eq!(
            read_connect_bearer_for_conn("main", "aaaa1111")
                .unwrap()
                .as_deref(),
            Some("bearer-legacy")
        );
        assert_eq!(
            read_trimmed(&scoped_bearer_account_with_fp("main", "aaaa1111"))
                .unwrap()
                .as_deref(),
            Some("bearer-legacy"),
            "必须回填，否则每次读都重跑一遍回落链"
        );
        // 锚已指向 A：另一台拿不到这份凭据。
        assert_eq!(
            read_connect_bearer_for_conn("main", "bbbb2222").unwrap(),
            None
        );
    }

    #[test]
    fn disconnecting_one_serve_leaves_the_other_and_the_window_level_slot_alone() {
        let _backend = testing::install();
        write_connect_bearer_for_conn("main", "aaaa1111", "bearer-a").unwrap();
        write_connect_bearer_for_conn("main", "bbbb2222", "bearer-b").unwrap();

        clear_connect_bearer_for_conn("main", "aaaa1111").unwrap();

        assert_eq!(
            read_connect_bearer_for_conn("main", "aaaa1111").unwrap(),
            None
        );
        assert_eq!(
            read_connect_bearer_for_conn("main", "bbbb2222")
                .unwrap()
                .as_deref(),
            Some("bearer-b")
        );
        assert_eq!(
            read_connect_bearer_for("main").unwrap().as_deref(),
            Some("bearer-b"),
            "一级槽当前指向 B，断开 A 不该清它"
        );
    }

    #[test]
    fn disconnecting_the_window_clears_every_slot_it_left_behind() {
        // 连接页的「断开」= 这个窗口不再连任何 serve，凭据不该留着。
        let _backend = testing::install();
        write_connect_bearer_for_conn("main", "aaaa1111", "bearer-a").unwrap();
        write_connect_bearer_for_conn("main", "bbbb2222", "bearer-b").unwrap();

        clear_all_connect_bearers_for("main").unwrap();

        assert_eq!(read_connect_bearer_for("main").unwrap(), None);
        assert_eq!(
            read_connect_bearer_for_conn("main", "aaaa1111").unwrap(),
            None
        );
        assert_eq!(
            read_connect_bearer_for_conn("main", "bbbb2222").unwrap(),
            None
        );
    }
}
