//! 左栏「连接」区（桌面多窗口）：会话列表下方常驻，对齐 TUI 的左栏 lane 区。
//!
//! 与状态栈那个 chip（[`super::status_connection_chip`]）的分工：
//!
//! - chip 在**右列**、随状态栈一起被窄屏规则隐藏，单连接时是「进设置」的按钮；
//! - 这里在**左栏**、每窗口一行，单连接也显示 —— 位置与 TUI 的「左栏下半」同位。
//!
//! 数据面复用 [`connection_feed`]：那是应用级 signal（每个 WASM 实例一份），订阅只装一次，
//! 两个消费者读同一份名单。
//!
//! **这不是 TUI lane 的等价物。** TUI 点击是「同进程切换当前 lane 的状态树」（聊天区立刻
//! 换一份 transcript，无重载）；这里点击是 `focus_connection`，切到**另一个 WebView 窗口**
//! —— 换整个 JS 堆。窗口都铺满工作区，所以切过去看到的是那个窗口自己的左栏。
//! 单窗口内并排多 lane（`tui_multi_server.md` §9.4.2 路线 B）明确不做，本组件只做
//! 「导航 + 状态总览」。
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

use super::shell_runtime_context::try_chat_shell_ctx;
use super::status_connection_chip::{connection_chip_name, connection_feed};

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

/// 一行。`multi` 为假（只有一条连接）时禁用点击：此时没有「别的窗口」可切，
/// 而「进设置改基址」那条路由归右列 chip —— 两个入口不重复。
fn connection_row(
    snapshot: ConnectionSnapshot,
    own_authority: String,
    multi: bool,
) -> impl IntoView {
    let is_active = snapshot.authority == own_authority;
    let is_waiting = snapshot.status == ConnectionStatus::WaitingApproval;
    let glyph = status_glyph(snapshot.status);
    let name = connection_chip_name(&snapshot);
    let meta = row_meta(&snapshot);
    let title = snapshot.authority.clone();
    let key = snapshot.key.as_str().to_string();

    view! {
        <button
            type="button"
            class="nav-rail-connection-row"
            class:nav-rail-connection-row--active=is_active
            prop:disabled=!multi
            prop:title=title
            data-testid="nav-rail-connection-row"
            on:click=move |_| {
                if multi {
                    tauri_focus_connection(&key);
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
pub(super) fn NavRailConnections(locale: RwSignal<Locale>) -> impl IntoView {
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
                        {move || i18n::status_chip_connection(locale.get())}
                    </div>
                    <For
                        each=move || rows.get()
                        key=|snapshot| snapshot.key.clone()
                        children=move |snapshot| {
                            let multi = rows.get().len() > 1;
                            connection_row(snapshot, own_authority.get_value(), multi)
                        }
                    />
                </div>
            </Show>
        </Show>
    }
}
