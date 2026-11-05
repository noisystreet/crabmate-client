//! 连接页资产 URL 记忆与建议服务器地址（无 Tauri）。

use std::sync::Mutex;

use url::Url;

use crate::navigation::is_app_origin;

/// 包内资产入口的默认值（Windows / Android）。
///
/// **Origin 随平台不同，而它决定「窗口能不能显示」**：`http(s)://tauri.localhost` 只在这两个
/// 平台成立 —— wry 的 `custom_protocol_workaround` 才给它注册 http(s) 处理器；Linux / macOS
/// 的包内页就是 `tauri://localhost`（tauri `Manager::tauri_protocol_url` 按同一条件分支）。
/// 写成 http 形式后导航到一个没有处理器的 host，load 在**异步**阶段失败，窗口里只剩底色 ——
/// 看到的就是「开了个空窗口」，而 `navigate()` 早已返回 Ok。
#[cfg(any(windows, target_os = "android"))]
const DEFAULT_CONNECT_HOME: &str = "http://tauri.localhost/connect.html";
/// 见上一条（Linux / macOS）。
#[cfg(not(any(windows, target_os = "android")))]
const DEFAULT_CONNECT_HOME: &str = "tauri://localhost/connect.html";

static CONNECT_HOME: Mutex<Option<Url>> = Mutex::new(None);

/// 连接页预填的建议服务器 URL；桌面默认本机 `8080`，移动端保持 `None` 直至用户填写。
#[derive(Debug, Default)]
pub struct SuggestedServerUrl(pub Mutex<Option<String>>);

impl SuggestedServerUrl {
    pub fn new(url: Option<String>) -> Self {
        Self(Mutex::new(url))
    }

    pub fn set(&self, url: Option<String>) {
        if let Ok(mut g) = self.0.lock() {
            *g = url;
        }
    }
}

pub(crate) fn remember_connect_home(url: &Url) {
    if !is_app_origin(url) {
        return;
    }
    let mut home = url.clone();
    home.set_fragment(None);
    // 保留路径（桌面 / 移动均为 /connect.html）；disconnect 时再设 manual query。
    if let Ok(mut g) = CONNECT_HOME.lock() {
        *g = Some(home);
    }
}

/// 包内资产入口（`/connect.html`）的平台默认值 —— 还没 [`seed_connect_home`] 过时的回落。
///
/// 要**导航**到包内页、或据它拼业务 UI 地址时用这个函数，**不要手写 Origin 字面量**：
/// 手写的那份一定在某平台上少一个处理器（见 `DEFAULT_CONNECT_HOME` 的说明）。
#[must_use]
pub fn default_connect_home() -> Url {
    Url::parse(DEFAULT_CONNECT_HOME).expect("DEFAULT_CONNECT_HOME is valid")
}

/// 壳自己记住的资产 URL（[`seed_connect_home`] 写入）；没记住时回落到
/// [`default_connect_home`]。
///
/// 业务 UI 的交接地址由它派生 —— `connect_remote`（连接页 → UI）与 Desktop 的建窗路径
/// （左栏「连接」）共用这一处，于是「导航目标」永远与「当前页所在 Origin」同源，
/// 不会各写一份字面量然后各自漂移。
#[cfg_attr(not(feature = "tauri"), allow(dead_code))]
pub fn connect_home_url() -> Url {
    CONNECT_HOME
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or_else(default_connect_home)
}

/// 在打开连接页后调用，确保断开时能回到正确的 App 资产 URL（`/connect.html`）。
pub fn seed_connect_home(url: &Url) {
    remember_connect_home(url);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_connect_home_only_keeps_app_origin() {
        let remote = Url::parse("http://192.168.1.10:8080/").unwrap();
        seed_connect_home(&remote);
        assert_eq!(connect_home_url(), default_connect_home());

        let u = Url::parse("http://tauri.localhost/connect.html?manual=1#frag").unwrap();
        seed_connect_home(&u);
        let home = connect_home_url();
        assert_eq!(home.path(), "/connect.html");
        assert!(home.fragment().is_none());

        seed_connect_home(&remote);
        assert_eq!(connect_home_url().path(), "/connect.html");
    }

    #[test]
    fn the_default_home_origin_matches_the_platform_asset_url() {
        // 这一条守的是「窗口能不能显示」：Linux / macOS 的包内页是 `tauri://localhost`，
        // 写成 `http://tauri.localhost` 就导航到一个没有处理器的 host —— load 在异步阶段
        // 失败，`navigate()` 却已经返回 Ok，界面上就是一个空窗口。
        let home = default_connect_home();
        if cfg!(any(windows, target_os = "android")) {
            assert_eq!(home.scheme(), "http");
            assert_eq!(home.host_str(), Some("tauri.localhost"));
        } else {
            assert_eq!(home.scheme(), "tauri");
            assert_eq!(home.host_str(), Some("localhost"));
        }
        assert_eq!(home.path(), "/connect.html");

        // 业务 UI 是同 Origin 的 `/index.html`：scheme + host 必须跟着走，否则交接地址
        // 又回到「导航到一个不存在的 host」。
        let ui = crate::handoff::local_business_ui_url(&home);
        assert_eq!(ui.scheme(), home.scheme());
        assert_eq!(ui.host_str(), home.host_str());
        assert_eq!(ui.path(), "/index.html");
    }
}
