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

use leptos::prelude::*;

use crabmate_client_api::connections::{ConnectionSnapshot, ConnectionStatus};
use crabmate_client_api::url::api_base_authority;

use crate::api::api_base_url;
use crate::i18n::{self, Locale};
use crate::tauri_shell::{tauri_focus_connection, tauri_shell_available};

use super::connection_feed::{connection_display_name, connection_feed};
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

/// 一行。`multi` 为真时点击切到那个窗口；为假（只有一条连接）时点击进「设置 → 连接」。
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

    view! {
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
                </div>
            </Show>
        </Show>
    }
}
