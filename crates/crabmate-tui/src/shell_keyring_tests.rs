//! `shell_keyring.rs` 的单元测试（`#[path]` 挂载；不触碰真实钥匙串）。

use std::cell::RefCell;

use crabmate_client_api::secrets::{
    DESKTOP_MAIN_WINDOW_LABEL, WEB_API_BEARER_KEYRING_ACCOUNT, scoped_bearer_account,
};

use super::{
    bearer_account_candidates, first_keyring_hit, hit_note, miss_note, with_shell_keyring_fallback,
};

#[test]
fn explicit_value_wins_over_keyring() {
    let v = with_shell_keyring_fallback(Some(" tok "), false, || panic!("must not read keyring"));
    assert_eq!(v.as_deref(), Some("tok"));
}

#[test]
fn explicit_empty_disables_keyring_fallback() {
    let v = with_shell_keyring_fallback(Some("   "), false, || panic!("must not read keyring"));
    assert!(v.is_none());
}

#[test]
fn missing_value_falls_back_to_keyring() {
    let v = with_shell_keyring_fallback(None, false, || Some("keyring-secret".into()));
    assert_eq!(v.as_deref(), Some("keyring-secret"));
}

#[test]
fn no_keyring_skips_fallback() {
    let v = with_shell_keyring_fallback(None, true, || panic!("must not read keyring"));
    assert!(v.is_none());
}

#[test]
fn missing_entry_yields_none() {
    let v = with_shell_keyring_fallback(None, false, || None);
    assert!(v.is_none());
}

#[test]
fn bearer_candidates_try_the_scoped_main_slot_first() {
    let accounts = bearer_account_candidates();
    assert_eq!(
        accounts,
        vec![
            scoped_bearer_account(DESKTOP_MAIN_WINDOW_LABEL),
            WEB_API_BEARER_KEYRING_ACCOUNT.to_string(),
        ]
    );
    // 桌面壳实际写下的账户名；与裸槽不同，否则分槽等于没做。
    assert_eq!(accounts[0], "tauri_connect_web_api_bearer:main");
    assert_ne!(accounts[0], accounts[1]);
}

#[test]
fn first_hit_stops_at_the_scoped_slot() {
    let accounts = bearer_account_candidates();
    let probes = RefCell::new(Vec::new());
    let hit = first_keyring_hit(&accounts, |account| {
        probes.borrow_mut().push(account.to_string());
        Some(format!("secret-for-{account}"))
    })
    .expect("hit");

    assert_eq!(hit.0, accounts[0]);
    assert_eq!(hit.1, format!("secret-for-{}", accounts[0]));
    // 短路：命中后不再探测裸槽——省一次钥匙串往返，也不会读到别的窗口的凭据。
    assert_eq!(probes.borrow().len(), 1);
}

#[test]
fn legacy_slot_still_answers_when_the_scoped_one_is_missing() {
    // 「从未用新版桌面连接过」的安装：分槽槽不存在，只有裸槽。
    let accounts = bearer_account_candidates();
    let legacy = WEB_API_BEARER_KEYRING_ACCOUNT.to_string();
    let hit = first_keyring_hit(&accounts, |account| {
        (account == legacy).then(|| "legacy-token".to_string())
    })
    .expect("legacy hit");

    assert_eq!(hit.0, legacy);
    assert_eq!(hit.1, "legacy-token");
}

#[test]
fn no_slot_means_no_credentials() {
    // 两个槽都空时必须如实返回 None：调用方据此判定「未提供 Bearer」，
    // 而不是拿空串去发一个注定 401 的匿名请求。
    let accounts = bearer_account_candidates();
    assert!(first_keyring_hit(&accounts, |_| None).is_none());
}

#[test]
fn hit_note_names_the_slot_and_keeps_the_manual_way_out() {
    let note = hit_note("tauri_connect_web_api_bearer:main");
    // 必须说明取自哪个槽：否则「读到了却仍 401」无从排查。
    assert!(note.contains("tauri_connect_web_api_bearer:main"), "{note}");
    // 那个槽不保证属于本次 `--api-base`，所以既要点出人工指定这条出口，
    // 也不能用「已修好」的语气。
    assert!(note.contains("--bearer"), "{note}");
    assert!(note.contains("不是同一台"), "{note}");
}

#[test]
fn miss_note_lists_probed_slots_and_conditions_the_advice() {
    let accounts = bearer_account_candidates();
    let note = miss_note(&accounts);
    for account in &accounts {
        assert!(note.contains(account.as_str()), "缺少 {account}：{note}");
    }
    // 「没有槽」不等于「需要 Bearer」（同 Origin 本地 serve 就不需要），
    // 所以建议必须挂在条件从句上，不能无条件让人去加 --bearer。
    assert!(note.contains("serve 启用 Bearer 鉴权时"), "{note}");
}
