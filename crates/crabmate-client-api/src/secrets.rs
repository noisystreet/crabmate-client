//! 钥匙串 / Keystore / 桥槽位**名字常量**（无 IO）。

/// 桌面系统钥匙串 service（Bearer 与 LLM / GitHub 槽共用）。
pub const KEYRING_SERVICE: &str = "com.crabmate.credentials";

/// 连接页 Web API Bearer 在钥匙串中的账户名。
///
/// 分槽写入时带 `:<label>` 后缀，见 [`scoped_bearer_account`]。
pub const WEB_API_BEARER_KEYRING_ACCOUNT: &str = "tauri_connect_web_api_bearer";

/// 桌面壳主窗口的 label（与 `desktop-tauri` 的 `MAIN_WINDOW_LABEL` 一致）。
///
/// 它不只是一个 UI 名字：Bearer 分槽后账户名带这个后缀，TUI 的钥匙串回退要靠它
/// 找到桌面壳写下的凭据。
pub const DESKTOP_MAIN_WINDOW_LABEL: &str = "main";

/// Bearer 分槽后的账户名：`<account>:<label>`。
///
/// 写方（`crabmate-connect` 连接成功时）与读方（TUI 的钥匙串回退）必须共用这一份
/// 拼接规则。两处各写一遍 `format!` 就会漂移：桌面只写分槽槽、TUI 只读裸槽，而桌面的
/// 迁移逻辑会把裸槽搬进分槽槽后**清空**它，于是 TUI 从此读到 `None`、发出匿名请求，
/// 症状是 serve 返回 401（而不是「读钥匙串失败」）。
#[must_use]
pub fn scoped_bearer_account(label: &str) -> String {
    format!("{WEB_API_BEARER_KEYRING_ACCOUNT}:{label}")
}

/// Bearer 分槽后的**连接级**账户名：`<account>:<label>@<fp>`。
///
/// 为什么还要第二级：`<account>:<label>` 只做到「一窗口一槽」（#391）。同一个窗口换连
/// 另一台 `serve` 时仍会覆盖那一槽，切回上一台就拿着新凭据去连 —— 401，且看起来像
/// 「serve 的 Bearer 配错了」。`fp`（基址指纹，见
/// [`crate::connection_scope::api_base_fingerprint`]）把「这个窗口连过的每一台」分开记。
///
/// `fp` 为空串时退回一级名：TUI 的钥匙串回退、以及不区分连接的调用方都走这一支，
/// 语义是「这个窗口最近一次连接」——写侧同时更新一级槽就是为了保住它。
#[must_use]
pub fn scoped_bearer_account_with_fp(label: &str, fp: &str) -> String {
    let fp = fp.trim();
    if fp.is_empty() {
        return scoped_bearer_account(label);
    }
    format!("{}@{fp}", scoped_bearer_account(label))
}

/// 主模型 / 执行器 / 已保存模型密钥表 / GitHub user token 槽。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretSlot {
    ClientLlm,
    ExecutorLlm,
    SavedModels,
    /// Device Flow 成功后的 GitHub user access token（壳专用；浏览器走 HttpOnly Cookie）。
    Github,
    /// GitHub OAuth refresh token（壳专用；每次刷新轮换；浏览器走 HttpOnly Cookie）。
    GithubRefresh,
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
            Self::GithubRefresh => "github_refresh",
        }
    }

    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "client_llm" => Some(Self::ClientLlm),
            "executor_llm" => Some(Self::ExecutorLlm),
            "saved_models" => Some(Self::SavedModels),
            "github" => Some(Self::Github),
            "github_refresh" => Some(Self::GithubRefresh),
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
            Self::GithubRefresh => "tauri_github_refresh_token",
        }
    }
}

/// 全部桥槽短名（文档 / 测试用）。
#[must_use]
pub fn secret_slot_names() -> [&'static str; 5] {
    [
        SecretSlot::ClientLlm.as_str(),
        SecretSlot::ExecutorLlm.as_str(),
        SecretSlot::SavedModels.as_str(),
        SecretSlot::Github.as_str(),
        SecretSlot::GithubRefresh.as_str(),
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

    #[test]
    fn scoped_bearer_account_appends_the_window_label() {
        assert_eq!(
            scoped_bearer_account(DESKTOP_MAIN_WINDOW_LABEL),
            "tauri_connect_web_api_bearer:main"
        );
        // 分槽槽名必须异于裸槽：相同就又回到「所有窗口共用一份凭据」。
        assert_ne!(
            scoped_bearer_account(DESKTOP_MAIN_WINDOW_LABEL),
            WEB_API_BEARER_KEYRING_ACCOUNT
        );
    }

    #[test]
    fn scoped_bearer_account_with_fp_separates_serves_within_one_window() {
        let main = DESKTOP_MAIN_WINDOW_LABEL;
        let a = scoped_bearer_account_with_fp(main, "0a1b2c3d");
        let b = scoped_bearer_account_with_fp(main, "0a1b2c3e");
        // 同一个窗口连两台 serve：一级槽相同，二级槽必须不同，否则切回上一台就是 401。
        assert_ne!(a, b);
        assert_eq!(a, "tauri_connect_web_api_bearer:main@0a1b2c3d");
    }

    #[test]
    fn empty_fp_falls_back_to_the_window_level_account() {
        // 没有指纹 = 「这个窗口最近一次连接」：TUI 的回退路径读的就是它。
        assert_eq!(
            scoped_bearer_account_with_fp(DESKTOP_MAIN_WINDOW_LABEL, ""),
            scoped_bearer_account(DESKTOP_MAIN_WINDOW_LABEL)
        );
        assert_eq!(
            scoped_bearer_account_with_fp(DESKTOP_MAIN_WINDOW_LABEL, "  "),
            scoped_bearer_account(DESKTOP_MAIN_WINDOW_LABEL)
        );
    }

    #[test]
    fn the_connection_level_account_differs_from_the_bare_legacy_slot() {
        assert_ne!(
            scoped_bearer_account_with_fp("main", "0a1b2c3d"),
            WEB_API_BEARER_KEYRING_ACCOUNT
        );
    }
}
