//! WebView 导航白名单（壳层共用决策；桌面另有外开行为，移动端默认拦截）。

use url::Url;

#[cfg(feature = "tauri")]
use tauri::{AppHandle, Manager, Runtime, Webview};

#[cfg(feature = "tauri")]
use crate::allowed_origin::AllowedServeOrigin;

/// 是否为壳内连接页 / App 资产 Origin（按 scheme + host，禁止路径子串误判）。
#[must_use]
pub fn is_app_origin(url: &Url) -> bool {
    let host = url.host_str().unwrap_or("");
    matches!(url.scheme(), "tauri" | "asset")
        || host.eq_ignore_ascii_case("tauri.localhost")
        || (host.eq_ignore_ascii_case("localhost") && url.path().contains("connect"))
}

/// 是否为壳内**连接页**（非业务 UI）。业务 UI 为同 Origin 的 `/index.html`。
#[must_use]
pub fn is_connect_page_url(url: &Url) -> bool {
    if !is_app_origin(url) {
        return false;
    }
    let path = url.path();
    path.ends_with("connect.html")
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
            if is_connect_page_url(url) {
                ShellNavigationDecision::AllowClearServeOrigin
            } else {
                ShellNavigationDecision::Allow
            }
        }
        "http" | "https" => {
            if is_connect_page_url(url) {
                return ShellNavigationDecision::AllowClearServeOrigin;
            }
            if is_app_origin(url) {
                // 包内业务 UI（index.html 等）：放行且**不清** allowlist
                return ShellNavigationDecision::Allow;
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

#[cfg(feature = "tauri")]
fn clear_allowed_serve_origin<R: Runtime>(app: &AppHandle<R>) {
    if let Some(allowed) = app.try_state::<AllowedServeOrigin>() {
        allowed.clear();
    }
}

/// 壳 WebView 导航钩子：仅放行 App Origin、已探测的 serve Origin。
///
/// **禁止**在此调用 [`Webview::url`]：Android 上 `url()` 经 MainPipe 同步 `GetUrl`，
/// 而 `on_navigation` 已在 MainPipe looper 回调内执行；再发 `GetUrl` 会超时，随后
/// `tx.send().unwrap()` panic → `abort_on_panic` 闪退（wryCreate / 导航路径）。
/// 同 Origin 内跳转依赖连接成功后的 [`AllowedServeOrigin`]（与目标 Origin 匹配即可）。
#[cfg(feature = "tauri")]
#[must_use]
pub fn allow_shell_navigation<R: Runtime>(webview: &Webview<R>, url: &Url) -> bool {
    let app = webview.app_handle();
    let allowed_matches = app
        .try_state::<AllowedServeOrigin>()
        .is_some_and(|s| s.matches_url(url));
    let decision = decide_shell_navigation(url, None, allowed_matches);
    match decision {
        ShellNavigationDecision::Allow => true,
        ShellNavigationDecision::AllowClearServeOrigin => {
            clear_allowed_serve_origin(app);
            true
        }
        ShellNavigationDecision::Deny => false,
    }
}

/// 页面已落到连接页时清空白名单。
///
/// Android 侧 `WebView.loadUrl` 有时不走 `on_navigation`；断开回连接页须在 page load 再清一次，
/// 避免旧 serve Origin 仍被放行。加载包内业务 UI **不会**清空。
#[cfg(feature = "tauri")]
pub fn clear_allowed_if_app_origin_loaded<R: Runtime>(app: &AppHandle<R>, url: &Url) {
    if is_connect_page_url(url) {
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
        let connect = Url::parse("http://tauri.localhost/connect.html?manual=1").unwrap();
        assert!(is_app_origin(&connect));
        assert!(is_connect_page_url(&connect));
        let ui = Url::parse("http://tauri.localhost/index.html").unwrap();
        assert!(is_app_origin(&ui));
        assert!(!is_connect_page_url(&ui));
        let remote = Url::parse("http://192.168.1.10:8080/").unwrap();
        assert!(!is_app_origin(&remote));
        let spoof = Url::parse("http://evil.example/connect.html").unwrap();
        assert!(!is_app_origin(&spoof));
        assert!(!is_connect_page_url(&spoof));
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
        assert_eq!(
            decide_shell_navigation(&next, None, true),
            ShellNavigationDecision::Allow
        );
    }

    #[test]
    fn return_to_connect_clears_allowlist_flag() {
        let serve = Url::parse("http://192.168.1.10:8080/").unwrap();
        let home = Url::parse("http://tauri.localhost/connect.html?manual=1").unwrap();
        assert_eq!(
            decide_shell_navigation(&home, Some(&serve), true),
            ShellNavigationDecision::AllowClearServeOrigin
        );
    }

    #[test]
    fn local_ui_does_not_clear_allowlist() {
        let serve = Url::parse("http://192.168.1.10:8080/").unwrap();
        let ui = Url::parse("http://tauri.localhost/index.html").unwrap();
        assert_eq!(
            decide_shell_navigation(&ui, Some(&serve), true),
            ShellNavigationDecision::Allow
        );
    }
}
