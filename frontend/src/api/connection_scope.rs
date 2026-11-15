//! 本页连接作用域的**唯一所有者**（#457 W0-1）：`api_base` + Web API Bearer 只存一份。
//!
//! 改动前它们是 `browser.rs` 里四个平铺的 `thread_local`（两个值 + 两个「已水合」标记），
//! 谁都能读、写点分散在 handoff / 设置页 / 钥匙串水合三处。现在值收敛成一个
//! [`ConnectionScope`]（规则与不变量在 `crabmate-client-api`），本模块只负责持有与
//! 更替，写入只经 [`set_api_base`] / [`set_bearer`]。
//!
//! # 这**没有**解除「单窗口多 lane」的拦路条件
//!
//! 说清楚，免得后来者以为 S4 已经完成：本页仍然只有**一个** scope 实例，`api_base_url()` /
//! `web_api_bearer_token()` 仍是「读当前那一份」的零参函数。多 lane 需要「每 lane 一份 scope，
//! 且调用点知道用哪一份」—— 那要求 `api_url()` / `prepare_api_auth()` 这一层的调用点显式传
//! scope，实测面是 `api_url` 21 处（12 个文件）+ `prepare_api_auth` / `auth_headers` 11 个文件。
//! 这笔账写在 #457 的进度评论里，属于 W1，不塞进本步。
//!
//! 本步兑现的是 issue 期望能力第 1 条：「这屏连的是哪台 serve、用什么 Bearer」在代码里只有
//! **一处**存储，且带类型与不变量（规范化基址、trim Bearer、`Debug` 不打印凭证）。
//!
//! # 为什么不放进组件 ctx
//!
//! 读这些值的绝大多数调用点在**非组件路径**上（`api/*.rs` 里的 async fetch 函数），拿不到
//! Leptos 的 `expect_context`。把值搬进 ctx 会让这 40+ 处全部改签名 —— 那是 W1 的事，且要
//! 和「每 lane 一棵组件子树」一起做才有意义。

use std::cell::{Cell, RefCell};

use crabmate_client_api::ConnectionKey;
use crabmate_client_api::connection_scope::ConnectionScope;

thread_local! {
    /// 唯一实例。`None` = 还没建（第一次读时按空值建，具体值由水合 / setter 填）。
    static SCOPE: RefCell<Option<ConnectionScope>> = const { RefCell::new(None) };
    /// 基址是否已经决定过一次（决定过就不再读 `localStorage`，避免每次读都打盘）。
    static BASE_SETTLED: Cell<bool> = const { Cell::new(false) };
    /// Bearer 是否已经水合过一次。
    static BEARER_SETTLED: Cell<bool> = const { Cell::new(false) };
}

/// 本页的连接标识。
///
/// 无标识（纯浏览器 / Android / 主窗口）时给一个稳定占位：它只影响展示与日志，
/// 存储是否分域由 [`super::storage_scope`] 单独决定 —— 两件事不共用开关，
/// 否则「跑在浏览器里」就会被迫与「键不分域」绑死。
fn identity() -> ConnectionKey {
    let raw = super::storage_scope::connection_key().unwrap_or_default();
    ConnectionKey::from_raw(&raw)
}

fn empty_scope() -> ConnectionScope {
    ConnectionScope::new(identity(), "", "")
}

/// 读当前 scope（第一次访问时建立空实例）。
pub(crate) fn with_scope<R>(f: impl FnOnce(&ConnectionScope) -> R) -> R {
    SCOPE.with(|cell| {
        if cell.borrow().is_none() {
            *cell.borrow_mut() = Some(empty_scope());
        }
        let guard = cell.borrow();
        f(guard.as_ref().expect("上面刚建好"))
    })
}

/// 用 `f` 产生下一版 scope（值语义替换，不做原地改字段）。
fn update(f: impl FnOnce(&ConnectionScope) -> ConnectionScope) {
    SCOPE.with(|cell| {
        // `take()` 而不是嵌套 borrow：`ConnectionScope` 只有三个字段，克隆比
        // 「borrow 一个字段改另一个字段」的借用体操更不容易写错。
        let current = cell.borrow_mut().take().unwrap_or_else(empty_scope);
        *cell.borrow_mut() = Some(f(&current));
    });
}

/// 当前基址（空 = 同 Origin）。**不**触发任何磁盘读取。
#[must_use]
pub(crate) fn api_base() -> String {
    with_scope(|s| s.api_base().to_string())
}

/// 当前 Bearer（空 = 没有）。**不**触发任何水合。
#[must_use]
pub(crate) fn bearer() -> String {
    with_scope(|s| s.bearer().to_string())
}

/// 换基址：值 + 标记一起更新，调用方不必记得「改完还要置 settled」。
pub(crate) fn set_api_base(raw: &str) {
    update(|s| s.with_api_base(raw));
    BASE_SETTLED.with(|c| c.set(true));
}

/// 换 Bearer（`memory_only` 与「从钥匙串水合」共用这一条路径）。
pub(crate) fn set_bearer(raw: &str) {
    update(|s| s.with_bearer(raw));
    BEARER_SETTLED.with(|c| c.set(true));
}

#[must_use]
pub(crate) fn base_settled() -> bool {
    BASE_SETTLED.with(Cell::get)
}

/// 只置标记、不动值：水合路径要「先声明读过盘，再填值」，中途失败也不会反复重读。
pub(crate) fn mark_base_settled() {
    BASE_SETTLED.with(|c| c.set(true));
}

#[must_use]
pub(crate) fn bearer_settled() -> bool {
    BEARER_SETTLED.with(Cell::get)
}

pub(crate) fn mark_bearer_settled() {
    BEARER_SETTLED.with(|c| c.set(true));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 同一个 `thread_local` 在同一测试线程内跨用例共享 —— 每条用例只断言自己写过的部分。
    #[test]
    fn setters_replace_the_value_and_settle_it() {
        set_api_base("  http://127.0.0.1:8080/ ");
        assert_eq!(api_base(), "http://127.0.0.1:8080", "规范化在 scope 内完成");
        assert!(base_settled());

        set_bearer("  tok  ");
        assert_eq!(bearer(), "tok");
        assert!(bearer_settled());
    }

    #[test]
    fn clearing_the_bearer_is_an_empty_value_not_a_space() {
        set_bearer("tok");
        set_bearer("   ");
        assert_eq!(bearer(), "", "清空是「没有」，不是「一个空格」");
    }

    #[test]
    fn swapping_one_field_keeps_the_other() {
        set_api_base("http://a:1");
        set_bearer("tok");
        set_api_base("http://b:2");
        assert_eq!(bearer(), "tok", "换基址不该把凭证丢掉");
        assert_eq!(api_base(), "http://b:2");
    }
}
