//! 状态栈「连接」chip：这一屏的数据来自哪台 `serve`。
//!
//! 对齐 TUI 的 lane 行（`docs/design/tui_multi_server.md` §4.6）：TUI 在**单连接**时
//! 也照画 lane 区，理由是「不显示的话，用户既看不出自己连的是哪台 serve，也看不出
//! 这客户端支持多个连接」。Desktop / Android / Web 共用同一份 WASM，所以放在状态栈里
//! 一次三端受益——这也是本仓唯一能「对齐 TUI 显示」而不先做多连接的部分。
//!
//! 三个刻意的取舍：
//!
//! 1. **只显示 `host:port`**：状态栈每个 chip 只占**一行**（`.status-chips` 纵向
//!    堆叠、各自撑满右列宽度），一个长 FQDN 会在这行内被省略号截掉，读不出后两段
//!    IP 的差别。完整基址进 `title`，正文本只留能区分两台 serve 的那一段——端口必须留，
//!    同机两个 serve 只靠它区分。
//! 2. **空基址整段不渲染**：空 = 同 Origin（WASM 的默认态），此时没有「哪台 serve」
//!    可讲；画一个 `—` 只会让人以为连接坏了。与 TUI 的 `ctx` 段占位取舍相反，因为那里
//!    的 `—` 表示「还没测到」，而这里表示「本来就没有这个值」。
//! 3. **不放窄屏**：`mobile.css` 的 `html[data-narrow-viewport]` 已把非 role / context
//!    的 chip 全部隐藏，本 chip 自动让位。这与 TUI 的 `lanes` 段同一条纪律——连接身份
//!    是「想知道时能查到」的信息，不是每回合都要读的东西，不该和 `ctx` 抢宽度。
//!    窄屏下它仍可在「设置 → 连接」看到。
//!
//! 点击跳「设置 → 连接」：这是唯一能改基址的地方，且用户此刻想问的往往正是
//! 「我想换一台」——把它做成可点而不是纯标签。
//!
//! **与 TUI 的 `serve` 段是同一件事**（`crates/crabmate-tui/src/tui_mode/status.rs`
//! 的 `serve_value`）：两端都回答「这一屏的数据来自哪台 serve」，都经
//! `api_base_authority()` 剥掉 scheme / 路径 / userinfo。差别只有一处，且是**刻意的**：
//! 终端没有 hover，`title` 那套用不了，所以 TUI 在列宽够时显示**完整** base（那是它
//! 唯一能看到全量 URL 的地方），真放不下才退 `host:port`；这里恒为 `host:port`，
//! 完整 URL 交给 `title`。**别把其中一边「统一」成另一边** —— 拉平会丢掉对应平台
//! 唯一的那项能力：TUI 失去全量 URL，本 chip 失去悬停可读的完整地址。
//!
//! # 多连接（桌面多窗口）
//!
//! 桌面壳里同时可以开多个窗口，每个窗口连一台 `serve`（#391）。此时这一屏的不再是
//! 「唯一那条连接」，而是**全部窗口的连接**——数据来自壳的协调面
//! （`list_connections` + `connections_changed`），由 `desktop-tauri` 的
//! `ConnectionRegistry` 维护。点了哪条就切到那个窗口（`focus_connection`）。
//!
//! 单连接（含 Android / Web，它们只有一条）时**保持原样**：仍然是可点的单条 chip、
//! 点击跳设置页。所以这份改造对另外两端是零行为变化，只是多了一次「壳是否支持名单」
//! 的探测。浏览器里那次探测会失败（没有 `__TAURI__`），退回单条。
//!
//! 窗口之间是**完全遮挡**的（都铺满工作区），所以「有别的连接在等审批」只能靠这里和
//! 系统通知——见 `desktop_connections::notify_approval`。

use leptos::prelude::*;

use crabmate_client_api::connections::ConnectionSnapshot;
use crabmate_client_api::url::api_base_authority;

use crate::api::api_base_url;
use crate::i18n::{self, Locale};

use super::settings_page::{SettingsSection, navigate_to_settings};
use super::shell_runtime_context::try_chat_shell_ctx;

/// chip 上显示什么：`None` = 整段不渲染。
///
/// 抽成纯函数而不是在闭包里现算，是为了能在非 WASM 目标下测「基址 → 短文本」
/// 这一步（它不碰任何信号）。
#[must_use]
fn connection_chip_text(base: &str) -> Option<String> {
    let authority = api_base_authority(base);
    if authority.is_empty() {
        None
    } else {
        Some(authority)
    }
}

/// 一条连接在 chip 上显示的名字；用户没起名时退回 `host:port`。
///
/// 抽出来是为了能测：名字与地址都在时用名字（用户取的就是给人读的），否则用地址
/// （否则会出现一个空 label 的 chip，点都不知道点的是什么）。
#[must_use]
fn connection_chip_name(snapshot: &ConnectionSnapshot) -> String {
    let name = snapshot.display_name.trim();
    if name.is_empty() {
        snapshot.authority.clone()
    } else {
        name.to_string()
    }
}

/// 从 `connections_changed` 的 JSON 里取出名单。
///
/// 解析失败返回 `None` 而不是空表：**空表**的含义是「没有连接」（会退回单条 chip），
/// 而解析失败是「不知道」，两者不该混为一谈——混了就会在协议变动时静默退回单条。
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
/// - 订阅只装一次，状态栈重挂载不会重复订阅；
/// - 重建后的新组件读的还是同一份 —— 之前的进程级 `INSTALLED` 标志配上每实例 signal，
///   会让重挂载后的 signal 永远等不到填充（列表冻结在空，退化成单条 chip）；
/// - 订阅与 signal 同寿（都到页面结束），所以**不需要** unlisten：回调永远不会
///   `set` 一个已经 drop 的 signal。
///
/// 代价是这一份在 `api_base` 上取了首次调用者的值。同一窗口内 `api_base` 不变
/// （换 serve = 换窗口或重载页面），所以这个前提成立。
#[cfg(target_arch = "wasm32")]
fn connection_feed(api_base: String) -> ArcRwSignal<Vec<ConnectionSnapshot>> {
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
fn connection_feed(_: String) -> ArcRwSignal<Vec<ConnectionSnapshot>> {
    ArcRwSignal::new(Vec::new())
}

/// 先 `register` 补自己的位置，再 `list` 补首帧，最后订阅增量。
///
/// 顺序不能反：注册表里没有「我」时，`list` 会返回一份只含别的窗口的名单 ——
/// `len` 永远差一条，多窗口下凑不到 2，chip 就永远不切成列表。
#[cfg(target_arch = "wasm32")]
fn install_connections_feed(feed: ArcRwSignal<Vec<ConnectionSnapshot>>, api_base: String) {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen_futures::spawn_local;

    spawn_local(async move {
        let _ = crate::tauri_shell::tauri_register_connection(&api_base, None).await;

        if let Ok(json) = crate::tauri_shell::tauri_list_connections_json().await
            && let Some(list) = parse_connection_list(&json)
        {
            feed.set(list);
        }

        let apply = move |json: String| {
            // 解析失败时保持现状：一次坏帧不该把列表清空。
            if let Some(list) = parse_connection_list(&json) {
                feed.set(list);
            }
        };
        let cb = Closure::<dyn FnMut(String)>::new(apply);
        let func: js_sys::Function = cb.into_js_value().unchecked_into();
        // `Ok(None)` = 这个壳没有 event 能力（浏览器 / 老壳）：名单停在首帧，不是错误。
        // 句柄故意丢弃：订阅与这份全局 signal 同寿，没有需要退订的时刻。
        let _ = crate::tauri_shell::tauri_listen_connections_changed(func).await;
    });
}

#[component]
pub fn StatusConnectionChip(
    locale: RwSignal<Locale>,
    settings_page: RwSignal<bool>,
) -> impl IntoView {
    // 缺 ctx（单测 / 独立子树）时退回「读一次 thread_local」：没有上下文不该让整条
    // 状态栈 panic，但那种情形下也不会有别的写入点，读一次就是终值。
    let api_base = try_chat_shell_ctx()
        .map(|c| c.api_base)
        .unwrap_or_else(|| RwSignal::new(api_base_url()));

    let text = Memo::new(move |_| connection_chip_text(&api_base.get()));
    let title = Memo::new(move |_| {
        i18n::status_chip_connection_tooltip(locale.get(), api_base.get().as_str())
    });

    // 包一层 `StoredValue`：`ArcRwSignal` 不是 `Copy`，而 `view!` 里多处闭包都要用它。
    // `StoredValue` 是 `Copy` 且 `get_value()` 只是 clone 一个 Arc 句柄，代价可忽略。
    let connections = StoredValue::new(connection_feed(api_base.get_untracked()));

    // 只有两条以上才切成列表：一条时那个「列表」就是现在这个 chip，改掉它只是让
    // 单连接用户多一次点击（还得点回设置页才能换基址）。
    //
    // 这里的 clone 是必须的：`ArcRwSignal` 不是 `Copy`，`Memo` 的闭包要 move 一份，
    // `For` 的 `each` 还要另一份。
    let multi = Memo::new(move |_| connections.get_value().get().len() > 1);

    view! {
        <Show when=move || text.get().is_some() && !multi.get()>
            <button
                type="button"
                class="status-chip status-chip-connection"
                data-testid="status-connection-chip"
                prop:title=move || title.get()
                on:click=move |_| {
                    navigate_to_settings(settings_page, SettingsSection::Connection);
                }
            >
                <span class="status-chip-label">
                    {move || i18n::status_chip_connection(locale.get())}
                </span>
                <span class="status-chip-value">{move || text.get().unwrap_or_default()}</span>
            </button>
        </Show>
        <Show when=move || multi.get()>
            <For
                each=move || connections.get_value().get()
                key=|snapshot| snapshot.key.clone()
                children=move |snapshot| {
                    let key = snapshot.key.as_str().to_string();
                    let name = connection_chip_name(&snapshot);
                    let authority = snapshot.authority.clone();
                    let tooltip = format!("{name} · {authority}");
                    // 等审批的那条打个标记：窗口互相完全遮挡，列表是用户唯一能看见
                    // 「谁在等我」的地方（系统通知在它没被聚焦时也会发一条）。
                    let pending = snapshot.status == crabmate_client_api::connections::ConnectionStatus::WaitingApproval;
                    view! {
                        <button
                            type="button"
                            class="status-chip status-chip-connection"
                            data-testid="status-connection-chip"
                            prop:title=tooltip
                            on:click=move |_| crate::tauri_shell::tauri_focus_connection(&key)
                        >
                            <span class="status-chip-label">
                                {if pending { "▲ " } else { "" }}{name}
                            </span>
                            <span class="status-chip-value">{authority}</span>
                        </button>
                    }
                }
            />
        </Show>
    }
}

#[cfg(test)]
mod tests {
    use super::{connection_chip_name, connection_chip_text, parse_connection_list};
    use crabmate_client_api::connections::ConnectionSnapshot;

    #[test]
    fn host_and_port_survive_but_scheme_and_path_do_not() {
        // 端口必须留：同机两个 serve 只靠它区分。
        assert_eq!(
            connection_chip_text("http://127.0.0.1:8081"),
            Some("127.0.0.1:8081".to_string())
        );
        assert_eq!(
            connection_chip_text("https://serve.example.com:8443/api"),
            Some("serve.example.com:8443".to_string())
        );
    }

    #[test]
    fn same_origin_renders_nothing_rather_than_a_placeholder() {
        // 空基址 = 同 Origin，没有「哪台 serve」可讲。
        assert_eq!(connection_chip_text(""), None);
        assert_eq!(connection_chip_text("   "), None);
        // 认不出的基址同样不渲染，而不是把原件当地址画出来。
        assert_eq!(connection_chip_text("not-a-url"), None);
    }

    #[test]
    fn named_connection_shows_its_name() {
        let mut snapshot = ConnectionSnapshot::new("生产环境", "http://a:1");
        snapshot.authority = "a:1".to_string();
        assert_eq!(connection_chip_name(&snapshot), "生产环境");

        // 空白名不是「起名为空格」：`rename` 会忽略它并**保留原名**（见
        // `ConnectionSnapshot::rename`），所以这里仍应是名字而不是地址。
        snapshot.rename("   ");
        assert_eq!(connection_chip_name(&snapshot), "生产环境");
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
        assert_eq!(connection_chip_name(&snapshot), "a:1");
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
        assert_eq!(connection_chip_name(&list[0]), "生产环境");
        assert_eq!(list[0].authority, "a:1");
        assert_eq!(
            list[0].status,
            crabmate_client_api::connections::ConnectionStatus::WaitingApproval
        );
        assert!(list[0].unread);
    }
}
