//! 左栏「连接」区（桌面多窗口）：会话列表下方常驻，对齐 TUI 的左栏 lane 区。
//!
//! **这是桌面上唯一的一处连接名单。** 右列状态栈里原先还有一排 `.status-chip`，与这块
//! 重复：两处读数迟早对不上，窄屏时又一起被隐藏，于是只留左栏。数据面仍在
//! [`connection_feed`]（订阅只装一次，左栏与状态上报读同一份 signal）。
//!
//! 版面与 TUI 的「左栏下半」是同一条口径：恒占左栏一半高度、与条数无关
//! （`tui_multi_server.md` §4.6 的 `layout::lane_pane_height`），条数少时下方留空、
//! 不拉伸。落地在 `frontend/styles/sidebar.css` 的 `.nav-rail-connections`
//! （`flex: 0 0 50%`）。
//!
//! **这不是 TUI lane 的等价物。** TUI 点击是「同进程切换当前 lane 的状态树」（聊天区立刻
//! 换一份 transcript，无重载）；这里点击是 `focus_connection`，切到**另一个 WebView 窗口**
//! —— 换整个 JS 堆。窗口都铺满工作区，所以切过去看到的是那个窗口自己的左栏。
//! 单窗口内并排多 lane（`tui_multi_server.md` §9.4.2 路线 B）明确不做，本组件只做
//! 「导航 + 状态总览」。
//!
//! 点击目标按条数分岔，两个入口互不重复：
//!
//! - **多连接**：切到那条连接所在的窗口（`focus_connection`）；
//! - **单连接**：进「设置 → 连接」—— 那是唯一能改基址的地方，这个入口原先正是右列那颗
//!   单条 chip 提供的。没有第二个窗口可切，所以这里不再有「点了也没用」的禁用态。
//!
//! `active` 不来自快照：`ConnectionRegistry::snapshots()` 广播时**强制** `active = false`
//! （同一份 payload 不可能对每个窗口都成立），所以这里按 `authority` 与自己的 `api_base`
//! 比对。唯一会让两行同时亮的场景（两个窗口连同一台 serve）已在连接时被
//! `ExistingConnectionProbe` 挡住 —— 同一个 `base_url` 只切过去、不新增第二条。
//!
//! # 两个分组：已连的窗口 / 可连的实例
//!
//! 上半是**已经连上的窗口**（每行一个窗口，右侧「关闭」）；下半是**控制面里声明过、
//! 还没连的实例**（右侧「连接」）—— 后者见 [`ctl_rows_for_rail`]。没有可连实例时
//! 整个下半组不渲染（「本机没跑 `crabmatectl`」是常态，不是告警）。
//!
//! 两个按钮都只是**已有的**壳动作：连接 = `open_connection`（撞 authority 时壳只聚焦），
//! 关闭 = `close_connection`（关窗 = 注销）。不存在「断开但保留窗口」的中间态，见
//! `docs/design/desktop_multi_window.md` §14。

use leptos::prelude::*;

use crabmate_client_api::connections::{ConnectionSnapshot, ConnectionStatus};
use crabmate_client_api::ctl::{CtlInstance, CtlObserved};
use crabmate_client_api::url::api_base_authority;

use crate::api::api_base_url;
use crate::i18n::{self, Locale};
use crate::tauri_shell::{tauri_close_connection, tauri_focus_connection, tauri_shell_available};

use super::connection_feed::{
    connection_display_name, connection_feed, ctl_instances_feed, refresh_ctl_instances,
};
use super::settings_page::{SettingsSection, navigate_to_settings};
use super::shell_runtime_context::try_chat_shell_ctx;

/// 状态点字形。与 TUI 的 `·●▲` 同口径；呼吸动画不进共享层（见 `ConnectionSnapshot`
/// 「刻意不含动画相位」），Web 侧靠 CSS transition。
fn status_glyph(status: ConnectionStatus) -> &'static str {
    match status {
        ConnectionStatus::Idle => "·",
        ConnectionStatus::Running => "●",
        ConnectionStatus::WaitingApproval => "▲",
        ConnectionStatus::Quitting => "×",
    }
}

/// 行尾读数：上下文 / 在途耗时 / 未读点。
///
/// 与 TUI lane 行同一份来源（`context` / `busy_secs` / `unread`），但排成一行 ——
/// 左栏 `--nav-rail-w` 是 260px，三列横排放不下。
fn row_meta(snapshot: &ConnectionSnapshot) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(context) = snapshot.context.as_deref().filter(|s| !s.trim().is_empty()) {
        parts.push(context.to_string());
    }
    if let Some(secs) = snapshot.busy_secs {
        parts.push(format!("{secs}s"));
    }
    if snapshot.unread {
        parts.push(String::from("•"));
    }
    parts.join(" ")
}

/// 一条「可连实例」在左栏的行模型（纯数据，便于单测）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CtlRow {
    /// 实例名（声明里的键），也是新窗口的显示名。
    pub name: String,
    /// 就绪时的 serve 根 URL；`Ready` 以外可能为空。
    pub api_base: String,
    pub state: CtlRowState,
}

/// 一行能做什么，由服务端的 `observed` 派生。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CtlRowState {
    /// `observed == ready` **且**带基址：可以点「连接」。
    Ready,
    /// 正在启动（`starting`）：不可点，等刷新按钮或下次打开。
    Starting,
    /// 其余（`failed` / `stopped` / `stale` / `held` / 缺字段 / 未知），带原因。
    Unavailable(String),
}

/// 不可连的原因：服务端 `last_error` 优先，其次 `observed` 原文，最后 `unknown`。
///
/// 不自己编措辞：`failed` 的原因只有 Server 知道（端口占用、工作区不存在……），
/// 抄过来比猜一句更可信。
fn unavailable_reason(instance: &CtlInstance) -> String {
    instance
        .last_error
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            instance
                .observed
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| String::from("unknown"))
}

/// 控制面实例 → 左栏「可连实例」行。
///
/// 四条规则，都是为了「不显示点了会失败的东西」：
///
/// 1. `enabled == Some(false)`：声明被软关闭（TUI 的 `/lane close`），连不上是预期；
/// 2. **已经在窗口里连着**（authority 相同）：它在上半组，画两遍会让人以为有两条连接；
/// 3. `ready` **但没基址**：开不了窗，落 `Unavailable` 而不是给一个点不动的「连接」；
/// 4. 判可连性用 [`CtlInstance::can_connect`]，**不**看 `state` —— `stale` 是「记录说
///    在跑但连不上」，信它就会连到死端口。
#[must_use]
pub(crate) fn ctl_rows_for_rail(
    instances: &[CtlInstance],
    connected: &[ConnectionSnapshot],
) -> Vec<CtlRow> {
    let taken: Vec<String> = connected
        .iter()
        .map(|snapshot| snapshot.authority.clone())
        .filter(|authority| !authority.is_empty())
        .collect();

    let mut rows = Vec::new();
    for instance in instances {
        if instance.enabled == Some(false) {
            continue;
        }
        let base = instance
            .base_url
            .as_deref()
            .unwrap_or("")
            .trim()
            .to_string();
        if !base.is_empty() && taken.contains(&api_base_authority(&base)) {
            continue;
        }
        let state = if instance.can_connect() && !base.is_empty() {
            CtlRowState::Ready
        } else if matches!(instance.observed_state(), CtlObserved::Starting) {
            CtlRowState::Starting
        } else {
            CtlRowState::Unavailable(unavailable_reason(instance))
        };
        rows.push(CtlRow {
            name: instance.name.clone(),
            api_base: base,
            state,
        });
    }
    rows
}

/// 一行「已连接的窗口」。`multi` 为真时点击切到那个窗口；为假（只有一条连接）时点击
/// 进「设置 → 连接」。
///
/// 行右侧的「关闭」只出现在**别人**的窗口上：关掉自己那一行等于退出应用，不是断开一条
/// 后台连接（本窗口自己的关闭在标题栏）。
fn connection_row(
    snapshot: ConnectionSnapshot,
    own_authority: String,
    multi: bool,
    settings_page: RwSignal<bool>,
    locale: RwSignal<Locale>,
) -> impl IntoView {
    let is_active = snapshot.authority == own_authority;
    let is_waiting = snapshot.status == ConnectionStatus::WaitingApproval;
    let glyph = status_glyph(snapshot.status);
    let name = connection_display_name(&snapshot);
    let meta = row_meta(&snapshot);
    let authority = snapshot.authority.clone();
    let key = snapshot.key.as_str().to_string();
    // 多连接时 `title` 就是那台 serve 的地址（行内是名字）；单连接时这一行是「改基址」
    // 的入口，`title` 要说清点下去去哪 —— 顺带给出用户唯一看得到的那份完整基址。
    let title = if multi {
        authority
    } else {
        i18n::connection_row_tooltip(locale.get_untracked(), &authority)
    };
    let close_key = key.clone();
    let closable = !is_active;

    view! {
        <div class="nav-rail-connection-item">
            <button
                type="button"
                class="nav-rail-connection-row"
                class:nav-rail-connection-row--active=is_active
                prop:title=title
                data-testid="nav-rail-connection-row"
                on:click=move |_| {
                    if multi {
                        tauri_focus_connection(&key);
                    } else {
                        navigate_to_settings(settings_page, SettingsSection::Connection);
                    }
                }
            >
                <span
                    class="nav-rail-connection-glyph"
                    class:nav-rail-connection-glyph--waiting=is_waiting
                    aria-hidden="true"
                >
                    {glyph}
                </span>
                <span class="nav-rail-connection-name">{name}</span>
                <span class="nav-rail-connection-meta">{meta}</span>
            </button>
            <Show when=move || closable>
                <button
                    type="button"
                    class="nav-rail-connection-action"
                    prop:title=i18n::ctl_row_close(locale.get_untracked())
                    data-testid="nav-rail-connection-close"
                    on:click={
                        // 每次渲染都克隆一份：`Show` 的 children 必须可重复调用（`Fn`），
                        // 直接把 `close_key` move 进点击闭包会退化成 `FnOnce`。
                        let key = close_key.clone();
                        move |_| tauri_close_connection(&key)
                    }
                >
                    "×"
                </button>
            </Show>
        </div>
    }
}

/// 一行「可连的实例」：名字 + 状态读数 +（就绪时）「连接」按钮。
fn ctl_row(row: CtlRow, locale: RwSignal<Locale>) -> impl IntoView {
    let CtlRow {
        name,
        api_base,
        state,
    } = row;
    let state_text = match &state {
        CtlRowState::Ready => api_base_authority(&api_base),
        CtlRowState::Starting => i18n::ctl_state_starting(locale.get_untracked()).to_string(),
        CtlRowState::Unavailable(reason) => {
            i18n::ctl_state_unavailable(locale.get_untracked(), reason)
        }
    };
    let is_starting = state == CtlRowState::Starting;
    let connectable = state == CtlRowState::Ready;
    let api_base_for_title = api_base.clone();
    let base_for_click = api_base;
    let name_for_click = name.clone();
    let name_for_title = name.clone();

    view! {
        <div class="nav-rail-ctl-row" prop:title=api_base_for_title>
            <span class="nav-rail-ctl-name" prop:title=name_for_title>{name}</span>
            <span
                class="nav-rail-ctl-state"
                class:nav-rail-ctl-state--starting=is_starting
            >
                {state_text}
            </span>
            <Show when=move || connectable>
                <button
                    type="button"
                    class="nav-rail-ctl-connect"
                    prop:title=i18n::ctl_row_connect(locale.get_untracked())
                    data-testid="nav-rail-ctl-connect"
                    on:click={
                        let base = base_for_click.clone();
                        let name = name_for_click.clone();
                        move |_| {
                            let base = base.clone();
                            let name = name.clone();
                            // 失败不弹错：地址不合法 / 建窗失败只落在壳日志，用户能再点一次。
                            leptos::task::spawn_local(async move {
                                let _ = crate::tauri_shell::tauri_open_connection(
                                    &base,
                                    Some(name.as_str()),
                                )
                                .await;
                            });
                        }
                    }
                >
                    {i18n::ctl_row_connect(locale.get_untracked())}
                </button>
            </Show>
        </div>
    }
}

/// 左栏连接区。非桌面壳（Android / Web）整段不渲染：那边没有窗口可以切。
#[component]
pub(super) fn NavRailConnections(
    locale: RwSignal<Locale>,
    settings_page: RwSignal<bool>,
) -> impl IntoView {
    let api_base = try_chat_shell_ctx()
        .map(|shell| shell.api_base)
        .unwrap_or_else(|| RwSignal::new(api_base_url()));
    let own_authority = StoredValue::new(api_base_authority(&api_base.get_untracked()));
    let connections = StoredValue::new(connection_feed(api_base.get_untracked()));
    let rows = Signal::derive(move || connections.get_value().get());
    // 控制面实例：只在桌面壳里才有来源；非桌面壳下壳没有这条命令，这份 signal 恒为空。
    let instances = StoredValue::new(ctl_instances_feed());
    let ctl_rows =
        Signal::derive(move || ctl_rows_for_rail(&instances.get_value().get(), &rows.get()));

    view! {
        <Show when=move || tauri_shell_available()>
            <Show when=move || !rows.get().is_empty()>
                <div class="nav-rail-connections" data-testid="nav-rail-connections">
                    <div class="nav-rail-connections-label">
                        // 条数与 TUI 的 `连接 [n/8]` 同一个位置、同一种写法。这里不带
                        // `/上限`：窗口数没有上限，那个数字在桌面壳里不存在。
                        {move || format!(
                            "{} [{}]",
                            i18n::connections_pane_label(locale.get()),
                            rows.get().len(),
                        )}
                    </div>
                    <For
                        each=move || rows.get()
                        key=|snapshot| snapshot.key.clone()
                        children=move |snapshot| {
                            let multi = rows.get().len() > 1;
                            connection_row(
                                snapshot,
                                own_authority.get_value(),
                                multi,
                                settings_page,
                                locale,
                            )
                        }
                    />
                    // 「可连实例」：控制面里声明过、还没连的那些。空则整组不渲染 ——
                    // 没跑 `crabmatectl` 是常态，不该常驻一块空标题。
                    <Show when=move || !ctl_rows.get().is_empty()>
                        <div class="nav-rail-ctl-group" data-testid="nav-rail-ctl-group">
                            <div class="nav-rail-ctl-head">
                                <span class="nav-rail-ctl-title">
                                    {i18n::ctl_group_label(locale.get())}
                                </span>
                                // 只读查询没有推送：`starting` 的实例不会自己变 `ready`。
                                <button
                                    type="button"
                                    class="nav-rail-ctl-refresh"
                                    prop:title=i18n::ctl_refresh_tooltip(locale.get_untracked())
                                    aria-label=i18n::ctl_refresh_tooltip(locale.get_untracked())
                                    data-testid="nav-rail-ctl-refresh"
                                    on:click=move |_| refresh_ctl_instances(instances.get_value())
                                >
                                    "⟳"
                                </button>
                            </div>
                            <For
                                each=move || ctl_rows.get()
                                key=|row| row.name.clone()
                                children=move |row| ctl_row(row, locale)
                            />
                        </div>
                    </Show>
                </div>
            </Show>
        </Show>
    }
}

#[cfg(test)]
mod tests {
    use super::{CtlRowState, ctl_rows_for_rail};
    use crabmate_client_api::connections::ConnectionSnapshot;
    use crabmate_client_api::ctl::CtlInstance;

    fn instance(name: &str, observed: &str, base: Option<&str>) -> CtlInstance {
        CtlInstance {
            name: name.to_string(),
            observed: Some(observed.to_string()),
            base_url: base.map(str::to_string),
            enabled: Some(true),
            ..CtlInstance::default()
        }
    }

    fn connected(base: &str) -> ConnectionSnapshot {
        ConnectionSnapshot::new("w", base)
    }

    #[test]
    fn a_ready_instance_with_a_base_becomes_connectable() {
        let rows = ctl_rows_for_rail(
            &[instance("prod", "ready", Some("http://127.0.0.1:8080"))],
            &[],
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "prod");
        assert_eq!(rows[0].state, CtlRowState::Ready);
    }

    #[test]
    fn a_ready_instance_without_a_base_is_not_connectable() {
        // `ready` 但没基址：开不了窗，给一个点不动的「连接」比说清原因更坏。
        let rows = ctl_rows_for_rail(&[instance("prod", "ready", None)], &[]);
        assert_eq!(rows[0].state, CtlRowState::Unavailable("ready".to_string()));
    }

    #[test]
    fn starting_is_pending_and_failed_carries_the_server_reason() {
        let mut failed = instance("bad", "failed", Some("http://127.0.0.1:8081"));
        failed.last_error = Some("port in use".to_string());
        let rows = ctl_rows_for_rail(&[instance("up", "starting", None), failed], &[]);
        assert_eq!(rows[0].state, CtlRowState::Starting);
        // 原因抄服务端的，不自己编。
        assert_eq!(
            rows[1].state,
            CtlRowState::Unavailable("port in use".to_string())
        );
    }

    #[test]
    fn stale_is_not_connectable_even_though_the_record_says_running() {
        // 判据是 `observed`，不是 `state`：`stale` 连过去只会打空端口。
        let mut stale = instance("stale", "stale", Some("http://127.0.0.1:8082"));
        stale.state = Some("running".to_string());
        let rows = ctl_rows_for_rail(&[stale], &[]);
        assert!(matches!(rows[0].state, CtlRowState::Unavailable(_)));
    }

    #[test]
    fn a_closed_declaration_is_hidden() {
        let mut closed = instance("off", "ready", Some("http://127.0.0.1:8083"));
        closed.enabled = Some(false);
        assert!(ctl_rows_for_rail(&[closed], &[]).is_empty());
    }

    #[test]
    fn an_instance_already_connected_is_not_listed_twice() {
        // 它在上半组（已连窗口）里；再画一次会让人以为有两条连接。
        let rows = ctl_rows_for_rail(
            &[instance("prod", "ready", Some("http://127.0.0.1:8080"))],
            &[connected("http://127.0.0.1:8080")],
        );
        assert!(rows.is_empty());
    }
}
