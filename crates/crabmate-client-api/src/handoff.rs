//! 壳 / `crabmate-web` → 业务 UI 的 URL hash 交接（无 IO、无 `history`）。
//!
//! Bearer **不得**写入查询串；hash 仅作短时手递。

/// 本地业务 UI 启动时写入的 API 基址（指向远程 `serve` 根）。
pub const API_BASE_HASH_KEY: &str = "cm_api_base";

/// Web API Bearer（≠ 模型 `API_KEY`）。
pub const BEARER_HASH_KEY: &str = "cm_web_api_bearer";

/// 是否为交接敏感键（消费后应从地址栏去掉）。
#[must_use]
pub fn is_handoff_hash_key(k: &str) -> bool {
    k == API_BASE_HASH_KEY || k == BEARER_HASH_KEY
}

/// RFC 3986 unreserved 原样保留，其余 `%HH`（大写十六进制）。
#[must_use]
pub fn percent_encode_unreserved(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn trimmed_nonempty(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|t| !t.is_empty())
}

/// Hash fragment（无前导 `#`）。空白项省略；顺序为 API 基址再 Bearer。
#[must_use]
pub fn handoff_hash_fragment(api_base: Option<&str>, bearer: Option<&str>) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(2);
    if let Some(base) = trimmed_nonempty(api_base) {
        parts.push(format!(
            "{API_BASE_HASH_KEY}={}",
            percent_encode_unreserved(base)
        ));
    }
    if let Some(token) = trimmed_nonempty(bearer) {
        parts.push(format!(
            "{BEARER_HASH_KEY}={}",
            percent_encode_unreserved(token)
        ));
    }
    parts.join("&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_stable() {
        assert_eq!(API_BASE_HASH_KEY, "cm_api_base");
        assert_eq!(BEARER_HASH_KEY, "cm_web_api_bearer");
        assert!(is_handoff_hash_key(API_BASE_HASH_KEY));
        assert!(is_handoff_hash_key(BEARER_HASH_KEY));
        assert!(!is_handoff_hash_key("cm_shell"));
    }

    #[test]
    fn encodes_slash_and_colon() {
        assert_eq!(percent_encode_unreserved("a/b"), "a%2Fb");
        assert_eq!(
            percent_encode_unreserved("http://127.0.0.1:8080"),
            "http%3A%2F%2F127.0.0.1%3A8080"
        );
    }

    #[test]
    fn fragment_order_and_blank_omit() {
        let both = handoff_hash_fragment(Some("http://127.0.0.1:8080"), Some("a/b"));
        assert_eq!(
            both,
            "cm_api_base=http%3A%2F%2F127.0.0.1%3A8080&cm_web_api_bearer=a%2Fb"
        );
        assert_eq!(
            handoff_hash_fragment(Some("http://127.0.0.1:8080"), Some("  ")),
            "cm_api_base=http%3A%2F%2F127.0.0.1%3A8080"
        );
        assert_eq!(
            handoff_hash_fragment(None, Some("tok")),
            "cm_web_api_bearer=tok"
        );
        assert!(handoff_hash_fragment(None, Some("")).is_empty());
    }
}
