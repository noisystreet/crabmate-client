//! 连接作用域的**同一台 serve 判定**（纯逻辑，无 IO）。
//!
//! TUI 与 Web 都需要回答同一个问题：**「这条要连的地址，是不是已经连着的那台？」** ——
//! 重复连会让同一台 serve 各开一条 SSE、各读一份会话，左栏连接数还会与 serve 侧的实例数
//! 对不上。两个客户端都必须用**同一份判据**，否则同一台 serve 会在 TUI 里判「同一条」、
//! 在 Web 里判「两条」。
//!
//! 判据只有一条：**规范化之后相等**。`http://h:8081/` 与 `http://h:8081` 是同一台；
//! `localhost` 与 `127.0.0.1` **不是**（那是两台不同的监听，解析到同一地址也不构成证据）。
//!
//! 空串 / 非绝对 `http(s)` 一律返回 `false`：那是「还不知道是哪台」，不能当作「同一台」
//! 的证据 —— 否则漏写 `http://` 的地址会去重到随便一条连接上，症状是「连上去没反应」。

use crate::url::normalize_api_base;

/// 两个 `api_base` 是否指向同一台 `serve`。
#[must_use]
pub fn same_api_base(a: &str, b: &str) -> bool {
    match (api_base_key(a), api_base_key(b)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

/// 该地址能否作为一个**可用的** `api_base`（绝对 `http(s)`）。
///
/// 与 [`same_api_base`] 共用同一把尺子（[`api_base_key`]）—— 这两件事必须同源，否则会出现
/// 「`same_api_base` 判它是同一台、地址校验却判它非法」的自相矛盾（`HTTP://h:1` 就是这种输入：
/// scheme 大小写在 URL 规范里不区分，去重认它，校验也应当认）。
#[must_use]
pub fn is_absolute_api_base(url: &str) -> bool {
    api_base_key(url).is_some()
}

/// 归一化比较键；非绝对 `http(s)` 或空串返回 `None`。
///
/// 公开是给「按 `api_base` 查表」这类调用方用的（TUI 的 ctl 实例查找就是）：它们要的是
/// 键本身而不是布尔结果。与 [`same_api_base`] / [`is_absolute_api_base`] 同一份算法 ——
/// 三处若各写一份，判据就会分叉。
///
/// 先整体转小写再交给 [`normalize_api_base`]：后者用 `starts_with("http://")` 判绝对 URL，
/// 大写的 `HTTP://…` 会被它当成非法并返回空串 —— 而 scheme / host 本就不区分大小写。
/// `api_base` 不携带路径，整体小写不改变语义。
#[must_use]
pub fn api_base_key(url: &str) -> Option<String> {
    let normalized = normalize_api_base(&url.trim().to_ascii_lowercase());
    (!normalized.is_empty()).then_some(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosmetic_differences_are_the_same_serve() {
        // 尾斜杠与大小写不参与比较：它们是同一条 serve 的不同写法。
        assert!(same_api_base(
            "http://127.0.0.1:8081/",
            "http://127.0.0.1:8081"
        ));
        assert!(same_api_base(
            "HTTP://127.0.0.1:8081",
            "http://127.0.0.1:8081"
        ));
        assert!(same_api_base("  http://h:1  ", "http://h:1"));
    }

    #[test]
    fn localhost_and_loopback_ip_are_not_merged() {
        // 不同监听就是不同连接：解析到同一地址不构成「同一台」的证据。
        assert!(!same_api_base(
            "http://localhost:8081",
            "http://127.0.0.1:8081"
        ));
    }

    #[test]
    fn different_ports_or_hosts_are_not_the_same() {
        assert!(!same_api_base(
            "http://127.0.0.1:8081",
            "http://127.0.0.1:8082"
        ));
        assert!(!same_api_base("http://a:1", "https://a:1"));
    }

    #[test]
    fn an_unusable_base_is_never_evidence() {
        // 空串 / 漏 scheme：不知道是哪台，不能判「同一台」。
        for bad in ["", "   ", "127.0.0.1:8081", "ws://a:1"] {
            assert!(!same_api_base(bad, "http://127.0.0.1:8081"), "{bad:?}");
            assert!(!same_api_base("http://127.0.0.1:8081", bad), "{bad:?}");
            assert!(!same_api_base(bad, bad), "{bad:?}");
        }
    }
}
