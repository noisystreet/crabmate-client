//! API 基址规范化与路径拼接（委托 [`crabmate_client_api::url`]）。

use crabmate_client_api::url::{self, ApiUrlError};

use crate::error::TermError;

/// 去掉尾斜杠；拒绝相对路径。
#[must_use]
pub fn normalize_api_base(raw: &str) -> String {
    url::normalize_api_base(raw)
}

/// `base` + `path`（`path` 须以 `/` 开头）。
pub fn api_url(base: &str, path: &str) -> Result<String, TermError> {
    url::join_api_path(base, path).map_err(|e| match e {
        ApiUrlError::InvalidBase(m) | ApiUrlError::InvalidPath(m) => TermError::InvalidApiBase(m),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_slash() {
        assert_eq!(
            normalize_api_base("http://127.0.0.1:8080/"),
            "http://127.0.0.1:8080"
        );
        assert_eq!(normalize_api_base("  "), "");
        assert_eq!(normalize_api_base("/relative"), "");
    }

    #[test]
    fn api_url_joins() {
        let u = api_url("http://127.0.0.1:8080/", "/chat/stream").unwrap();
        assert_eq!(u, "http://127.0.0.1:8080/chat/stream");
    }
}
