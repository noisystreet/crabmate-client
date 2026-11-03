//! 全部窗口的连接名单（应用级 signal）+ 连接在 UI 上的显示名。
//!
//! 数据来自桌面壳的协调面（`list_connections` + `connections_changed`），由
//! `desktop-tauri` 的 `ConnectionRegistry` 维护。**一处订阅、两处读**：左栏的「连接」区
//! （[`super::nav_rail_connections`]）把它画成列表，`app_shell_effects::connection_report`
//! 据此上报本窗口状态 —— 两者读的是同一份 signal。
//!
//! # 右列那排 chip 已并入左栏
//!
//! 这份名单原先在两个角落各画一遍：左栏一块，右列状态栈里一排 `.status-chip`。
//! 两处读数迟早对不上，窄屏时又一起被隐藏 —— 于是只留左栏（TUI 的 lane 区是同一个
//! 位置、同一条「恒占左栏一半」的版面契约）。连带搬过去的两件事：
//!
//! 1. **点一条就切到那个窗口**（`focus_connection`），多连接时成立；
//! 2. **单连接时点它进「设置 → 连接」** —— 那里是唯一能改基址的地方，而这个入口原先
//!    正是右列那颗单条 chip 提供的。
//!
//! 窗口之间是**完全遮挡**的（都铺满工作区），所以「切过去」与「谁在等我」都落在左栏：
//! 那是唯一在每个窗口里都看得见的名单。右列那颗 chip 曾经承担「谁在等审批」的提示，
//! 现在由左栏的 `▲` 与系统通知（`desktop_connections::notify_approval`）接手。
//!
//! Android / Web 没有第二个窗口（`tauri_shell_available()` 为假），整块不渲染；
//! 那种壳里本模块的订阅也拿不到名单（没有 `__TAURI__`），只剩一个空 signal。

use leptos::prelude::*;

use crabmate_client_api::connections::ConnectionSnapshot;

/// 一条连接在左栏行上显示的名字；用户没起名时退回 `host:port`。
///
/// 抽出来是为了能测：名字与地址都在时用名字（用户取的就是给人读的），否则用地址
/// （否则会出现一个空 label 的行，点都不知道点的是什么）。
#[must_use]
pub(crate) fn connection_display_name(snapshot: &ConnectionSnapshot) -> String {
    let name = snapshot.display_name.trim();
    if name.is_empty() {
        snapshot.authority.clone()
    } else {
        name.to_string()
    }
}

/// 从 `connections_changed` 的 JSON 里取出名单。
///
/// 解析失败返回 `None` 而不是空表：**空表**的含义是「没有连接」，而解析失败是「不知道」，
/// 两者不该混为一谈——混了就会在协议变动时静默退回「一件都不知道」的状态。
#[cfg(any(target_arch = "wasm32", test))]
#[must_use]
fn parse_connection_list(json: &str) -> Option<Vec<ConnectionSnapshot>> {
    serde_json::from_str::<Vec<ConnectionSnapshot>>(json).ok()
}

/// 安装名单订阅（只装一次）。
///
/// 先 `list_connections` 补首帧，再订阅增量：反之会漏掉「上一次广播到订阅生效之间」
/// 发生的变化，那正是窗口刚开出来、注册表刚登记完的那一瞬间。
/// 全部窗口的连接名单（每个 WASM 实例一份）。
///
/// 做成全局而不是每组件一份，是因为它描述的是**整个应用**的状态（所有窗口连了谁），
/// 不是某个组件自己的状态。这样三个问题一起消失：
///
/// - 订阅只装一次，左栏重挂载不会重复订阅；
/// - 重建后的新组件读的还是同一份 —— 之前的进程级 `INSTALLED` 标志配上每实例 signal，
///   会让重挂载后的 signal 永远等不到填充（列表冻结在空）；
/// - 订阅与 signal 同寿（都到页面结束），所以**不需要** unlisten：回调永远不会
///   `set` 一个已经 drop 的 signal。
///
/// 代价是这一份在 `api_base` 上取了首次调用者的值。同一窗口内 `api_base` 不变
/// （换 serve = 换窗口或重载页面），所以这个前提成立。
#[cfg(target_arch = "wasm32")]
pub(crate) fn connection_feed(api_base: String) -> ArcRwSignal<Vec<ConnectionSnapshot>> {
    thread_local! {
        static FEED: std::cell::OnceCell<ArcRwSignal<Vec<ConnectionSnapshot>>> =
            const { std::cell::OnceCell::new() };
    }

    FEED.with(|cell| {
        cell.get_or_init(|| {
            let feed = ArcRwSignal::new(Vec::new());
            // `ArcRwSignal` 不是 `Copy`：订阅拿到的是同一信号的另一句柄。
            install_connections_feed(feed.clone(), api_base);
            feed
        })
        .clone()
    })
}

/// 非 wasm（测试 / SSR）：没有壳，也就没有名单来源。
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn connection_feed(_: String) -> ArcRwSignal<Vec<ConnectionSnapshot>> {
    ArcRwSignal::new(Vec::new())
}

/// 「本窗口已进壳的注册表」。
///
/// 上报方（`app_shell_effects::connection_report`）必须在**登记之后**才发第一帧状态：
/// 壳按 window label 查注册表，未登记的 label 会被 `set_status` 静默丢弃
/// （返回 `None` 就 return），而登记是在 [`install_connections_feed`] 里 spawn 出去的
/// 一次异步调用 —— 首帧的 `Effect` 很可能比它先跑，那一次上报就白发了。
///
/// 用信号而不是 `bool`：订阅方要在**登记完成的那一刻**补发一次当前状态，而那时它
/// 自己订阅的其它信号（回合生命周期 / 待审批）可能一个都没变过。
///
/// 非 wasm（浏览器 / 测试）初值即 `true`：那里没有注册表，也就没有「等登记」这件事，
/// 让它停在 `false` 只会让将来的订阅方永远不发。
pub(crate) fn connections_registered() -> RwSignal<bool> {
    thread_local! {
        static REGISTERED: std::cell::OnceCell<RwSignal<bool>> =
            const { std::cell::OnceCell::new() };
    }

    REGISTERED.with(|cell| *cell.get_or_init(|| RwSignal::new(!cfg!(target_arch = "wasm32"))))
}

/// 先订阅、再 `register`、最后 `list` 补首帧。
///
/// 三步的顺序都有理由，任何一步换位都会退化成「名单永远空」：
///
/// - **订阅必须早于 `register`**：壳在登记那一刻就广播一次全量名单，订阅晚一步就收
///   不到那一次 —— 而它正是「我自己那条」进列表的时机（窗口自己不会再有别的上报）。
/// - `register` 必须早于 `list`：注册表里没有「我」时，`list` 会返回一份只含别的
///   窗口的名单 —— `len` 永远差一条，多窗口下凑不到 2，左栏就永远只有自己那一行。
/// - `list` 只在名单**仍为空**时落盘：广播可能已经先到并给出更新的快照，用首帧把它
///   覆盖回去会让状态往回倒一帧（这也是 `as_string()` 那条老 bug 之外的第二重保险，
///   首帧读不到不再是致命的）。
#[cfg(target_arch = "wasm32")]
fn install_connections_feed(feed: ArcRwSignal<Vec<ConnectionSnapshot>>, api_base: String) {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen_futures::spawn_local;

    spawn_local(async move {
        let apply = {
            let feed = feed.clone();
            move |json: String| {
                // 解析失败时保持现状：一次坏帧不该把列表清空。
                if let Some(list) = parse_connection_list(&json) {
                    feed.set(list);
                }
            }
        };
        let cb = Closure::<dyn FnMut(String)>::new(apply);
        let func: js_sys::Function = cb.into_js_value().unchecked_into();
        // `Ok(None)` = 这个壳没有 event 能力（浏览器 / 老壳）：名单停在首帧，不是错误。
        // 句柄故意丢弃：订阅与这份全局 signal 同寿，没有需要退订的时刻。
        let _ = crate::tauri_shell::tauri_listen_connections_changed(func).await;

        // 登记成功才置位：上报方据此补发第一帧状态。失败（老壳不认识这条命令）时
        // 保持 `false` —— 那种壳里也没有第二个窗口要读它，停在不上报是对的。
        let registered = crate::tauri_shell::tauri_register_connection(&api_base, None).await;
        if registered.is_ok() && crate::tauri_shell::tauri_shell_available() {
            connections_registered().set(true);
        }

        if feed.get_untracked().is_empty()
            && let Ok(json) = crate::tauri_shell::tauri_list_connections_json().await
            && let Some(list) = parse_connection_list(&json)
        {
            feed.set(list);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{connection_display_name, parse_connection_list};
    use crabmate_client_api::connections::ConnectionSnapshot;

    #[test]
    fn named_connection_shows_its_name() {
        let mut snapshot = ConnectionSnapshot::new("生产环境", "http://a:1");
        snapshot.authority = "a:1".to_string();
        assert_eq!(connection_display_name(&snapshot), "生产环境");

        // 空白名不是「起名为空格」：`rename` 会忽略它并**保留原名**（见
        // `ConnectionSnapshot::rename`），所以这里仍应是名字而不是地址。
        snapshot.rename("   ");
        assert_eq!(connection_display_name(&snapshot), "生产环境");
    }

    #[test]
    fn an_unnamed_connection_falls_back_to_its_address() {
        // `ConnectionSnapshot::new` 本身就会把空名换成地址，所以走正常构造拿不到
        // 「空 display_name」这个状态。但前端拿到的快照是**反序列化**来的（壳经 JSON
        // 传过来），不保证一定经过 `new`；UI 侧的回退是为那条路径留的。
        let mut snapshot = ConnectionSnapshot::new("x", "http://a:1");
        snapshot.authority = "a:1".to_string();
        snapshot.display_name = String::new();
        assert!(snapshot.display_name.is_empty());
        assert_eq!(connection_display_name(&snapshot), "a:1");
    }

    #[test]
    fn a_broken_frame_is_unknown_not_empty() {
        // 空数组 = 真的没有连接（退回单条 chip）；坏 JSON = 不知道。
        // 两者若混为一谈，协议一变就会静默退回单条，看不出出了问题。
        assert_eq!(parse_connection_list("[]").map(|v| v.len()), Some(0));
        assert!(parse_connection_list("not json").is_none());
        assert!(parse_connection_list(r#"{"unexpected":"shape"}"#).is_none());
    }

    #[test]
    fn a_round_tripped_list_keeps_the_fields_the_ui_reads() {
        // 桌面壳经 JSON 传过来，字段名由 serde 决定；这条测的就是「壳写的名字 =
        // 前端读的名字」，改名会在这里断。
        let json = r#"[{
            "key": "prod",
            "display_name": "生产环境",
            "api_base": "http://a:1",
            "authority": "a:1",
            "status": "WaitingApproval",
            "unread": true,
            "active": false
        }]"#;
        let list = parse_connection_list(json).expect("should parse");
        assert_eq!(list.len(), 1);
        assert_eq!(connection_display_name(&list[0]), "生产环境");
        assert_eq!(list[0].authority, "a:1");
        assert_eq!(
            list[0].status,
            crabmate_client_api::connections::ConnectionStatus::WaitingApproval
        );
        assert!(list[0].unread);
    }
}
