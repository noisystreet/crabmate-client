//! 严格绝对 API 基址规范化与 path join。
//!
//! 连接页「可补 `http://`、拒 `0.0.0.0`」等输入规范化仍属 `crabmate-connect`；
//! 本模块只接受已是 `http(s)://…` 的基址。

/// API 基址或 path 不合法。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiUrlError {
    InvalidBase(String),
    InvalidPath(String),
}

impl core::fmt::Display for ApiUrlError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidBase(m) | Self::InvalidPath(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for ApiUrlError {}

/// 去掉首尾空白与尾 `/`；非绝对 `http(s)://` 返回空串。
#[must_use]
pub fn normalize_api_base(raw: &str) -> String {
    let t = raw.trim();
    if t.is_empty() {
        return String::new();
    }
    let without_slash = t.trim_end_matches('/');
    if !(without_slash.starts_with("http://") || without_slash.starts_with("https://")) {
        return String::new();
    }
    without_slash.to_string()
}

/// `base` + `path`（`path` 空视为 `/`，否则须以 `/` 开头；`base` 必须是绝对 http(s)）。
pub fn join_api_path(base: &str, path: &str) -> Result<String, ApiUrlError> {
    let base = normalize_api_base(base);
    if base.is_empty() {
        return Err(ApiUrlError::InvalidBase(
            "API base must be an absolute http(s) URL".into(),
        ));
    }
    let path = if path.is_empty() {
        "/"
    } else if path.starts_with('/') {
        path
    } else {
        return Err(ApiUrlError::InvalidPath(format!(
            "path must start with /: {path}"
        )));
    };
    Ok(format!("{base}{path}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_slash_and_rejects_relative() {
        assert_eq!(
            normalize_api_base("http://127.0.0.1:8080/"),
            "http://127.0.0.1:8080"
        );
        assert_eq!(
            normalize_api_base("https://api.example.com/v1/"),
            "https://api.example.com/v1"
        );
        assert_eq!(normalize_api_base("  "), "");
        assert_eq!(normalize_api_base("/relative"), "");
        assert_eq!(normalize_api_base("ftp://x"), "");
        // 去尾 `/` 后不再是 http(s):// → 空（与 frontend 一致；严于旧 tui `http://`→`http:`）
        assert_eq!(normalize_api_base("http://"), "");
    }

    #[test]
    fn join_requires_absolute_base_and_slash_path() {
        let u = join_api_path("http://127.0.0.1:8080/", "/chat/stream").unwrap();
        assert_eq!(u, "http://127.0.0.1:8080/chat/stream");
        assert!(join_api_path("", "/health").is_err());
        assert!(join_api_path("http://127.0.0.1:8080", "health").is_err());
        assert_eq!(
            join_api_path("http://127.0.0.1:8080", "").unwrap(),
            "http://127.0.0.1:8080/"
        );
    }
}
