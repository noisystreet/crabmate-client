//! 「这台 `serve` 已经有一个窗口连着」的探测口（由桌面壳注册）。
//!
//! `connect_remote` 住在 `crabmate-connect`，连接注册表住在 `desktop-tauri`，依赖方向是
//! desktop → connect，所以这里不能引用那个类型。反过来传一个闭包：壳在 setup 时 `manage`
//! 一份，连接命令取到就查；取不到（浏览器 / Android / 老壳）就当「没有冲突」——
//! 单连接平台本来就没有第二条可以撞。
//!
//! **为什么要在连接时拦，而不是在开窗时拦**：设置页的「新建连接窗口」不带地址
//! （B 案：连哪台由用户在新窗口里选），所以开窗那一刻没有任何基址可以比对。真正知道
//! 「要连哪台」的时机只有 `connect_remote` 探测成功之后。
//!
//! **口径对齐 TUI 的 `/lane attach`**：同一个 `base_url` 上换一个名字只切过去、不新增
//! lane（`tui_multi_server.md`）。Desktop 的等价物就是「聚焦已有窗口，本窗口留在连接页」。

use std::sync::Arc;

/// 查「除 `label` 之外，是否已有窗口连着 `authority`」，命中返回那个窗口的 label。
///
/// 参数是 authority（`host:port`）而不是完整基址：两个窗口用 `http://h:8080` 与
/// `http://h:8080/` 连同一台 serve 是常事，尾斜杠与大小写都该算同一个。
pub type SameServeLookup = dyn Fn(&str, &str) -> Option<String> + Send + Sync;

/// 壳提供的占用探测。没注册 = 平台不支持多连接，永远「不冲突」。
#[derive(Clone)]
pub struct ExistingConnectionProbe(Arc<SameServeLookup>);

impl ExistingConnectionProbe {
    #[must_use]
    pub fn new(lookup: impl Fn(&str, &str) -> Option<String> + Send + Sync + 'static) -> Self {
        Self(Arc::new(lookup))
    }

    /// 直接转发给壳的回调。**排除 `label` 自己由回调负责** —— 只有注册表知道完整集合，
    /// 在这一层再过滤一次会让「谁是调用方」出现两个判据。
    #[must_use]
    pub fn same_serve_window(&self, label: &str, authority: &str) -> Option<String> {
        (self.0)(label, authority)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delegates_and_skips_itself() {
        let probe = ExistingConnectionProbe::new(|label, authority| {
            (authority == "127.0.0.1:8080" && label != "other").then(|| "taken".to_string())
        });
        assert_eq!(
            probe.same_serve_window("new-window", "127.0.0.1:8080"),
            Some("taken".to_string())
        );
        // 自己那一台：调用方（壳的回调）负责排除，本层只转发。
        assert_eq!(probe.same_serve_window("other", "127.0.0.1:8080"), None);
        assert_eq!(
            probe.same_serve_window("new-window", "127.0.0.1:9090"),
            None
        );
    }
}
