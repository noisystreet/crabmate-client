//! 连接页资产 URL 记忆与建议服务器地址（无 Tauri）。

use std::sync::Mutex;

use url::Url;

use crate::navigation::is_app_origin;

/// Android 默认资产源（`useHttpsScheme=false`）；桌面 Tauri 2 亦常用此 origin。
#[cfg_attr(not(feature = "tauri"), allow(dead_code))]
const DEFAULT_CONNECT_HOME: &str = "http://tauri.localhost/connect.html";

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

#[cfg_attr(not(feature = "tauri"), allow(dead_code))]
pub(crate) fn connect_home_url() -> Url {
    CONNECT_HOME
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or_else(|| Url::parse(DEFAULT_CONNECT_HOME).expect("DEFAULT_CONNECT_HOME is valid"))
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
        assert_eq!(connect_home_url().as_str(), DEFAULT_CONNECT_HOME);

        let u = Url::parse("http://tauri.localhost/connect.html?manual=1#frag").unwrap();
        seed_connect_home(&u);
        let home = connect_home_url();
        assert_eq!(home.path(), "/connect.html");
        assert!(home.fragment().is_none());

        seed_connect_home(&remote);
        assert_eq!(connect_home_url().path(), "/connect.html");
    }
}
