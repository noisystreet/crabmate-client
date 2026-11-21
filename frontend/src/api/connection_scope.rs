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

use std::cell::RefCell;
use std::rc::Rc;

use crabmate_client_api::ConnectionKey;
use crabmate_client_api::connection_scope::ConnectionScope;

/// 一份连接作用域的**持有者**（#558）。
///
/// 值活在这里，**不再直接躺在 `thread_local` 里**：一条 lane / 一个窗口各持一份，
/// 将来由组件 ctx 提供，`join_api_path(scope.api_base(), …)` 那一族入口就能显式拿到它。
///
/// 本步只做「搬家」：`ACTIVE` 记的是「当前这条 lane 用哪一份」的**指针**，零参读取口
/// （`api_base()` / `bearer()`）经它转发 —— 所以 43 处调用点一行不用改。删掉 `ACTIVE`、
/// 改成从 ctx 取，是 #558 的下一步（那一步才需要改调用点）。
#[derive(Clone)]
pub(crate) struct ScopeHolder(Rc<RefCell<ScopeState>>);

#[derive(Default)]
struct ScopeState {
    /// `None` = 还没建（第一次读时按空值建，具体值由水合 / setter 填）。
    scope: Option<ConnectionScope>,
    /// 基址是否已经决定过一次（决定过就不再读 `localStorage`，避免每次读都打盘）。
    base_settled: bool,
    /// Bearer 是否已经水合过一次。
    bearer_settled: bool,
}

impl ScopeHolder {
    fn new() -> Self {
        Self(Rc::new(RefCell::new(ScopeState::default())))
    }

    /// 读这一份（第一次访问时建立空实例）。
    fn read<R>(&self, f: impl FnOnce(&ConnectionScope) -> R) -> R {
        {
            let mut state = self.0.borrow_mut();
            if state.scope.is_none() {
                state.scope = Some(empty_scope());
            }
        }
        // 只在取值这一步借用：`f` 内部若再读 scope 不会被挡住。
        let state = self.0.borrow();
        f(state.scope.as_ref().expect("上面刚建好"))
    }

    /// 用 `f` 产生下一版 scope（值语义替换，不做原地改字段）。
    fn update(&self, f: impl FnOnce(&ConnectionScope) -> ConnectionScope) {
        let mut state = self.0.borrow_mut();
        let current = state.scope.take().unwrap_or_else(empty_scope);
        state.scope = Some(f(&current));
    }

    fn mark_base_settled(&self) {
        self.0.borrow_mut().base_settled = true;
    }

    fn is_base_settled(&self) -> bool {
        self.0.borrow().base_settled
    }

    /// 标记「这一份的 Bearer 已经读过盘」——水合失败也要标，否则每次请求都重读钥匙串。
    pub(crate) fn mark_bearer_settled(&self) {
        self.0.borrow_mut().bearer_settled = true;
    }

    /// Bearer 是否已经水合过一次（幂等开关，**per-holder**：A 读过不代表 B 读过）。
    pub(crate) fn is_bearer_settled(&self) -> bool {
        self.0.borrow().bearer_settled
    }

    /// 这一份的基址（空 = 同 Origin）。
    ///
    /// #558 的显式入口：拿到 holder 的调用方（UI 层、将来的每条 lane）直接问它，
    /// 不必经过 `ACTIVE`。
    #[must_use]
    pub(crate) fn api_base(&self) -> String {
        self.read(|s| s.api_base().to_string())
    }

    /// 这一份的 Bearer（空 = 没有）。**不要写进日志**。
    #[must_use]
    pub(crate) fn bearer(&self) -> String {
        self.read(|s| s.bearer().to_string())
    }

    /// 换这一份的基址：值 + 标记一起更新。
    pub(crate) fn set_api_base(&self, raw: &str) {
        self.update(|s| s.with_api_base(raw));
        self.mark_base_settled();
    }

    /// 换这一份的 Bearer（`memory_only` 与「从钥匙串水合」共用这一条路径）。
    pub(crate) fn set_bearer(&self, raw: &str) {
        self.update(|s| s.with_bearer(raw));
        self.mark_bearer_settled();
    }
}

thread_local! {
    /// **当前这条 lane 用哪一份**（指针，不是值本身）。值在 [`ScopeHolder`] 里。
    static ACTIVE: RefCell<Option<ScopeHolder>> = const { RefCell::new(None) };
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

/// 当前 lane 的持有者；第一次访问时建一份空 scope 的。
pub(crate) fn active_holder() -> ScopeHolder {
    ACTIVE.with(|cell| {
        if cell.borrow().is_none() {
            *cell.borrow_mut() = Some(ScopeHolder::new());
        }
        cell.borrow().as_ref().expect("刚建好").clone()
    })
}

/// 换一份新的空 scope（bootstrap / 新 lane 的起点）；旧的那份随之丢弃。
pub(crate) fn install_new_holder() {
    ACTIVE.with(|cell| *cell.borrow_mut() = Some(ScopeHolder::new()));
}

/// 当前基址（空 = 同 Origin）。**不**触发任何磁盘读取。
///
/// 这是「**前台默认** holder」的读取口（#558 收窄后的口径）：跨 lane 的路径不要用它，
/// 改问自己那条 lane 的 [`ScopeHolder`]。
#[must_use]
pub(crate) fn api_base() -> String {
    active_holder().api_base()
}

/// 当前 Bearer（空 = 没有）。**不**触发任何水合。口径同 [`api_base`]。
#[must_use]
pub(crate) fn bearer() -> String {
    active_holder().bearer()
}

/// 换基址：值 + 标记一起更新，调用方不必记得「改完还要置 settled」。
pub(crate) fn set_api_base(raw: &str) {
    active_holder().set_api_base(raw);
}

/// 换 Bearer（`memory_only` 与「从钥匙串水合」共用这一条路径）。
pub(crate) fn set_bearer(raw: &str) {
    active_holder().set_bearer(raw);
}

#[must_use]
pub(crate) fn base_settled() -> bool {
    active_holder().is_base_settled()
}

/// 只置标记、不动值：水合路径要「先声明读过盘，再填值」，中途失败也不会反复重读。
pub(crate) fn mark_base_settled() {
    active_holder().mark_base_settled();
}

#[must_use]
pub(crate) fn bearer_settled() -> bool {
    active_holder().is_bearer_settled()
}

pub(crate) fn mark_bearer_settled() {
    active_holder().mark_bearer_settled();
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

    /// #558 的验收地基：两个持有者必须有各自的基址 / Bearer / 水合标记。
    ///
    /// 这条用**两份独立的 holder** 直接验，不碰 `ACTIVE` —— 它是「同一窗口内两条 lane
    /// 打不同 serve 而不串」的最小证据。哪天有人把状态又挪回全局，这条会红。
    #[test]
    fn two_holders_do_not_share_state() {
        let a = ScopeHolder::new();
        let b = ScopeHolder::new();

        a.update(|s| s.with_api_base("http://a:1").with_bearer("tok-a"));
        a.mark_base_settled();
        b.update(|s| s.with_api_base("http://b:2"));

        assert_eq!(a.read(|s| s.api_base().to_string()), "http://a:1");
        assert_eq!(a.read(|s| s.bearer().to_string()), "tok-a");
        assert_eq!(b.read(|s| s.api_base().to_string()), "http://b:2");
        assert_eq!(
            b.read(|s| s.bearer().to_string()),
            "",
            "B 不该拿到 A 的凭据"
        );

        // 水合标记也是 per-holder 的：A 决定过基址，不代表 B 决定过。
        assert!(a.is_base_settled());
        assert!(!b.is_base_settled());
        assert!(!b.is_bearer_settled());
    }
}
