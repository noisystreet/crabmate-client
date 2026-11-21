//! 钥匙串读写的可替换后端（生产：系统钥匙串；测试：内存）。
//!
//! # 为什么需要这一层
//!
//! `crabmate-connect` 的钥匙串读写此前直接调 `keyring::Entry`，于是唯一能在
//! `cargo test` 里验证的方式是碰**真**钥匙串 —— 在无 Secret Service 的 CI 上直接失败，
//! 在开发机上则读写用户真实凭据。两者都不可接受，结果就是「写进去 → 读回来」这条
//! 最该被测的路径从来没被测过（`keyring_bearer` 只有两条账户名拼接的断言）。
//!
//! 官方 mock 也帮不上：`keyring` 4 的 `Entry` 是 `v1` 包装，首次 `new()` 会把平台 store
//! 装进 `keyring_core` 的进程级默认槽（`v1::set_credential_store` 里那句
//! `keyring_core::set_default_store`），预先注入的 `keyring_core::mock` 会被它覆盖；
//! 无后端时更直接返回 `NoDefaultStore`。所以「哪块存储」必须在**本 crate 内**可替换。
//!
//! # 语义归属
//!
//! trim / 空串即删除 / `NoEntry → None` 这些**业务规则**放在 [`read_trimmed`] /
//! [`write_trimmed`] 里，与后端无关；后端只负责原始读写。这样测试跑的是同一条规则，
//! 而不是在替身里复刻一份、与生产实现慢慢漂移。

use std::sync::{Arc, Mutex};

use crabmate_client_api::secrets::KEYRING_SERVICE;

/// 一个账户名上的**原始**读写（不做 trim，不做空串特判）。
pub(crate) trait SecretBackend: Send + Sync {
    /// 原样返回已存的值；无条目 → `Ok(None)`。
    fn raw_get(&self, account: &str) -> Result<Option<String>, String>;
    /// 原样写入；空串表示删除该条目。
    fn raw_set(&self, account: &str, value: &str) -> Result<(), String>;
}

/// 生产后端：系统钥匙串（macOS Keychain / Windows Credential Manager / Secret Service）。
struct KeyringBackend;

impl SecretBackend for KeyringBackend {
    fn raw_get(&self, account: &str) -> Result<Option<String>, String> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, account)
            .map_err(|e| format!("系统钥匙串不可用: {e}"))?;
        match entry.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(format!("读取系统钥匙串失败: {e}")),
        }
    }

    fn raw_set(&self, account: &str, value: &str) -> Result<(), String> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, account)
            .map_err(|e| format!("系统钥匙串不可用: {e}"))?;
        if value.is_empty() {
            match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(e) => Err(format!("清除系统钥匙串失败: {e}")),
            }
        } else {
            entry
                .set_password(value)
                .map_err(|e| format!("写入系统钥匙串失败: {e}"))
        }
    }
}

/// 测试注入槽。生产路径恒为 `None`（零开销：一次判空）。
static OVERRIDE: Mutex<Option<Arc<dyn SecretBackend>>> = Mutex::new(None);

fn backend() -> Arc<dyn SecretBackend> {
    match OVERRIDE.lock() {
        Ok(guard) => guard.clone().unwrap_or_else(|| Arc::new(KeyringBackend)),
        // 中毒只可能来自别的测试 panic（`OVERRIDE` 只在测试里被写）；回退生产后端而不是再 panic。
        Err(_) => Arc::new(KeyringBackend),
    }
}

/// 只在测试里写入：[`OVERRIDE`] 生产路径永远是 `None`。
#[cfg(test)]
fn set_override(next: Option<Arc<dyn SecretBackend>>) {
    match OVERRIDE.lock() {
        Ok(mut guard) => *guard = next,
        Err(poisoned) => *poisoned.into_inner() = next,
    }
}

/// 读并 trim；无条目与空白值都视为 `None`。
pub(crate) fn read_trimmed(account: &str) -> Result<Option<String>, String> {
    let Some(raw) = backend().raw_get(account)? else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed.to_string()))
    }
}

/// 写入 trim 后的值；空串（含全空白）即删除条目。
pub(crate) fn write_trimmed(account: &str, value: &str) -> Result<(), String> {
    backend().raw_set(account, value.trim())
}

#[cfg(test)]
pub(crate) mod testing {
    use std::collections::HashMap;

    use super::{SecretBackend, set_override};
    use std::sync::{Arc, Mutex, MutexGuard};

    /// 无持久化的内存后端，等价于「一块干净的钥匙串」。
    #[derive(Default)]
    pub(crate) struct MemoryBackend {
        entries: Mutex<HashMap<String, String>>,
    }

    impl SecretBackend for MemoryBackend {
        fn raw_get(&self, account: &str) -> Result<Option<String>, String> {
            match self.entries.lock() {
                Ok(guard) => Ok(guard.get(account).cloned()),
                Err(_) => Err("memory backend poisoned".to_string()),
            }
        }

        fn raw_set(&self, account: &str, value: &str) -> Result<(), String> {
            let Ok(mut guard) = self.entries.lock() else {
                return Err("memory backend poisoned".to_string());
            };
            if value.is_empty() {
                guard.remove(account);
            } else {
                guard.insert(account.to_string(), value.to_string());
            }
            Ok(())
        }
    }

    /// 串行锁：默认后端是**进程级单值**，并行跑测试时 A 装上新 store 会把 B 刚写的
    /// 条目一起丢掉，症状是随机失败。持有它就等于「这段时间只有我在用钥匙串」。
    static SERIAL: Mutex<()> = Mutex::new(());

    /// 装上内存后端；drop 时恢复生产后端并释放锁。
    pub(crate) struct Installed {
        _serial: MutexGuard<'static, ()>,
    }

    pub(crate) fn install() -> Installed {
        let serial = match SERIAL.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        set_override(Some(Arc::new(MemoryBackend::default())));
        Installed { _serial: serial }
    }

    impl Drop for Installed {
        fn drop(&mut self) {
            set_override(None);
        }
    }
}
