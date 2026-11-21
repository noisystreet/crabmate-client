//! 模型 API 密钥：本机系统钥匙串（非 TOML / 非明文 localStorage）。
//!
//! 与连接 Bearer 共用 service，账户名区分槽位。Android 上 `keyring` 常不可用，
//! 由壳层 `CrabMateMobile` + Keystore 桥兜底（见 frontend / MainActivity）。
//!
//! 槽位短名 / 账户名常量见 [`crabmate_client_api::secrets`]。

use crabmate_client_api::secrets::SecretSlot;

use crate::secret_backend::{read_trimmed, write_trimmed};

/// 主模型 / 执行器 / 已保存模型密钥表（JSON）/ GitHub user token 在钥匙串中的账户名。
pub type LlmSecretSlot = SecretSlot;

/// 读取槽位；无条目或空白值返回 `None`。
pub fn read_llm_secret(slot: LlmSecretSlot) -> Result<Option<String>, String> {
    read_trimmed(slot.keyring_account())
}

/// 写入或清除槽位（空串删除条目）。
pub fn write_llm_secret(slot: LlmSecretSlot, value: &str) -> Result<(), String> {
    write_trimmed(slot.keyring_account(), value)
}

#[cfg(test)]
mod tests {
    use super::{LlmSecretSlot, read_llm_secret, write_llm_secret};
    use crate::secret_backend::testing;

    #[test]
    fn written_secret_reads_back_trimmed() {
        let _backend = testing::install();
        write_llm_secret(LlmSecretSlot::ClientLlm, "  sk-e2e-key\n").unwrap();
        assert_eq!(
            read_llm_secret(LlmSecretSlot::ClientLlm)
                .unwrap()
                .as_deref(),
            Some("sk-e2e-key")
        );
    }

    #[test]
    fn slots_do_not_share_one_entry() {
        let _backend = testing::install();
        write_llm_secret(LlmSecretSlot::ClientLlm, "main-key").unwrap();
        write_llm_secret(LlmSecretSlot::ExecutorLlm, "exec-key").unwrap();

        assert_eq!(
            read_llm_secret(LlmSecretSlot::ClientLlm)
                .unwrap()
                .as_deref(),
            Some("main-key")
        );
        assert_eq!(
            read_llm_secret(LlmSecretSlot::ExecutorLlm)
                .unwrap()
                .as_deref(),
            Some("exec-key")
        );
    }

    #[test]
    fn blank_write_clears_the_slot() {
        let _backend = testing::install();
        write_llm_secret(LlmSecretSlot::ClientLlm, "sk-e2e-key").unwrap();
        write_llm_secret(LlmSecretSlot::ClientLlm, "   ").unwrap();

        assert_eq!(read_llm_secret(LlmSecretSlot::ClientLlm).unwrap(), None);
    }

    #[test]
    fn reading_an_empty_keystore_yields_none() {
        let _backend = testing::install();

        assert_eq!(read_llm_secret(LlmSecretSlot::SavedModels).unwrap(), None);
    }
}
