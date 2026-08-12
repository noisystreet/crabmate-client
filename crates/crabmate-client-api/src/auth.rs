//! Web API 鉴权头名与 Bearer / `X-API-Key` 值形状（无 IO）。

/// `Authorization` 头名。
pub const HEADER_AUTHORIZATION: &str = "Authorization";
/// 与 Bearer 同值的备用头（serve 接受二者之一）。
pub const HEADER_X_API_KEY: &str = "X-API-Key";
/// 壳 Device Flow 后附带的 GitHub user token 头。
pub const HEADER_GITHUB_TOKEN: &str = "X-CrabMate-GitHub-Token";

/// 一对 Web API 凭证头值（非空 token 时）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebApiCredentialPair {
    /// `Authorization: Bearer <token>` 的完整头值（含 `Bearer ` 前缀）。
    pub authorization: String,
    /// `X-API-Key` 头值（原始 token，无前缀）。
    pub api_key: String,
}

/// 非空 Bearer → 双头值；空白返回 `None`。
#[must_use]
pub fn web_api_credential_pair(bearer: &str) -> Option<WebApiCredentialPair> {
    let t = bearer.trim();
    if t.is_empty() {
        return None;
    }
    Some(WebApiCredentialPair {
        authorization: format!("Bearer {t}"),
        api_key: t.to_string(),
    })
}

/// 非空 GitHub token → 头值；空白返回 `None`。
#[must_use]
pub fn github_token_header_value(token: &str) -> Option<&str> {
    let t = token.trim();
    if t.is_empty() { None } else { Some(t) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_pair_skips_blank() {
        assert!(web_api_credential_pair("").is_none());
        assert!(web_api_credential_pair("  ").is_none());
        let p = web_api_credential_pair(" secret ").unwrap();
        assert_eq!(p.authorization, "Bearer secret");
        assert_eq!(p.api_key, "secret");
    }

    #[test]
    fn github_header_value_trims() {
        assert_eq!(github_token_header_value(" abc "), Some("abc"));
        assert!(github_token_header_value("").is_none());
    }
}
