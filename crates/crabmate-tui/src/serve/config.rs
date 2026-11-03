//! 连接配置与错误类型。

use crate::serve::url::normalize_api_base;

/// Web Bearer 的**来源**（只报来源，不报值）。
///
/// 设置面板「连接」栏要回答的第一个排障问题是「我以为带了 token，其实带了吗」——
/// 那只有来源能答：值为空是「没带」，非空也还要分清是本机钥匙串回退来的、还是本次
/// 启动显式给的（前者可能属于桌面主窗口正在连的**另一台** serve）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BearerSource {
    /// `--bearer` / `CM_WEB_API_BEARER_TOKEN` 显式给出。
    Cli,
    /// 未显式指定时回退读到桌面壳钥匙串槽（见 `crate::shell_keyring`）。
    Keyring,
    /// 没提供（同 Origin 本地开发不需要鉴权的常态）。
    #[default]
    Missing,
}

/// 指向远程 `serve` 的连接参数。
#[derive(Debug, Clone)]
pub struct ConnectionConfig {
    /// `serve` 根 URL（无尾斜杠），如 `http://127.0.0.1:8080`。
    pub api_base: String,
    /// Web API Bearer（`CM_WEB_API_BEARER_TOKEN`）；可空（同 Origin 开发）。
    pub bearer_token: String,
    /// Bearer 是从哪来的（面板只展示来源）。构造时默认「未提供」，由装配处按实际
    /// 解析结果标注 —— 值本身不可逆推：空值也可能来自显式的空 `--bearer`。
    pub bearer_source: BearerSource,
}

impl ConnectionConfig {
    #[must_use]
    pub fn new(api_base: impl Into<String>, bearer_token: impl Into<String>) -> Self {
        Self {
            api_base: normalize_api_base(&api_base.into()),
            bearer_token: bearer_token.into().trim().to_string(),
            bearer_source: BearerSource::Missing,
        }
    }

    /// 标注 Bearer 来源（链式，构造点保持一行）。
    #[must_use]
    pub fn with_bearer_source(mut self, source: BearerSource) -> Self {
        self.bearer_source = source;
        self
    }
}
