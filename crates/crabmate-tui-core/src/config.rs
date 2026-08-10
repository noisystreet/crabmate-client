//! 连接配置与错误类型。

use crate::url::normalize_api_base;

/// 指向远程 `serve` 的连接参数。
#[derive(Debug, Clone)]
pub struct ConnectionConfig {
    /// `serve` 根 URL（无尾斜杠），如 `http://127.0.0.1:8080`。
    pub api_base: String,
    /// Web API Bearer（`CM_WEB_API_BEARER_TOKEN`）；可空（同 Origin 开发）。
    pub bearer_token: String,
}

impl ConnectionConfig {
    #[must_use]
    pub fn new(api_base: impl Into<String>, bearer_token: impl Into<String>) -> Self {
        Self {
            api_base: normalize_api_base(&api_base.into()),
            bearer_token: bearer_token.into().trim().to_string(),
        }
    }
}
