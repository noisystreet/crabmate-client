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
    /// Bearer 是否已经「有过一次决定」：**水合读过盘**，或**被显式设过值**（含清空）。
    ///
    /// 一义两用是有意的：两者都表示「内存里的值已权威，不必再读钥匙串」；代价是清空
    /// 之后不会自动去钥匙串重读 —— 而清空正是用户的显式意图。多 lane 之后若要区分
    /// 「B 没水合过」与「B 被清空过」，得把它拆成两个标记（届时这两条路径会分开走）。
    bearer_settled: bool,
    /// 这一份的 `localStorage` 域后缀（`<基础键>#<label>`）；`None` = 不分域。
    ///
    /// #558 项 3b：域必须**跟着 holder 走**，不能每次去问壳注入的全局值 —— 单窗口多
    /// lane 时一个页面会同时存在多份 holder，而注入值只有一个（= 窗口 label）。单 lane
    /// 时两者相等（[`ScopeHolder::new`] 就取注入值），所以单连接语义与迁移前一致。
    ///
    /// 为什么不用 `scope.identity()` 当域：`ConnectionKey::from_raw("")` 会回落到
    /// `"serve"`（非空），于是「无标识」与「标识恰好是 serve」无法区分 —— 拿它判断
    /// 「要不要加分域后缀」会静默改变纯浏览器路径的键名。
    storage_label: Option<String>,
}

impl ScopeHolder {
    /// 建一份 holder，域取壳注入的连接标识（单 lane / 主窗口的既有行为）。
    fn new() -> Self {
        Self::new_with_storage_label(super::storage_scope::connection_key())
    }

    /// 建一份 holder，**显式指定** `localStorage` 域后缀（#558 项 3b）。
    ///
    /// 多 lane 时由 lane 容器给（`None` = 该 lane 不分域，等于历史键名）；这样
    /// 「值写进哪份 holder」与「读写哪个存储域」是同一个答案，不会一半一半。
    pub(crate) fn new_with_storage_label(storage_label: Option<String>) -> Self {
        Self(Rc::new(RefCell::new(ScopeState {
            storage_label,
            ..ScopeState::default()
        })))
    }

    /// 这一份的存储域后缀；`None` = 用历史键名（纯浏览器 / Android / 未注入）。
    #[must_use]
    pub(crate) fn storage_label(&self) -> Option<String> {
        self.0.borrow().storage_label.clone()
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

    /// 这一份的基址是否已经决定过（per-holder；`api::browser` 算指纹前要判它）。
    #[must_use]
    pub(crate) fn is_base_settled(&self) -> bool {
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

/// `holder` 是不是当前前台那一份（同一份 `Rc`，不是「值相等」）。
///
/// 用途只有一个：`api::browser` 的显式 holder 分布式读要把「零参路径原本隐含的基址
/// 水合」补回来 —— 只有 ACTIVE 那一份该被顺手水合，别的 lane 的基址由它自己的启动
/// 路径决定（#558 收窄后的口径：跨 lane 路径必须显式带 holder）。
#[must_use]
pub(crate) fn is_active(holder: &ScopeHolder) -> bool {
    ACTIVE.with(|cell| {
        cell.borrow()
            .as_ref()
            .is_some_and(|a| Rc::ptr_eq(&a.0, &holder.0))
    })
}

/// 把**已有的**那一份设为前台默认（切 lane 的唯一入口，#561 / #558）。
///
/// 与 [`install_new_holder`] 的分工：那个是「造一份空的再设为前台」（启动 / 新 lane
/// 的起点），这个只移动指针 —— 值、水合标记、存储域都留在原来那份里，所以切回上一
/// 条 lane 时配置与凭据不用重新读盘。
///
/// 单 lane 时调用它等于没动（零行为变化）。
///
/// **为什么现在就加**：它让「谁当前台」只有一条写入路径。将来 lane 容器的
/// `set_active` 直接调这里，不必再碰 `ACTIVE` 这个 `thread_local` —— 否则「切换」
/// 会散成两处（容器改下标一处、指针另一处），漏一处就是「下标说 A、请求发到 B」。
pub(crate) fn activate(holder: &ScopeHolder) {
    ACTIVE.with(|cell| *cell.borrow_mut() = Some(holder.clone()));
}

/// 换一份新的空 scope（bootstrap / 新 lane 的起点）；旧的那份随之丢弃。
pub(crate) fn install_new_holder() {
    activate(&ScopeHolder::new());
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
        // 标记问 holder 本身（#558：`bearer_settled` / `mark_bearer_settled` 的零参
        // 包装已删 —— 它们只是「转发给 ACTIVE」，留着等于给跨 lane 误用留门）。
        assert!(active_holder().is_bearer_settled());
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

    /// #558 项 3b 的地基：**存储域**也是 per-holder 的，不是每次去问壳注入值。
    ///
    /// 单窗口多 lane 时一个页面有多份 holder，而壳注入的连接标识只有一个（窗口
    /// label）—— 域若仍取全局值，两条 lane 会读写同一个 `localStorage` 命名空间。
    #[test]
    fn holders_carry_their_own_storage_label() {
        let a = ScopeHolder::new_with_storage_label(Some("win-a".to_string()));
        let b = ScopeHolder::new_with_storage_label(Some("win-b".to_string()));
        assert_eq!(a.storage_label().as_deref(), Some("win-a"));
        assert_eq!(b.storage_label().as_deref(), Some("win-b"));

        // 不分域（纯浏览器 / Android / 未注入标识）保持 `None`：键名退回历史形态。
        assert_eq!(
            ScopeHolder::new_with_storage_label(None).storage_label(),
            None
        );
        // `new()` 取壳注入值；host 目标没有注入，所以是 `None`。
        assert_eq!(ScopeHolder::new().storage_label(), None);
    }

    /// 域后缀与 `ACTIVE` 是两个独立维度：换了 ACTIVE 指针不改已有 holder 的域。
    #[test]
    fn swapping_the_active_holder_keeps_each_holders_label() {
        let mine = ScopeHolder::new_with_storage_label(Some("lane-1".to_string()));
        install_new_holder();
        assert_eq!(
            mine.storage_label().as_deref(),
            Some("lane-1"),
            "换前台不该重写别条 lane 的域"
        );
    }

    /// #558 项 1b 的地基：`is_active` 认的是**同一份 `Rc`**，不是「值相等」。
    ///
    /// `AppSignals` 存的就是这一份（`new()` 传 `active_holder()`），而 `ConnSignals::new`
    /// 用它决定「要不要走零参水合路径」。判定若不严，两条一模一样的 lane 会互相认错。
    #[test]
    fn active_identity_is_the_same_rc_not_equal_values() {
        let first = active_holder();
        assert!(is_active(&first), "多次取到的应是同一份");
        assert_eq!(first.storage_label(), None, "host 无注入标识");

        let lookalike = ScopeHolder::new_with_storage_label(None);
        assert!(!is_active(&lookalike), "值相同也不是前台那一份");
    }

    /// 切前台的唯一入口：只移动指针，不动值、不动水合标记、不动域。
    ///
    /// 这是「切回上一条 lane 不用重新读盘」的地基。若 `activate` 顺手重置 settled
    /// 或换掉域后缀，切回来就会重新水合（多打一次钥匙串），甚至读写到别人的键。
    #[test]
    fn activating_a_holder_moves_the_pointer_without_touching_state() {
        // 先记住原先前台那一份：测完要放回去，否则同线程后续用例会看到被切走的指针。
        let original = active_holder();

        let other = ScopeHolder::new_with_storage_label(Some("lane-2".to_string()));
        other.set_api_base("http://other:9000");
        activate(&other);

        assert!(is_active(&other), "activate 之后它就是前台那一份");
        assert!(!is_active(&original), "切走之后原来那份不再是前台");
        assert_eq!(
            other.storage_label().as_deref(),
            Some("lane-2"),
            "域后缀跟着 holder 走，activate 不该改它"
        );
        assert!(other.is_base_settled(), "水合标记原样保留");

        // 切回去：状态仍然原样。
        activate(&original);
        assert!(is_active(&original));
        assert!(!is_active(&other));
        assert_eq!(other.storage_label().as_deref(), Some("lane-2"));
    }
}
