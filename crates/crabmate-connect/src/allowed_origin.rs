//! 连接成功后允许留在 WebView 内的 `serve` Origin（防止连接页任意 http 导航）。
//!
//! **按窗口 label 持有**，不是一个进程级单值。Desktop 要多窗口各连一台 serve：
//! 单值会被最后连接的那个覆盖，先连的窗口在那条兜底路径上就失去放行依据；而「全清」
//! 又会让一个窗口的断开动作清掉别的窗口的白名单。两个方向都错，所以按 label 分开存。
//!
//! 按 label 存的另一个好处是能**精确回收**：窗口关掉时 `remove` 那一条，不必留一个
//! 只增不减的 origin 集合 —— 安全边界不该随使用时间变宽。
//!
//! 注意这不是 Desktop 的主判定：`desktop_main_window.rs` 的导航钩子捕获了每窗一份的
//! `backend_origin`，同源判定走它；这里只是「最近一次成功探测」的兜底。

use std::collections::HashMap;
use std::sync::Mutex;

use url::{Origin, Url};

/// 窗口 label → 该窗口最近一次 `connect_remote` 探测成功的 `serve` Origin。
#[derive(Debug, Default)]
pub struct AllowedServeOrigin(Mutex<HashMap<String, Origin>>);

impl AllowedServeOrigin {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 记录 `label` 这个窗口连上了 `url` 的 Origin。
    pub fn set_from_url(&self, label: &str, url: &Url) {
        if let Ok(mut g) = self.0.lock() {
            g.insert(label.to_string(), url.origin());
        }
    }

    /// `label` 这个窗口是否允许导航到 `url`（同 Origin）。
    ///
    /// 只认**自己**那条：别的窗口连过什么与本窗口无关。这正是按 label 分开的意义 ——
    /// 单值时代这里会给「别人连过、我没连过」的 origin 放行。
    #[must_use]
    pub fn matches_url(&self, label: &str, url: &Url) -> bool {
        self.0
            .lock()
            .ok()
            .and_then(|g| g.get(label).cloned())
            .is_some_and(|o| o == url.origin())
    }

    /// 摘掉单个窗口的白名单（断开连接 / 窗口关闭）。
    pub fn clear_label(&self, label: &str) {
        if let Ok(mut g) = self.0.lock() {
            g.remove(label);
        }
    }

    /// 当前挂了几条（诊断用；`#[cfg(test)]` 也用它断言回收确实发生）。
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.lock().map(|g| g.len()).unwrap_or(0)
    }

    /// 一条都没挂。
    ///
    /// 与 [`Self::len`] 成对存在：只给 `len` 不给 `is_empty` 会触发
    /// `clippy::len_without_is_empty`，而这两者本来就该一起提供。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(raw: &str) -> Url {
        Url::parse(raw).expect("test url")
    }

    #[test]
    fn each_window_only_trusts_its_own_origin() {
        let allowed = AllowedServeOrigin::new();
        allowed.set_from_url("win-a", &url("http://127.0.0.1:8080/"));
        allowed.set_from_url("win-b", &url("http://10.0.0.5:9000/"));

        // 各自认自己的。
        assert!(allowed.matches_url("win-a", &url("http://127.0.0.1:8080/x")));
        assert!(allowed.matches_url("win-b", &url("http://10.0.0.5:9000/")));
        // 交叉不认 —— 这正是单值时代做不到的（后连的那个会盖掉先连的）。
        assert!(!allowed.matches_url("win-a", &url("http://10.0.0.5:9000/")));
        assert!(!allowed.matches_url("win-b", &url("http://127.0.0.1:8080/")));
    }

    #[test]
    fn clearing_one_window_leaves_the_others_alone() {
        let allowed = AllowedServeOrigin::new();
        allowed.set_from_url("win-a", &url("http://127.0.0.1:8080/"));
        allowed.set_from_url("win-b", &url("http://10.0.0.5:9000/"));
        assert_eq!(allowed.len(), 2);

        allowed.clear_label("win-a");
        assert_eq!(allowed.len(), 1);
        assert!(!allowed.matches_url("win-a", &url("http://127.0.0.1:8080/")));
        assert!(allowed.matches_url("win-b", &url("http://10.0.0.5:9000/")));
    }

    #[test]
    fn an_unknown_label_is_denied_not_trusted() {
        let allowed = AllowedServeOrigin::new();
        // 没有任何窗口登记过时不能放行（默认拒绝）。
        assert!(!allowed.matches_url("win-a", &url("http://127.0.0.1:8080/")));
        // 摘掉一个不存在的 label 也不该 panic。
        allowed.clear_label("nope");
        assert_eq!(allowed.len(), 0);
    }
}
