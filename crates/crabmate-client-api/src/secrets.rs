//! 钥匙串 / Keystore / 桥槽位**名字常量**（无 IO）。

/// 桌面系统钥匙串 service（Bearer 与 LLM / GitHub 槽共用）。
pub const KEYRING_SERVICE: &str = "com.crabmate.credentials";

/// 连接页 Web API Bearer 在钥匙串中的账户名。
pub const WEB_API_BEARER_KEYRING_ACCOUNT: &str = "tauri_connect_web_api_bearer";

/// 主模型 / 执行器 / 已保存模型密钥表 / GitHub user token 槽。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretSlot {
    ClientLlm,
    ExecutorLlm,
    SavedModels,
    /// Device Flow 成功后的 GitHub user access token（壳专用；浏览器走 HttpOnly Cookie）。
    Github,
}

impl SecretSlot {
    /// 桥 / invoke 使用的短名（`client_llm` 等）。
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClientLlm => "client_llm",
            Self::ExecutorLlm => "executor_llm",
            Self::SavedModels => "saved_models",
            Self::Github => "github",
        }
    }

    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "client_llm" => Some(Self::ClientLlm),
            "executor_llm" => Some(Self::ExecutorLlm),
            "saved_models" => Some(Self::SavedModels),
            "github" => Some(Self::Github),
            _ => None,
        }
    }

    /// 桌面钥匙串账户名（与 Android Keystore 别名策略独立，仅 Desktop `keyring`）。
    #[must_use]
    pub fn keyring_account(self) -> &'static str {
        match self {
            Self::ClientLlm => "tauri_client_llm_api_key",
            Self::ExecutorLlm => "tauri_executor_llm_api_key",
            Self::SavedModels => "tauri_saved_model_api_keys",
            Self::Github => "tauri_github_access_token",
        }
    }
}

/// 全部桥槽短名（文档 / 测试用）。
#[must_use]
pub fn secret_slot_names() -> [&'static str; 4] {
    [
        SecretSlot::ClientLlm.as_str(),
        SecretSlot::ExecutorLlm.as_str(),
        SecretSlot::SavedModels.as_str(),
        SecretSlot::Github.as_str(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_roundtrip_and_accounts() {
        for name in secret_slot_names() {
            let s = SecretSlot::parse(name).expect(name);
            assert_eq!(s.as_str(), name);
            assert!(!s.keyring_account().is_empty());
        }
        assert!(SecretSlot::parse("nope").is_none());
        assert_eq!(
            WEB_API_BEARER_KEYRING_ACCOUNT,
            "tauri_connect_web_api_bearer"
        );
        assert_eq!(KEYRING_SERVICE, "com.crabmate.credentials");
    }
}
