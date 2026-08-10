//! API 基址规范化与路径拼接。

use crate::error::TermError;

/// 去掉尾斜杠；拒绝相对路径。
#[must_use]
pub fn normalize_api_base(raw: &str) -> String {
    let t = raw.trim();
    if t.is_empty() {
        return String::new();
    }
    if !(t.starts_with("http://") || t.starts_with("https://")) {
        return String::new();
    }
    t.trim_end_matches('/').to_string()
}

/// `base` + `path`（`path` 须以 `/` 开头）。
pub fn api_url(base: &str, path: &str) -> Result<String, TermError> {
    let base = normalize_api_base(base);
    if base.is_empty() {
        return Err(TermError::InvalidApiBase(
            "API base must be an absolute http(s) URL".into(),
        ));
    }
    let path = if path.is_empty() {
        "/"
    } else if path.starts_with('/') {
        path
    } else {
        return Err(TermError::InvalidApiBase(format!(
            "path must start with /: {path}"
        )));
    };
    Ok(format!("{base}{path}"))
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
