//! WebView 导航白名单（壳层共用决策；桌面另有外开行为，移动端默认拦截）。

use tauri::{AppHandle, Manager, Runtime, Webview};
use url::Url;

use crate::allowed_origin::AllowedServeOrigin;

/// 是否为壳内连接页 / App 资产 Origin（按 scheme + host，禁止路径子串误判）。
#[must_use]
pub fn is_app_origin(url: &Url) -> bool {
    let host = url.host_str().unwrap_or("");
    matches!(url.scheme(), "tauri" | "asset")
        || host.eq_ignore_ascii_case("tauri.localhost")
        || (host.eq_ignore_ascii_case("localhost") && url.path().contains("connect"))
}

/// 纯决策（便于单测）：是否允许导航，以及是否应清空 [`AllowedServeOrigin`]。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellNavigationDecision {
    /// 放行。
    Allow,
    /// 放行并清空已允许的 serve Origin（回到连接页）。
    AllowClearServeOrigin,
    /// 拒绝（留在当前页）。
    Deny,
}

/// 根据目标 URL、当前页与是否命中已探测 Origin 做决策。
#[must_use]
pub fn decide_shell_navigation(
    url: &Url,
    current: Option<&Url>,
    allowed_matches: bool,
) -> ShellNavigationDecision {
    match url.scheme() {
        "tauri" | "asset" => {
            if is_app_origin(url) {
                ShellNavigationDecision::AllowClearServeOrigin
            } else {
                ShellNavigationDecision::Allow
            }
        }
        "http" | "https" => {
            if is_app_origin(url) {
                return ShellNavigationDecision::AllowClearServeOrigin;
            }
            if allowed_matches {
                return ShellNavigationDecision::Allow;
            }
            if let Some(cur) = current
                && matches!(cur.scheme(), "http" | "https")
                && cur.origin() == url.origin()
            {
                return ShellNavigationDecision::Allow;
            }
            ShellNavigationDecision::Deny
        }
        _ => ShellNavigationDecision::Deny,
    }
}

fn clear_allowed_serve_origin<R: Runtime>(app: &AppHandle<R>) {
    if let Some(allowed) = app.try_state::<AllowedServeOrigin>() {
        allowed.clear();
    }
}

/// 壳 WebView 导航钩子：仅放行 App Origin、已探测的 serve Origin，或同 Origin 内跳转。
#[must_use]
pub fn allow_shell_navigation<R: Runtime>(webview: &Webview<R>, url: &Url) -> bool {
    let app = webview.app_handle();
    let allowed_matches = app
        .try_state::<AllowedServeOrigin>()
        .is_some_and(|s| s.matches_url(url));
    let current = webview.url().ok();
    let decision = decide_shell_navigation(url, current.as_ref(), allowed_matches);
    match decision {
        ShellNavigationDecision::Allow => true,
        ShellNavigationDecision::AllowClearServeOrigin => {
            clear_allowed_serve_origin(app);
            true
        }
        ShellNavigationDecision::Deny => false,
    }
}

/// 页面已落到连接页 / App Origin 时清空白名单。
///
/// Android 侧 `WebView.loadUrl` 有时不走 `on_navigation`；断开回连接页须在 page load 再清一次，
/// 避免旧 serve Origin 仍被放行。
pub fn clear_allowed_if_app_origin_loaded<R: Runtime>(app: &AppHandle<R>, url: &Url) {
    if is_app_origin(url) {
        clear_allowed_serve_origin(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_origin_requires_trusted_host() {
        let home = Url::parse("http://tauri.localhost/").unwrap();
        assert!(is_app_origin(&home));
        let connect = Url::parse("http://tauri.localhost/?manual=1").unwrap();
        assert!(is_app_origin(&connect));
        let remote = Url::parse("http://192.168.1.10:8080/").unwrap();
        assert!(!is_app_origin(&remote));
        // 路径含 connect.html 但 host 不可信 → 不得当作壳 Origin
        let spoof = Url::parse("http://evil.example/connect.html").unwrap();
        assert!(!is_app_origin(&spoof));
        let query_spoof = Url::parse("http://evil.example/?x=http://tauri.localhost/").unwrap();
        assert!(!is_app_origin(&query_spoof));
    }

    #[test]
    fn deny_cross_origin_without_allowlist() {
        let cur = Url::parse("http://192.168.1.10:8080/chat").unwrap();
        let evil = Url::parse("http://evil.example/").unwrap();
        assert_eq!(
            decide_shell_navigation(&evil, Some(&cur), false),
            ShellNavigationDecision::Deny
        );
    }

    #[test]
    fn allow_same_origin_and_allowlisted() {
        let cur = Url::parse("http://192.168.1.10:8080/chat").unwrap();
        let next = Url::parse("http://192.168.1.10:8080/settings").unwrap();
        assert_eq!(
            decide_shell_navigation(&next, Some(&cur), false),
            ShellNavigationDecision::Allow
        );
        let other = Url::parse("http://10.0.0.2:8080/").unwrap();
        assert_eq!(
            decide_shell_navigation(&other, Some(&cur), true),
            ShellNavigationDecision::Allow
        );
    }

    #[test]
    fn return_to_connect_clears_allowlist_flag() {
        let serve = Url::parse("http://192.168.1.10:8080/").unwrap();
        let home = Url::parse("http://tauri.localhost/?manual=1").unwrap();
        assert_eq!(
            decide_shell_navigation(&home, Some(&serve), true),
            ShellNavigationDecision::AllowClearServeOrigin
        );
    }
}
