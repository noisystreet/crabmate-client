//! 壳连接页 → 包内业务 UI 的一次性交接（URL hash）。
//!
//! 连接成功后导航到本地 `{app}/index.html#cm_api_base=…&cm_web_api_bearer=…`；
//! 本模块在前端启动时消费并写入 [`super::browser::set_api_base_url`] /
//! [`super::browser::set_web_api_bearer_token`]，再从地址栏清除敏感参数
//!（`history.replaceState`）。
//!
//! **勿**把 Bearer 写入查询串（易进访问日志）；hash 仍可能进 WebView 历史，仅作短时手递。

use super::browser::{set_api_base_url, set_web_api_bearer_token, window};

/// Hash 参数名（与 `crabmate-connect` 一致）。
pub const CM_WEB_API_BEARER_HASH_KEY: &str = "cm_web_api_bearer";

/// API 基址 hash 键（指向远程 `serve`）。
pub const CM_API_BASE_HASH_KEY: &str = "cm_api_base";

/// 从 `location.hash`（`#a=1&b=2`）解析键值；值已做 URL 解码。
#[must_use]
pub fn parse_hash_params(hash: &str) -> Vec<(String, String)> {
    let raw = hash.trim().trim_start_matches('#');
    if raw.is_empty() {
        return Vec::new();
    }
    raw.split('&')
        .filter_map(|pair| {
            let pair = pair.trim();
            if pair.is_empty() {
                return None;
            }
            let (k, v) = match pair.split_once('=') {
                Some((k, v)) => (k, v),
                None => (pair, ""),
            };
            let k = urlencoding::decode(k)
                .unwrap_or_else(|_| k.into())
                .into_owned();
            let v = urlencoding::decode(v)
                .unwrap_or_else(|_| v.into())
                .into_owned();
            if k.is_empty() { None } else { Some((k, v)) }
        })
        .collect()
}

fn is_handoff_key(k: &str) -> bool {
    k == CM_WEB_API_BEARER_HASH_KEY || k == CM_API_BASE_HASH_KEY
}

/// 去掉交接敏感键后重建 hash（无参数时返回空串，不含 `#`）。
#[must_use]
pub fn hash_without_handoff_keys(hash: &str) -> String {
    let kept: Vec<(String, String)> = parse_hash_params(hash)
        .into_iter()
        .filter(|(k, _)| !is_handoff_key(k))
        .collect();
    if kept.is_empty() {
        return String::new();
    }
    kept.into_iter()
        .map(|(k, v)| {
            if v.is_empty() {
                urlencoding::encode(&k).into_owned()
            } else {
                format!("{}={}", urlencoding::encode(&k), urlencoding::encode(&v))
            }
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// 若 hash 含 API 基址和/或 Bearer：写入本页凭证并 `replaceState` 清掉地址栏中的敏感参数。
///
/// 在 WASM App 启动尽早调用（先于依赖鉴权的 `/status` 拉取）。
pub fn consume_mobile_connect_handoff() {
    let Some(w) = window() else {
        return;
    };
    let loc = w.location();
    let Ok(hash) = loc.hash() else {
        return;
    };
    let params = parse_hash_params(&hash);
    let mut consumed = false;

    if let Some((_, api_base)) = params.iter().find(|(k, _)| k == CM_API_BASE_HASH_KEY) {
        set_api_base_url(api_base);
        consumed = true;
    }

    if let Some((_, bearer)) = params.iter().find(|(k, _)| k == CM_WEB_API_BEARER_HASH_KEY) {
        // 空串也消费：用于明确「无 Bearer」并清掉残留 hash 键
        set_web_api_bearer_token(bearer);
        consumed = true;
    }

    if !consumed {
        return;
    }

    let new_hash = hash_without_handoff_keys(&hash);
    let Ok(path) = loc.pathname() else {
        return;
    };
    let search = loc.search().unwrap_or_default();
    let next = if new_hash.is_empty() {
        format!("{path}{search}")
    } else {
        format!("{path}{search}#{new_hash}")
    };
    if let Ok(hist) = w.history() {
        let _ = hist.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&next));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bearer_and_strip() {
        let h = "#cm_web_api_bearer=sec%2Fret&cm_shell=mobile";
        let p = parse_hash_params(h);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].0, CM_WEB_API_BEARER_HASH_KEY);
        assert_eq!(p[0].1, "sec/ret");
        assert_eq!(hash_without_handoff_keys(h), "cm_shell=mobile");
    }

    #[test]
    fn parse_api_base_and_strip() {
        let h = "#cm_api_base=http%3A%2F%2F127.0.0.1%3A8080&cm_web_api_bearer=tok";
        let p = parse_hash_params(h);
        assert_eq!(p[0].0, CM_API_BASE_HASH_KEY);
        assert_eq!(p[0].1, "http://127.0.0.1:8080");
        assert_eq!(hash_without_handoff_keys(h), "");
    }

    #[test]
    fn empty_hash() {
        assert!(parse_hash_params("").is_empty());
        assert!(parse_hash_params("#").is_empty());
        assert_eq!(hash_without_handoff_keys("#cm_web_api_bearer=abc"), "");
    }
}
