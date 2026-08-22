//! URL 规范化与本地 UI 交接（Bearer + API 基址 hash）。
//!
//! Hash 键名与 fragment 编码见 [`crabmate_client_api::handoff`]。

use crabmate_client_api::handoff_hash_fragment;
use url::Url;

pub use crabmate_client_api::{API_BASE_HASH_KEY, BEARER_HASH_KEY};

fn is_unspecified_connect_host(host: &str) -> bool {
    let h = host
        .trim()
        .trim_matches(|c| c == '[' || c == ']')
        .to_ascii_lowercase();
    matches!(h.as_str(), "0.0.0.0" | "::" | "0:0:0:0:0:0:0:0")
}

pub fn normalize_base_url(raw: &str) -> Result<Url, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("请填写服务器地址".into());
    }
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("http://{trimmed}")
    };
    let mut u = Url::parse(&with_scheme).map_err(|e| format!("地址无效: {e}"))?;
    match u.scheme() {
        "http" | "https" => {}
        other => return Err(format!("仅支持 http/https，收到 {other}")),
    }
    if u.host_str().is_some_and(is_unspecified_connect_host) {
        return Err(
            "0.0.0.0 / :: 是服务端监听地址，不能作为客户端连接目标。请改用 http://127.0.0.1:端口/ 或本机局域网 IP"
                .into(),
        );
    }
    if !u.path().ends_with('/') {
        let p = if u.path().is_empty() {
            "/".to_string()
        } else {
            format!("{}/", u.path())
        };
        u.set_path(&p);
    }
    Ok(u)
}

/// 包内业务 UI 入口（与 `connect.html` 同 Origin 的 `index.html`）。
#[must_use]
pub fn local_business_ui_url(connect_home: &Url) -> Url {
    let mut ui = connect_home.clone();
    ui.set_path("/index.html");
    ui.set_query(None);
    ui.set_fragment(None);
    ui
}

/// 在本地 UI URL 上写入 API 基址；Bearer 非空时一并写入（空 Bearer 不写，避免清掉页内已有凭证）。
#[must_use]
pub fn build_local_ui_handoff_url(mut ui: Url, api_base: &Url, bearer: &str) -> Url {
    let api = api_base.as_str().trim_end_matches('/');
    let b = bearer.trim();
    let bearer = if b.is_empty() { None } else { Some(b) };
    ui.set_fragment(Some(&handoff_hash_fragment(Some(api), bearer)));
    ui
}

/// 兼容旧调用：仅 Bearer、目标为任意基址（旧「导航到 serve」路径）。
#[must_use]
pub fn build_handoff_url(mut base: Url, bearer: &str) -> Url {
    let b = bearer.trim();
    let frag = handoff_hash_fragment(None, Some(b));
    if frag.is_empty() {
        base.set_fragment(None);
    } else {
        base.set_fragment(Some(&frag));
    }
    base
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_adds_scheme_and_slash() {
        let u = normalize_base_url("192.168.1.10:8080").unwrap();
        assert_eq!(u.scheme(), "http");
        assert!(u.path().ends_with('/'));
        assert!(u.as_str().contains("192.168.1.10:8080"));
    }

    #[test]
    fn normalize_rejects_unspecified_bind_host() {
        let err = normalize_base_url("http://0.0.0.0:8080/").unwrap_err();
        assert!(err.contains("127.0.0.1"), "{err}");
        let err6 = normalize_base_url("http://[::]:8080/").unwrap_err();
        assert!(err6.contains("127.0.0.1"), "{err6}");
    }

    #[test]
    fn handoff_puts_bearer_in_fragment_when_non_empty() {
        let base = normalize_base_url("http://127.0.0.1:8080").unwrap();
        let u = build_handoff_url(base, "a/b");
        assert_eq!(u.fragment().unwrap(), "cm_web_api_bearer=a%2Fb");
    }

    #[test]
    fn handoff_omits_fragment_when_bearer_empty() {
        let base = normalize_base_url("http://127.0.0.1:8080").unwrap();
        let u = build_handoff_url(base, "  ");
        assert!(u.fragment().is_none());
    }

    #[test]
    fn local_ui_handoff_includes_api_base_and_bearer() {
        let home = Url::parse("http://tauri.localhost/connect.html").unwrap();
        let api = normalize_base_url("http://127.0.0.1:8080").unwrap();
        let u = build_local_ui_handoff_url(local_business_ui_url(&home), &api, "tok/en");
        assert_eq!(u.path(), "/index.html");
        let frag = u.fragment().unwrap();
        assert!(frag.contains("cm_api_base=http%3A%2F%2F127.0.0.1%3A8080"));
        assert!(frag.contains("cm_web_api_bearer=tok%2Fen"));
    }

    #[test]
    fn local_ui_handoff_omits_empty_bearer() {
        let home = Url::parse("http://tauri.localhost/connect.html").unwrap();
        let api = normalize_base_url("http://127.0.0.1:8080").unwrap();
        let u = build_local_ui_handoff_url(local_business_ui_url(&home), &api, "  ");
        let frag = u.fragment().unwrap();
        assert!(frag.starts_with("cm_api_base="));
        assert!(!frag.contains("cm_web_api_bearer"));
    }
}
