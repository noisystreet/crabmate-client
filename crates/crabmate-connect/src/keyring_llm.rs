//! 模型 API 密钥：本机系统钥匙串（非 TOML / 非明文 localStorage）。
//!
//! 与连接 Bearer 共用 service，账户名区分槽位。Android 上 `keyring` 常不可用，
//! 由壳层 `CrabMateMobile` + Keystore 桥兜底（见 frontend / MainActivity）。
//!
//! 槽位短名 / 账户名常量见 [`crabmate_client_api::secrets`]。

use crabmate_client_api::secrets::{KEYRING_SERVICE, SecretSlot};

/// 主模型 / 执行器 / 已保存模型密钥表（JSON）/ GitHub user token 在钥匙串中的账户名。
pub type LlmSecretSlot = SecretSlot;

fn entry(slot: LlmSecretSlot) -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, slot.keyring_account())
        .map_err(|e| format!("系统钥匙串不可用: {e}"))
}

/// 读取槽位；无条目或空串返回 `None`。
pub fn read_llm_secret(slot: LlmSecretSlot) -> Result<Option<String>, String> {
    let entry = entry(slot)?;
    match entry.get_password() {
        Ok(secret) => {
            let t = secret.trim();
            if t.is_empty() {
                Ok(None)
            } else {
                Ok(Some(t.to_string()))
            }
        }
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("读取系统钥匙串失败: {e}")),
    }
}

/// 写入或清除槽位（空串删除条目）。
pub fn write_llm_secret(slot: LlmSecretSlot, value: &str) -> Result<(), String> {
    let entry = entry(slot)?;
    let b = value.trim();
    if b.is_empty() {
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(format!("清除系统钥匙串失败: {e}")),
        }
    } else {
        entry
            .set_password(b)
            .map_err(|e| format!("写入系统钥匙串失败: {e}"))
    }
}
